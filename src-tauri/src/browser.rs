//! A small, authenticated loopback bridge for the optional browser companion.
//! Messages contain a provider label and a random page ID, never a URL, meeting
//! code, page content, transcript, screenshot, or browser-account credential.

use crate::meeting::MeetingHint;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::Read,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Notify, Semaphore},
};

const MAX_HEADERS: usize = 8192;
const MAX_BODY: usize = 4096;
const MAX_CONNECTIONS: usize = 16;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RECENT_PAGES: usize = 256;
const DEDUPE_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserConnection {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl BrowserConnection {
    fn unavailable(detail: &str) -> Self {
        Self {
            available: false,
            port: None,
            token: None,
            detail: Some(detail.into()),
        }
    }
}

struct BrowserState {
    ready: bool,
    connection: BrowserConnection,
}

struct BrowserInner {
    directory: PathBuf,
    state: Mutex<BrowserState>,
    initialized: AtomicBool,
    stopped: AtomicBool,
    ready: Notify,
    shutdown: Notify,
}

#[derive(Clone)]
pub struct BrowserService {
    inner: Arc<BrowserInner>,
}

impl BrowserService {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            inner: Arc::new(BrowserInner {
                directory,
                state: Mutex::new(BrowserState {
                    ready: false,
                    connection: BrowserConnection::unavailable(
                        "Preparing the local browser connection.",
                    ),
                }),
                initialized: AtomicBool::new(false),
                stopped: AtomicBool::new(false),
                ready: Notify::new(),
                shutdown: Notify::new(),
            }),
        }
    }

    /// Call after Services has been managed. Detection remains opt-in, and every
    /// authenticated request checks the current setting before emitting a hint.
    pub fn initialize(&self, app: AppHandle) {
        if self.inner.initialized.swap(true, Ordering::AcqRel) {
            return;
        }
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(detail) = service.serve(app).await {
                service.publish(BrowserConnection::unavailable(&detail));
            }
        });
    }

    /// Only expose this through a trusted NextCue-window command. The frontend
    /// must not persist or log the connection token with ordinary app settings.
    pub async fn get_connection(&self) -> BrowserConnection {
        let wait = async {
            loop {
                let notification = self.inner.ready.notified();
                {
                    let state = self
                        .inner
                        .state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    if state.ready {
                        return state.connection.clone();
                    }
                }
                notification.await;
            }
        };
        tokio::time::timeout(Duration::from_secs(8), wait)
            .await
            .unwrap_or_else(|_| {
                BrowserConnection::unavailable(
                    "The browser connection is not ready. Try again shortly.",
                )
            })
    }

    pub fn shutdown(&self) {
        self.inner.stopped.store(true, Ordering::Release);
        self.inner.shutdown.notify_waiters();
        self.publish(BrowserConnection::unavailable("NextCue is shutting down."));
    }

    fn publish(&self, connection: BrowserConnection) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.ready = true;
        state.connection = connection;
        drop(state);
        self.inner.ready.notify_waiters();
    }

    async fn serve(&self, app: AppHandle) -> Result<(), String> {
        let directory = self.inner.directory.clone();
        let keychain_service = format!("{}.browser", app.config().identifier);
        let pairing_directory = directory.clone();
        let preferred_port =
            tokio::task::spawn_blocking(move || read_preferred_port(&pairing_directory))
                .await
                .map_err(|_| "Could not prepare the browser connection.".to_owned())?;

        let listener = match preferred_port {
            Some(port) => match TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await {
                Ok(listener) => listener,
                Err(_) => TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                    .await
                    .map_err(|_| "Could not start the local browser connection.".to_owned())?,
            },
            None => TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .map_err(|_| "Could not start the local browser connection.".to_owned())?,
        };
        let port = listener
            .local_addr()
            .map_err(|_| "Could not read the browser connection port.")?
            .port();
        // An old extension code may now point at a different local process.
        // If the port changes, invalidate that credential instead of reusing it
        // on the new listener. Normal same-port launches keep the pairing.
        let reuse = preferred_port == Some(port);
        let token = tokio::task::spawn_blocking(move || pairing_token(&keychain_service, reuse))
            .await
            .map_err(|_| "Could not prepare the browser connection.".to_owned())??;
        write_preferred_port(&directory, port);
        let detail = if preferred_port.is_some_and(|preferred| preferred != port) {
            "The previous port was occupied. Copy this connection code to the browser companion again."
        } else {
            "The companion sends only meeting-page hints to this computer. Opening a page does not confirm an active call."
        };
        if self.inner.stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        self.publish(BrowserConnection {
            available: true,
            port: Some(port),
            token: Some(token.clone()),
            detail: Some(detail.into()),
        });
        let token = Arc::new(token);
        let recent = Arc::new(Mutex::new(RecentPages::default()));
        let capacity = Arc::new(Semaphore::new(MAX_CONNECTIONS));

        loop {
            let stop = self.inner.shutdown.notified();
            if self.inner.stopped.load(Ordering::Acquire) {
                return Ok(());
            }
            let accepted = tokio::select! {
                _ = stop => return Ok(()),
                result = listener.accept() => result,
            };
            let (stream, peer) =
                accepted.map_err(|_| "The local browser connection stopped.".to_owned())?;
            if !peer.ip().is_loopback() {
                continue;
            }
            let Ok(permit) = capacity.clone().try_acquire_owned() else {
                continue;
            };
            let app = app.clone();
            let token = token.clone();
            let recent = recent.clone();
            let inner = self.inner.clone();
            tauri::async_runtime::spawn(async move {
                let _permit = permit;
                let _ = tokio::time::timeout(REQUEST_TIMEOUT, async {
                    handle_connection(
                        stream,
                        port,
                        &token,
                        &recent,
                        &inner.stopped,
                        || {
                            app.state::<crate::Services>()
                                .background
                                .current()
                                .meeting_detection
                        },
                        |hint| {
                            app.state::<crate::Services>()
                                .background
                                .emit_meeting_hint(&app, hint);
                            true
                        },
                    )
                    .await
                })
                .await;
            });
        }
    }
}

