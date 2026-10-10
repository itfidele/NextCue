//! Native capture and offline Whisper inference. Only `setup` accesses the network.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

const MODEL_FILE: &str = "ggml-base.bin";
const MODEL_BYTES: u64 = 147_951_465;
const MODEL_SHA: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";
const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.bin";
const RATE: usize = 16_000;
const CHUNK_SAMPLES: usize = RATE * 6;
const QUEUE_CAPACITY: usize = 3;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStatus {
    pub ready: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartListening {
    pub device: String,
    pub language: String,
    pub session_id: String,
    #[serde(default = "microphone_source")]
    pub source: String,
}

fn microphone_source() -> String {
    "microphone".into()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Transcript {
    session_id: String,
    text: String,
    source: String,
    time: u64,
    start_ms: u64,
    end_ms: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceError {
    session_id: String,
    error: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureStatus {
    session_id: String,
    listening: bool,
}

#[derive(Clone, Serialize)]
struct SetupProgress {
    progress: u8,
    status: String,
}

struct AudioChunk {
    epoch: u64,
    session_id: String,
    language: String,
    source: String,
    time: u64,
    duration_ms: u64,
    samples: Vec<f32>,
    app: AppHandle,
}

enum Work {
    Audio(AudioChunk),
    Drain(Sender<()>),
}

struct Worker {
    sender: SyncSender<Work>,
    _thread: thread::JoinHandle<()>,
}

struct CaptureSession {
    stop: Sender<()>,
    done: Receiver<Result<(), String>>,
    thread: thread::JoinHandle<()>,
}

pub struct VoiceService {
    model_dir: PathBuf,
    epoch: Arc<AtomicU64>,
    operation: Mutex<()>,
    capture: Mutex<Option<CaptureSession>>,
    worker: Mutex<Option<Worker>>,
    setup_running: AtomicBool,
    verified_model: AtomicBool,
}

impl VoiceService {
    pub fn new(model_dir: PathBuf) -> Self {
        Self {
            model_dir,
            epoch: Arc::new(AtomicU64::new(0)),
            operation: Mutex::new(()),
            capture: Mutex::new(None),
            worker: Mutex::new(None),
            setup_running: AtomicBool::new(false),
            verified_model: AtomicBool::new(false),
        }
    }

    pub fn status(&self) -> VoiceStatus {
        let result = self.check_model();
        VoiceStatus {
            ready: result.is_ok(),
            error: result.err(),
        }
    }

    fn check_model(&self) -> Result<(), String> {
        let path = self.model_dir.join(MODEL_FILE);
        let metadata = fs::metadata(&path).map_err(|_| {
            "Set up local transcription to download the Whisper base model once (148 MB)."
                .to_string()
        })?;
        if metadata.len() != MODEL_BYTES {
            return Err("The local voice model is incomplete. Run voice setup again.".into());
        }
        // Verify every process launch, then reuse the result while the model has the expected size.
        if !self.verified_model.load(Ordering::Acquire) {
            verify_model(&path)?;
            self.verified_model.store(true, Ordering::Release);
        }
        Ok(())
    }

    pub async fn setup(&self, app: AppHandle) -> Result<(), String> {
        if self
            .setup_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("Voice setup is already running.".into());
        }
        let _reset = ResetFlag(&self.setup_running);
        if self.status().ready {
            progress(&app, 100, "Local transcription is ready");
            return Ok(());
        }
        tokio::fs::create_dir_all(&self.model_dir)
            .await
            .map_err(|e| format!("Cannot create voice model folder: {e}"))?;
        let temporary = self.model_dir.join(format!("{MODEL_FILE}.download"));
        progress(&app, 0, "Downloading local Whisper model (148 MB)");
        let result = async {
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(900))
                .build()
                .map_err(|e| e.to_string())?;
            let mut response = client
                .get(MODEL_URL)
                .send()
                .await
                .map_err(|e| format!("Voice model download failed: {e}"))?
                .error_for_status()
                .map_err(|e| format!("Voice model download failed: {e}"))?;
            let mut file = tokio::fs::File::create(&temporary)
                .await
                .map_err(|e| e.to_string())?;
            let mut digest = Sha256::new();
            let mut bytes = 0_u64;
            let mut last_percent = 0_u8;
            while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
                bytes += chunk.len() as u64;
                if bytes > MODEL_BYTES {
                    return Err("Voice model download exceeded the expected size.".into());
                }
                digest.update(&chunk);
                file.write_all(&chunk).await.map_err(|e| e.to_string())?;
                let percent = ((bytes * 98 / MODEL_BYTES).min(98)) as u8;
                if percent != last_percent {
                    progress(&app, percent, "Downloading local Whisper model (148 MB)");
                    last_percent = percent;
                }
            }
            file.flush().await.map_err(|e| e.to_string())?;
            file.sync_all().await.map_err(|e| e.to_string())?;
            drop(file);
            if bytes != MODEL_BYTES || format!("{:x}", digest.finalize()) != MODEL_SHA {
                return Err("Voice model integrity check failed. Please run setup again.".into());
            }
            progress(&app, 99, "Verifying the local model");
            let check_path = temporary.clone();
            tokio::task::spawn_blocking(move || load_context(&check_path).map(|_| ()))
                .await
                .map_err(|e| e.to_string())??;
            tokio::fs::rename(&temporary, self.model_dir.join(MODEL_FILE))
                .await
                .map_err(|e| e.to_string())?;
            self.verified_model.store(true, Ordering::Release);
            progress(&app, 100, "Local transcription is ready");
            Ok(())
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&temporary).await;
        }
        result
    }

    pub fn list_devices(&self) -> Result<Vec<InputDevice>, String> {
        let host = cpal::default_host();
        let default_name = host.default_input_device().and_then(|d| d.name().ok());
        let mut inputs = vec![InputDevice {
            id: "default".into(),
            name: default_name
                .as_ref()
                .map(|n| format!("Default · {n}"))
                .unwrap_or_else(|| "Default microphone".into()),
            is_default: true,
        }];
        for (index, device) in host.input_devices().map_err(|e| e.to_string())?.enumerate() {
            if let Ok(name) = device.name() {
                inputs.push(InputDevice {
                    // CPAL does not expose portable hardware UIDs. The index disambiguates
                    // identical device names; re-check the name when reopening the input.
                    id: format!("input:{index}:{name}"),
                    is_default: default_name.as_ref() == Some(&name),
                    name,
                });
            }
        }
        #[cfg(target_os = "macos")]
        inputs.push(InputDevice {
            id: "system".into(),
            name: "Mac system audio".into(),
            is_default: false,
        });
        Ok(inputs)
    }

    pub fn start(&self, app: AppHandle, request: StartListening) -> Result<(), String> {
        validate_request(&request)?;
        let _operation = self.operation.lock().map_err(|e| e.to_string())?;
        self.check_model()?;
        // Cancel earlier inference before flushing/releasing an earlier capture session.
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.stop_capture(Instant::now() + Duration::from_secs(15))?;
        let sender = self.ensure_worker()?;
        let (stop_tx, stop_rx) = mpsc::channel();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (done_tx, done_rx) = mpsc::channel();
        let current = self.epoch.clone();
        let handle = thread::Builder::new()
            .name("nextcue-audio-capture".into())
            .spawn(move || {
                let callback_current = current.clone();
                let result = capture_audio(
                    app.clone(),
                    request.clone(),
                    epoch,
                    current,
                    sender,
                    stop_rx,
                    started_tx,
                );
                let completed = result.clone();
                if callback_current.load(Ordering::Acquire) == epoch {
                    if let Err(error) = result {
                        emit_error(&app, &request.session_id, error);
                    }
                    let _ = app.emit(
                        "voice-status",
                        CaptureStatus {
                            session_id: request.session_id,
                            listening: false,
                        },
                    );
                }
                let _ = done_tx.send(completed);
            })
            .map_err(|e| format!("Cannot start audio capture: {e}"))?;
        match started_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(())) => {
                *self.capture.lock().map_err(|e| e.to_string())? = Some(CaptureSession {
                    stop: stop_tx,
                    done: done_rx,
                    thread: handle,
                });
                Ok(())
            }
            Ok(Err(error)) => {
                let _ = handle.join();
                Err(error)
            }
            Err(_) => {
                let _ = self.epoch.compare_exchange(
                    epoch,
                    epoch + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                let _ = stop_tx.send(());
                // Do not block the UI on a stalled native permission request.
                drop(handle);
                Err(
                    "Audio capture did not start. Check microphone permission and try again."
                        .into(),
                )
            }
        }
    }

    pub fn stop(&self) -> Result<(), String> {
        let _operation = self.operation.lock().map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(60);
        let epoch = self.epoch.load(Ordering::Acquire);
        if let Err(error) = self.stop_capture(deadline) {
            let _ =
                self.epoch
                    .compare_exchange(epoch, epoch + 1, Ordering::AcqRel, Ordering::Acquire);
            return Err(error);
        }
        // A FIFO barrier ensures the final segment is delivered before stop resolves.
        if let Some(worker) = self.worker.lock().map_err(|e| e.to_string())?.as_ref() {
            let (done_tx, done_rx) = mpsc::channel();
            let result =
                send_until(&worker.sender, Work::Drain(done_tx), deadline).and_then(|_| {
                    done_rx
                        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        .map_err(|_| "Local transcription took too long to finish.".to_string())
                });
            if let Err(error) = result {
                let _ = self.epoch.compare_exchange(
                    epoch,
                    epoch + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                return Err(error);
            }
        }
        Ok(())
    }

    /// App exit cancels queued inference immediately, without waiting on a model or OS dialog.
    pub fn shutdown(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut capture) = self.capture.lock() {
            if let Some(capture) = capture.take() {
                let _ = capture.stop.send(());
            }
        }
    }

    fn stop_capture(&self, deadline: Instant) -> Result<(), String> {
        let capture = self.capture.lock().map_err(|e| e.to_string())?.take();
        if let Some(capture) = capture {
            let _ = capture.stop.send(());
            let result = capture
                .done
                .recv_timeout(deadline.saturating_duration_since(Instant::now()));
            match result {
                Ok(result) => {
                    capture
                        .thread
                        .join()
                        .map_err(|_| "Audio capture worker stopped unexpectedly.".to_string())?;
                    result?;
                }
                Err(_) => {
                    return Err(
                        "Audio capture took too long to stop. Please restart listening.".into(),
                    )
                }
            }
        }
        Ok(())
    }

    fn ensure_worker(&self) -> Result<SyncSender<Work>, String> {
        let mut worker = self.worker.lock().map_err(|e| e.to_string())?;
        if let Some(existing) = worker.as_ref() {
            if !existing._thread.is_finished() {
                return Ok(existing.sender.clone());
            }
        }
        let model_path = self.model_dir.join(MODEL_FILE);
        let epoch = self.epoch.clone();
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let handle = thread::Builder::new()
            .name("nextcue-local-whisper".into())
            .spawn(move || inference_worker(&model_path, epoch, receiver, ready_tx))
            .map_err(|e| e.to_string())?;
        ready_rx
            .recv_timeout(Duration::from_secs(30))
            .map_err(|_| "Local voice model took too long to load.".to_string())??;
        *worker = Some(Worker {
            sender: sender.clone(),
            _thread: handle,
        });
        Ok(sender)
    }
}

