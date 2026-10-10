use futures_util::{
    future::{AbortHandle, Abortable},
    StreamExt,
};
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::{
    process::{Child, Command},
    sync::Mutex as AsyncMutex,
};
use url::Url;

pub const DEFAULT_URL: &str = "http://127.0.0.1:11434";
pub const DEFAULT_MODEL: &str = "qwen2.5:7b-instruct";
const MAX_RESPONSE: usize = 2 * 1024 * 1024;
const MAX_LINE: usize = 512 * 1024;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub request_id: String,
    pub model: String,
    pub question: String,
    pub context: String,
    pub instructions: String,
    pub transcript: String,
    pub kind: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub session_mode: Option<String>,
    #[serde(default)]
    pub session_title: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub company: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenEvent {
    pub request_id: String,
    pub token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResult {
    pub text: String,
    pub model: String,
    pub duration_ms: u128,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Serialize)]
pub struct OllamaStatus {
    pub ollama: bool,
    pub models: Vec<Model>,
    pub error: Option<String>,
}

pub fn local_url(value: &str) -> Result<Url, String> {
    if value.len() > 256 {
        return Err("Ollama URL is too long.".into());
    }
    let url = Url::parse(value).map_err(|_| "Enter a valid local Ollama URL.")?;
    let host = url.host_str().unwrap_or_default();
    if !["http", "https"].contains(&url.scheme())
        || !["127.0.0.1", "localhost", "[::1]", "::1"].contains(&host)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !["", "/"].contains(&url.path())
    {
        return Err("Ollama must use a loopback URL, such as http://127.0.0.1:11434.".into());
    }
    Ok(url)
}

fn cloud_model(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with("-cloud") || name.contains(":cloud") || name.contains("-cloud:")
}

fn has_remote(value: &Value) -> bool {
    ["remote_model", "remote_host"].iter().any(|key| {
        value
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty())
    })
}

pub fn validate_chat(request: &ChatRequest) -> Result<(), String> {
    crate::context::validate_request(request)?;
    if cloud_model(&request.model) {
        return Err("Cloud models are disabled. Choose a locally installed Ollama model.".into());
    }
    Ok(())
}

pub struct OllamaService {
    client: Client,
    requests: Mutex<HashMap<String, AbortHandle>>,
    daemon: Mutex<Option<Child>>,
    starting: AsyncMutex<()>,
}

