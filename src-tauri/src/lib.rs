mod background;
mod browser;
mod chatgpt;
mod claude;
mod companion;
mod context;
mod jev;
mod meeting;
mod ollama;
mod store;
mod voice;

use background::{
    ActionSource, BackgroundAction, BackgroundActivity, BackgroundConfiguration, BackgroundService,
    BackgroundStatus,
};
use browser::{BrowserConnection, BrowserService};
use chatgpt::{ChatgptService, ChatgptStatus};
use claude::{ClaudeService, ClaudeStatus};
use companion::{CompanionMode, CompanionService, CompanionStatus};
use jev::{JevAssessment, JevService, JevStatus};
use ollama::{ChatRequest, ChatResult, OllamaService};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use store::{StateStore, MAX_CONTEXT_BYTES};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use voice::{InputDevice, StartListening, VoiceService};

struct Services {
    ollama: OllamaService,
    chatgpt: ChatgptService,
    claude: ClaudeService,
    jev: JevService,
    store: StateStore,
    voice: Arc<VoiceService>,
    companion: CompanionService,
    background: BackgroundService,
    browser: BrowserService,
    allow_exit: AtomicBool,
}

fn trusted_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err("This command is only available in the main workspace.".into());
    }
    let url = window.url().map_err(|e| e.to_string())?;
    let packaged = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (["http", "https"].contains(&url.scheme()) && url.host_str() == Some("tauri.localhost"));
    let development = cfg!(debug_assertions)
        && url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port() == Some(5173);
    if !packaged && !development {
        return Err("Desktop commands are unavailable to this page.".into());
    }
    Ok(())
}

#[tauri::command]
async fn set_companion_mode(
    window: WebviewWindow,
    services: State<'_, Services>,
    mode: CompanionMode,
) -> Result<CompanionStatus, String> {
    trusted_window(&window)?;
    services.companion.set_mode(&window, mode)
}

#[tauri::command]
async fn configure_companion(
    window: WebviewWindow,
    app: AppHandle,
    services: State<'_, Services>,
    capture_protection: bool,
    answer_shortcut: String,
    visibility_shortcut: String,
    meeting_shortcut: Option<String>,
) -> Result<CompanionStatus, String> {
    trusted_window(&window)?;
    services.companion.configure(
        &app,
        &window,
        capture_protection,
        answer_shortcut,
        visibility_shortcut,
        meeting_shortcut.unwrap_or_else(|| companion::DEFAULT_MEETING_SHORTCUT.into()),
    )
}

#[tauri::command]
async fn configure_background(
    window: WebviewWindow,
    app: AppHandle,
    services: State<'_, Services>,
    config: BackgroundConfiguration,
) -> Result<BackgroundStatus, String> {
    trusted_window(&window)?;
    let enable_browser_connection = config.meeting_detection;
    let status = services.background.configure(config)?;
    if enable_browser_connection {
        services.browser.initialize(app);
    }
    Ok(status)
}

#[tauri::command]
async fn get_background_status(
    window: WebviewWindow,
    services: State<'_, Services>,
) -> Result<BackgroundStatus, String> {
    trusted_window(&window)?;
    Ok(services.background.status())
}

#[tauri::command]
async fn background_ready(
    window: WebviewWindow,
    app: AppHandle,
    services: State<'_, Services>,
) -> Result<(), String> {
    trusted_window(&window)?;
    services.background.ready(&app);
    Ok(())
}

#[tauri::command]
async fn get_browser_connection(
    window: WebviewWindow,
    app: AppHandle,
    services: State<'_, Services>,
) -> Result<BrowserConnection, String> {
    trusted_window(&window)?;
    services.browser.initialize(app);
    Ok(services.browser.get_connection().await)
}

#[tauri::command]
async fn update_background_activity(
    window: WebviewWindow,
    app: AppHandle,
    services: State<'_, Services>,
    listening: bool,
    mode: BackgroundActivity,
) -> Result<(), String> {
    trusted_window(&window)?;
    services.background.update_activity(&app, listening, mode)
}