fn pairing_token(service: &str, reuse: bool) -> Result<String, String> {
    let entry = keyring::Entry::new(service, "browser-pairing")
        .map_err(|_| "Could not open the operating-system keychain for the browser companion.")?;
    if reuse {
        match entry.get_password() {
            Ok(token) if valid_pairing_token(&token) => return Ok(token),
            Ok(_) | Err(keyring::Error::NoEntry) => {}
            Err(_) => return Err("The browser companion cannot access the operating-system keychain. Unlock it and restart NextCue.".into()),
        }
    }
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token = URL_SAFE_NO_PAD.encode(bytes);
    entry.set_password(&token).map_err(|_| {
        "Could not save the browser pairing token in the operating-system keychain."
    })?;
    Ok(token)
}

fn valid_pairing_token(token: &str) -> bool {
    token.len() == 43
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn read_preferred_port(directory: &std::path::Path) -> Option<u16> {
    let file = std::fs::File::open(directory.join("browser-port.json")).ok()?;
    let mut bytes = Vec::new();
    file.take(33).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 32 {
        return None;
    }
    serde_json::from_slice::<u16>(&bytes)
        .ok()
        .filter(|port| *port > 1023)
}

fn write_preferred_port(directory: &std::path::Path, port: u16) {
    // This file contains only a public port number, never the pairing token.
    if std::fs::create_dir_all(directory).is_ok() {
        let _ = std::fs::write(directory.join("browser-port.json"), port.to_string());
    }
}

#[derive(Debug)]
struct RequestHead {
    method: String,
    origin: Option<String>,
    authorization: Option<String>,
    length: usize,
}

fn extension_origin(origin: &str) -> bool {
    let identifier = if let Some(identifier) = origin.strip_prefix("chrome-extension://") {
        return identifier.len() == 32
            && identifier.bytes().all(|byte| (b'a'..=b'p').contains(&byte));
    } else if let Some(identifier) = origin.strip_prefix("moz-extension://") {
        identifier
    } else {
        return false;
    };
    identifier.len() == 36 && uuid::Uuid::parse_str(identifier).is_ok()
}

fn parse_head(text: &str, port: u16) -> Result<RequestHead, ()> {
    if text.len() > MAX_HEADERS {
        return Err(());
    }
    let mut lines = text.split("\r\n");
    let line = lines.next().ok_or(())?;
    let mut words = line.split(' ');
    let method = words.next().ok_or(())?;
    if !matches!(method, "POST" | "OPTIONS")
        || words.next() != Some("/v1/meeting")
        || words.next() != Some("HTTP/1.1")
        || words.next().is_some()
    {
        return Err(());
    }
    let mut headers = HashMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(())?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value.bytes().any(|byte| byte < b' ' && byte != b'\t')
        {
            return Err(());
        }
        if headers
            .insert(name.to_ascii_lowercase(), value.trim().to_owned())
            .is_some()
        {
            return Err(());
        }
    }
    if headers.get("host") != Some(&format!("127.0.0.1:{port}"))
        || headers.contains_key("transfer-encoding")
    {
        return Err(());
    }
    let origin = headers.get("origin").cloned();
    if origin
        .as_deref()
        .is_some_and(|origin| !extension_origin(origin))
    {
        return Err(());
    }
    let length = match headers.get("content-length") {
        Some(length) if length.bytes().all(|byte| byte.is_ascii_digit()) => {
            length.parse::<usize>().map_err(|_| ())?
        }
        Some(_) => return Err(()),
        None if method == "OPTIONS" => 0,
        None => return Err(()),
    };
    if length > MAX_BODY || (method == "OPTIONS" && length != 0) {
        return Err(());
    }
    if method == "POST"
        && !headers.get("content-type").is_some_and(|content_type| {
            content_type
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
        })
    {
        return Err(());
    }
    if method == "OPTIONS"
        && (origin.is_none()
            || headers
                .get("access-control-request-method")
                .map(String::as_str)
                != Some("POST"))
    {
        return Err(());
    }
    Ok(RequestHead {
        method: method.into(),
        origin,
        authorization: headers.get("authorization").cloned(),
        length,
    })
}

