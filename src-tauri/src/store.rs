use crate::ollama::{local_url, DEFAULT_MODEL, DEFAULT_URL};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};
use uuid::Uuid;

pub const MAX_STATE_BYTES: usize = 12 * 1024 * 1024;
pub const MAX_CONTEXT_BYTES: usize = 100000;

pub fn default_state() -> Value {
    json!({ "sessions": [], "settings": { "model": DEFAULT_MODEL, "ollamaUrl": DEFAULT_URL, "language": "en", "autoAnswer": true, "audioSource": "microphone", "inputDevice": "" } })
}

fn string<'a>(
    value: &'a Value,
    key: &str,
    maximum: usize,
    required: bool,
) -> Result<&'a str, String> {
    let text = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be text."))?;
    if text.len() > maximum || (required && text.trim().is_empty()) {
        return Err(format!("{key} is invalid or exceeds its size limit."));
    }
    Ok(text)
}
fn number(value: &Value, key: &str) -> Result<f64, String> {
    let number = value
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("{key} must be a number."))?;
    if !number.is_finite() || number < 0.0 || number > 9007199254740991.0 {
        return Err(format!("{key} is out of range."));
    }
    Ok(number)
}
fn enumeration(value: &Value, key: &str, options: &[&str]) -> Result<(), String> {
    let field = value.get(key).and_then(Value::as_str).unwrap_or_default();
    if !options.contains(&field) {
        return Err(format!("{key} has an invalid value."));
    }
    Ok(())
}
fn array<'a>(value: &'a Value, key: &str, maximum: usize) -> Result<&'a Vec<Value>, String> {
    let array = value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key} must be a list."))?;
    if array.len() > maximum {
        return Err(format!("{key} exceeds its size limit."));
    }
    Ok(array)
}

pub fn validate_session(value: &Value) -> Result<(), String> {
    if !value.is_object() {
        return Err("Session must be an object.".into());
    }
    string(value, "id", 128, true)?;
    for key in ["title", "company", "role"] {
        string(value, key, 256, false)?;
    }
    enumeration(value, "mode", &["Interview", "Practice", "Meeting"])?;
    enumeration(value, "status", &["ready", "active", "ended"])?;
    number(value, "createdAt")?;
    number(value, "duration")?;
    string(value, "context", MAX_CONTEXT_BYTES, false)?;
    string(value, "instructions", 20000, false)?;
    for entry in array(value, "transcript", 5000)? {
        string(entry, "id", 128, true)?;
        string(entry, "text", 16000, false)?;
        number(entry, "time")?;
        enumeration(entry, "source", &["microphone", "system", "typed"])?;
    }
    for answer in array(value, "answers", 1000)? {
        string(answer, "id", 128, true)?;
        string(answer, "question", 16000, false)?;
        string(answer, "text", 200000, false)?;
        string(answer, "model", 256, false)?;
        number(answer, "time")?;
        if answer.get("durationMs").is_some() {
            number(answer, "durationMs")?;
        }
    }
    if value.get("summary").is_some() {
        string(value, "summary", 200000, false)?;
    }
    Ok(())
}

pub fn validate_state(value: &Value) -> Result<(), String> {
    let settings = value
        .get("settings")
        .filter(|v| v.is_object())
        .ok_or("Settings must be an object.")?;
    string(settings, "model", 256, true)?;
    local_url(string(settings, "ollamaUrl", 256, true)?)?;
    string(settings, "language", 32, true)?;
    string(settings, "inputDevice", 512, false)?;
    enumeration(settings, "audioSource", &["microphone", "system"])?;
    if !settings.get("autoAnswer").is_some_and(Value::is_boolean) {
        return Err("autoAnswer must be a boolean.".into());
    }
    for key in ["captureProtection", "jevEnabled"] {
        if settings.get(key).is_some_and(|value| !value.is_boolean()) {
            return Err(format!("{key} must be a boolean."));
        }
    }
    if settings.get("answerProvider").is_some() {
        enumeration(settings, "answerProvider", &["local", "chatgpt", "claude"])?;
    }
    if settings.get("chatgptModel").is_some() {
        string(settings, "chatgptModel", 256, false)?;
    }
    for key in ["answerShortcut", "visibilityShortcut"] {
        if settings.get(key).is_some() {
            string(settings, key, 128, true)?;
        }
    }
    let mut ids = HashSet::new();
    for session in array(value, "sessions", 300)? {
        validate_session(session)?;
        if !ids.insert(string(session, "id", 128, true)?) {
            return Err("Session IDs must be unique.".into());
        }
    }
    if serde_json::to_vec(value).map_err(|e| e.to_string())?.len() > MAX_STATE_BYTES {
        return Err(
            "Saved sessions exceed 12 MB. Export older sessions before removing them.".into(),
        );
    }
    Ok(())
}