#[tauri::command]
async fn get_companion_status(
    window: WebviewWindow,
    services: State<'_, Services>,
) -> Result<CompanionStatus, String> {
    trusted_window(&window)?;
    services.companion.status(&window)
}

#[tauri::command]
async fn set_companion_visible(
    window: WebviewWindow,
    services: State<'_, Services>,
    visible: bool,
) -> Result<CompanionStatus, String> {
    trusted_window(&window)?;
    services.companion.set_visible(&window, visible)
}

#[tauri::command]
async fn copy_text(window: WebviewWindow, app: AppHandle, text: String) -> Result<(), String> {
    trusted_window(&window)?;
    if text.len() > 1024 * 1024 {
        return Err("The answer is too large to copy.".into());
    }
    if text.trim().is_empty() {
        return Err("There is no answer to copy yet.".into());
    }
    app.clipboard()
        .write_text(text)
        .map_err(|e| format!("Cannot copy this answer: {e}"))
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ProviderPage {
    #[serde(rename = "chatgpt-usage")]
    ChatgptUsage,
    #[serde(rename = "claude-setup")]
    ClaudeSetup,
}

fn provider_page_url(page: ProviderPage) -> &'static str {
    match page {
        ProviderPage::ChatgptUsage => "https://chatgpt.com/settings/usage",
        ProviderPage::ClaudeSetup => "https://code.claude.com/docs/en/setup",
    }
}