fn authenticated(authorization: Option<&str>, token: &str) -> bool {
    let Some(provided) = authorization.and_then(|value| value.strip_prefix("Bearer ")) else {
        return false;
    };
    if provided.len() != token.len() {
        return false;
    }
    provided
        .bytes()
        .zip(token.bytes())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageHint {
    app: String,
    id: String,
    kind: String,
}

impl PageHint {
    fn valid(&self) -> bool {
        self.kind == "browser-page"
            && matches!(
                self.app.as_str(),
                "Google Meet" | "Zoom" | "Microsoft Teams" | "Webex" | "Jitsi" | "Whereby"
            )
            && self.id.len() == 36
            && uuid::Uuid::parse_str(&self.id).is_ok()
    }
}

#[derive(Default)]
struct RecentPages {
    pages: HashMap<String, Instant>,
}

impl RecentPages {
    fn insert(&mut self, id: &str, now: Instant) -> bool {
        self.pages
            .retain(|_, seen| now.saturating_duration_since(*seen) < DEDUPE_TTL);
        if self.pages.contains_key(id) {
            return false;
        }
        // At capacity, discard the oldest ID. This bounded cache is advisory;
        // browser-side IDs also dedupe each currently open meeting page.
        if self.pages.len() >= MAX_RECENT_PAGES {
            if let Some(oldest) = self
                .pages
                .iter()
                .min_by_key(|(_, seen)| **seen)
                .map(|(id, _)| id.clone())
            {
                self.pages.remove(&oldest);
            }
        }
        self.pages.insert(id.into(), now);
        true
    }
}

async fn handle_connection<E, M>(
    mut stream: TcpStream,
    port: u16,
    token: &str,
    recent: &Mutex<RecentPages>,
    stopped: &AtomicBool,
    enabled: E,
    emit: M,
) -> Result<(), ()>
where
    E: Fn() -> bool,
    M: Fn(MeetingHint) -> bool,
{
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).await.map_err(|_| ())?;
        if read == 0 {
            return Err(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            if position + 4 > MAX_HEADERS {
                return Err(());
            }
            break position + 4;
        }
        if buffer.len() >= MAX_HEADERS {
            return Err(());
        }
    };
    let header_text = std::str::from_utf8(&buffer[..header_end]).map_err(|_| ())?;
    let head = parse_head(header_text, port)?;
    if head.method == "OPTIONS" {
        return respond(&mut stream, 204, head.origin.as_deref(), "").await;
    }
    if !authenticated(head.authorization.as_deref(), token) {
        return respond(
            &mut stream,
            401,
            head.origin.as_deref(),
            "{\"accepted\":false}",
        )
        .await;
    }
    while buffer.len() - header_end < head.length {
        let missing = head.length - (buffer.len() - header_end);
        let read = stream
            .read(&mut chunk[..missing.min(1024)])
            .await
            .map_err(|_| ())?;
        if read == 0 {
            return Err(());
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    if buffer.len() - header_end != head.length {
        return Err(());
    }
    let hint: PageHint = serde_json::from_slice(&buffer[header_end..]).map_err(|_| ())?;
    if !hint.valid() {
        return respond(
            &mut stream,
            400,
            head.origin.as_deref(),
            "{\"accepted\":false}",
        )
        .await;
    }
    if stopped.load(Ordering::Acquire) || !enabled() {
        return respond(
            &mut stream,
            200,
            head.origin.as_deref(),
            "{\"accepted\":false,\"reason\":\"disabled\"}",
        )
        .await;
    }
    let fresh = recent
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(&hint.id, Instant::now());
    if !fresh {
        return respond(
            &mut stream,
            200,
            head.origin.as_deref(),
            "{\"accepted\":false,\"reason\":\"duplicate\"}",
        )
        .await;
    }
    let emitted = emit(MeetingHint {
        detail: format!("{} meeting page opened. NextCue cannot confirm that you joined or that a call is active.", hint.app),
        app: hint.app,
        kind: "browser-page".into(),
    });
    respond(
        &mut stream,
        200,
        head.origin.as_deref(),
        if emitted {
            "{\"accepted\":true}"
        } else {
            "{\"accepted\":false}"
        },
    )
    .await
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    origin: Option<&str>,
    body: &str,
) -> Result<(), ()> {
    let status_text = match status {
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        _ => "OK",
    };
    let cors = origin.filter(|origin| extension_origin(origin)).map(|origin| format!(
        "Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Methods: POST\r\nAccess-Control-Allow-Headers: Authorization, Content-Type\r\nVary: Origin\r\n"
    )).unwrap_or_default();
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n{cors}\r\n{body}", body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|_| ())?;
    stream.shutdown().await.map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(extra: &str) -> String {
        format!("POST /v1/meeting HTTP/1.1\r\nHost: 127.0.0.1:32123\r\nContent-Type: application/json\r\nContent-Length: 2\r\n{extra}\r\n")
    }

    #[test]
    fn cors_accepts_only_exact_extension_origins() {
        assert!(extension_origin(
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop"
        ));
        assert!(extension_origin(
            "moz-extension://12345678-1234-1234-1234-123456789abc"
        ));
        for origin in [
            "null",
            "https://meet.google.com",
            "http://127.0.0.1:32123",
            "chrome-extension://short",
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop/",
            "moz-extension://not-a-uuid",
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop.evil.test",
        ] {
            assert!(!extension_origin(origin));
        }
    }

    #[test]
    fn request_validation_enforces_local_host_path_framing_and_bounds() {
        assert!(parse_head(&head(""), 32123).is_ok());
        for request in [
            head("Origin: https://evil.test\r\n"),
            head("Transfer-Encoding: chunked\r\n"),
            head("Content-Length: 2\r\n"),
            head("Broken header\r\n"),
            head("").replace("127.0.0.1:32123", "evil.test:32123"),
            head("").replace("/v1/meeting", "/v1/meeting?token=secret"),
            head("").replace("Content-Length: 2", "Content-Length: 4097"),
            head("").replace("Content-Length: 2", "Content-Length: +2"),
            head("").replace("Content-Type: application/json", "Content-Type: text/plain"),
            format!("{}{}", head(""), "x".repeat(MAX_HEADERS)),
        ] {
            assert!(parse_head(&request, 32123).is_err());
        }
    }

    #[test]
    fn preflight_requires_extension_origin_and_post_method() {
        let preflight = "OPTIONS /v1/meeting HTTP/1.1\r\nHost: 127.0.0.1:32123\r\nOrigin: chrome-extension://abcdefghijklmnopabcdefghijklmnop\r\nAccess-Control-Request-Method: POST\r\n\r\n";
        assert!(parse_head(preflight, 32123).is_ok());
        assert!(parse_head(
            &preflight.replace("Request-Method: POST", "Request-Method: GET"),
            32123
        )
        .is_err());
        assert!(parse_head(
            &preflight.replace(
                "Origin: chrome-extension://abcdefghijklmnopabcdefghijklmnop\r\n",
                ""
            ),
            32123
        )
        .is_err());
    }

    #[test]
    fn authentication_rejects_missing_wrong_prefix_and_mismatched_tokens() {
        let token = "abcdefghijklmnopabcdefghijklmnopabcdefghijk";
        assert!(valid_pairing_token(token));
        assert!(authenticated(Some(&format!("Bearer {token}")), token));
        for header in [
            None,
            Some(""),
            Some("bearer abc"),
            Some("Bearer abc"),
            Some("Bearer abcdefghijklmnopabcdefghijklmnopabcdefghijl"),
        ] {
            assert!(!authenticated(header, token));
        }
        assert!(!valid_pairing_token(&format!("{token}\n")));
    }

    #[test]
    fn message_allows_only_provider_and_random_id_and_no_url_or_contents() {
        let id = "12345678-1234-1234-1234-123456789abc";
        let valid = format!(r#"{{"app":"Google Meet","id":"{id}","kind":"browser-page"}}"#);
        assert!(serde_json::from_str::<PageHint>(&valid).unwrap().valid());
        for body in [
            valid.replace("Google Meet", "Other"),
            valid.replace("browser-page", "desktop-app"),
            valid.replace(id, "meeting-code-secret"),
            valid.replace("}", ",\"url\":\"https://meet.google.com/abc-defg-hij\"}"),
        ] {
            assert!(!serde_json::from_str::<PageHint>(&body).is_ok_and(|hint| hint.valid()));
        }
    }

    #[test]
    fn duplicate_cache_is_bounded_and_expires_without_retriggering_on_each_post() {
        let mut recent = RecentPages::default();
        let now = Instant::now();
        assert!(recent.insert("same-page", now));
        assert!(!recent.insert("same-page", now + Duration::from_secs(1)));
        assert!(recent.insert("same-page", now + DEDUPE_TTL));
        for index in 0..(MAX_RECENT_PAGES + 10) {
            recent.insert(
                &format!("page-{index}"),
                now + DEDUPE_TTL + Duration::from_secs(index as u64),
            );
        }
        assert_eq!(recent.pages.len(), MAX_RECENT_PAGES);
    }

    #[tokio::test]
    async fn loopback_requests_authenticate_dedupe_and_never_emit_while_disabled() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("mock browser bridge must bind to loopback");
        let port = listener.local_addr().unwrap().port();
        let token = "a".repeat(43);
        let emitted = Arc::new(Mutex::new(Vec::<MeetingHint>::new()));
        let enabled = Arc::new(AtomicBool::new(false));
        let server_enabled = enabled.clone();
        let server_emitted = emitted.clone();
        let server_token = token.clone();
        let server = tokio::spawn(async move {
            let recent = Mutex::new(RecentPages::default());
            let stopped = AtomicBool::new(false);
            for _ in 0..7 {
                let (stream, _) = listener.accept().await.unwrap();
                let _ = handle_connection(
                    stream,
                    port,
                    &server_token,
                    &recent,
                    &stopped,
                    || server_enabled.load(Ordering::Acquire),
                    |hint| {
                        server_emitted.lock().unwrap().push(hint);
                        true
                    },
                )
                .await;
            }
        });
        let body = r#"{"app":"Google Meet","id":"12345678-1234-1234-1234-123456789abc","kind":"browser-page"}"#;
        async fn post(port: u16, token: &str, body: &str, origin: &str) -> String {
            let mut stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
                .await
                .unwrap();
            let request = format!(
                "POST /v1/meeting HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAuthorization: Bearer {token}\r\n{origin}\r\n{body}",
                body.len()
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            response
        }
        let extension_origin = "Origin: chrome-extension://abcdefghijklmnopabcdefghijklmnop\r\n";
        assert!(post(port, "wrong", body, extension_origin)
            .await
            .contains("401 Unauthorized"));
        assert!(post(port, &token, body, extension_origin)
            .await
            .contains("\"reason\":\"disabled\""));
        assert!(emitted.lock().unwrap().is_empty());
        enabled.store(true, Ordering::Release);
        // Privileged extension fetch implementations may omit Origin. The
        // bearer token is still required; web-origin requests fail CORS/parser.
        assert!(post(port, &token, body, "")
            .await
            .contains("\"accepted\":true"));
        assert!(post(port, &token, body, extension_origin)
            .await
            .contains("\"reason\":\"duplicate\""));
        let with_url = body.replace('}', ",\"url\":\"https://meet.google.com/abc-defg-hij\"}");
        assert!(post(port, &token, &with_url, extension_origin)
            .await
            .is_empty());
        assert!(post(port, &token, body, "Origin: https://evil.test\r\n")
            .await
            .is_empty());
        let unknown = body.replace("Google Meet", "Unknown provider");
        assert!(post(port, &token, &unknown, extension_origin)
            .await
            .contains("400 Bad Request"));
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
        let events = emitted.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].app, "Google Meet");
        assert_eq!(events[0].kind, "browser-page");
        assert!(!events[0].detail.contains("abc-defg-hij"));
    }
}