impl OllamaService {
    pub fn new() -> Result<Self, String> {
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            requests: Mutex::new(HashMap::new()),
            daemon: Mutex::new(None),
            starting: AsyncMutex::new(()),
        })
    }

    pub async fn status(&self, endpoint: &str) -> OllamaStatus {
        match self.models(endpoint).await {
            Ok(models) => OllamaStatus {
                ollama: true,
                models,
                error: None,
            },
            Err(error) => OllamaStatus {
                ollama: false,
                models: vec![],
                error: Some(error),
            },
        }
    }

    async fn models(&self, endpoint: &str) -> Result<Vec<Model>, String> {
        let url = local_url(endpoint)?
            .join("api/tags")
            .map_err(|e| e.to_string())?;
        let response = self
            .client
            .get(url)
            .timeout(Duration::from_secs(4))
            .send()
            .await
            .map_err(http_error)?;
        let data = response_json(response, MAX_RESPONSE).await?;
        let models = data
            .get("models")
            .and_then(Value::as_array)
            .ok_or("Ollama returned an invalid model list.")?;
        Ok(models
            .iter()
            .take(500)
            .filter(|model| !has_remote(model))
            .filter_map(|model| serde_json::from_value::<Model>(model.clone()).ok())
            .filter(|model| {
                !model.name.is_empty() && model.name.len() <= 256 && !cloud_model(&model.name)
            })
            .collect())
    }

    pub async fn chat<F>(
        &self,
        endpoint: &str,
        request: ChatRequest,
        on_token: F,
    ) -> Result<ChatResult, String>
    where
        F: Fn(TokenEvent) + Send + Sync,
    {
        validate_chat(&request)?;
        let url = local_url(endpoint)?;
        let (abort, registration) = AbortHandle::new_pair();
        let cancelled = abort.clone();
        {
            let mut requests = self
                .requests
                .lock()
                .map_err(|_| "Answer service is unavailable.")?;
            if requests.contains_key(&request.request_id) {
                return Err("This answer is already running.".into());
            }
            if requests.len() >= 2 {
                return Err("Please wait for the current answers to finish.".into());
            }
            requests.insert(request.request_id.clone(), abort);
        }
        let id = request.request_id.clone();
        let result = tokio::time::timeout(
            Duration::from_secs(180),
            Abortable::new(self.chat_inner(url, request, on_token), registration),
        )
        .await;
        let was_cancelled = cancelled.is_aborted();
        if let Ok(mut requests) = self.requests.lock() {
            requests.remove(&id);
        }
        if was_cancelled {
            return Err("Answer generation was cancelled.".into());
        }
        match result {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Answer generation was cancelled.".into()),
            Err(_) => Err("Ollama took too long to respond. Try a smaller model.".into()),
        }
    }

    async fn chat_inner<F>(
        &self,
        url: Url,
        request: ChatRequest,
        on_token: F,
    ) -> Result<ChatResult, String>
    where
        F: Fn(TokenEvent) + Send + Sync,
    {
        let started = Instant::now();
        // A renamed cloud model can have a harmless-looking tag. Inspect its origin before inference.
        let show = self
            .client
            .post(url.join("api/show").map_err(|e| e.to_string())?)
            .timeout(Duration::from_secs(10))
            .json(&json!({ "model": request.model }))
            .send()
            .await
            .map_err(http_error)?;
        let info = response_json(show, MAX_RESPONSE).await?;
        if has_remote(&info) {
            return Err(
                "This model runs in the cloud. Choose a locally installed Ollama model.".into(),
            );
        }
        let summary = request.kind.as_deref() == Some("summary");
        let (system, user) = crate::context::prompts(&request);
        let response = self.client.post(url.join("api/chat").map_err(|e| e.to_string())?).json(&json!({
            "model": request.model, "stream": true, "think": false,
            "messages": [{"role":"system","content":system}, {"role":"user","content":user}],
            "options": {"temperature": if summary { 0.3 } else { 0.6 }, "num_predict": if summary { 1000 } else { 800 }}
        })).send().await.map_err(http_error)?;
        if !response.status().is_success() {
            return Err(response_error(response).await);
        }
        let mut stream = response.bytes_stream();
        let mut parser = StreamParser::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(http_error)?;
            let tokens = parser.feed(&bytes)?;
            for token in tokens {
                on_token(TokenEvent {
                    request_id: request.request_id.clone(),
                    token,
                });
            }
            if parser.done {
                break;
            }
        }
        for token in parser.finish()? {
            on_token(TokenEvent {
                request_id: request.request_id.clone(),
                token,
            });
        }
        Ok(ChatResult {
            text: parser.text,
            model: request.model,
            duration_ms: started.elapsed().as_millis(),
        })
    }

    pub fn cancel(&self, request_id: &str) -> Result<(), String> {
        if request_id.is_empty() || request_id.len() > 128 {
            return Err("Invalid request ID.".into());
        }
        if let Some(handle) = self
            .requests
            .lock()
            .map_err(|_| "Answer service is unavailable.")?
            .get(request_id)
        {
            handle.abort();
        }
        Ok(())
    }

    pub async fn start(&self, endpoint: &str) -> Result<(), String> {
        let url = local_url(endpoint)?;
        if url.port_or_known_default() != Some(11434) {
            return Err(
                "Automatic start uses port 11434. Set Ollama URL to http://127.0.0.1:11434.".into(),
            );
        }
        let _starting = self.starting.lock().await;
        if self.status(endpoint).await.ollama {
            return Ok(());
        }
        let executable = ollama_executable()
            .ok_or("Ollama is not installed. Install Ollama, then reopen NextCue.")?;
        {
            let mut daemon = self
                .daemon
                .lock()
                .map_err(|_| "Ollama process manager is unavailable.")?;
            let running = daemon
                .as_mut()
                .is_some_and(|child| child.try_wait().ok().flatten().is_none());
            if !running {
                let child = Command::new(executable)
                    .arg("serve")
                    .env("OLLAMA_HOST", "127.0.0.1:11434")
                    .env("OLLAMA_NO_CLOUD", "1")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|e| format!("Cannot start Ollama: {e}"))?;
                *daemon = Some(child);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if self.status(endpoint).await.ollama {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        Err("Ollama is starting slowly. Check the status again in a moment.".into())
    }

    pub fn shutdown(&self) {
        if let Ok(mut requests) = self.requests.lock() {
            for handle in requests.values() {
                handle.abort();
            }
            requests.clear();
        }
        if let Ok(mut daemon) = self.daemon.lock() {
            if let Some(child) = daemon.as_mut() {
                let _ = child.start_kill();
            }
            *daemon = None;
        }
    }
}

fn ollama_executable() -> Option<PathBuf> {
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/bin/ollama"),
        PathBuf::from("/usr/local/bin/ollama"),
        PathBuf::from("/usr/bin/ollama"),
        PathBuf::from("/Applications/Ollama.app/Contents/Resources/ollama"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join("bin/ollama"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("Programs/Ollama/ollama.exe"));
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn http_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "Ollama took too long to respond. Try a smaller model.".into()
    } else if error.is_connect() {
        "Cannot reach Ollama. Start Ollama and check the local server URL.".into()
    } else {
        format!("Ollama request failed: {error}")
    }
}

async fn response_bytes(response: Response, maximum: usize) -> Result<Vec<u8>, String> {
    let mut stream = response.bytes_stream();
    let mut output = Vec::new();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(http_error)?;
        if output.len() + bytes.len() > maximum {
            return Err("Ollama response exceeds the maximum size.".into());
        }
        output.extend_from_slice(&bytes);
    }
    Ok(output)
}

