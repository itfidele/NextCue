//! Subscription-backed Responses API integration using OpenAI's public OAuth flow.
//! Credentials never cross the webview bridge. Each account has its own OS-keyring
//! record; the disk registry contains identity mappings and public model names only.
use crate::{
    context,
    ollama::{ChatRequest, ChatResult, TokenEvent},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use fs2::FileExt;
use futures_util::{
    future::{AbortHandle, Abortable},
    StreamExt,
};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use rand::{rngs::OsRng, RngCore};
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    path::PathBuf,
    process::Stdio,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex as AsyncMutex,
};
use url::Url;

const ISSUER: &str = "https://auth.openai.com";
const AUTHORIZE: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
const JWKS: &str = "https://auth.openai.com/.well-known/jwks.json";
const DISCOVERY: &str = "https://auth.openai.com/.well-known/openid-configuration";
const RESOURCE: &str = "https://api.openai.com/v1";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const DYNAMIC_CLIENT: &str = "dynamic_agent_client";
const MAX_JSON: usize = 1024 * 1024;
const MAX_EVENT: usize = 512 * 1024;
const MAX_STREAM: usize = 8 * 1024 * 1024;
const KEYRING_CHUNK: usize = 2000; // Windows Credential Manager has a small per-item limit.
const MAX_ANSWER: usize = 200000; // Matches the persisted session answer limit.

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChatgptModel {
    pub slug: String,
    pub display_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatgptAccount {
    pub id: String,
    pub label: String,
    pub email: Option<String>,
    pub connected: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatgptStatus {
    pub connected: bool,
    pub email: Option<String>,
    pub plan_usage: bool,
    pub models: Vec<ChatgptModel>,
    pub error: Option<String>,
    pub accounts: Vec<ChatgptAccount>,
    pub active_account_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Registration {
    id: String,
    label: String,
    email: Option<String>,
    client_id: String,
    subject: String,
    #[serde(default)]
    models: Vec<ChatgptModel>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
struct Registry {
    host_id: String,
    #[serde(default)]
    accounts: Vec<Registration>,
    active_account_id: Option<String>,
    #[serde(default)]
    pending_registration: Option<PendingRegistration>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PendingRegistration {
    id: String,
    client_id: String,
    label: String,
}

// Deliberately no Debug implementation: token-bearing structures must not be logged.
#[derive(Clone, Serialize, Deserialize)]
struct Credentials {
    access_token: String,
    refresh_token: Option<String>,
    id_token: String,
    expires_at: u64,
    scopes: Vec<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<u64>,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct Identity {
    sub: String,
    nonce: Option<String>,
    email: Option<String>,
    iat: u64,
}

#[derive(Serialize, Deserialize)]
struct KeyringManifest {
    version: String,
    chunks: usize,
}

struct SignInAttempt {
    state: String,
    nonce: String,
    verifier: String,
    redirect_uri: String,
    registration: Option<Registration>,
}

struct Callback {
    code: String,
    client_id: String,
}

pub struct ChatgptService {
    directory: PathBuf,
    keyring_service: String,
    client: Client,
    registry: AsyncMutex<Registry>,
    credential_operations: AsyncMutex<()>,
    sign_in: Mutex<Option<(String, AbortHandle)>>,
    requests: Mutex<HashMap<String, AbortHandle>>,
    startup_error: Mutex<Option<String>>,
}

impl ChatgptService {
    pub fn new(directory: PathBuf, keyring_service: String) -> Result<Self, String> {
        let (registry, startup_error) = match read_registry(&directory) {
            Ok(registry) => (registry, None),
            Err(error) => (Registry::default(), Some(error)),
        };
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|_| "Cannot initialize the ChatGPT connection.")?;
        Ok(Self {
            directory,
            keyring_service,
            client,
            registry: AsyncMutex::new(registry),
            credential_operations: AsyncMutex::new(()),
            sign_in: Mutex::new(None),
            requests: Mutex::new(HashMap::new()),
            startup_error: Mutex::new(startup_error),
        })
    }

    /// Loading status never contacts OpenAI unless the user explicitly refreshes.
    pub async fn status(&self, refresh_models: bool) -> ChatgptStatus {
        let mut error = self
            .startup_error
            .lock()
            .ok()
            .and_then(|error| error.clone());
        if refresh_models {
            if let Err(message) = self.refresh_models().await {
                error = Some(message);
            }
        }
        let registry = self.registry.lock().await.clone();
        let mut accounts = Vec::new();
        let mut connected = false;
        let mut plan_usage = false;
        let mut email = None;
        let mut models = vec![];
        for account in &registry.accounts {
            let credentials = match self.load_credentials(&account.id) {
                Ok(value) => value,
                Err(message) => {
                    if error.is_none() {
                        error = Some(message);
                    }
                    None
                }
            };
            let available = credentials.is_some();
            accounts.push(ChatgptAccount {
                id: account.id.clone(),
                label: account.label.clone(),
                email: account.email.clone(),
                connected: available,
            });
            if registry.active_account_id.as_deref() == Some(account.id.as_str()) {
                connected = available;
                plan_usage = credentials.as_ref().is_some_and(plan_enabled);
                email = account.email.clone();
                if connected {
                    models = account.models.clone();
                }
            }
        }
        ChatgptStatus {
            connected,
            email,
            plan_usage,
            models,
            error,
            accounts,
            active_account_id: registry.active_account_id,
        }
    }

    pub async fn sign_in(&self, account_id: Option<String>) -> Result<ChatgptStatus, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let (abort, registration) = AbortHandle::new_pair();
        {
            let mut pending = self
                .sign_in
                .lock()
                .map_err(|_| "ChatGPT sign-in is unavailable.")?;
            if pending.is_some() {
                return Err("A ChatGPT sign-in is already open. Finish or cancel it first.".into());
            }
            *pending = Some((id.clone(), abort));
        }
        let result = tokio::time::timeout(
            Duration::from_secs(180),
            Abortable::new(self.sign_in_inner(account_id), registration),
        )
        .await;
        if let Ok(mut pending) = self.sign_in.lock() {
            if pending
                .as_ref()
                .is_some_and(|(pending_id, _)| pending_id == &id)
            {
                *pending = None;
            }
        }
        match result {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("ChatGPT sign-in was cancelled.".into()),
            Err(_) => {
                Err("ChatGPT sign-in timed out. Choose Continue with ChatGPT to try again.".into())
            }
        }
    }

    async fn sign_in_inner(&self, account_id: Option<String>) -> Result<ChatgptStatus, String> {
        let (host_id, selected) = {
            let _operation = self.credential_operations.lock().await;
            let _process = self.process_lock().await?;
            let mut registry = self.registry.lock().await;
            *registry = self.latest_registry()?;
            if registry.host_id.is_empty() {
                registry.host_id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
                self.save_registry(&registry)?;
            }
            let selected = match account_id {
                Some(id) => Some(
                    registry
                        .accounts
                        .iter()
                        .find(|account| account.id == id)
                        .cloned()
                        .ok_or("Choose a saved ChatGPT account.")?,
                ),
                None => {
                    if registry.accounts.len() >= 20 {
                        return Err("The saved ChatGPT account limit has been reached.".into());
                    }
                    registry
                        .pending_registration
                        .as_ref()
                        .map(|pending| Registration {
                            id: pending.id.clone(),
                            label: pending.label.clone(),
                            email: None,
                            client_id: pending.client_id.clone(),
                            subject: String::new(),
                            models: vec![],
                        })
                }
            };
            (registry.host_id.clone(), selected)
        };
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| "Cannot open the local ChatGPT sign-in callback.")?;
        let port = listener
            .local_addr()
            .map_err(|_| "Cannot open the local ChatGPT sign-in callback.")?
            .port();
        let attempt = SignInAttempt {
            state: random_value(32),
            nonce: random_value(32),
            verifier: random_value(64),
            redirect_uri: format!("http://127.0.0.1:{port}/auth/callback"),
            registration: selected.clone(),
        };
        let hints = selected
            .as_ref()
            .map(|account| self.load_credentials(&account.id))
            .transpose()?
            .flatten();
        let url = authorization_url(
            &attempt,
            &host_id,
            hints
                .as_ref()
                .map(|credentials| credentials.id_token.as_str()),
        )?;
        let mut url = url;
        if hints
            .as_ref()
            .is_some_and(|credentials| !plan_enabled(credentials))
        {
            // Explicit reauthorization after declined plan usage asks for consent again.
            url.query_pairs_mut().append_pair("prompt", "consent");
        }
        open_browser(url).await?;
        let callback = listen_callback(listener, &attempt).await?;
        // The issued dynamic client survives an invalid_grant/code-exchange failure;
        // no account identity becomes active until its signed ID token is validated.
        let account_id = if let Some(selected) = &selected {
            selected.id.clone()
        } else {
            let _operation = self.credential_operations.lock().await;
            let _process = self.process_lock().await?;
            let mut registry = self.registry.lock().await;
            *registry = self.latest_registry()?;
            let pending = PendingRegistration {
                id: uuid::Uuid::new_v4().to_string(),
                client_id: callback.client_id.clone(),
                label: format!("ChatGPT account {}", registry.accounts.len() + 1),
            };
            let id = pending.id.clone();
            registry.pending_registration = Some(pending);
            self.save_registry(&registry)?;
            id
        };
        let response = self
            .client
            .post(TOKEN)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", callback.client_id.as_str()),
                ("code", callback.code.as_str()),
                ("code_verifier", attempt.verifier.as_str()),
                ("redirect_uri", attempt.redirect_uri.as_str()),
                ("resource", RESOURCE),
            ])
            .send()
            .await
            .map_err(network_error)?;
        let token_response: TokenResponse = serde_json::from_value(read_json(response).await?)
            .map_err(|_| "ChatGPT returned an invalid sign-in response.")?;
        let id_token = token_response
            .id_token
            .as_deref()
            .ok_or("ChatGPT did not return a verified identity.")?;
        let keys: JwkSet = serde_json::from_value(
            read_json(self.client.get(JWKS).send().await.map_err(network_error)?).await?,
        )
        .map_err(|_| "Cannot verify the ChatGPT signing keys.")?;
        let identity = verify_identity(id_token, &keys, &callback.client_id, Some(&attempt.nonce))?;
        if selected
            .as_ref()
            .is_some_and(|account| !account.subject.is_empty() && account.subject != identity.sub)
        {
            return Err("The signed-in ChatGPT identity does not match the selected account. Add it as a different account instead.".into());
        }
        let credentials = credentials_from_response(token_response, None)?;
        {
            let _operation = self.credential_operations.lock().await;
            let _process = self.process_lock().await?;
            let mut registry = self.registry.lock().await;
            *registry = self.latest_registry()?;
            // A newly added registration may use the same email as another workspace;
            // only validated subject + issued client ID can identify that registration.
            if let Some(existing) = registry.accounts.iter().find(|account| {
                account.client_id == callback.client_id && account.subject == identity.sub
            }) {
                if existing.id != account_id {
                    return Err("This ChatGPT account is already saved. Choose its saved account to continue.".into());
                }
            }
            self.save_credentials(&account_id, &credentials)?;
            let registration = Registration {
                id: account_id.clone(),
                label: selected
                    .as_ref()
                    .map(|account| account.label.clone())
                    .unwrap_or_else(|| format!("ChatGPT account {}", registry.accounts.len() + 1)),
                email: identity.email.filter(|email| email.len() <= 320),
                client_id: callback.client_id,
                subject: identity.sub,
                models: selected.map(|account| account.models).unwrap_or_default(),
            };
            if let Some(account) = registry
                .accounts
                .iter_mut()
                .find(|account| account.id == account_id)
            {
                *account = registration;
            } else {
                registry.accounts.push(registration);
            }
            registry.active_account_id = Some(account_id);
            if registry
                .pending_registration
                .as_ref()
                .is_some_and(|pending| pending.client_id == registration_client_id(&registry))
            {
                registry.pending_registration = None;
            }
            self.save_registry(&registry)?;
        }
        Ok(self.status(true).await)
    }

    pub fn cancel_sign_in(&self) -> Result<(), String> {
        if let Some((_, handle)) = self
            .sign_in
            .lock()
            .map_err(|_| "ChatGPT sign-in is unavailable.")?
            .as_ref()
        {
            handle.abort();
        }
        Ok(())
    }

    pub async fn select_account(&self, account_id: String) -> Result<ChatgptStatus, String> {
        self.cancel_all();
        let _operation = self.credential_operations.lock().await;
        let _process = self.process_lock().await?;
        let mut registry = self.registry.lock().await;
        *registry = self.latest_registry()?;
        if !registry
            .accounts
            .iter()
            .any(|account| account.id == account_id)
        {
            return Err("Choose a saved ChatGPT account.".into());
        }
        registry.active_account_id = Some(account_id);
        self.save_registry(&registry)?;
        drop(registry);
        drop(_process);
        drop(_operation);
        Ok(self.status(false).await)
    }

    pub async fn sign_out(&self) -> Result<ChatgptStatus, String> {
        self.cancel_sign_in()?;
        self.cancel_all();
        let _operation = self.credential_operations.lock().await;
        let _process = self.process_lock().await?;
        let registry = self.latest_registry()?;
        let mut warning = None;
        if let Some(account) = active_registration(&registry) {
            if let Some(credentials) = self.load_credentials(&account.id)? {
                // Clear local access immediately, even when remote revocation is slow.
                self.delete_credentials(&account.id)?;
                if let Some(refresh_token) = credentials.refresh_token {
                    if self
                        .revoke(&account.client_id, &refresh_token)
                        .await
                        .is_err()
                    {
                        warning = Some("Signed out locally. Remote revocation was not confirmed; disconnect NextCue in ChatGPT Settings if needed.".to_string());
                    }
                }
            }
        }
        drop(_process);
        drop(_operation);
        let mut status = self.status(false).await;
        if warning.is_some() {
            status.error = warning;
        }
        Ok(status)
    }

    async fn revoke(&self, client_id: &str, refresh_token: &str) -> Result<(), String> {
        let discovery = read_json(
            self.client
                .get(DISCOVERY)
                .timeout(Duration::from_secs(10))
                .send()
                .await
                .map_err(network_error)?,
        )
        .await?;
        if discovery["issuer"].as_str() != Some(ISSUER) {
            return Err("Invalid ChatGPT issuer.".into());
        }
        let endpoint = discovery["revocation_endpoint"]
            .as_str()
            .ok_or("No ChatGPT revocation endpoint.")?;
        let url = Url::parse(endpoint).map_err(|_| "Invalid ChatGPT revocation endpoint.")?;
        if url.scheme() != "https"
            || url.host_str() != Some("auth.openai.com")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Invalid ChatGPT revocation endpoint.".into());
        }
        for attempt in 0..2 {
            let result = self
                .client
                .post(url.clone())
                .timeout(Duration::from_secs(10))
                .form(&[
                    ("token", refresh_token),
                    ("token_type_hint", "refresh_token"),
                    ("client_id", client_id),
                ])
                .send()
                .await;
            match result {
                Ok(response) if response.status() == reqwest::StatusCode::OK => return Ok(()),
                Ok(response) if !response.status().is_server_error() => {
                    return Err("ChatGPT session revocation was not confirmed.".into())
                }
                _ if attempt == 0 => tokio::time::sleep(Duration::from_millis(250)).await,
                _ => break,
            }
        }
        Err("ChatGPT session revocation was not confirmed.".into())
    }

    async fn authorized(&self) -> Result<(Registration, Credentials), String> {
        let _operation = self.credential_operations.lock().await;
        let _process = self.process_lock().await?;
        let registry = self.latest_registry()?;
        *self.registry.lock().await = registry.clone();
        let account = active_registration(&registry)
            .cloned()
            .ok_or("Connect ChatGPT in Settings first.")?;
        let mut credentials = self
            .load_credentials(&account.id)?
            .ok_or("Continue with ChatGPT in Settings to reconnect this account.")?;
        if !plan_enabled(&credentials) {
            return Err("ChatGPT plan usage is not enabled. Continue with ChatGPT in Settings and approve plan usage.".into());
        }
        if credentials.expires_at <= unix_time() + 60 {
            let refresh_token = credentials
                .refresh_token
                .as_deref()
                .ok_or("This ChatGPT session has expired. Sign in again from Settings.")?;
            let response = self
                .client
                .post(TOKEN)
                .timeout(Duration::from_secs(30))
                .form(&[
                    ("grant_type", "refresh_token"),
                    ("client_id", account.client_id.as_str()),
                    ("refresh_token", refresh_token),
                    ("resource", RESOURCE),
                ])
                .send()
                .await
                .map_err(network_error)?;
            if !response.status().is_success() {
                let status = response.status();
                let data = response_value(response).await.unwrap_or(Value::Null);
                let code = error_code(&data);
                if matches!(
                    code,
                    Some(
                        "invalid_grant"
                            | "invalid_refresh_token"
                            | "token_expired"
                            | "refresh_token_expired"
                            | "refresh_token_invalidated"
                            | "refresh_token_reused"
                    )
                ) {
                    self.delete_credentials(&account.id)?;
                    return Err("This ChatGPT connection has expired or was disconnected. Continue with ChatGPT in Settings.".into());
                }
                return Err(provider_error(status.as_u16(), code));
            }
            let response: TokenResponse =
                serde_json::from_value(response_value(response).await?)
                    .map_err(|_| "ChatGPT returned an invalid refresh response.")?;
            if let Some(id_token) = response.id_token.as_deref() {
                let keys: JwkSet = serde_json::from_value(
                    read_json(self.client.get(JWKS).send().await.map_err(network_error)?).await?,
                )
                .map_err(|_| "Cannot verify the ChatGPT signing keys.")?;
                let identity = verify_identity(id_token, &keys, &account.client_id, None)?;
                if identity.sub != account.subject {
                    return Err(
                        "The refreshed ChatGPT identity does not match this account.".into(),
                    );
                }
            }
            credentials = credentials_from_response(response, Some(&credentials))?;
            self.save_credentials(&account.id, &credentials)?;
            if !plan_enabled(&credentials) {
                return Err("ChatGPT plan usage is no longer enabled for this connection.".into());
            }
        }
        Ok((account, credentials))
    }

    async fn refresh_models(&self) -> Result<(), String> {
        let (account, credentials) = self.authorized().await?;
        let response = self
            .client
            .get(format!("{RESOURCE}/models"))
            .bearer_auth(&credentials.access_token)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(network_error)?;
        let data = read_json(response).await?;
        let models = parse_models(&data)?;
        let _operation = self.credential_operations.lock().await;
        let _process = self.process_lock().await?;
        let mut registry = self.registry.lock().await;
        *registry = self.latest_registry()?;
        if let Some(current) = registry
            .accounts
            .iter_mut()
            .find(|item| item.id == account.id)
        {
            current.models = models;
        }
        self.save_registry(&registry)
    }

    pub async fn chat<F>(&self, request: ChatRequest, on_token: F) -> Result<ChatResult, String>
    where
        F: Fn(TokenEvent) + Send + Sync,
    {
        context::validate_request(&request)?;
        let (abort, registration) = AbortHandle::new_pair();
        {
            let mut requests = self
                .requests
                .lock()
                .map_err(|_| "ChatGPT answer service is unavailable.")?;
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
            Abortable::new(self.chat_inner(request, on_token), registration),
        )
        .await;
        if let Ok(mut requests) = self.requests.lock() {
            requests.remove(&id);
        }
        match result {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Answer generation was cancelled.".into()),
            Err(_) => Err("ChatGPT took too long to respond. Try again.".into()),
        }
    }

    async fn chat_inner<F>(&self, request: ChatRequest, on_token: F) -> Result<ChatResult, String>
    where
        F: Fn(TokenEvent) + Send + Sync,
    {
        let started = Instant::now();
        let (account, credentials) = self.authorized().await?;
        if !account
            .models
            .iter()
            .any(|model| model.slug == request.model)
        {
            return Err("Refresh the ChatGPT models in Settings and choose a model available to this account.".into());
        }
        let (system, user) = context::prompts(&request);
        let response = self.client.post(format!("{RESOURCE}/responses")).bearer_auth(&credentials.access_token)
            .json(&json!({"model":request.model,"instructions":system,"input":[{"role":"user","content":user}],"store":false,"stream":true}))
            .send().await.map_err(network_error)?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let data = response_value(response).await.unwrap_or(Value::Null);
            return Err(provider_error(status, error_code(&data)));
        }
        let mut stream = response.bytes_stream();
        let mut parser = ResponsesParser::default();
        while let Some(chunk) = stream.next().await {
            for token in parser.feed(&chunk.map_err(network_error)?)? {
                on_token(TokenEvent {
                    request_id: request.request_id.clone(),
                    token,
                });
            }
            if parser.completed {
                break;
            }
        }
        parser.finish()?;
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
            .map_err(|_| "ChatGPT answer service is unavailable.")?
            .get(request_id)
        {
            handle.abort();
        }
        Ok(())
    }
    fn cancel_all(&self) {
        if let Ok(requests) = self.requests.lock() {
            for handle in requests.values() {
                handle.abort();
            }
        }
    }
    pub fn shutdown(&self) {
        let _ = self.cancel_sign_in();
        self.cancel_all();
    }

    fn entry(&self, id: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(&self.keyring_service, id).map_err(|_| secure_storage_error())
    }
    fn load_credentials(&self, id: &str) -> Result<Option<Credentials>, String> {
        let manifest = match self.entry(id)?.get_password() {
            Ok(value) => value,
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(_) => return Err(secure_storage_error()),
        };
        let manifest: KeyringManifest =
            serde_json::from_str(&manifest).map_err(|_| secure_storage_error())?;
        if uuid::Uuid::parse_str(&manifest.version).is_err()
            || manifest.chunks == 0
            || manifest.chunks > 64
        {
            return Err(secure_storage_error());
        }
        let mut bytes = Vec::new();
        for index in 0..manifest.chunks {
            let chunk = self
                .entry(&format!("{id}:{}:{index}", manifest.version))?
                .get_secret()
                .map_err(|_| secure_storage_error())?;
            if chunk.is_empty() || chunk.len() > KEYRING_CHUNK {
                return Err(secure_storage_error());
            }
            bytes.extend(chunk);
        }
        let credentials: Credentials =
            serde_json::from_slice(&bytes).map_err(|_| secure_storage_error())?;
        validate_credentials(&credentials)?;
        Ok(Some(credentials))
    }
    fn save_credentials(&self, id: &str, credentials: &Credentials) -> Result<(), String> {
        let bytes = serde_json::to_vec(credentials).map_err(|_| secure_storage_error())?;
        if bytes.is_empty() || bytes.len() > KEYRING_CHUNK * 64 {
            return Err(secure_storage_error());
        }
        let old = self
            .entry(id)?
            .get_password()
            .ok()
            .and_then(|value| serde_json::from_str::<KeyringManifest>(&value).ok());
        let manifest = KeyringManifest {
            version: uuid::Uuid::new_v4().to_string(),
            chunks: bytes.chunks(KEYRING_CHUNK).count(),
        };
        for (index, chunk) in bytes.chunks(KEYRING_CHUNK).enumerate() {
            if self
                .entry(&format!("{id}:{}:{index}", manifest.version))?
                .set_secret(chunk)
                .is_err()
            {
                self.delete_chunks(id, &manifest);
                return Err(secure_storage_error());
            }
        }
        let value = serde_json::to_string(&manifest).map_err(|_| secure_storage_error())?;
        if self.entry(id)?.set_password(&value).is_err() {
            self.delete_chunks(id, &manifest);
            return Err(secure_storage_error());
        }
        if let Some(old) = old {
            self.delete_chunks(id, &old);
        }
        Ok(())
    }
    fn delete_chunks(&self, id: &str, manifest: &KeyringManifest) {
        if uuid::Uuid::parse_str(&manifest.version).is_ok() && manifest.chunks <= 64 {
            for index in 0..manifest.chunks {
                if let Ok(entry) = self.entry(&format!("{id}:{}:{index}", manifest.version)) {
                    let _ = entry.delete_credential();
                }
            }
        }
    }
    fn delete_credentials(&self, id: &str) -> Result<(), String> {
        let entry = self.entry(id)?;
        let manifest = entry
            .get_password()
            .ok()
            .and_then(|value| serde_json::from_str::<KeyringManifest>(&value).ok());
        if let Some(manifest) = manifest {
            self.delete_chunks(id, &manifest);
        }
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(secure_storage_error()),
        }
    }
    fn save_registry(&self, registry: &Registry) -> Result<(), String> {
        std::fs::create_dir_all(&self.directory)
            .map_err(|_| "Cannot save the ChatGPT account registry.")?;
        let temporary = self
            .directory
            .join(format!("chatgpt-accounts-{}.tmp", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| {
            use std::io::Write;
            let mut file = options.open(&temporary)?;
            file.write_all(&serde_json::to_vec(registry).map_err(std::io::Error::other)?)?;
            file.sync_all()?;
            std::fs::rename(&temporary, self.directory.join("chatgpt-accounts.json"))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
            return Err("Cannot save the ChatGPT account registry.".into());
        }
        if let Ok(mut error) = self.startup_error.lock() {
            *error = None;
        }
        Ok(())
    }
    fn latest_registry(&self) -> Result<Registry, String> {
        // A malformed optional registry cannot break local startup. Replacing it
        // requires an explicit sign-in rather than a background metadata mutation.
        read_registry(&self.directory)
    }
    async fn process_lock(&self) -> Result<File, String> {
        std::fs::create_dir_all(&self.directory)
            .map_err(|_| "Cannot open the ChatGPT account storage.")?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(self.directory.join("chatgpt-session.lock"))
            .map_err(|_| "Cannot lock the ChatGPT session.")?;
        for _ in 0..300 {
            if file.try_lock_exclusive().is_ok() {
                return Ok(file);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("Another NextCue process is updating this ChatGPT session. Try again shortly.".into())
    }
}

fn read_registry(directory: &std::path::Path) -> Result<Registry, String> {
    let path = directory.join("chatgpt-accounts.json");
    if !path.exists() {
        return Ok(Registry::default());
    }
    let metadata =
        std::fs::metadata(&path).map_err(|_| "Cannot read the ChatGPT account registry.")?;
    if metadata.len() > MAX_JSON as u64 {
        return Err("The ChatGPT account registry is too large.".into());
    }
    let bytes = std::fs::read(path).map_err(|_| "Cannot read the ChatGPT account registry.")?;
    let registry: Registry =
        serde_json::from_slice(&bytes).map_err(|_| "Cannot read the ChatGPT account registry.")?;
    if registry.accounts.len() > 20
        || (!registry.host_id.is_empty()
            && !registry
                .host_id
                .strip_prefix("urn:uuid:")
                .is_some_and(|value| uuid::Uuid::parse_str(value).is_ok()))
        || registry.accounts.iter().any(|account| {
            uuid::Uuid::parse_str(&account.id).is_err()
                || !valid_client_id(&account.client_id)
                || account.subject.is_empty()
                || account.subject.len() > 512
                || account.label.len() > 200
                || account.models.len() > 500
        })
        || registry
            .pending_registration
            .as_ref()
            .is_some_and(|pending| {
                uuid::Uuid::parse_str(&pending.id).is_err()
                    || !valid_client_id(&pending.client_id)
                    || pending.label.len() > 200
            })
    {
        return Err("The ChatGPT account registry is invalid.".into());
    }
    Ok(registry)
}

fn active_registration(registry: &Registry) -> Option<&Registration> {
    registry
        .accounts
        .iter()
        .find(|account| Some(account.id.as_str()) == registry.active_account_id.as_deref())
}
fn registration_client_id(registry: &Registry) -> &str {
    active_registration(registry)
        .map(|account| account.client_id.as_str())
        .unwrap_or_default()
}
fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn random_value(bytes: usize) -> String {
    let mut output = vec![0; bytes];
    OsRng.fill_bytes(&mut output);
    URL_SAFE_NO_PAD.encode(output)
}
fn plan_enabled(credentials: &Credentials) -> bool {
    credentials
        .scopes
        .iter()
        .any(|scope| scope == "chatgpt.tokens.use.direct")
        && credentials
            .scopes
            .iter()
            .any(|scope| scope == "resource.invoke")
}
fn validate_credentials(credentials: &Credentials) -> Result<(), String> {
    if (credentials.access_token.is_empty() && plan_enabled(credentials))
        || credentials.access_token.len() > 32768
        || credentials.id_token.is_empty()
        || credentials.id_token.len() > 32768
        || credentials
            .refresh_token
            .as_ref()
            .is_some_and(|token| token.is_empty() || token.len() > 32768)
        || credentials.scopes.len() > 128
        || credentials
            .scopes
            .iter()
            .any(|scope| scope.len() > 256 || scope.chars().any(char::is_whitespace))
        || credentials.expires_at > unix_time() + 86400
    {
        return Err(secure_storage_error());
    }
    Ok(())
}
fn valid_client_id(value: &str) -> bool {
    value.starts_with("oaiapp_")
        && value.len() <= 256
        && value.len() > 7
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn authorization_url(
    attempt: &SignInAttempt,
    host_id: &str,
    id_token_hint: Option<&str>,
) -> Result<Url, String> {
    let mut url = Url::parse(AUTHORIZE).map_err(|_| "Cannot prepare ChatGPT sign-in.")?;
    let client_id = attempt
        .registration
        .as_ref()
        .map(|account| account.client_id.as_str())
        .unwrap_or(DYNAMIC_CLIENT);
    {
        let mut query = url.query_pairs_mut();
        query.extend_pairs([
            ("client_id", client_id),
            ("ext_agent_host_id", host_id),
            ("response_type", "code"),
            ("redirect_uri", attempt.redirect_uri.as_str()),
            ("scope", SCOPES),
            ("resource", RESOURCE),
            ("state", attempt.state.as_str()),
            ("nonce", attempt.nonce.as_str()),
            ("code_challenge_method", "S256"),
        ]);
        query.append_pair(
            "code_challenge",
            &URL_SAFE_NO_PAD.encode(Sha256::digest(attempt.verifier.as_bytes())),
        );
        if attempt.registration.is_none() {
            query.append_pair("agent_name_hint", "NextCue");
        } else {
            if let Some(hint) = id_token_hint {
                query.append_pair("id_token_hint", hint);
            }
            if let Some(email) = attempt
                .registration
                .as_ref()
                .and_then(|account| account.email.as_deref())
            {
                query.append_pair("login_hint", email);
            }
        }
    }
    Ok(url)
}

fn parse_callback(target: &str, attempt: &SignInAttempt) -> Result<Callback, String> {
    if !target.starts_with("/auth/callback?") {
        return Err("Unexpected callback path.".into());
    }
    let callback = Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| "Invalid ChatGPT callback.")?;
    if callback.path() != "/auth/callback" || callback.fragment().is_some() {
        return Err("Invalid ChatGPT callback.".into());
    };
    let mut pairs = HashMap::new();
    for (key, value) in callback.query_pairs() {
        if pairs.insert(key.to_string(), value.to_string()).is_some() {
            return Err("Duplicate callback parameter.".into());
        }
    }
    if pairs.get("state") != Some(&attempt.state) {
        return Err("ChatGPT sign-in state did not match.".into());
    }
    if pairs.contains_key("error") {
        return Err("ChatGPT sign-in was declined. Try again when ready.".into());
    }
    let code = pairs
        .get("code")
        .filter(|code| !code.is_empty() && code.len() <= 4096)
        .cloned()
        .ok_or("ChatGPT did not return an authorization code.")?;
    let client_id = match &attempt.registration {
        Some(account) => {
            if pairs
                .get("client_id")
                .is_some_and(|id| id != &account.client_id)
            {
                return Err(
                    "The ChatGPT callback changed the selected account registration.".into(),
                );
            }
            account.client_id.clone()
        }
        None => pairs
            .get("client_id")
            .filter(|id| valid_client_id(id))
            .cloned()
            .ok_or("ChatGPT did not finish registering this app.")?,
    };
    Ok(Callback { code, client_id })
}

async fn listen_callback(
    listener: TcpListener,
    attempt: &SignInAttempt,
) -> Result<Callback, String> {
    let expected_host = Url::parse(&attempt.redirect_uri)
        .map_err(|_| "Invalid callback listener.")?
        .authority()
        .to_string();
    loop {
        let (mut stream, peer) = listener
            .accept()
            .await
            .map_err(|_| "The local ChatGPT callback stopped.")?;
        if !peer.ip().is_loopback() {
            continue;
        }
        let mut request = vec![];
        let read = tokio::time::timeout(Duration::from_secs(5), async {
            let mut buffer = [0u8; 1024];
            while !request.windows(4).any(|chunk| chunk == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).await?;
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                if request.len() > 8192 {
                    break;
                }
            }
            Ok::<(), std::io::Error>(())
        })
        .await;
        let mut callback = Err("Invalid ChatGPT callback request.".to_string());
        let mut matched_state = false;
        if read.is_ok() && request.len() <= 8192 {
            if let Ok(text) = std::str::from_utf8(&request) {
                let mut lines = text.split("\r\n");
                let mut first = lines.next().unwrap_or_default().split_whitespace();
                let method = first.next();
                let target = first.next();
                let version = first.next();
                let hosts: Vec<&str> = lines
                    .filter_map(|line| line.split_once(':'))
                    .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
                    .map(|(_, value)| value.trim())
                    .collect();
                if method == Some("GET")
                    && version == Some("HTTP/1.1")
                    && hosts == [expected_host.as_str()]
                {
                    if let Some(target) = target {
                        matched_state = Url::parse(&format!("http://127.0.0.1{target}"))
                            .ok()
                            .is_some_and(|url| {
                                url.query_pairs()
                                    .filter(|(key, _)| key == "state")
                                    .collect::<Vec<_>>()
                                    == [("state".into(), attempt.state.as_str().into())]
                            });
                        callback = parse_callback(target, attempt);
                    }
                }
            }
        }
        let (status, message) = if callback.is_ok() {
            (
                "200 OK",
                "Return to NextCue to finish signing in. You can close this tab.",
            )
        } else {
            (
                "400 Bad Request",
                "This sign-in callback could not be verified. Return to NextCue.",
            )
        };
        let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{message}",message.len());
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            stream.write_all(response.as_bytes()),
        )
        .await;
        if callback.is_ok() || matched_state {
            return callback;
        }
    }
}

fn verify_identity(
    token: &str,
    keys: &JwkSet,
    client_id: &str,
    expected_nonce: Option<&str>,
) -> Result<Identity, String> {
    let header =
        decode_header(token).map_err(|_| "The ChatGPT identity token could not be verified.")?;
    if header.alg != Algorithm::RS256 {
        return Err("The ChatGPT identity signing algorithm is unsupported.".into());
    }
    let kid = header
        .kid
        .ok_or("The ChatGPT identity token has no signing key.")?;
    let key = keys
        .find(&kid)
        .ok_or("The ChatGPT identity signing key could not be verified.")?;
    let decoding_key =
        DecodingKey::from_jwk(key).map_err(|_| "The ChatGPT signing key is invalid.")?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[client_id]);
    validation.set_required_spec_claims(&["sub", "exp", "iat", "iss", "aud"]);
    validation.leeway = 5;
    validation.validate_nbf = true;
    let identity = decode::<Identity>(token, &decoding_key, &validation)
        .map_err(|_| {
            "The ChatGPT identity signature, issuer, audience, or expiration could not be verified."
        })?
        .claims;
    if identity.sub.is_empty()
        || identity.sub.len() > 512
        || identity.iat > unix_time() + 5
        || expected_nonce.is_some_and(|nonce| identity.nonce.as_deref() != Some(nonce))
    {
        return Err("The ChatGPT identity did not match this sign-in attempt.".into());
    }
    Ok(identity)
}