impl Drop for VoiceService {
    fn drop(&mut self) {
        self.shutdown();
        // Dropping the sender closes the persistent inference worker after the queue drains.
        let _ = self.worker.get_mut().map(|worker| worker.take());
    }
}

struct ResetFlag<'a>(&'a AtomicBool);
impl Drop for ResetFlag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn progress(app: &AppHandle, percent: u8, message: &str) {
    let _ = app.emit(
        "voice-progress",
        SetupProgress {
            progress: percent,
            status: message.into(),
        },
    );
}

fn emit_error(app: &AppHandle, session: &str, error: String) {
    let _ = app.emit(
        "voice-error",
        VoiceError {
            session_id: session.into(),
            error,
        },
    );
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn send_until(sender: &SyncSender<Work>, mut work: Work, deadline: Instant) -> Result<(), String> {
    loop {
        match sender.try_send(work) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Disconnected(_)) => {
                return Err("Local transcription worker stopped.".into())
            }
            Err(TrySendError::Full(item)) => {
                work = item;
            }
        }
        if Instant::now() >= deadline {
            return Err("Local transcription took too long to finish.".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn validate_request(request: &StartListening) -> Result<(), String> {
    if request.session_id.is_empty() || request.session_id.len() > 200 {
        return Err("A valid interview session is required to start listening.".into());
    }
    if request.device.len() > 512 {
        return Err("Invalid audio input device.".into());
    }
    if !matches!(
        request.language.as_str(),
        "auto"
            | "en"
            | "es"
            | "fr"
            | "de"
            | "it"
            | "pt"
            | "ja"
            | "zh"
            | "ko"
            | "hi"
            | "ar"
            | "ru"
            | "nl"
    ) {
        return Err("Choose a supported transcription language.".into());
    }
    if !matches!(request.source.as_str(), "microphone" | "system") {
        return Err("Choose microphone or system audio.".into());
    }
    if request.source == "microphone" && request.device == "system" {
        return Err(
            "Choose a microphone input, or switch the audio source to system audio.".into(),
        );
    }
    if request.source == "system" && !cfg!(target_os = "macos") && !is_loopback(&request.device) {
        return Err("Select a virtual audio input such as BlackHole or Loopback for system audio, then route your call audio to it.".into());
    }
    Ok(())
}

fn is_loopback(device: &str) -> bool {
    let name = device.to_lowercase();
    [
        "blackhole",
        "loopback",
        "soundflower",
        "vb-audio",
        "cable output",
        "stereo mix",
        "monitor of",
    ]
    .iter()
    .any(|name_part| name.contains(name_part))
}

fn verify_model(path: &Path) -> Result<(), String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if format!("{:x}", hash.finalize()) != MODEL_SHA {
        return Err("The local voice model is damaged. Run voice setup again.".into());
    }
    Ok(())
}

fn load_context(path: &Path) -> Result<WhisperContext, String> {
    // The native debug build otherwise prints every recognized token to stderr.
    // No logging backend is enabled; surfaced errors use the app's explicit events.
    whisper_rs::install_logging_hooks();
    let mut params = WhisperContextParameters::default();
    params.use_gpu(false);
    WhisperContext::new_with_params(path.to_str().ok_or("Invalid model path")?, params)
        .map_err(|e| format!("Cannot load the local voice model: {e}"))
}

fn inference_worker(
    path: &Path,
    current: Arc<AtomicU64>,
    receiver: Receiver<Work>,
    ready: SyncSender<Result<(), String>>,
) {
    let context = match load_context(path) {
        Ok(context) => context,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let mut state = match context.create_state() {
        Ok(state) => state,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    while let Ok(work) = receiver.recv() {
        let chunk = match work {
            Work::Drain(done) => {
                let _ = done.send(());
                continue;
            }
            Work::Audio(chunk) => chunk,
        };
        if current.load(Ordering::Acquire) != chunk.epoch || !has_speech(&chunk.samples) {
            continue;
        }
        let segments = match decode_samples(
            &mut state,
            &chunk.samples,
            &chunk.language,
            current.clone(),
            chunk.epoch,
            chunk.duration_ms,
        ) {
            Ok(segments) => segments,
            Err(error) => {
                if current.load(Ordering::Acquire) == chunk.epoch {
                    emit_error(&chunk.app, &chunk.session_id, error);
                }
                continue;
            }
        };
        for segment in segments {
            if current.load(Ordering::Acquire) != chunk.epoch {
                break;
            }
            let start_ms = chunk.time.saturating_add(segment.start_ms);
            let end_ms = chunk.time.saturating_add(segment.end_ms);
            let _ = chunk.app.emit(
                "transcript",
                Transcript {
                    session_id: chunk.session_id.clone(),
                    text: segment.text,
                    source: chunk.source.clone(),
                    time: start_ms,
                    start_ms,
                    end_ms,
                },
            );
        }
    }
}

fn decode_samples(
    state: &mut WhisperState,
    samples: &[f32],
    language: &str,
    current: Arc<AtomicU64>,
    epoch: u64,
    duration_ms: u64,
) -> Result<Vec<DecodedSegment>, String> {
    if current.load(Ordering::Acquire) != epoch || !has_speech(samples) {
        return Ok(Vec::new());
    }
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(thread::available_parallelism().map_or(4, |n| n.get().min(4)) as i32);
    params.set_language(if language == "auto" {
        None
    } else {
        Some(language)
    });
    params.set_translate(false);
    params.set_no_context(true);
    params.set_no_timestamps(false);
    params.set_print_realtime(false);
    params.set_print_progress(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    params.set_suppress_nst(true);
    params.set_max_tokens(128);
    // whisper-rs 0.15.1's closure wrapper erases its userdata type incorrectly.
    // Keep one concrete, stable context alive throughout this synchronous FFI call.
    let mut cancellation = Box::new(AbortContext {
        current: current.clone(),
        epoch,
    });
    unsafe {
        params.set_abort_callback(Some(abort_inference));
        params.set_abort_callback_user_data((&mut *cancellation as *mut AbortContext).cast());
    }
    state
        .full(params, samples)
        .map_err(|e| format!("Local transcription failed: {e}"))?;
    if current.load(Ordering::Acquire) != epoch {
        return Ok(Vec::new());
    }
    let mut segments = Vec::new();
    for segment in state.as_iter() {
        if segment.no_speech_probability() < 0.6 {
            if let Ok(words) = segment.to_str_lossy() {
                if let Some(segment) = timed_segment(
                    &words,
                    segment.start_timestamp(),
                    segment.end_timestamp(),
                    duration_ms,
                ) {
                    segments.push(segment);
                }
            }
        }
    }
    Ok(segments)
}

#[derive(Debug)]
struct DecodedSegment {
    text: String,
    // Millisecond offsets within the captured chunk, before conversion to epoch time.
    start_ms: u64,
    end_ms: u64,
}

fn timed_segment(
    text: &str,
    start_centiseconds: i64,
    end_centiseconds: i64,
    duration_ms: u64,
) -> Option<DecodedSegment> {
    let text = text.trim();
    // Whisper timestamps use 10 ms units. Clamp to captured PCM, excluding padding
    // added to help the model recognize a short final utterance.
    let offset_ms = |centiseconds: i64| {
        (centiseconds.max(0) as u64)
            .saturating_mul(10)
            .min(duration_ms)
    };
    let start_ms = offset_ms(start_centiseconds);
    let end_ms = offset_ms(end_centiseconds);
    if text.is_empty() || end_ms <= start_ms {
        return None;
    }
    Some(DecodedSegment {
        text: text.into(),
        start_ms,
        end_ms,
    })
}

struct AbortContext {
    current: Arc<AtomicU64>,
    epoch: u64,
}

unsafe extern "C" fn abort_inference(pointer: *mut std::ffi::c_void) -> bool {
    if pointer.is_null() {
        return true;
    }
    // The owner above retains this Box until Whisper finishes every worker thread.
    let context = unsafe { &*pointer.cast::<AbortContext>() };
    context.current.load(Ordering::Acquire) != context.epoch
}

fn capture_audio(
    app: AppHandle,
    request: StartListening,
    epoch: u64,
    current: Arc<AtomicU64>,
    sender: SyncSender<Work>,
    stop: Receiver<()>,
    started: SyncSender<Result<(), String>>,
) -> Result<(), String> {
    if current.load(Ordering::Acquire) != epoch || stop.try_recv() != Err(mpsc::TryRecvError::Empty)
    {
        let _ = started.send(Err("Audio startup was cancelled.".into()));
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    if request.source == "system" && !is_loopback(&request.device) {
        return mac_system_audio::capture(app, request, epoch, current, sender, stop, started);
    }
    let result = build_capture(&app, &request, epoch, current.clone(), sender.clone());
    let (stream, chunker, failed, last_packet) = match result {
        Ok(parts) => parts,
        Err(error) => {
            let _ = started.send(Err(error.clone()));
            return Err(error);
        }
    };
    if current.load(Ordering::Acquire) != epoch || stop.try_recv() != Err(mpsc::TryRecvError::Empty)
    {
        let _ = started.send(Err("Audio startup was cancelled.".into()));
        return Ok(());
    }
    if let Err(error) = stream.play() {
        let message = format!("Cannot start audio input. Check microphone permission: {error}");
        let _ = started.send(Err(message.clone()));
        return Err(message);
    }
    let _ = started.send(Ok(()));
    let _ = app.emit(
        "voice-status",
        CaptureStatus {
            session_id: request.session_id.clone(),
            listening: true,
        },
    );
    while stop.recv_timeout(Duration::from_millis(100)) == Err(mpsc::RecvTimeoutError::Timeout) {
        if failed.load(Ordering::Acquire) || current.load(Ordering::Acquire) != epoch {
            break;
        }
        if now_ms().saturating_sub(last_packet.load(Ordering::Acquire)) > 8_000 {
            emit_error(
                &app,
                &request.session_id,
                "No audio is arriving. Check the input device and microphone permission.".into(),
            );
            break;
        }
    }
    // Stream teardown occurs on its owning thread, including platforms where Stream is !Send.
    drop(stream);
    if let Ok(mut buffer) = chunker.lock() {
        if let Some(audio) = buffer.flush() {
            if current.load(Ordering::Acquire) == epoch {
                send_until(
                    &sender,
                    Work::Audio(AudioChunk {
                        epoch,
                        session_id: request.session_id,
                        language: request.language,
                        source: request.source,
                        time: audio.time,
                        duration_ms: audio.duration_ms,
                        samples: audio.samples,
                        app,
                    }),
                    Instant::now() + Duration::from_secs(10),
                )?;
            }
        }
    }
    Ok(())
}

type CaptureParts = (
    cpal::Stream,
    Arc<Mutex<Chunker>>,
    Arc<AtomicBool>,
    Arc<AtomicU64>,
);
fn build_capture(
    app: &AppHandle,
    request: &StartListening,
    epoch: u64,
    current: Arc<AtomicU64>,
    sender: SyncSender<Work>,
) -> Result<CaptureParts, String> {
    let host = cpal::default_host();
    let device = if request.device.is_empty() || request.device == "default" {
        host.default_input_device()
            .ok_or("No microphone was found.")?
    } else if let Some(selection) = request.device.strip_prefix("input:") {
        let (index, name) = selection
            .split_once(':')
            .ok_or("Invalid audio input selection.")?;
        let index = index
            .parse::<usize>()
            .map_err(|_| "Invalid audio input selection.")?;
        let device = host
            .input_devices()
            .map_err(|e| e.to_string())?
            .nth(index)
            .ok_or("The selected audio input is no longer connected.")?;
        if device.name().ok().as_deref() != Some(name) {
            return Err("Audio devices changed. Select the input again.".into());
        }
        device
    } else {
        host.input_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.name().ok().as_deref() == Some(&request.device))
            .ok_or("The selected audio input is no longer connected.")?
    };
    let supported = device
        .default_input_config()
        .map_err(|e| format!("Cannot open audio input: {e}"))?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let chunker = Arc::new(Mutex::new(Chunker::new(config.sample_rate.0)));
    let failed = Arc::new(AtomicBool::new(false));
    let last_packet = Arc::new(AtomicU64::new(now_ms()));
    let stream = match sample_format {
        cpal::SampleFormat::F32 => typed_stream::<f32>(
            &device,
            &config,
            chunker.clone(),
            app.clone(),
            request.clone(),
            epoch,
            current,
            sender,
            failed.clone(),
            last_packet.clone(),
        ),
        cpal::SampleFormat::I16 => typed_stream::<i16>(
            &device,
            &config,
            chunker.clone(),
            app.clone(),
            request.clone(),
            epoch,
            current,
            sender,
            failed.clone(),
            last_packet.clone(),
        ),
        cpal::SampleFormat::U16 => typed_stream::<u16>(
            &device,
            &config,
            chunker.clone(),
            app.clone(),
            request.clone(),
            epoch,
            current,
            sender,
            failed.clone(),
            last_packet.clone(),
        ),
        cpal::SampleFormat::I32 => typed_stream::<i32>(
            &device,
            &config,
            chunker.clone(),
            app.clone(),
            request.clone(),
            epoch,
            current,
            sender,
            failed.clone(),
            last_packet.clone(),
        ),
        _ => {
            return Err(
                "This audio input uses an unsupported sample format. Choose another input.".into(),
            )
        }
    }
    .map_err(|e| format!("Cannot capture audio. Check microphone permission: {e}"))?;
    Ok((stream, chunker, failed, last_packet))
}

#[allow(clippy::too_many_arguments)]
fn typed_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    chunker: Arc<Mutex<Chunker>>,
    app: AppHandle,
    request: StartListening,
    epoch: u64,
    current: Arc<AtomicU64>,
    sender: SyncSender<Work>,
    failed: Arc<AtomicBool>,
    last_packet: Arc<AtomicU64>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    use cpal::Sample;
    let channels = config.channels as usize;
    let source_rate = config.sample_rate.0 as u64;
    let error_app = app.clone();
    let error_session = request.session_id.clone();
    let error_epoch = current.clone();
    let output_failed = failed.clone();
    let mut last_level = Instant::now();
    let mut queue_warned = false;
    device.build_input_stream(config, move |data: &[T], info| {
        let received_ms = now_ms();
        last_packet.store(received_ms, Ordering::Release);
        if current.load(Ordering::Acquire) != epoch {return;}
        let Ok(mut buffer) = chunker.try_lock() else {return;};
        let mut energy = 0_f32;
        let frames = data.len() / channels;
        let timestamp = info.timestamp();
        let capture_delay = timestamp.callback.duration_since(&timestamp.capture)
            .map(|delay| delay.as_millis() as u64)
            .unwrap_or(frames as u64 * 1_000 / source_rate);
        let packet_start = received_ms.saturating_sub(capture_delay);
        for (index, frame) in data.chunks_exact(channels).enumerate() {
            let mono = frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32;
            energy += mono * mono;
            let sample_time = packet_start.saturating_add(index as u64 * 1_000 / source_rate);
            if let Some(audio) = buffer.push(mono, sample_time) {
                let chunk = AudioChunk {epoch, session_id: request.session_id.clone(), language: request.language.clone(), source: request.source.clone(), time: audio.time, duration_ms: audio.duration_ms, samples: audio.samples, app: app.clone()};
                match sender.try_send(Work::Audio(chunk)) {
                    Ok(()) => queue_warned = false,
                    Err(TrySendError::Full(_)) => {
                        if !queue_warned {emit_error(&app, &request.session_id, "Transcription is falling behind. A short audio segment was skipped; try pausing other apps.".into()); queue_warned = true;}
                    }
                    Err(TrySendError::Disconnected(_)) => {output_failed.store(true, Ordering::Release); emit_error(&app, &request.session_id, "Local transcription worker stopped. Restart listening.".into()); return;}
                }
            }
        }
        if last_level.elapsed() > Duration::from_millis(500) && frames > 0 {
            let level = (energy / frames as f32).sqrt().min(1.0);
            let _ = app.emit("voice-level", serde_json::json!({"sessionId": request.session_id, "level": level}));
            last_level = Instant::now();
        }
    }, move |error| {
        failed.store(true, Ordering::Release);
        if error_epoch.load(Ordering::Acquire) == epoch {emit_error(&error_app, &error_session, format!("Audio input stopped: {error}"));}
    }, None)
}

/// PCM retains its capture start and duration even when inference is queued or PCM is padded.
struct CapturedAudio {
    time: u64,
    duration_ms: u64,
    samples: Vec<f32>,
}

/// Continuous resampling keeps phase across callbacks; capture buffers stay below six seconds.
struct Chunker {
    source_rate: f64,
    index: u64,
    next: f64,
    previous: f32,
    samples: Vec<f32>,
    trailing_silence: usize,
    start_ms: Option<u64>,
}

impl Chunker {
    fn new(source_rate: u32) -> Self {
        Self {
            source_rate: source_rate.max(1) as f64,
            index: 0,
            next: 0.0,
            previous: 0.0,
            samples: Vec::with_capacity(CHUNK_SAMPLES),
            trailing_silence: 0,
            start_ms: None,
        }
    }

    fn push(&mut self, sample: f32, time: u64) -> Option<CapturedAudio> {
        let sample = if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let mut finished = None;
        while self.next <= self.index as f64 {
            let value = if self.index == 0 {
                sample
            } else {
                let fraction = (self.next - (self.index - 1) as f64).clamp(0.0, 1.0) as f32;
                self.previous + (sample - self.previous) * fraction
            };
            self.start_ms.get_or_insert(time);
            self.samples.push(value);
            self.trailing_silence = if value.abs() < 0.006 {
                self.trailing_silence + 1
            } else {
                0
            };
            self.next += self.source_rate / RATE as f64;
            if self.samples.len() >= CHUNK_SAMPLES
                || (self.samples.len() >= RATE * 2 && self.trailing_silence >= RATE * 4 / 5)
            {
                if let Some(chunk) = self.flush() {
                    finished = Some(chunk);
                }
            }
        }
        self.previous = sample;
        self.index += 1;
        finished
    }

    fn flush(&mut self) -> Option<CapturedAudio> {
        self.trailing_silence = 0;
        let time = self.start_ms.take()?;
        let mut samples = std::mem::replace(&mut self.samples, Vec::with_capacity(CHUNK_SAMPLES));
        if samples.len() < RATE / 4 || !has_speech(&samples) {
            return None;
        }
        let duration_ms = (samples.len() as u64 * 1_000).div_ceil(RATE as u64);
        // Whisper handles a final short utterance better with at least one second of PCM.
        if samples.len() < RATE {
            samples.resize(RATE, 0.0);
        }
        Some(CapturedAudio {
            time,
            duration_ms,
            samples,
        })
    }
}

fn has_speech(samples: &[f32]) -> bool {
    if samples.is_empty() {
        return false;
    }
    let energy = samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32;
    let voiced = samples.iter().filter(|x| x.abs() > 0.008).count();
    energy.sqrt() > 0.002 && voiced > RATE / 50
}

#[cfg(target_os = "macos")]
mod mac_system_audio {
    use super::*;
    use screencapturekit::{prelude::*, stream::ErrorHandler};

    // CoreMedia buffers are borrowed native objects. Copy PCM before returning the callback.
    fn pcm_mono(sample: &CMSampleBuffer) -> Option<(Vec<f32>, u32)> {
        let format = sample.format_description()?;
        if !format.is_pcm() || !format.audio_is_float() || format.audio_bits_per_channel()? != 32 {
            return None;
        }
        let rate = format.audio_sample_rate()? as u32;
        if !(8_000..=192_000).contains(&rate) {
            return None;
        }
        let list = sample.audio_buffer_list().ok()?;
        let buffers: Vec<_> = list.iter().collect();
        let channels: usize = buffers.iter().map(|b| b.number_channels as usize).sum();
        if channels == 0 {
            return None;
        }
        let frames = buffers
            .iter()
            .map(|b| {
                let channels = b.number_channels as usize;
                if channels == 0 {
                    0
                } else {
                    b.data().len() / (4 * channels)
                }
            })
            .min()?;
        if frames > rate as usize * 2 {
            return None;
        }
        let mut mono = Vec::with_capacity(frames);
        for frame in 0..frames {
            let mut sum = 0.0_f32;
            for buffer in &buffers {
                for channel in 0..buffer.number_channels as usize {
                    let offset = (frame * buffer.number_channels as usize + channel) * 4;
                    let bytes: [u8; 4] = buffer.data()[offset..offset + 4].try_into().ok()?;
                    let value = if format.audio_is_big_endian() {
                        f32::from_be_bytes(bytes)
                    } else {
                        f32::from_le_bytes(bytes)
                    };
                    if value.is_finite() {
                        sum += value;
                    }
                }
            }
            mono.push(sum / channels as f32);
        }
        Some((mono, rate))
    }

    pub(super) fn capture(
        app: AppHandle,
        request: StartListening,
        epoch: u64,
        current: Arc<AtomicU64>,
        sender: SyncSender<Work>,
        stop: Receiver<()>,
        started: SyncSender<Result<(), String>>,
    ) -> Result<(), String> {
        let chunker = Arc::new(Mutex::new(Chunker::new(48_000)));
        let failed = Arc::new(AtomicBool::new(false));
        let last_packet = Arc::new(AtomicU64::new(now_ms()));
        let create_stream = || -> Result<SCStream, String> {
            let content = SCShareableContent::get().map_err(|e| format!("System audio needs Screen & System Audio Recording permission in macOS System Settings: {e}"))?;
            let display = content
                .displays()
                .into_iter()
                .next()
                .ok_or("No Mac display is available for system audio capture.")?;
            let filter = SCContentFilter::create()
                .with_display(&display)
                .with_excluding_windows(&[])
                .build()
                .map_err(|e| e.to_string())?;
            // Only register an Audio output; no screenshots or video frames are read or saved.
            let configuration = SCStreamConfiguration::new()
                .with_width(2)
                .with_height(2)
                .with_minimum_frame_interval(&CMTime::new(1, 1))
                .with_captures_audio(true)
                .with_excludes_current_process_audio(true)
                .with_sample_rate(48_000)
                .with_channel_count(1);
            let error_app = app.clone();
            let error_request = request.clone();
            let error_current = current.clone();
            let error_failed = failed.clone();
            let delegate = ErrorHandler::new(move |error| {
                error_failed.store(true, Ordering::Release);
                if error_current.load(Ordering::Acquire) == epoch {
                    emit_error(
                        &error_app,
                        &error_request.session_id,
                        format!("System audio capture stopped: {error}"),
                    );
                }
            });
            let mut stream = SCStream::new_with_delegate(&filter, &configuration, delegate)
                .map_err(|e| e.to_string())?;
            let output_app = app.clone();
            let output_request = request.clone();
            let output_current = current.clone();
            let output_sender = sender.clone();
            let output_chunker = chunker.clone();
            let output_failed = failed.clone();
            let output_packet = last_packet.clone();
            let last_level = Mutex::new(Instant::now());
            let warned = AtomicBool::new(false);
            let decode_failures = AtomicU64::new(0);
            stream.add_output_handler(move |sample: CMSampleBuffer, kind| {
                if kind != SCStreamOutputType::Audio || output_current.load(Ordering::Acquire) != epoch {return;}
                let Some((mono, rate)) = pcm_mono(&sample) else {
                    if decode_failures.fetch_add(1, Ordering::AcqRel) == 4 {
                        output_failed.store(true, Ordering::Release);
                        emit_error(&output_app, &output_request.session_id, "System audio arrived in an unsupported format. Restart listening or select a virtual audio input.".into());
                    }
                    return;
                };
                if mono.is_empty() {return;}
                decode_failures.store(0, Ordering::Release);
                let received_ms = now_ms();
                output_packet.store(received_ms, Ordering::Release);
                let Ok(mut buffer) = output_chunker.try_lock() else {return;};
                if buffer.source_rate as u32 != rate {*buffer = Chunker::new(rate);}
                let energy = mono.iter().map(|value| value * value).sum::<f32>() / mono.len() as f32;
                let packet_start = received_ms.saturating_sub(mono.len() as u64 * 1_000 / rate as u64);
                for (index, value) in mono.into_iter().enumerate() {
                    let sample_time = packet_start.saturating_add(index as u64 * 1_000 / rate as u64);
                    if let Some(audio) = buffer.push(value, sample_time) {
                        let chunk = AudioChunk {epoch, session_id: output_request.session_id.clone(), language: output_request.language.clone(), source: "system".into(), time: audio.time, duration_ms: audio.duration_ms, samples: audio.samples, app: output_app.clone()};
                        match output_sender.try_send(Work::Audio(chunk)) {
                            Ok(()) => {warned.store(false, Ordering::Release);}
                            Err(TrySendError::Full(_)) => {
                                if !warned.swap(true, Ordering::AcqRel) {emit_error(&output_app, &output_request.session_id, "Transcription is falling behind. A short audio segment was skipped; try pausing other apps.".into());}
                            }
                            Err(TrySendError::Disconnected(_)) => {output_failed.store(true, Ordering::Release); emit_error(&output_app, &output_request.session_id, "Local transcription worker stopped. Restart listening.".into()); return;}
                        }
                    }
                }
                if let Ok(mut last) = last_level.try_lock() {
                    if last.elapsed() > Duration::from_millis(500) {
                        let _ = output_app.emit("voice-level", serde_json::json!({"sessionId": output_request.session_id, "level": energy.sqrt().min(1.0)}));
                        *last = Instant::now();
                    }
                }
            }, SCStreamOutputType::Audio).map_err(|e| e.to_string())?;
            if current.load(Ordering::Acquire) != epoch
                || stop.try_recv() != Err(mpsc::TryRecvError::Empty)
            {
                return Err("Audio startup was cancelled.".into());
            }
            stream.start_capture().map_err(|e| format!("Cannot capture system audio. Allow Screen & System Audio Recording in macOS System Settings and restart NextCue: {e}"))?;
            Ok(stream)
        };
        let stream = match create_stream() {
            Ok(stream) => stream,
            Err(error) => {
                let _ = started.send(Err(error.clone()));
                return Err(error);
            }
        };
        if current.load(Ordering::Acquire) != epoch
            || stop.try_recv() != Err(mpsc::TryRecvError::Empty)
        {
            let _ = stream.stop_capture();
            let _ = started.send(Err("Audio startup was cancelled.".into()));
            return Ok(());
        }
        let _ = started.send(Ok(()));
        let _ = app.emit(
            "voice-status",
            CaptureStatus {
                session_id: request.session_id.clone(),
                listening: true,
            },
        );
        let mut no_packet_warning = false;
        while stop.recv_timeout(Duration::from_millis(100)) == Err(mpsc::RecvTimeoutError::Timeout)
        {
            if failed.load(Ordering::Acquire) || current.load(Ordering::Acquire) != epoch {
                break;
            }
            if !no_packet_warning
                && now_ms().saturating_sub(last_packet.load(Ordering::Acquire)) > 15_000
            {
                emit_error(&app, &request.session_id, "Waiting for system audio. Play your call audio; if it stays silent, check Screen & System Audio Recording permission.".into());
                no_packet_warning = true;
            }
        }
        stream
            .stop_capture()
            .map_err(|e| format!("Cannot stop system audio capture: {e}"))?;
        drop(stream);
        if let Ok(mut buffer) = chunker.lock() {
            if let Some(audio) = buffer.flush() {
                if current.load(Ordering::Acquire) == epoch {
                    send_until(
                        &sender,
                        Work::Audio(AudioChunk {
                            epoch,
                            session_id: request.session_id,
                            language: request.language,
                            source: "system".into(),
                            time: audio.time,
                            duration_ms: audio.duration_ms,
                            samples: audio.samples,
                            app,
                        }),
                        Instant::now() + Duration::from_secs(10),
                    )?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_is_continuous_and_bounded() {
        let mut chunker = Chunker::new(48_000);
        let mut lengths = Vec::new();
        for i in 0..48_000 * 19 {
            let value = ((i as f32 / 48_000.0) * 440.0 * std::f32::consts::TAU).sin() * 0.2;
            if let Some(audio) = chunker.push(value, i as u64 * 1_000 / 48_000) {
                lengths.push(audio.samples.len());
            }
            assert!(chunker.samples.len() < CHUNK_SAMPLES);
        }
        assert_eq!(lengths, vec![CHUNK_SAMPLES; 3]);
        assert!(chunker.flush().unwrap().samples.len().abs_diff(RATE) <= 1);
    }

    #[test]
    fn silence_and_nonfinite_audio_never_transcribe() {
        let mut chunker = Chunker::new(44_100);
        for i in 0..44_100 * 15 {
            assert!(chunker
                .push(
                    if i % 2 == 0 { 0.0 } else { f32::NAN },
                    i as u64 * 1_000 / 44_100
                )
                .is_none());
        }
        assert!(chunker.flush().is_none());
        assert!(!has_speech(&[]));
    }

    #[test]
    fn pause_flushes_a_question_and_stop_flushes_short_tail() {
        let mut chunker = Chunker::new(16_000);
        for i in 0..RATE {
            assert!(chunker.push(0.1, i as u64 * 1_000 / RATE as u64).is_none());
        }
        let mut delivered = None;
        for i in RATE..RATE * 2 {
            if let Some(chunk) = chunker.push(0.0, i as u64 * 1_000 / RATE as u64) {
                delivered = Some(chunk);
            }
        }
        let delivered = delivered.unwrap();
        assert_eq!(delivered.samples.len(), RATE * 2);
        assert_eq!((delivered.time, delivered.duration_ms), (0, 2_000));
        for i in RATE * 2..RATE * 5 / 2 {
            chunker.push(0.1, i as u64 * 1_000 / RATE as u64);
        }
        let tail = chunker.flush().unwrap();
        assert_eq!(tail.samples.len(), RATE);
        assert_eq!((tail.time, tail.duration_ms), (2_000, 500));
        assert!(chunker.flush().is_none());
    }

    #[test]
    fn subtitle_segments_keep_whisper_offsets_and_exclude_tail_padding() {
        let first = timed_segment("  First sentence.  ", 12, 154, 2_375).unwrap();
        assert_eq!(first.text, "First sentence.");
        assert_eq!((first.start_ms, first.end_ms), (120, 1_540));
        let final_segment = timed_segment("Short tail.", 154, 300, 2_375).unwrap();
        assert_eq!(
            (final_segment.start_ms, final_segment.end_ms),
            (1_540, 2_375)
        );
        assert!(timed_segment("Padding only.", 240, 300, 2_375).is_none());
        assert!(timed_segment(" ", 0, 100, 2_375).is_none());
        assert!(timed_segment("Reversed.", 100, 99, 2_375).is_none());
        let negative_start = timed_segment("Start.", -1, 10, 2_375).unwrap();
        assert_eq!((negative_start.start_ms, negative_start.end_ms), (0, 100));
    }

    #[test]
    fn discarded_silence_and_new_packets_do_not_reuse_old_capture_times() {
        let mut chunker = Chunker::new(RATE as u32);
        for i in 0..CHUNK_SAMPLES {
            assert!(chunker
                .push(0.0, 10_000 + i as u64 * 1_000 / RATE as u64)
                .is_none());
        }
        for i in 0..RATE / 2 {
            assert!(chunker
                .push(0.1, 23_000 + i as u64 * 1_000 / RATE as u64)
                .is_none());
        }
        let tail = chunker.flush().unwrap();
        assert_eq!((tail.time, tail.duration_ms), (23_000, 500));
        assert_eq!(tail.samples.len(), RATE);
        // Delivery may occur long after capture; event time always describes the audio.
        let segment = timed_segment("Hello.", 10, 100, tail.duration_ms).unwrap();
        assert_eq!(
            (tail.time + segment.start_ms, tail.time + segment.end_ms),
            (23_100, 23_500)
        );
    }

    #[test]
    fn missing_and_damaged_weights_are_not_ready() {
        let directory = std::env::temp_dir().join(format!("nextcue-voice-test-{}", now_ms()));
        fs::create_dir_all(&directory).unwrap();
        let voice = VoiceService::new(directory.clone());
        assert!(!voice.status().ready);
        fs::write(directory.join(MODEL_FILE), b"not a model").unwrap();
        assert!(!voice.status().ready);
        fs::remove_file(directory.join(MODEL_FILE)).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn queue_is_bounded_and_barrier_preserves_order() {
        let (tx, rx) = mpsc::sync_channel(QUEUE_CAPACITY);
        for _ in 0..QUEUE_CAPACITY {
            let (done, _) = mpsc::channel();
            assert!(tx.try_send(Work::Drain(done)).is_ok());
        }
        let (done, _) = mpsc::channel();
        assert!(matches!(
            tx.try_send(Work::Drain(done)),
            Err(TrySendError::Full(_))
        ));
        for _ in 0..QUEUE_CAPACITY {
            assert!(matches!(rx.recv().unwrap(), Work::Drain(_)));
        }
    }

    #[test]
    fn a_stalled_inference_queue_cannot_block_teardown_forever() {
        let (tx, _rx) = mpsc::sync_channel(1);
        let (done, _) = mpsc::channel();
        assert!(tx.try_send(Work::Drain(done)).is_ok());
        let (done, _) = mpsc::channel();
        let began = Instant::now();
        let result = send_until(&tx, Work::Drain(done), began + Duration::from_millis(25));
        assert!(result.is_err());
        assert!(began.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn audio_sources_require_valid_inputs_and_sessions() {
        let mut request = StartListening {
            device: "default".into(),
            language: "en".into(),
            session_id: "session-a".into(),
            source: "system".into(),
        };
        assert_eq!(
            validate_request(&request).is_ok(),
            cfg!(target_os = "macos")
        );
        request.device = "BlackHole 2ch".into();
        assert!(validate_request(&request).is_ok());
        request.session_id.clear();
        assert!(validate_request(&request).is_err());
    }

    #[test]
    #[ignore = "Requires explicitly downloaded local model and a 16 kHz mono test WAV"]
    fn real_offline_whisper_transcribes_repeated_segments() {
        let model =
            PathBuf::from(std::env::var("NEXTCUE_TEST_MODEL").expect("Set NEXTCUE_TEST_MODEL"));
        let wav = std::env::var("NEXTCUE_TEST_WAV").expect("Set NEXTCUE_TEST_WAV");
        let mut reader = hound::WavReader::open(wav).unwrap();
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, RATE as u32);
        assert_eq!(reader.spec().bits_per_sample, 16);
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|sample| sample.unwrap() as f32 / 32768.0)
            .collect();
        assert!(samples.len() <= RATE * 30);
        verify_model(&model).unwrap();
        let context = load_context(&model).unwrap();
        let mut state = context.create_state().unwrap();
        let epoch = Arc::new(AtomicU64::new(1));
        let expected = std::env::var("NEXTCUE_TEST_EXPECT").unwrap_or_else(|_| "country".into());
        let duration_ms = (samples.len() as u64 * 1_000).div_ceil(RATE as u64);
        let transcript_text = |segments: &[DecodedSegment]| {
            segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        };
        for _ in 0..3 {
            let segments =
                decode_samples(&mut state, &samples, "en", epoch.clone(), 1, duration_ms).unwrap();
            assert!(!segments.is_empty());
            assert!(segments
                .iter()
                .all(|segment| segment.start_ms < segment.end_ms && segment.end_ms <= duration_ms));
            let text = transcript_text(&segments);
            eprintln!("Offline transcript: {text}");
            assert!(text.to_lowercase().contains(&expected.to_lowercase()));
        }
        assert!(
            decode_samples(&mut state, &[0.0; RATE], "en", epoch.clone(), 1, 1_000)
                .unwrap()
                .is_empty()
        );
        epoch.store(2, Ordering::Release);
        assert!(
            decode_samples(&mut state, &samples, "en", epoch.clone(), 1, duration_ms)
                .unwrap()
                .is_empty()
        );
        epoch.store(3, Ordering::Release);
        let cancelled = epoch.clone();
        let cancel_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            cancelled.store(4, Ordering::Release);
        });
        let cancelled_result =
            decode_samples(&mut state, &samples, "en", epoch.clone(), 3, duration_ms);
        cancel_thread.join().unwrap();
        assert!(cancelled_result.is_err() || cancelled_result.unwrap().is_empty());
        // A cancelled graph must not poison the persistent worker's next interview.
        epoch.store(5, Ordering::Release);
        let recovered = decode_samples(&mut state, &samples, "en", epoch, 5, duration_ms).unwrap();
        assert!(transcript_text(&recovered)
            .to_lowercase()
            .contains(&expected.to_lowercase()));
    }
}