#[tauri::command]
async fn open_provider_page(window: WebviewWindow, page: ProviderPage) -> Result<(), String> {
    trusted_window(&window)?;
    let url = provider_page_url(page);
    tauri::async_runtime::spawn_blocking(move || {
        // The browser receives only a constant public destination. Prompt text,
        // session context, account tokens, and arbitrary URLs are never accepted.
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = std::process::Command::new("/usr/bin/open");
            command.arg(url);
            command
        };
        #[cfg(target_os = "windows")]
        let mut command = {
            use std::os::windows::process::CommandExt;
            let system_directory = std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or("Cannot find the Windows browser launcher.")?;
            let mut command =
                std::process::Command::new(system_directory.join("System32").join("rundll32.exe"));
            command
                .args(["url.dll,FileProtocolHandler", url])
                .creation_flags(0x08000000); // CREATE_NO_WINDOW
            command
        };
        #[cfg(target_os = "linux")]
        let mut command = {
            let mut command = std::process::Command::new("/usr/bin/xdg-open");
            command.arg(url);
            command
        };
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            use std::process::Stdio;
            let status = command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|error| format!("Cannot open your default browser: {error}"))?;
            if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "Cannot open your default browser. Open {url} manually."
                ))
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        Err("External assistants are available from the desktop app.".to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn open_browser_companion(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    trusted_window(&window)?;
    // The destination is fixed by the package. A caller cannot supply a path,
    // executable, URL, or launch arguments through this command.
    let folder = app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?
        .join("browser-companion");
    if !folder.is_dir() || !folder.join("manifest.json").is_file() {
        return Err("The browser companion files are missing. Install the latest complete NextCue desktop package.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        let mut command = std::process::Command::new("/usr/bin/open");
        #[cfg(target_os = "windows")]
        let mut command = {
            use std::os::windows::process::CommandExt;
            let system_root = std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or("Cannot find Windows File Explorer.")?;
            let mut command = std::process::Command::new(system_root.join("explorer.exe"));
            command.creation_flags(0x08000000);
            command
        };
        #[cfg(target_os = "linux")]
        let mut command = std::process::Command::new("/usr/bin/xdg-open");
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            use std::process::Stdio;
            let status = command
                .arg(folder)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|error| format!("Cannot open the browser companion folder: {error}"))?;
            if status.success() {
                Ok(())
            } else {
                Err("Cannot open the browser companion folder in your file manager.".into())
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        Err("The browser companion is available in the desktop package.".into())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_state(window: WebviewWindow, services: State<'_, Services>) -> Result<Value, String> {
    trusted_window(&window)?;
    services.store.load().await
}
#[tauri::command]
async fn save_state(
    window: WebviewWindow,
    services: State<'_, Services>,
    state: Value,
) -> Result<(), String> {
    trusted_window(&window)?;
    services.store.save(state).await
}

#[tauri::command]
async fn quit_app(
    window: WebviewWindow,
    app: AppHandle,
    services: State<'_, Services>,
    state: Option<Value>,
) -> Result<(), String> {
    trusted_window(&window)?;
    if let Some(state) = state {
        services.store.save(state).await?;
    }
    services.allow_exit.store(true, Ordering::SeqCst);
    app.exit(0);
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatus {
    ollama: bool,
    models: Vec<ollama::Model>,
    transcription: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    transcription_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    platform: &'static str,
}

#[tauri::command]
async fn status(
    window: WebviewWindow,
    services: State<'_, Services>,
) -> Result<RuntimeStatus, String> {
    trusted_window(&window)?;
    let endpoint = services.store.ollama_url().await?;
    let ollama = services.ollama.status(&endpoint).await;
    let voice = services.voice.status();
    Ok(RuntimeStatus {
        ollama: ollama.ollama,
        models: ollama.models,
        error: ollama.error,
        transcription: voice.ready,
        transcription_error: voice.error,
        platform: std::env::consts::OS,
    })
}
#[tauri::command]
async fn start_ollama(window: WebviewWindow, services: State<'_, Services>) -> Result<(), String> {
    trusted_window(&window)?;
    services
        .ollama
        .start(&services.store.ollama_url().await?)
        .await
}

#[tauri::command]
async fn chatgpt_status(
    window: WebviewWindow,
    services: State<'_, Services>,
    refresh_models: bool,
) -> Result<ChatgptStatus, String> {
    trusted_window(&window)?;
    Ok(services.chatgpt.status(refresh_models).await)
}

#[tauri::command]
async fn chatgpt_sign_in(
    window: WebviewWindow,
    services: State<'_, Services>,
    account_id: Option<String>,
) -> Result<ChatgptStatus, String> {
    trusted_window(&window)?;
    services.chatgpt.sign_in(account_id).await
}

#[tauri::command]
async fn chatgpt_cancel_sign_in(
    window: WebviewWindow,
    services: State<'_, Services>,
) -> Result<(), String> {
    trusted_window(&window)?;
    services.chatgpt.cancel_sign_in()
}

#[tauri::command]
async fn chatgpt_sign_out(
    window: WebviewWindow,
    services: State<'_, Services>,
) -> Result<ChatgptStatus, String> {
    trusted_window(&window)?;
    services.chatgpt.sign_out().await
}

#[tauri::command]
async fn chatgpt_select_account(
    window: WebviewWindow,
    services: State<'_, Services>,
    account_id: String,
) -> Result<ChatgptStatus, String> {
    trusted_window(&window)?;
    services.chatgpt.select_account(account_id).await
}

#[tauri::command]
async fn claude_status(
    window: WebviewWindow,
    services: State<'_, Services>,
) -> Result<ClaudeStatus, String> {
    trusted_window(&window)?;
    Ok(services.claude.status().await)
}

#[tauri::command]
async fn jev_status(
    window: WebviewWindow,
    services: State<'_, Services>,
) -> Result<JevStatus, String> {
    trusted_window(&window)?;
    services.jev.status()
}

#[tauri::command]
async fn jev_save_key(
    window: WebviewWindow,
    services: State<'_, Services>,
    key: String,
) -> Result<JevStatus, String> {
    trusted_window(&window)?;
    services.jev.save_key(key)
}

#[tauri::command]
async fn jev_delete_key(
    window: WebviewWindow,
    services: State<'_, Services>,
) -> Result<JevStatus, String> {
    trusted_window(&window)?;
    services.jev.delete_key()
}

#[tauri::command]
async fn jev_assess(
    window: WebviewWindow,
    services: State<'_, Services>,
    request: ChatRequest,
) -> Result<JevAssessment, String> {
    trusted_window(&window)?;
    context::validate_request(&request)?;
    services.jev.assess(request).await
}

#[tauri::command]
async fn chat(
    window: WebviewWindow,
    services: State<'_, Services>,
    request: ChatRequest,
) -> Result<ChatResult, String> {
    trusted_window(&window)?;
    context::validate_request(&request)?;
    let on_token = move |event| {
        let _ = window.emit("ollama-token", event);
    };
    match request.provider.as_deref().unwrap_or("local") {
        "local" => {
            let endpoint = services.store.ollama_url().await?;
            services.ollama.chat(&endpoint, request, on_token).await
        }
        "chatgpt" => services.chatgpt.chat(request, on_token).await,
        "claude" => services.claude.chat(request, on_token).await,
        _ => Err("Invalid answer provider.".into()),
    }
}
#[tauri::command]
fn cancel_chat(
    window: WebviewWindow,
    services: State<'_, Services>,
    request_id: String,
) -> Result<(), String> {
    trusted_window(&window)?;
    // A provider may change while a response is running. Cancel by request ID
    // across services so the original process or stream is always stopped.
    services.ollama.cancel(&request_id)?;
    services.chatgpt.cancel(&request_id)?;
    services.claude.cancel(&request_id)
}

#[tauri::command]
async fn input_devices(
    window: WebviewWindow,
    services: State<'_, Services>,
) -> Result<Vec<InputDevice>, String> {
    trusted_window(&window)?;
    let voice = services.voice.clone();
    tauri::async_runtime::spawn_blocking(move || voice.list_devices())
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn setup_voice(
    window: WebviewWindow,
    app: AppHandle,
    services: State<'_, Services>,
) -> Result<(), String> {
    trusted_window(&window)?;
    services.voice.setup(app).await
}
#[tauri::command]
async fn start_listening(
    window: WebviewWindow,
    app: AppHandle,
    services: State<'_, Services>,
    request: StartListening,
) -> Result<(), String> {
    trusted_window(&window)?;
    let voice = services.voice.clone();
    tauri::async_runtime::spawn_blocking(move || voice.start(app, request))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn stop_listening(
    window: WebviewWindow,
    services: State<'_, Services>,
) -> Result<(), String> {
    trusted_window(&window)?;
    let voice = services.voice.clone();
    tauri::async_runtime::spawn_blocking(move || voice.stop())
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
struct ImportedContext {
    name: String,
    text: String,
}

#[tauri::command]
async fn import_context(
    window: WebviewWindow,
    app: AppHandle,
) -> Result<Option<ImportedContext>, String> {
    trusted_window(&window)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Import resume or interview context")
        .add_filter("Text documents", &["txt", "md", "json"])
        .pick_file(move |file| {
            let _ = sender.send(file);
        });
    let Some(file) = receiver.await.map_err(|_| "The file dialog was closed.")? else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|_| "Choose a local text file.")?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !["txt", "md", "json"].contains(&extension.as_str()) {
        return Err("Import a .txt, .md, or .json document.".into());
    }
    let bytes = store::read_bounded(&path, MAX_CONTEXT_BYTES)
        .await
        .map_err(|e| format!("Cannot import this document: {e}"))?;
    let text = String::from_utf8(bytes).map_err(|_| "The document must contain UTF-8 text.")?;
    let text = text.trim_start_matches('\u{feff}');
    if extension == "json" {
        serde_json::from_str::<Value>(&text).map_err(|_| "The document contains invalid JSON.")?;
    }
    Ok(Some(ImportedContext {
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        text: text.to_string(),
    }))
}

#[tauri::command]
async fn export_session(
    window: WebviewWindow,
    app: AppHandle,
    session: Value,
) -> Result<bool, String> {
    trusted_window(&window)?;
    let markdown = store::session_markdown(&session)?;
    let title = session["title"].as_str().unwrap_or("session");
    let filename: String = title
        .chars()
        .take(80)
        .map(|c| {
            if c.is_ascii_alphanumeric() || ['-', '_'].contains(&c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let filename = format!(
        "{}.md",
        if filename.trim_matches('-').is_empty() {
            "nextcue-session"
        } else {
            filename.trim_matches('-')
        }
    );
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Export session")
        .set_file_name(filename)
        .add_filter("Markdown", &["md"])
        .save_file(move |file| {
            let _ = sender.send(file);
        });
    let Some(file) = receiver.await.map_err(|_| "The save dialog was closed.")? else {
        return Ok(false);
    };
    let mut path = file
        .into_path()
        .map_err(|_| "Choose a local save location.")?;
    if path.extension().is_none() {
        path.set_extension("md");
    }
    tokio::fs::write(&path, markdown)
        .await
        .map_err(|e| format!("Cannot export this session: {e}"))?;
    Ok(true)
}

fn data_directory(app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    match std::env::var_os("NEXTCUE_DATA_DIR") {
        Some(path) if !path.is_empty() => {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err("NEXTCUE_DATA_DIR must be an absolute path.".into());
            }
            Ok(path)
        }
        _ => Ok(app.path().app_data_dir()?),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let directory = data_directory(app.handle())?;
            let voice = Arc::new(VoiceService::new(directory.join("models")));
            let ollama = OllamaService::new().map_err(std::io::Error::other)?;
            let keyring_service = app.config().identifier.clone();
            let chatgpt = ChatgptService::new(directory.clone(), keyring_service.clone())
                .map_err(std::io::Error::other)?;
            let claude = ClaudeService::new(directory.join("claude-workspace"));
            let jev = JevService::new(&keyring_service).map_err(std::io::Error::other)?;
            let companion = CompanionService::new(directory.clone());
            let browser = BrowserService::new(directory.clone());
            let store = StateStore::new(directory);
            // Read before showing the window: launching quietly must not flash
            // the workspace while the webview restores its saved state.
            let saved_state = tauri::async_runtime::block_on(store.load())
                .unwrap_or_else(|_| store::default_state());
            let background =
                BackgroundService::new(BackgroundConfiguration::from_state(&saved_state));
            app.manage(Services {
                ollama,
                chatgpt,
                claude,
                jev,
                store,
                voice,
                companion,
                background,
                browser,
                allow_exit: AtomicBool::new(false),
            });
            #[cfg(desktop)]
            {
                use companion::ShortcutAction;
                use tauri_plugin_global_shortcut::ShortcutState;
                let plugin = tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(|app, received, event| {
                        if event.state() != ShortcutState::Pressed {
                            return;
                        }
                        let action = app.state::<Services>().companion.shortcut_action(received);
                        let app = app.clone();
                        // Window getters and configuration may wait on the main
                        // thread. Keep shortcut callbacks free of service locks.
                        tauri::async_runtime::spawn(async move {
                            let Some(window) = app.get_webview_window("main") else {
                                return;
                            };
                            let services = app.state::<Services>();
                            match action {
                                Some(ShortcutAction::Answer) => {
                                    if window.is_visible().ok() == Some(false) {
                                        if let Ok(status) =
                                            services.companion.set_visible(&window, true)
                                        {
                                            // Restoring an already open answer card
                                            // must not immediately collapse it again.
                                            if status.mode == CompanionMode::Answer {
                                                return;
                                            }
                                        }
                                    }
                                    let _ = window.emit("companion-toggle", ());
                                }
                                Some(ShortcutAction::Visibility) => {
                                    if let Ok(visible) = window.is_visible() {
                                        let _ = services.companion.set_visible(&window, !visible);
                                    }
                                }
                                Some(ShortcutAction::Meeting) => {
                                    services.background.emit_action(
                                        &app,
                                        BackgroundAction::MeetingTrigger,
                                        ActionSource::Shortcut,
                                    );
                                }
                                None => {}
                            }
                        });
                    })
                    .build();
                if app.handle().plugin(plugin).is_ok() {
                    app.state::<Services>()
                        .companion
                        .initialize_shortcuts(app.handle());
                    if let Some(window) = app.get_webview_window("main") {
                        let settings = &saved_state["settings"];
                        let _ = app.state::<Services>().companion.configure(
                            app.handle(),
                            &window,
                            settings["captureProtection"].as_bool().unwrap_or(true),
                            settings["answerShortcut"]
                                .as_str()
                                .unwrap_or(companion::DEFAULT_ANSWER_SHORTCUT)
                                .into(),
                            settings["visibilityShortcut"]
                                .as_str()
                                .unwrap_or(companion::DEFAULT_VISIBILITY_SHORTCUT)
                                .into(),
                            settings["meetingShortcut"]
                                .as_str()
                                .unwrap_or(companion::DEFAULT_MEETING_SHORTCUT)
                                .into(),
                        );
                    }
                }
            }
            let services = app.state::<Services>();
            services.background.initialize(app.handle());
            if services.background.current().meeting_detection {
                services.browser.initialize(app.handle().clone());
            }
            if let Some(window) = app.get_webview_window("main") {
                if !services.background.current().start_hidden
                    || !services.companion.recovery_available()
                {
                    let _ = window.show();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            save_state,
            quit_app,
            status,
            start_ollama,
            chatgpt_status,
            chatgpt_sign_in,
            chatgpt_cancel_sign_in,
            chatgpt_sign_out,
            chatgpt_select_account,
            claude_status,
            jev_status,
            jev_save_key,
            jev_delete_key,
            jev_assess,
            chat,
            cancel_chat,
            input_devices,
            setup_voice,
            start_listening,
            stop_listening,
            import_context,
            export_session,
            set_companion_mode,
            configure_companion,
            configure_background,
            get_background_status,
            background_ready,
            get_browser_connection,
            update_background_activity,
            get_companion_status,
            set_companion_visible,
            copy_text,
            open_provider_page,
            open_browser_companion
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Moved(position) = event {
                if let Some(services) = window.try_state::<Services>() {
                    services.companion.record_position(*position);
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if let Some(services) = window.try_state::<Services>() {
                    if !services.allow_exit.load(Ordering::SeqCst) {
                        api.prevent_close();
                        if window.label() == "main" && services.background.close_to_tray() {
                            let app = window.app_handle().clone();
                            tauri::async_runtime::spawn(async move {
                                if let Some(window) = app.get_webview_window("main") {
                                    let _ = app
                                        .state::<Services>()
                                        .companion
                                        .set_visible(&window, false);
                                }
                            });
                        } else {
                            services.background.request_quit(window.app_handle());
                        }
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("Cannot start NextCue")
        .run(|app, event| match event {
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                background::dispatch_action(
                    app.clone(),
                    BackgroundAction::Show,
                    ActionSource::Dock,
                );
            }
            tauri::RunEvent::ExitRequested { api, .. } => {
                let services = app.state::<Services>();
                if !services.allow_exit.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    services.background.request_quit(app);
                }
            }
            tauri::RunEvent::Exit => {
                let services = app.state::<Services>();
                if let Some(window) = app.get_webview_window("main") {
                    if let Ok(position) = window.outer_position() {
                        services.companion.record_position(position);
                    }
                }
                services.ollama.shutdown();
                services.chatgpt.shutdown();
                services.claude.shutdown();
                services.background.shutdown();
                services.browser.shutdown();
                services.voice.shutdown();
            }
            _ => {}
        });
}

#[cfg(test)]
mod provider_page_tests {
    use super::*;

    #[test]
    fn handoff_destinations_are_allowlisted_and_never_contain_context_or_auth() {
        for (name, host, path) in [
            ("chatgpt-usage", "chatgpt.com", "/settings/usage"),
            ("claude-setup", "code.claude.com", "/docs/en/setup"),
        ] {
            let page: ProviderPage = serde_json::from_value(Value::String(name.into())).unwrap();
            let destination = url::Url::parse(provider_page_url(page)).unwrap();
            assert_eq!(destination.scheme(), "https");
            assert_eq!(destination.host_str(), Some(host));
            assert_eq!(destination.path(), path);
            assert!(destination.username().is_empty());
            assert!(destination.password().is_none());
            assert!(destination.query().is_none());
            assert!(destination.fragment().is_none());
        }
        for value in ["https://example.com", "chatgpt?prompt=private", "other"] {
            assert!(serde_json::from_value::<ProviderPage>(Value::String(value.into())).is_err());
        }
    }
}