fn credentials_from_response(
    response: TokenResponse,
    previous: Option<&Credentials>,
) -> Result<Credentials, String> {
    let scopes: Vec<String> = match response.scope {
        Some(scope) if scope.len() <= 4096 => {
            scope.split_whitespace().map(str::to_string).collect()
        }
        Some(_) => return Err("ChatGPT returned invalid granted permissions.".into()),
        None => previous
            .map(|value| value.scopes.clone())
            .unwrap_or_default(),
    };
    let plan_usage = scopes
        .iter()
        .any(|scope| scope == "chatgpt.tokens.use.direct")
        && scopes.iter().any(|scope| scope == "resource.invoke");
    let access_token = response.access_token.unwrap_or_default();
    if access_token.len() > 32768 || (plan_usage && access_token.is_empty()) {
        return Err("ChatGPT did not return a usable access token.".into());
    }
    if !access_token.is_empty()
        && !response
            .token_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("Bearer"))
    {
        return Err("ChatGPT returned an unsupported credential type.".into());
    }
    let expires_in = response
        .expires_in
        .or_else(|| {
            if access_token.is_empty() {
                Some(3600)
            } else {
                None
            }
        })
        .filter(|expiry| *expiry > 0 && *expiry <= 86400)
        .ok_or("ChatGPT returned an invalid token lifetime.")?;
    let id_token = response
        .id_token
        .or_else(|| previous.map(|value| value.id_token.clone()))
        .filter(|token| !token.is_empty() && token.len() <= 32768)
        .ok_or("ChatGPT did not return an identity token.")?;
    if previous.is_some() && response.refresh_token.is_none() {
        return Err(
            "ChatGPT did not return its replacement renewable session. Reconnect in Settings."
                .into(),
        );
    }
    let refresh_token = response.refresh_token;
    if refresh_token
        .as_ref()
        .is_some_and(|token| token.is_empty() || token.len() > 32768)
    {
        return Err("ChatGPT returned an invalid renewable session.".into());
    };
    let credentials = Credentials {
        access_token,
        refresh_token,
        id_token,
        expires_at: unix_time() + expires_in,
        scopes,
    };
    validate_credentials(&credentials)?;
    Ok(credentials)
}

