//! Native tray controls keep the one workspace webview alive. Audio only starts
//! after an explicit tray/shortcut action or an enabled meeting action in React.
use crate::{
    companion::CompanionMode,
    meeting::{MeetingDetector, MeetingHint},
    Services,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{menu::MenuItem, AppHandle, Emitter, Manager};

fn enabled_by_default() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MeetingAction {
    #[default]
    None,
    Prompt,
    ListenNotes,
    ListenAnswer,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackgroundConfiguration {
    #[serde(default = "enabled_by_default")]
    pub run_in_background: bool,
    #[serde(default)]
    pub start_hidden: bool,
    #[serde(default)]
    pub meeting_detection: bool,
    #[serde(default)]
    pub meeting_action: MeetingAction,
}

impl Default for BackgroundConfiguration {
    fn default() -> Self {
        Self {
            run_in_background: true,
            start_hidden: false,
            meeting_detection: false,
            meeting_action: MeetingAction::None,
        }
    }
}

impl BackgroundConfiguration {
    pub fn from_state(state: &Value) -> Self {
        let Some(settings) = state.get("settings").and_then(Value::as_object) else {
            return Self::default();
        };
        let mut saved = serde_json::Map::new();
        for key in [
            "runInBackground",
            "startHidden",
            "meetingDetection",
            "meetingAction",
        ] {
            if let Some(value) = settings.get(key) {
                saved.insert(key.into(), value.clone());
            }
        }
        serde_json::from_value(Value::Object(saved)).unwrap_or_default()
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundStatus {
    #[serde(flatten)]
    config: BackgroundConfiguration,
    tray_available: bool,
    detection_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundAction {
    Show,
    Settings,
    MeetingTrigger,
    Listen,
    Notes,
    Answers,
    Pause,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionSource {
    Tray,
    Shortcut,
    #[cfg(target_os = "macos")]
    Dock,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundActivity {
    #[default]
    Manual,
    Notes,
    Answers,
}

fn activity_label(listening: bool, mode: BackgroundActivity) -> &'static str {
    if !listening {
        return "NextCue · Paused";
    }
    match mode {
        BackgroundActivity::Manual => "NextCue · Listening",
        BackgroundActivity::Notes => "NextCue · Listening · Meeting notes",
        BackgroundActivity::Answers => "NextCue · Listening · Question answers",
    }
}

#[derive(Clone, Copy, Serialize)]
struct ActionEvent {
    action: BackgroundAction,
    source: ActionSource,
}

pub struct BackgroundService {
    configuration: Mutex<BackgroundConfiguration>,
    tray_available: AtomicBool,
    detail: Mutex<Option<String>>,
    tray_status: Mutex<Option<MenuItem<tauri::Wry>>>,
    frontend_ready: AtomicBool,
    pending_quit: AtomicBool,
    pending_actions: Mutex<Vec<ActionEvent>>,
    pending_hint: Mutex<Option<MeetingHint>>,
    stopped: Arc<AtomicBool>,
}

impl BackgroundService {
    pub fn new(configuration: BackgroundConfiguration) -> Self {
        Self {
            configuration: Mutex::new(configuration),
            tray_available: AtomicBool::new(false),
            detail: Mutex::new(None),
            tray_status: Mutex::new(None),
            frontend_ready: AtomicBool::new(false),
            pending_quit: AtomicBool::new(false),
            pending_actions: Mutex::new(Vec::new()),
            pending_hint: Mutex::new(None),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn current(&self) -> BackgroundConfiguration {
        self.configuration
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default()
    }

    pub fn configure(
        &self,
        configuration: BackgroundConfiguration,
    ) -> Result<BackgroundStatus, String> {
        *self
            .configuration
            .lock()
            .map_err(|_| "Cannot update background settings.")? = configuration;
        Ok(self.status())
    }

    pub fn status(&self) -> BackgroundStatus {
        BackgroundStatus {
            config: self.current(),
            tray_available: self.tray_available.load(Ordering::SeqCst),
            detection_available: MeetingDetector::available(),
            detail: self.detail.lock().ok().and_then(|detail| detail.clone()),
        }
    }

    pub fn close_to_tray(&self) -> bool {
        self.current().run_in_background && self.tray_available.load(Ordering::SeqCst)
    }

    pub fn ready(&self, app: &AppHandle) {
        // This lock also serializes an action arriving as listeners become ready.
        let Ok(mut pending) = self.pending_actions.lock() else {
            return;
        };
        self.frontend_ready.store(true, Ordering::SeqCst);
        if self.pending_quit.swap(false, Ordering::SeqCst) {
            pending.clear();
            if let Ok(mut hint) = self.pending_hint.lock() {
                hint.take();
            }
            let _ = app.emit("before-quit", ());
            return;
        }
        for event in pending.drain(..) {
            let _ = app.emit("background-action", event);
        }
        if let Ok(mut hint) = self.pending_hint.lock() {
            if let Some(hint) = hint.take() {
                if self.current().meeting_detection {
                    let _ = app.emit("meeting-hint", hint);
                }
            }
        }
    }

    pub fn request_quit(&self, app: &AppHandle) {
        let Ok(_pending) = self.pending_actions.lock() else {
            return;
        };
        if !self.frontend_ready.load(Ordering::SeqCst) {
            self.pending_quit.store(true, Ordering::SeqCst);
        }
        // Quit handlers mount before saved data finishes loading. Emit now so
        // an unreadable state file cannot prevent quitting; keep the startup
        // flag as recovery if the listener itself has not mounted yet.
        let _ = app.emit("before-quit", ());
    }

    pub fn emit_action(&self, app: &AppHandle, action: BackgroundAction, source: ActionSource) {
        let event = ActionEvent { action, source };
        let Ok(mut pending) = self.pending_actions.lock() else {
            return;
        };
        if self.frontend_ready.load(Ordering::SeqCst) {
            let _ = app.emit("background-action", event);
        } else {
            if pending.len() == 16 {
                pending.remove(0);
            }
            pending.push(event);
        }
    }

    pub fn emit_meeting_hint(&self, app: &AppHandle, hint: MeetingHint) {
        let Ok(_pending) = self.pending_actions.lock() else {
            return;
        };
        if !self.current().meeting_detection {
            return;
        }
        if self.frontend_ready.load(Ordering::SeqCst) {
            let _ = app.emit("meeting-hint", hint);
        } else if let Ok(mut pending_hint) = self.pending_hint.lock() {
            // Retain one startup suggestion rather than create several sessions
            // if multiple meeting apps/pages open while the webview is loading.
            *pending_hint = Some(hint);
        }
    }

    pub fn initialize(&self, app: &AppHandle) {
        let result = install_tray(app);
        let available = result.is_ok();
        self.tray_available.store(available, Ordering::SeqCst);
        app.state::<Services>()
            .companion
            .set_tray_recovery(available);
        match result {
            Ok(status) => {
                if let Ok(mut tray_status) = self.tray_status.lock() {
                    *tray_status = Some(status);
                }
            }
            Err(error) => {
                if let Ok(mut detail) = self.detail.lock() {
                    *detail = Some(format!("The system tray is unavailable. Close will quit safely, and hide/show shortcuts remain available. {error}"));
                }
            }
        }
        let stopped = self.stopped.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut detector = MeetingDetector::new();
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(8));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            while !stopped.load(Ordering::SeqCst) {
                interval.tick().await;
                if stopped.load(Ordering::SeqCst) {
                    break;
                }
                let background = &app.state::<Services>().background;
                let enabled = background.current().meeting_detection;
                if let Some(hint) = detector.poll(enabled).await {
                    // Disabling detection while a bounded OS query is in progress
                    // takes effect before publishing the observation.
                    if background.current().meeting_detection {
                        background.emit_meeting_hint(&app, hint);
                    }
                }
            }
        });
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    pub fn update_activity(
        &self,
        app: &AppHandle,
        listening: bool,
        mode: BackgroundActivity,
    ) -> Result<(), String> {
        let label = activity_label(listening, mode);
        let status = self
            .tray_status
            .lock()
            .map_err(|_| "Cannot update tray activity.")?
            .clone();
        if let Some(status) = status {
            status
                .set_text(label)
                .map_err(|error| format!("Cannot update tray activity: {error}"))?;
        }
        if let Some(tray) = app.tray_by_id("nextcue-assistant") {
            // Tooltip support differs by desktop. The disabled menu status is
            // the primary activity indicator, so tooltip updates are best effort.
            let _ = tray.set_tooltip(Some(label));
        }
        Ok(())
    }
}

fn install_tray(app: &AppHandle) -> Result<MenuItem<tauri::Wry>, String> {
    use tauri::{menu::MenuBuilder, tray::TrayIconBuilder};
    let status = MenuItem::with_id(
        app,
        "nextcue-status",
        "NextCue · Paused",
        false,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let menu = MenuBuilder::new(app)
        .item(&status)
        .separator()
        .text("nextcue-show", "Show NextCue")
        .text("nextcue-hide", "Hide NextCue")
        .separator()
        .text("nextcue-meeting", "Run meeting trigger")
        .text("nextcue-listen", "Start listening")
        .text("nextcue-notes", "Take meeting notes")
        .text("nextcue-answers", "Answer questions")
        .text("nextcue-pause", "Pause listening")
        .separator()
        .text("nextcue-settings", "Settings…")
        .text("nextcue-quit", "Quit NextCue")
        .build()
        .map_err(|error| error.to_string())?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("The assistant icon is unavailable.")?;
    TrayIconBuilder::with_id("nextcue-assistant")
        .icon(icon)
        .tooltip("NextCue — desktop assistant")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let action = match event.id().as_ref() {
                "nextcue-show" => Some(BackgroundAction::Show),
                "nextcue-settings" => Some(BackgroundAction::Settings),
                "nextcue-meeting" => Some(BackgroundAction::MeetingTrigger),
                "nextcue-listen" => Some(BackgroundAction::Listen),
                "nextcue-notes" => Some(BackgroundAction::Notes),
                "nextcue-answers" => Some(BackgroundAction::Answers),
                "nextcue-pause" => Some(BackgroundAction::Pause),
                "nextcue-quit" => {
                    app.state::<Services>().background.request_quit(app);
                    None
                }
                "nextcue-hide" => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = app
                                .state::<Services>()
                                .companion
                                .set_visible(&window, false);
                        }
                    });
                    None
                }
                _ => None,
            };
            if let Some(action) = action {
                dispatch_action(app.clone(), action, ActionSource::Tray);
            }
        })
        .build(app)
        .map_err(|error| error.to_string())?;
    Ok(status)
}

pub fn dispatch_action(app: AppHandle, action: BackgroundAction, source: ActionSource) {
    tauri::async_runtime::spawn(async move {
        let services = app.state::<Services>();
        if matches!(action, BackgroundAction::Show | BackgroundAction::Settings) {
            if let Some(window) = app.get_webview_window("main") {
                let _ = services.companion.set_mode(&window, CompanionMode::Main);
                let _ = window.unminimize();
                let _ = services.companion.set_visible(&window, true);
            }
        }
        // Listening and generation actions keep the existing visibility so a
        // meeting shortcut does not unexpectedly place a window over a call.
        services.background.emit_action(&app, action, source);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn old_settings_keep_a_visible_launch_and_no_automatic_audio() {
        let configuration =
            BackgroundConfiguration::from_state(&json!({"settings": {"model": "local"}}));
        assert_eq!(configuration, BackgroundConfiguration::default());
        assert!(configuration.run_in_background);
        assert!(!configuration.start_hidden);
        assert!(!configuration.meeting_detection);
        assert_eq!(configuration.meeting_action, MeetingAction::None);
    }

    #[test]
    fn startup_restores_only_background_fields_from_saved_settings() {
        let configuration = BackgroundConfiguration::from_state(&json!({"settings": {
            "runInBackground": false, "startHidden": true,
            "meetingDetection": true, "meetingAction": "listen-notes",
            "model": "irrelevant", "privateContext": "never emitted"
        }}));
        assert!(!configuration.run_in_background);
        assert!(configuration.start_hidden && configuration.meeting_detection);
        assert_eq!(configuration.meeting_action, MeetingAction::ListenNotes);
        let serialized = serde_json::to_value(configuration).unwrap();
        assert!(serialized.get("privateContext").is_none());
        assert!(serialized.get("model").is_none());
    }

    #[test]
    fn malformed_configuration_rejects_unknown_actions_and_wrong_types() {
        for configuration in [
            json!({"meetingAction": "record-screen"}),
            json!({"startHidden": "yes"}),
            json!({"extra": true}),
        ] {
            assert!(serde_json::from_value::<BackgroundConfiguration>(configuration).is_err());
        }
    }

    #[test]
    fn close_requires_both_background_preference_and_a_recoverable_tray() {
        let service = BackgroundService::new(BackgroundConfiguration::default());
        assert!(!service.close_to_tray());
        service.tray_available.store(true, Ordering::SeqCst);
        assert!(service.close_to_tray());
        service
            .configure(BackgroundConfiguration {
                run_in_background: false,
                ..Default::default()
            })
            .unwrap();
        assert!(!service.close_to_tray());
    }

    #[test]
    fn action_events_match_the_frontend_contract() {
        let value = serde_json::to_value(ActionEvent {
            action: BackgroundAction::MeetingTrigger,
            source: ActionSource::Shortcut,
        })
        .unwrap();
        assert_eq!(
            value,
            json!({"action": "meeting-trigger", "source": "shortcut"})
        );
    }

    #[test]
    fn tray_activity_indicates_audio_state_without_session_context() {
        assert_eq!(
            activity_label(false, BackgroundActivity::Answers),
            "NextCue · Paused"
        );
        assert_eq!(
            activity_label(true, BackgroundActivity::Manual),
            "NextCue · Listening"
        );
        assert_eq!(
            activity_label(true, BackgroundActivity::Notes),
            "NextCue · Listening · Meeting notes"
        );
        assert_eq!(
            activity_label(true, BackgroundActivity::Answers),
            "NextCue · Listening · Question answers"
        );
        assert!(serde_json::from_value::<BackgroundActivity>(json!("screen")).is_err());
    }
}