pub struct StateStore {
    directory: PathBuf,
    transaction: Mutex<()>,
    read_failed: AtomicBool,
}
impl StateStore {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            transaction: Mutex::new(()),
            read_failed: AtomicBool::new(false),
        }
    }
    pub async fn load(&self) -> Result<Value, String> {
        let _guard = self.transaction.lock().await;
        let file = self.directory.join("state.json");
        let bytes = match read_bounded(&file, MAX_STATE_BYTES).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(default_state())
            }
            Err(error) => {
                self.read_failed.store(true, Ordering::SeqCst);
                return Err(format!(
                    "Cannot read saved sessions at {}: {error}",
                    file.display()
                ));
            }
        };
        let result = serde_json::from_slice::<Value>(&bytes)
            .map_err(|e| e.to_string())
            .and_then(|state| {
                validate_state(&state)?;
                Ok(state)
            });
        if let Err(error) = result.as_ref() {
            self.read_failed.store(true, Ordering::SeqCst);
            return Err(format!(
                "Saved sessions could not be read. Your file is preserved at {}. {error}",
                file.display()
            ));
        }
        self.read_failed.store(false, Ordering::SeqCst);
        result
    }
    pub async fn save(&self, state: Value) -> Result<(), String> {
        validate_state(&state)?;
        let _guard = self.transaction.lock().await;
        if self.read_failed.load(Ordering::SeqCst) {
            return Err("Your saved session file needs repair before it can be overwritten. The original file has been preserved.".into());
        }
        fs::create_dir_all(&self.directory)
            .await
            .map_err(|e| format!("Cannot create session storage: {e}"))?;
        let bytes = serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err("Saved sessions exceed the storage limit.".into());
        }
        let temporary = self
            .directory
            .join(format!(".state-{}.tmp", Uuid::new_v4()));
        let result = async {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options
                .open(&temporary)
                .await
                .map_err(|e| format!("Cannot save sessions: {e}"))?;
            file.write_all(&bytes)
                .await
                .map_err(|e| format!("Cannot save sessions: {e}"))?;
            file.sync_all()
                .await
                .map_err(|e| format!("Cannot flush saved sessions: {e}"))?;
            drop(file);
            fs::rename(&temporary, self.directory.join("state.json"))
                .await
                .map_err(|e| format!("Cannot finish saving sessions: {e}"))?;
            Ok(())
        }
        .await;
        let _ = fs::remove_file(&temporary).await;
        result
    }
    pub async fn ollama_url(&self) -> Result<String, String> {
        let state = self.load().await?;
        Ok(state["settings"]["ollamaUrl"]
            .as_str()
            .unwrap_or(DEFAULT_URL)
            .to_owned())
    }
}

pub async fn read_bounded(path: &Path, maximum: usize) -> std::io::Result<Vec<u8>> {
    let file = fs::File::open(path).await?;
    if !file.metadata().await?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Choose a regular text file.",
        ));
    }
    let mut bytes = vec![];
    file.take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > maximum {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "File exceeds its size limit.",
        ));
    }
    Ok(bytes)
}

pub fn session_markdown(session: &Value) -> Result<String, String> {
    validate_session(session)?;
    let clean = |key| {
        session[key]
            .as_str()
            .unwrap_or_default()
            .replace(['\r', '\n'], " ")
    };
    let title = clean("title");
    let duration = session["duration"].as_f64().unwrap_or_default() as u64;
    let created = session["createdAt"].as_f64().unwrap_or_default() as u64;
    let mut output = format!(
        "# {}\n\n- Date: {}\n- Mode: {}\n- Company: {}\n- Role: {}\n- Duration: {}m {}s\n\n",
        if title.is_empty() {
            "Interview session"
        } else {
            &title
        },
        utc_date(created),
        clean("mode"),
        clean("company"),
        clean("role"),
        duration / 60,
        duration % 60
    );
    if let Some(summary) = session
        .get("summary")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        output.push_str(&format!("## Session recap\n\n{summary}\n\n"));
    }
    output.push_str("## Transcript\n\n");
    for entry in session["transcript"].as_array().unwrap() {
        let timestamp = entry["time"].as_f64().unwrap_or_default() as u64;
        output.push_str(&format!(
            "**{} · {}**\n\n{}\n\n",
            utc_date(timestamp),
            entry["source"].as_str().unwrap_or_default(),
            entry["text"].as_str().unwrap_or_default()
        ));
    }
    if session["transcript"].as_array().unwrap().is_empty() {
        output.push_str("_No transcript recorded._\n\n");
    }
    output.push_str("## Suggested answers\n\n");
    for answer in session["answers"].as_array().unwrap() {
        let question = answer["question"]
            .as_str()
            .unwrap_or_default()
            .replace(['\r', '\n'], " ");
        output.push_str(&format!(
            "### {question}\n\n{}\n\n_Model: {}_\n\n",
            answer["text"].as_str().unwrap_or_default(),
            answer["model"].as_str().unwrap_or_default()
        ));
    }
    if session["answers"].as_array().unwrap().is_empty() {
        output.push_str("_No answers generated._\n");
    }
    Ok(output)
}