fn parse_models(data: &Value) -> Result<Vec<ChatgptModel>, String> {
    let models = data
        .get("models")
        .and_then(Value::as_array)
        .ok_or("ChatGPT returned an invalid account model catalog.")?;
    Ok(models
        .iter()
        .take(500)
        .filter(|model| model["visibility"].as_str() == Some("list"))
        .filter_map(|model| {
            let slug = model["slug"].as_str()?;
            let display_name = model["display_name"].as_str()?;
            if slug.is_empty()
                || slug.len() > 256
                || display_name.is_empty()
                || display_name.len() > 256
            {
                return None;
            }
            Some(ChatgptModel {
                slug: slug.to_string(),
                display_name: display_name.to_string(),
            })
        })
        .collect())
}

async fn open_browser(url: Url) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = std::process::Command::new("/usr/bin/open");
            command.arg(url.as_str());
            command
        };
        #[cfg(target_os = "linux")]
        let mut command = {
            let mut command = std::process::Command::new("/usr/bin/xdg-open");
            command.arg(url.as_str());
            command
        };
        #[cfg(target_os = "windows")]
        let mut command = {
            use std::os::windows::process::CommandExt;
            let system = std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or("Cannot find the Windows browser launcher.")?;
            let mut command = std::process::Command::new(system.join("System32/rundll32.exe"));
            command
                .args(["url.dll,FileProtocolHandler", url.as_str()])
                .creation_flags(0x08000000);
            command
        };
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            if command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|_| "Cannot open the browser for ChatGPT sign-in.")?
                .success()
            {
                Ok(())
            } else {
                Err("Cannot open the browser for ChatGPT sign-in.".to_string())
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        Err("ChatGPT sign-in requires the desktop app.".to_string())
    })
    .await
    .map_err(|_| "Cannot start ChatGPT sign-in.".to_string())?
}