async fn response_error(response: Response) -> String {
    let status = response.status();
    let bytes = response_bytes(response, 16384).await.unwrap_or_default();
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(|e| e.chars().take(1000).collect())
        })
        .unwrap_or_else(|| format!("Ollama returned HTTP {status}."))
}

async fn response_json(response: Response, maximum: usize) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(response_error(response).await);
    }
    let value: Value = serde_json::from_slice(&response_bytes(response, maximum).await?)
        .map_err(|_| "Ollama returned invalid JSON.".to_string())?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(error.chars().take(1000).collect());
    }
    Ok(value)
}

struct StreamParser {
    buffer: Vec<u8>,
    received: usize,
    text: String,
    done: bool,
}
impl StreamParser {
    fn new() -> Self {
        Self {
            buffer: vec![],
            received: 0,
            text: String::new(),
            done: false,
        }
    }
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        self.received += bytes.len();
        if self.received > MAX_RESPONSE {
            return Err("Ollama response exceeds the maximum size.".into());
        }
        self.buffer.extend_from_slice(bytes);
        let mut tokens = vec![];
        while !self.done {
            let Some(newline) = self.buffer.iter().position(|&b| b == b'\n') else {
                break;
            };
            if newline > MAX_LINE {
                return Err("Ollama response line exceeds the maximum size.".into());
            }
            let line: Vec<u8> = self.buffer.drain(..=newline).collect();
            if let Some(token) = self.line(&line)? {
                tokens.push(token);
            }
        }
        if !self.done && self.buffer.len() > MAX_LINE {
            return Err("Ollama response line exceeds the maximum size.".into());
        }
        Ok(tokens)
    }
    fn line(&mut self, bytes: &[u8]) -> Result<Option<String>, String> {
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(None);
        }
        let data: Value = serde_json::from_slice(bytes)
            .map_err(|_| "Ollama returned an invalid response stream.")?;
        if let Some(error) = data.get("error").and_then(Value::as_str) {
            return Err(error.chars().take(1000).collect());
        }
        let token = data
            .pointer("/message/content")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        if let Some(token) = &token {
            self.text.push_str(token);
        }
        self.done = data.get("done").and_then(Value::as_bool) == Some(true);
        Ok(token)
    }
    fn finish(&mut self) -> Result<Vec<String>, String> {
        let mut tokens = vec![];
        if !self.done && !self.buffer.is_empty() {
            let bytes = std::mem::take(&mut self.buffer);
            if let Some(token) = self.line(&bytes)? {
                tokens.push(token);
            }
        }
        if !self.done {
            return Err("Ollama stopped before completing the answer. Please try again.".into());
        }
        if self.text.trim().is_empty() {
            return Err("The model returned no answer. Try another model.".into());
        }
        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_loopback_endpoints_are_allowed() {
        for value in [DEFAULT_URL, "http://localhost:11434", "http://[::1]:11434"] {
            assert!(local_url(value).is_ok(), "{value}");
        }
        for value in [
            "http://ollama.com",
            "http://127.0.0.1.evil.test",
            "file:///tmp/test",
            "http://user@localhost:11434",
            "http://localhost:11434/evil",
            "http://localhost:11434?next=evil",
        ] {
            assert!(local_url(value).is_err(), "{value}");
        }
    }
    #[test]
    fn stream_handles_split_utf8_and_ignores_thinking() {
        let body = "{\"message\":{\"thinking\":\"secret\",\"content\":\"Hé\"},\"done\":false}\n{\"message\":{\"content\":\"llo\"},\"done\":true}\n";
        let mut parser = StreamParser::new();
        let mut tokens = vec![];
        for byte in body.as_bytes() {
            tokens.extend(parser.feed(&[*byte]).unwrap());
        }
        tokens.extend(parser.finish().unwrap());
        assert_eq!(tokens, ["Hé", "llo"]);
        assert_eq!(parser.text, "Héllo");
    }
    #[test]
    fn stream_requires_terminal_done_and_rejects_errors() {
        let mut truncated = StreamParser::new();
        truncated
            .feed(b"{\"message\":{\"content\":\"partial\"},\"done\":false}\n")
            .unwrap();
        assert!(truncated.finish().is_err());
        let mut error = StreamParser::new();
        assert_eq!(
            error.feed(b"{\"error\":\"model missing\"}\n").unwrap_err(),
            "model missing"
        );
        let mut invalid = StreamParser::new();
        assert!(invalid.feed(b"not json\n").is_err());
    }
    #[test]
    fn stream_accepts_final_line_without_newline_and_bounds_buffer() {
        let mut parser = StreamParser::new();
        parser
            .feed(b"{\"message\":{\"content\":\"answer\"},\"done\":true}")
            .unwrap();
        assert_eq!(parser.finish().unwrap(), ["answer"]);
        assert!(StreamParser::new().feed(&vec![b'x'; MAX_LINE + 1]).is_err());
    }
    #[test]
    fn cloud_names_and_renamed_remote_models_are_rejected() {
        assert!(cloud_model("gpt-oss:120b-cloud"));
        assert!(cloud_model("gpt-oss:cloud"));
        assert!(!cloud_model("qwen2.5:7b-instruct"));
        assert!(has_remote(
            &json!({"remote_model":"gpt-oss:120b", "remote_host":"https://ollama.com"})
        ));
    }

    #[tokio::test]
    #[ignore = "requires a running local Ollama daemon and smollm2:135m (or NEXTCUE_TEST_OLLAMA_MODEL)"]
    async fn real_ollama_streaming_and_cancellation() {
        let endpoint =
            std::env::var("NEXTCUE_TEST_OLLAMA_URL").unwrap_or_else(|_| DEFAULT_URL.to_owned());
        let model = std::env::var("NEXTCUE_TEST_OLLAMA_MODEL")
            .unwrap_or_else(|_| "smollm2:135m".to_owned());
        let service = OllamaService::new().unwrap();
        let status = service.status(&endpoint).await;
        assert!(
            status.ollama,
            "Local Ollama is unavailable: {:?}",
            status.error
        );
        assert!(
            status
                .models
                .iter()
                .any(|installed| installed.name == model),
            "Install {model} before running this explicit integration test."
        );
        let request = ChatRequest {
            request_id: "real-ollama-stream".to_owned(),
            model: model.clone(),
            question: "Tell me briefly about your experience with Rust.".to_owned(),
            context: "I am a software engineer who built a local desktop application in Rust."
                .to_owned(),
            instructions: "Give one short sentence.".to_owned(),
            transcript: String::new(),
            kind: None,
            provider: None,
            session_mode: None,
            session_title: None,
            role: None,
            company: None,
        };
        let received = Mutex::new(Vec::<TokenEvent>::new());
        let result = service
            .chat(&endpoint, request, |event| {
                received.lock().unwrap().push(event)
            })
            .await
            .unwrap();
        let tokens = received.lock().unwrap();
        assert!(
            !tokens.is_empty(),
            "No actual streamed tokens were received."
        );
        assert!(tokens
            .iter()
            .all(|event| event.request_id == "real-ollama-stream"));
        assert_eq!(
            tokens
                .iter()
                .map(|event| event.token.as_str())
                .collect::<String>(),
            result.text
        );
        assert!(!result.text.trim().is_empty());
        assert_eq!(result.model, model);
        drop(tokens);

        let cancel_id = "real-ollama-cancel";
        let cancelled_request = ChatRequest {
            request_id: cancel_id.to_owned(), model,
            question: "List one hundred numbered Rust interview questions, with detailed answers for each.".to_owned(),
            context: String::new(), instructions: String::new(), transcript: String::new(), kind: None,
            provider: None, session_mode: None, session_title: None, role: None, company: None,
        };
        let cancel_tokens = Mutex::new(0usize);
        let result = service
            .chat(&endpoint, cancelled_request, |_| {
                *cancel_tokens.lock().unwrap() += 1;
                service.cancel(cancel_id).unwrap();
            })
            .await;
        assert!(
            *cancel_tokens.lock().unwrap() > 0,
            "Cancellation test did not reach the inference stream."
        );
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(
            service.requests.lock().unwrap().is_empty(),
            "Cancelled request was not released."
        );
        service.shutdown();
    }
}