fn utc_date(timestamp_ms: u64) -> String {
    // Civil-date conversion from epoch days, keeping exports independent of machine locale.
    let seconds = timestamp_ms / 1000;
    let z = (seconds / 86400) as i64 + 719468;
    let era = z / 146097;
    let day = z - era * 146097;
    let year_of_era = (day - day / 1460 + day / 36524 - day / 146096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let date = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = month_index + if month_index < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    format!(
        "{year:04}-{month:02}-{date:02} {:02}:{:02}:{:02} UTC",
        seconds % 86400 / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Value {
        json!({ "id":"session1", "title":"Practice", "company":"Acme", "role":"Engineer", "mode":"Practice", "createdAt":0, "status":"ready", "duration":65, "context":"", "instructions":"", "transcript":[{"id":"t1","text":"A question?","time":5000,"source":"typed"}], "answers":[{"id":"a1","question":"A question?","text":"My answer.","time":6000,"model":DEFAULT_MODEL}] })
    }
    #[test]
    fn validation_rejects_remote_settings_and_duplicate_sessions() {
        let mut state = default_state();
        state["settings"]["ollamaUrl"] = json!("http://example.com");
        assert!(validate_state(&state).is_err());
        state = default_state();
        state["sessions"] = json!([session(), session()]);
        assert!(validate_state(&state).is_err());
        state["sessions"] = json!([session()]);
        assert!(validate_state(&state).is_ok());
    }
    #[test]
    fn optional_provider_preferences_roundtrip_and_reject_invalid_values() {
        let mut state = default_state();
        state["settings"]["answerProvider"] = json!("chatgpt");
        state["settings"]["chatgptModel"] = json!("account-model");
        state["settings"]["jevEnabled"] = json!(true);
        assert!(validate_state(&state).is_ok());
        state["settings"]["answerProvider"] = json!("unexpected");
        assert!(validate_state(&state).is_err());
        state["settings"]["answerProvider"] = json!("local");
        state["settings"]["jevEnabled"] = json!("true");
        assert!(validate_state(&state).is_err());
        state["settings"]["jevEnabled"] = json!(false);
        state["settings"]["chatgptModel"] = json!("x".repeat(257));
        assert!(validate_state(&state).is_err());
    }
    #[tokio::test]
    async fn atomic_save_roundtrips_and_invalid_state_preserves_existing_file() {
        let directory = std::env::temp_dir().join(format!("nextcue-test-{}", Uuid::new_v4()));
        let store = StateStore::new(directory.clone());
        assert_eq!(store.load().await.unwrap(), default_state());
        let mut state = default_state();
        state["sessions"] = json!([session()]);
        store.save(state.clone()).await.unwrap();
        assert_eq!(store.load().await.unwrap(), state);
        let original = fs::read(directory.join("state.json")).await.unwrap();
        assert!(store.save(json!({})).await.is_err());
        assert_eq!(
            fs::read(directory.join("state.json")).await.unwrap(),
            original
        );
        fs::remove_dir_all(directory).await.unwrap();
    }
    #[tokio::test]
    async fn corrupted_storage_is_preserved_without_overwriting() {
        let directory = std::env::temp_dir().join(format!("nextcue-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).await.unwrap();
        fs::write(directory.join("state.json"), b"broken")
            .await
            .unwrap();
        let store = StateStore::new(directory.clone());
        assert!(store.load().await.is_err());
        assert!(store.save(default_state()).await.is_err());
        assert_eq!(
            fs::read(directory.join("state.json")).await.unwrap(),
            b"broken"
        );
        fs::remove_dir_all(directory).await.unwrap();
    }
    #[test]
    fn markdown_export_contains_transcript_answers_and_utc_date() {
        let output = session_markdown(&session()).unwrap();
        assert!(output.contains("1970-01-01 00:00:00 UTC"));
        assert!(output.contains("1970-01-01 00:00:05 UTC · typed"));
        assert!(output.contains("My answer."));
        assert!(output.contains("1m 5s"));
        assert_eq!(utc_date(86400000), "1970-01-02 00:00:00 UTC");
    }
}