fn secure_storage_error() -> String {
    "Cannot access secure OS credential storage. Unlock your keychain or install a Secret Service keyring on Linux, then try again.".into()
}
fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "The ChatGPT connection timed out. Try again.".into()
    } else if error.is_connect() {
        "Cannot reach ChatGPT. Check your internet connection.".into()
    } else {
        "The ChatGPT connection failed. Try again.".into()
    }
}
async fn response_value(response: Response) -> Result<Value, String> {
    let mut stream = response.bytes_stream();
    let mut bytes = vec![];
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(network_error)?;
        if bytes.len() + chunk.len() > MAX_JSON {
            return Err("The ChatGPT response exceeded its size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "ChatGPT returned an invalid response.".into())
}
async fn read_json(response: Response) -> Result<Value, String> {
    let status = response.status();
    let value = response_value(response).await?;
    if status.is_success() {
        Ok(value)
    } else {
        Err(provider_error(status.as_u16(), error_code(&value)))
    }
}
fn error_code(value: &Value) -> Option<&str> {
    value["error"]["code"]
        .as_str()
        .or_else(|| value["error"].as_str())
}
fn provider_error(status: u16, code: Option<&str>) -> String {
    match code {
        Some("subscription_sharing_usage_limit_exceeded")=>"The ChatGPT usage limit for this app has been reached. Check ChatGPT Settings → Usage before trying again.".into(),
        Some("subscription_sharing_user_not_eligible")=>"This ChatGPT account or workspace is not eligible for plan usage in NextCue. Check its plan and workspace policy.".into(),
        Some("subscription_sharing_usage_unavailable"|"subscription_sharing_user_unavailable")=>"ChatGPT cannot check plan usage right now. Try again later.".into(),
        Some("subscription_sharing_unsupported_capability")=>"This request or model is not supported by ChatGPT plan usage. Choose another available model.".into(),
        Some("subscription_sharing_route_not_supported"|"chatpass_v2_scope_not_authorized"|"chatpass_v2_invalid_authorization_context")=>"ChatGPT plan usage did not authorize this request. Check the connection and granted permissions in Settings.".into(),
        Some("subscription_sharing_invalid_user")=>"ChatGPT could not verify this subscriber connection. Sign in again from Settings.".into(),
        Some("invalid_client")=>"ChatGPT did not accept this app registration. Reconnect the saved account in Settings.".into(),
        _=>match status {401=>"ChatGPT did not accept this account's permission. Reconnect in Settings.".into(),403=>"ChatGPT denied this request because of account, region, or workspace policy.".into(),429=>"ChatGPT is limiting requests. Wait before trying again.".into(),503=>"ChatGPT plan routing is temporarily unavailable. Try again later.".into(),_=>format!("ChatGPT returned HTTP {status}. Try again or check the connection in Settings.")}
    }
}

#[derive(Default)]
struct ResponsesParser {
    buffer: Vec<u8>,
    received: usize,
    text: String,
    completed: bool,
}
impl ResponsesParser {
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        if self.completed {
            return Ok(vec![]);
        }
        self.received += bytes.len();
        if self.received > MAX_STREAM {
            return Err("The ChatGPT stream exceeded its size limit.".into());
        }
        self.buffer.extend_from_slice(bytes);
        let mut tokens = vec![];
        while let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=index).collect();
            if line.len() > MAX_EVENT {
                return Err("A ChatGPT event exceeded its size limit.".into());
            }
            let line = std::str::from_utf8(&line)
                .map_err(|_| "ChatGPT returned an invalid event encoding.")?
                .trim_end_matches(['\r', '\n']);
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim_start();
                if data == "[DONE]" {
                    continue;
                }
                let value: Value = serde_json::from_str(data)
                    .map_err(|_| "ChatGPT returned an invalid stream event.")?;
                match value["type"].as_str() {
                    Some("response.output_text.delta")=>{let token=value["delta"].as_str().ok_or("ChatGPT returned an invalid text event.")?; if self.text.len()+token.len()>MAX_ANSWER {return Err("The ChatGPT answer exceeds its size limit. Try a shorter request.".into());} self.text.push_str(token); tokens.push(token.to_string());},
                    Some("response.completed")=>{ self.completed=true; break; },
                    Some("response.failed")=>return Err(provider_error(503,error_code(&value["response"]))),
                    Some("response.incomplete")=>return Err("ChatGPT stopped before completing this answer. Try again with a shorter request.".into()),
                    Some("error")=>return Err(provider_error(400,value["code"].as_str().or_else(||error_code(&value)))),
                    _=>{},
                }
            }
        }
        if self.buffer.len() > MAX_EVENT {
            return Err("A ChatGPT event exceeded its size limit.".into());
        }
        Ok(tokens)
    }
    fn finish(&self) -> Result<(), String> {
        if !self.completed {
            Err("The ChatGPT stream ended before the answer was completed. Try again.".into())
        } else if self.text.trim().is_empty() {
            Err("ChatGPT completed without a text answer. Try another model.".into())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn attempt() -> SignInAttempt {
        SignInAttempt {
            state: "expected".into(),
            nonce: "nonce".into(),
            verifier: "verifier".into(),
            redirect_uri: "http://127.0.0.1:12345/auth/callback".into(),
            registration: None,
        }
    }
    #[test]
    fn authorization_uses_dynamic_client_and_pkce() {
        let attempt = attempt();
        let url = authorization_url(&attempt, "urn:uuid:host", None).unwrap();
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(pairs["client_id"], DYNAMIC_CLIENT);
        assert_eq!(pairs["agent_name_hint"], "NextCue");
        assert_eq!(pairs["redirect_uri"], attempt.redirect_uri);
        assert_eq!(
            pairs["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(b"verifier"))
        );
        assert!(pairs["scope"].contains("chatgpt.tokens.use.direct"));
    }
    #[test]
    fn callback_rejects_state_duplicates_missing_registration_and_errors() {
        let attempt = attempt();
        for target in [
            "/auth/callback?code=secret&state=wrong&client_id=oaiapp_a",
            "/auth/callback?code=secret&state=expected&state=expected&client_id=oaiapp_a",
            "/auth/callback?code=secret&state=expected",
            "/auth/callback?error=access_denied&state=expected",
            "/callback?code=secret&state=expected&client_id=oaiapp_a",
        ] {
            assert!(parse_callback(target, &attempt).is_err());
        }
        let callback = parse_callback(
            "/auth/callback?code=secret&state=expected&client_id=oaiapp_a",
            &attempt,
        )
        .unwrap();
        assert_eq!(callback.client_id, "oaiapp_a");
    }
    #[test]
    fn returning_callback_cannot_replace_registration() {
        let mut attempt = attempt();
        attempt.registration = Some(Registration {
            id: "id".into(),
            label: "account".into(),
            email: None,
            client_id: "oaiapp_a".into(),
            subject: "sub".into(),
            models: vec![],
        });
        assert!(parse_callback(
            "/auth/callback?code=secret&state=expected&client_id=oaiapp_b",
            &attempt
        )
        .is_err());
        assert_eq!(
            parse_callback("/auth/callback?code=secret&state=expected", &attempt)
                .unwrap()
                .client_id,
            "oaiapp_a"
        );
        let url = authorization_url(&attempt, "host", Some("secret-id-hint")).unwrap();
        assert!(!url.query_pairs().any(|(key, _)| key == "agent_name_hint"));
    }
    #[test]
    fn identity_never_accepts_unsigned_or_hmac_token() {
        let keys = JwkSet { keys: vec![] };
        for token in [
            "eyJhbGciOiJub25lIn0.eyJzdWIiOiJhdHRhY2tlciJ9.",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJhdHRhY2tlciJ9.c2lnbmF0dXJl",
        ] {
            assert!(verify_identity(token, &keys, "oaiapp_a", Some("nonce")).is_err());
        }
    }
    #[test]
    fn scopes_do_not_confuse_identity_with_plan_usage() {
        let credentials = Credentials {
            access_token: "secret".into(),
            refresh_token: None,
            id_token: "secret".into(),
            expires_at: 0,
            scopes: vec!["openid".into(), "resource.invoke".into()],
        };
        assert!(!plan_enabled(&credentials));
    }
    #[test]
    fn account_catalog_filters_visibility_and_preserves_order() {
        let models=parse_models(&json!({"models":[{"slug":"first","display_name":"First","visibility":"list"},{"slug":"hidden","display_name":"Hidden","visibility":"hide"},{"slug":"second","display_name":"Second","visibility":"list"}]})).unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.slug.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    }
    #[test]
    fn stream_handles_split_utf8_and_requires_completed() {
        let mut parser = ResponsesParser::default();
        let event = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Héllo\"}\r\n\r\n"
            .as_bytes();
        for chunk in event.chunks(3) {
            parser.feed(chunk).unwrap();
        }
        assert_eq!(parser.text, "Héllo");
        assert!(parser.finish().is_err());
        parser
            .feed(b"data: {\"type\":\"response.completed\"}\n\n")
            .unwrap();
        assert!(parser.finish().is_ok());
    }
    #[test]
    fn usage_limit_after_deltas_is_a_failure() {
        let mut parser = ResponsesParser::default();
        parser
            .feed(b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n")
            .unwrap();
        let error=parser.feed(b"data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\"}}}\n\n").unwrap_err();
        assert!(error.contains("usage limit"));
        assert!(!parser.completed);
    }
    #[test]
    fn incomplete_or_oversized_stream_cannot_succeed() {
        assert!(ResponsesParser::default()
            .feed(b"data: {\"type\":\"response.incomplete\"}\n\n")
            .is_err());
        assert!(ResponsesParser::default()
            .feed(&vec![b'x'; MAX_EVENT + 1])
            .is_err());
    }
    #[test]
    fn signed_identity_requires_issuer_audience_expiration_nonce_and_signature() {
        // This generated private key is only a test fixture, never an app credential.
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!(
            "../test-fixtures/chatgpt-test-key.pem"
        ))
        .unwrap();
        let keys: JwkSet =
            serde_json::from_str(include_str!("../test-fixtures/chatgpt-test-jwks.json")).unwrap();
        let mut header = jsonwebtoken::Header::new(Algorithm::RS256);
        header.kid = Some("fixture".into());
        let claims = json!({"iss":ISSUER,"aud":"oaiapp_a","sub":"verified-subject","iat":unix_time(),"exp":unix_time()+600,"nonce":"nonce","email":"test@example.com"});
        let token = jsonwebtoken::encode(&header, &claims, &key).unwrap();
        assert_eq!(
            verify_identity(&token, &keys, "oaiapp_a", Some("nonce"))
                .unwrap()
                .sub,
            "verified-subject"
        );
        for (field, value) in [
            ("iss", json!("https://attacker.example")),
            ("aud", json!("oaiapp_other")),
            ("exp", json!(unix_time() - 100)),
            ("nonce", json!("wrong")),
            ("iat", json!(unix_time() + 100)),
            ("sub", json!("")),
        ] {
            let mut changed = claims.clone();
            changed[field] = value;
            let token = jsonwebtoken::encode(&header, &changed, &key).unwrap();
            assert!(
                verify_identity(&token, &keys, "oaiapp_a", Some("nonce")).is_err(),
                "accepted altered {field}"
            );
        }
        let mut bytes = token.into_bytes();
        let last = bytes.len() - 4;
        bytes[last] = if bytes[last] == b'a' { b'b' } else { b'a' };
        assert!(verify_identity(
            std::str::from_utf8(&bytes).unwrap(),
            &keys,
            "oaiapp_a",
            Some("nonce")
        )
        .is_err());
    }
    #[test]
    fn refresh_replaces_tokens_together_and_retains_scopes_when_omitted() {
        let previous = Credentials {
            access_token: "old-access".into(),
            refresh_token: Some("old-refresh".into()),
            id_token: "old-id".into(),
            expires_at: 0,
            scopes: vec!["resource.invoke".into(), "chatgpt.tokens.use.direct".into()],
        };
        let response = TokenResponse {
            access_token: Some("new-access".into()),
            refresh_token: Some("new-refresh".into()),
            id_token: None,
            token_type: Some("Bearer".into()),
            expires_in: Some(3600),
            scope: None,
        };
        let credentials = credentials_from_response(response, Some(&previous)).unwrap();
        assert_eq!(credentials.access_token, "new-access");
        assert_eq!(credentials.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(credentials.id_token, "old-id");
        assert!(plan_enabled(&credentials));
        assert!(credentials.expires_at > unix_time());
    }
    #[test]
    fn verified_identity_without_plan_permission_remains_connected_but_cannot_infer() {
        let response = TokenResponse {
            access_token: None,
            refresh_token: None,
            id_token: Some("already-verified-id-token".into()),
            token_type: None,
            expires_in: None,
            scope: Some("openid profile email".into()),
        };
        let credentials = credentials_from_response(response, None).unwrap();
        assert!(!plan_enabled(&credentials));
        assert!(credentials.access_token.is_empty());
        assert!(validate_credentials(&credentials).is_ok());
    }
    #[test]
    fn refresh_requires_rotating_replacement_refresh_token() {
        let previous = Credentials {
            access_token: "old-access".into(),
            refresh_token: Some("old-refresh".into()),
            id_token: "old-id".into(),
            expires_at: 0,
            scopes: vec!["resource.invoke".into(), "chatgpt.tokens.use.direct".into()],
        };
        let response = TokenResponse {
            access_token: Some("new-access".into()),
            refresh_token: None,
            id_token: None,
            token_type: Some("Bearer".into()),
            expires_in: Some(3600),
            scope: None,
        };
        assert!(credentials_from_response(response, Some(&previous)).is_err());
    }
    #[test]
    fn pending_registration_has_no_authenticated_account() {
        let registry = Registry {
            host_id: format!("urn:uuid:{}", uuid::Uuid::new_v4()),
            pending_registration: Some(PendingRegistration {
                id: uuid::Uuid::new_v4().to_string(),
                client_id: "oaiapp_retry".into(),
                label: "Pending account".into(),
            }),
            ..Registry::default()
        };
        let decoded: Registry =
            serde_json::from_str(&serde_json::to_string(&registry).unwrap()).unwrap();
        assert!(decoded.accounts.is_empty());
        assert!(active_registration(&decoded).is_none());
        assert_eq!(
            decoded.pending_registration.unwrap().client_id,
            "oaiapp_retry"
        );
    }
    #[test]
    fn credential_validation_rejects_oversized_or_malformed_secrets() {
        let mut credentials = Credentials {
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            id_token: "id".into(),
            expires_at: unix_time() + 3600,
            scopes: vec!["openid".into()],
        };
        assert!(validate_credentials(&credentials).is_ok());
        credentials.access_token = "x".repeat(32769);
        assert!(validate_credentials(&credentials).is_err());
        credentials.access_token = "access".into();
        credentials.scopes = vec!["bad scope".into()];
        assert!(validate_credentials(&credentials).is_err());
    }
    #[test]
    fn answer_limit_is_checked_before_emitting_excess_text() {
        let mut parser = ResponsesParser::default();
        let event = format!(
            "data: {}\n\n",
            json!({"type":"response.output_text.delta","delta":"x".repeat(MAX_ANSWER+1)})
        );
        assert!(parser.feed(event.as_bytes()).is_err());
        assert!(parser.text.is_empty());
    }
    #[tokio::test]
    async fn cancelling_pending_sign_in_aborts_the_attempt() {
        let directory =
            std::env::temp_dir().join(format!("nextcue-oauth-cancel-{}", uuid::Uuid::new_v4()));
        let service =
            ChatgptService::new(directory, "nextcue-test-no-keyring-access".into()).unwrap();
        let (handle, registration) = AbortHandle::new_pair();
        *service.sign_in.lock().unwrap() = Some(("attempt".into(), handle));
        service.cancel_sign_in().unwrap();
        assert!(Abortable::new(std::future::pending::<()>(), registration)
            .await
            .is_err());
    }
    #[tokio::test]
    async fn loopback_ignores_forged_state_then_accepts_real_callback() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut attempt = attempt();
        attempt.redirect_uri = format!("http://127.0.0.1:{port}/auth/callback");
        let task = tokio::spawn(async move { listen_callback(listener, &attempt).await });
        for state in ["forged", "expected"] {
            let mut stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
                .await
                .unwrap();
            stream.write_all(format!("GET /auth/callback?code=secret&state={state}&client_id=oaiapp_a HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.starts_with(if state == "forged" {
                "HTTP/1.1 400"
            } else {
                "HTTP/1.1 200"
            }));
        }
        assert_eq!(task.await.unwrap().unwrap().client_id, "oaiapp_a");
    }
}
