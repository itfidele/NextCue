//! The companion is the existing workspace window, so live audio and streamed
//! answers remain mounted while the user moves between the workspace and pin.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, Monitor, PhysicalPosition, PhysicalSize,
    WebviewWindow,
};
#[cfg(desktop)]
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

const PIN_SIZE: LogicalSize<f64> = LogicalSize::new(72.0, 72.0);
const ANSWER_SIZE: LogicalSize<f64> = LogicalSize::new(390.0, 480.0);
const MAIN_MIN_SIZE: LogicalSize<f64> = LogicalSize::new(860.0, 620.0);
pub const DEFAULT_ANSWER_SHORTCUT: &str = "CommandOrControl+Shift+Space";
pub const DEFAULT_VISIBILITY_SHORTCUT: &str = "CommandOrControl+Shift+H";
pub const DEFAULT_MEETING_SHORTCUT: &str = "CommandOrControl+Shift+M";

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CompanionMode {
    #[default]
    Main,
    Pin,
    Answer,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureProtection {
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    BestEffort,
    Unavailable,
    Disabled,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionStatus {
    pub(crate) mode: CompanionMode,
    capture_protection: CaptureProtection,
    shortcut_available: bool,
    visibility_shortcut_available: bool,
    meeting_shortcut_available: bool,
    emergency_shortcut_available: bool,
    visible: bool,
    answer_shortcut: String,
    visibility_shortcut: String,
    meeting_shortcut: String,
    capture_protection_enabled: bool,
    click_through: bool,
}

#[derive(Clone)]
struct Preferences {
    capture_protection_enabled: bool,
    click_through: bool,
    answer_shortcut: String,
    visibility_shortcut: String,
    meeting_shortcut: String,
    shortcut_available: bool,
    visibility_shortcut_available: bool,
    meeting_shortcut_available: bool,
    emergency_shortcut_available: bool,
    #[cfg(desktop)]
    registered: Vec<Shortcut>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            capture_protection_enabled: true,
            click_through: false,
            answer_shortcut: DEFAULT_ANSWER_SHORTCUT.into(),
            visibility_shortcut: DEFAULT_VISIBILITY_SHORTCUT.into(),
            meeting_shortcut: DEFAULT_MEETING_SHORTCUT.into(),
            shortcut_available: false,
            visibility_shortcut_available: false,
            meeting_shortcut_available: false,
            emergency_shortcut_available: false,
            #[cfg(desktop)]
            registered: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    Answer,
    Visibility,
    Meeting,
}

#[cfg(desktop)]
fn parse_shortcut(value: &str, default: &str) -> Result<(String, Shortcut), String> {
    let value = if value.trim().is_empty() {
        default
    } else {
        value.trim()
    };
    if value.len() > 100 {
        return Err("Use a shorter keyboard shortcut.".into());
    }
    let shortcut: Shortcut = value
        .parse()
        .map_err(|_| "Use a valid shortcut, such as CommandOrControl+Shift+Space.".to_string())?;
    if !shortcut
        .mods
        .intersects(Modifiers::CONTROL | Modifiers::SUPER | Modifiers::ALT)
    {
        return Err("Include Ctrl, Command, or Alt so normal typing stays available.".into());
    }
    let system_key = matches!(
        shortcut.key,
        Code::Escape | Code::Tab | Code::Delete | Code::Backspace
    );
    let primary_only = shortcut.mods == Modifiers::CONTROL || shortcut.mods == Modifiers::SUPER;
    let app_key = shortcut.key == Code::Enter
        || (primary_only
            && matches!(
                shortcut.key,
                Code::KeyQ
                    | Code::KeyW
                    | Code::KeyH
                    | Code::KeyC
                    | Code::KeyV
                    | Code::KeyX
                    | Code::KeyZ
                    | Code::KeyA
            ));
    if system_key
        || app_key
        || (shortcut.key == Code::Space && !shortcut.mods.contains(Modifiers::SHIFT))
    {
        return Err(
            "Choose a shortcut that does not replace typing, navigation, or system controls."
                .into(),
        );
    }
    Ok((value.to_string(), shortcut))
}

#[cfg(desktop)]
fn shortcut_pair(
    answer: &str,
    visibility: &str,
) -> Result<(String, Shortcut, String, Shortcut), String> {
    let (answer_name, answer) = parse_shortcut(answer, DEFAULT_ANSWER_SHORTCUT)?;
    let (visibility_name, visibility) = parse_shortcut(visibility, DEFAULT_VISIBILITY_SHORTCUT)?;
    let emergency: Shortcut = DEFAULT_VISIBILITY_SHORTCUT
        .parse()
        .expect("valid built-in shortcut");
    if answer == visibility || answer == emergency {
        return Err("Use different shortcuts for answers and visibility. Ctrl/Command+Shift+H is reserved for hide/show.".into());
    }
    Ok((answer_name, answer, visibility_name, visibility))
}

#[cfg(desktop)]
fn shortcut_set(
    answer: &str,
    visibility: &str,
    meeting: &str,
) -> Result<(String, Shortcut, String, Shortcut, String, Shortcut), String> {
    let (answer_name, answer, visibility_name, visibility) = shortcut_pair(answer, visibility)?;
    let (meeting_name, meeting) = parse_shortcut(meeting, DEFAULT_MEETING_SHORTCUT)?;
    let emergency: Shortcut = DEFAULT_VISIBILITY_SHORTCUT
        .parse()
        .expect("valid built-in shortcut");
    if meeting == answer || meeting == visibility || meeting == emergency {
        return Err("Use different shortcuts for answers, hide/show, and the meeting trigger. Ctrl/Command+Shift+H is reserved for hide/show.".into());
    }
    Ok((
        answer_name,
        answer,
        visibility_name,
        visibility,
        meeting_name,
        meeting,
    ))
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
struct Point {
    x: i32,
    y: i32,
}

impl From<PhysicalPosition<i32>> for Point {
    fn from(position: PhysicalPosition<i32>) -> Self {
        Self {
            x: position.x,
            y: position.y,
        }
    }
}

impl From<Point> for PhysicalPosition<i32> {
    fn from(point: Point) -> Self {
        Self::new(point.x, point.y)
    }
}

#[derive(Debug, Clone, Copy)]
struct WorkArea {
    origin: Point,
    size: PhysicalSize<u32>,
}

impl From<&Monitor> for WorkArea {
    fn from(monitor: &Monitor) -> Self {
        let work_area = monitor.work_area();
        Self {
            origin: work_area.position.into(),
            size: work_area.size,
        }
    }
}

impl WorkArea {
    fn contains(self, point: Point) -> bool {
        let x = i64::from(point.x);
        let y = i64::from(point.y);
        x >= i64::from(self.origin.x)
            && x < i64::from(self.origin.x) + i64::from(self.size.width)
            && y >= i64::from(self.origin.y)
            && y < i64::from(self.origin.y) + i64::from(self.size.height)
    }
}

/// Use physical coordinates throughout: monitor origins can be negative and
/// different displays can have different display scale factors.
fn clamp_position(point: Point, size: PhysicalSize<u32>, area: WorkArea) -> Point {
    let clamp_axis = |value: i32, origin: i32, available: u32, window: u32| {
        let start = i64::from(origin);
        let end = start + i64::from(available.saturating_sub(window));
        i64::from(value).clamp(start, end) as i32
    };
    Point {
        x: clamp_axis(point.x, area.origin.x, area.size.width, size.width),
        y: clamp_axis(point.y, area.origin.y, area.size.height, size.height),
    }
}

fn answer_position(
    anchor: Point,
    pin_size: PhysicalSize<u32>,
    answer_size: PhysicalSize<u32>,
    area: WorkArea,
) -> Point {
    let align_axis = |coordinate: i32, origin: i32, available: u32, pin: u32, answer: u32| {
        let coordinate = i64::from(coordinate);
        if coordinate > i64::from(origin) + i64::from(available) / 2 {
            (coordinate + i64::from(pin) - i64::from(answer)) as i32
        } else {
            coordinate as i32
        }
    };
    clamp_position(
        Point {
            x: align_axis(
                anchor.x,
                area.origin.x,
                area.size.width,
                pin_size.width,
                answer_size.width,
            ),
            y: align_axis(
                anchor.y,
                area.origin.y,
                area.size.height,
                pin_size.height,
                answer_size.height,
            ),
        },
        answer_size,
        area,
    )
}

struct WorkspaceBounds {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    decorated: bool,
    always_on_top: bool,
    resizable: bool,
    maximizable: Option<bool>,
    maximized: bool,
    fullscreen: bool,
}

impl WorkspaceBounds {
    fn read(window: &WebviewWindow) -> Result<Self, String> {
        Ok(Self {
            position: window.outer_position().map_err(|e| e.to_string())?,
            size: window.inner_size().map_err(|e| e.to_string())?,
            decorated: window.is_decorated().map_err(|e| e.to_string())?,
            always_on_top: window.is_always_on_top().map_err(|e| e.to_string())?,
            resizable: window.is_resizable().map_err(|e| e.to_string())?,
            // Linux does not expose the native maximize-button state.
            maximizable: window.is_maximizable().ok(),
            maximized: window.is_maximized().map_err(|e| e.to_string())?,
            fullscreen: window.is_fullscreen().map_err(|e| e.to_string())?,
        })
    }

    fn restore(&self, window: &WebviewWindow, focus: bool) -> tauri::Result<()> {
        window.set_ignore_cursor_events(false)?;
        window.set_focusable(true)?;
        // Tao's macOS decoration update captures the current resizable value
        // and applies that style asynchronously. Restore resizing first so its
        // pending titlebar style cannot remove it again afterward.
        window.set_resizable(self.resizable)?;
        window.set_decorations(self.decorated)?;
        window.set_min_size(Some(MAIN_MIN_SIZE))?;
        window.set_size(self.size)?;
        window.set_position(self.position)?;
        window.set_always_on_top(self.always_on_top)?;
        let _ = window.set_shadow(true);
        if self.maximized {
            window.maximize()?;
        }
        if self.fullscreen {
            window.set_fullscreen(true)?;
        }
        if let Some(maximizable) = self.maximizable {
            window.set_maximizable(maximizable)?;
        }
        if focus {
            window.set_focus()?;
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct WindowPolicy {
    focusable: bool,
    ignore_cursor_events: bool,
    application_visible: bool,
}

fn window_policy(mode: CompanionMode, visible: bool, click_through: bool) -> WindowPolicy {
    let main = mode == CompanionMode::Main;
    WindowPolicy {
        focusable: main,
        ignore_cursor_events: !main && click_through,
        application_visible: main && visible,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RevealIntent {
    Recovery,
    Answer,
}

fn visibility_mode(
    mode: CompanionMode,
    visible: bool,
    click_through: bool,
    intent: RevealIntent,
) -> CompanionMode {
    if visible && click_through && intent == RevealIntent::Recovery {
        CompanionMode::Main
    } else {
        mode
    }
}

fn apply_interaction(
    window: &WebviewWindow,
    mode: CompanionMode,
    click_through: bool,
) -> tauri::Result<()> {
    let policy = window_policy(mode, true, click_through);
    window.set_ignore_cursor_events(policy.ignore_cursor_events)?;
    window.set_focusable(policy.focusable)
}

fn apply_presence(window: &WebviewWindow, mode: CompanionMode, visible: bool) -> tauri::Result<()> {
    let policy = window_policy(mode, visible, false);
    #[cfg(target_os = "macos")]
    {
        // AppHandle's supported setter dispatches to Tauri's main event loop;
        // do not wait on a callback while holding the transition/state locks.
        window
            .app_handle()
            .set_activation_policy(if policy.application_visible {
                tauri::ActivationPolicy::Regular
            } else {
                tauri::ActivationPolicy::Accessory
            })
    }
    #[cfg(not(target_os = "macos"))]
    window.set_skip_taskbar(!policy.application_visible)
}

#[derive(Default)]
struct CompanionState {
    mode: CompanionMode,
    workspace: Option<WorkspaceBounds>,
    anchor: Option<Point>,
    // Keeps the original pin anchor even when the expanded card is dragged.
    anchor_offset: Point,
}

impl Default for Point {
    fn default() -> Self {
        Self { x: 0, y: 0 }
    }
}

pub struct CompanionService {
    transition: Mutex<()>,
    state: Mutex<CompanionState>,
    position_file: PathBuf,
    preferences: Mutex<Preferences>,
    tray_recovery_available: AtomicBool,
}

impl CompanionService {
    pub fn new(directory: PathBuf) -> Self {
        let position_file = directory.join("companion-position.json");
        let anchor = fs::metadata(&position_file)
            .ok()
            .filter(|metadata| metadata.len() <= 128)
            .and_then(|_| fs::read(&position_file).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        Self {
            transition: Mutex::new(()),
            state: Mutex::new(CompanionState {
                anchor,
                ..Default::default()
            }),
            position_file,
            preferences: Mutex::new(Preferences::default()),
            tray_recovery_available: AtomicBool::new(false),
        }
    }

    #[cfg(desktop)]
    pub fn initialize_shortcuts(&self, app: &AppHandle) {
        let emergency: Shortcut = DEFAULT_VISIBILITY_SHORTCUT
            .parse()
            .expect("valid built-in shortcut");
        if app.global_shortcut().register(emergency).is_ok() {
            if let Ok(mut preferences) = self.preferences.lock() {
                preferences.registered.push(emergency);
                preferences.emergency_shortcut_available = true;
                preferences.visibility_shortcut_available = true;
            }
        }
        // A busy meeting shortcut must not disable the established answer and
        // recovery controls at startup. User rebinding remains transactional.
        for (name, meeting) in [
            (DEFAULT_ANSWER_SHORTCUT, false),
            (DEFAULT_MEETING_SHORTCUT, true),
        ] {
            let shortcut: Shortcut = name.parse().expect("valid built-in shortcut");
            if app.global_shortcut().register(shortcut).is_ok() {
                if let Ok(mut preferences) = self.preferences.lock() {
                    preferences.registered.push(shortcut);
                    if meeting {
                        preferences.meeting_shortcut_available = true;
                    } else {
                        preferences.shortcut_available = true;
                    }
                }
            }
        }
        if let Some(window) = app.get_webview_window("main") {
            let _ = self.configure(
                app,
                &window,
                true,
                DEFAULT_ANSWER_SHORTCUT.into(),
                DEFAULT_VISIBILITY_SHORTCUT.into(),
                DEFAULT_MEETING_SHORTCUT.into(),
                false,
            );
        }
    }

    #[cfg(desktop)]
    pub fn shortcut_action(&self, shortcut: &Shortcut) -> Option<ShortcutAction> {
        let emergency: Shortcut = DEFAULT_VISIBILITY_SHORTCUT.parse().ok()?;
        if shortcut == &emergency {
            return Some(ShortcutAction::Visibility);
        }
        // Registration waits for the native main thread. A shortcut event must
        // not block that thread on the configuration lock during rebinding.
        let preferences = self.preferences.try_lock().ok()?;
        if preferences
            .answer_shortcut
            .parse::<Shortcut>()
            .ok()
            .as_ref()
            == Some(shortcut)
        {
            Some(ShortcutAction::Answer)
        } else if preferences
            .meeting_shortcut
            .parse::<Shortcut>()
            .ok()
            .as_ref()
            == Some(shortcut)
        {
            Some(ShortcutAction::Meeting)
        } else if preferences
            .visibility_shortcut
            .parse::<Shortcut>()
            .ok()
            .as_ref()
            == Some(shortcut)
        {
            Some(ShortcutAction::Visibility)
        } else {
            None
        }
    }

    pub fn configure(
        &self,
        app: &AppHandle,
        window: &WebviewWindow,
        capture_protection_enabled: bool,
        answer_shortcut: String,
        visibility_shortcut: String,
        meeting_shortcut: String,
        click_through: bool,
    ) -> Result<CompanionStatus, String> {
        let _transition = self
            .transition
            .lock()
            .map_err(|_| "Cannot update the floating assistant.")?;
        if click_through && !self.recovery_available() {
            return Err("Set up an available hide/show shortcut or system tray before making the floating assistant click-through.".into());
        }
        let mode = self
            .state
            .lock()
            .map_err(|_| "Cannot access the floating assistant.")?
            .mode;
        #[cfg(desktop)]
        {
            let (answer_name, answer, visibility_name, visibility, meeting_name, meeting) =
                shortcut_set(&answer_shortcut, &visibility_shortcut, &meeting_shortcut)?;
            let emergency: Shortcut = DEFAULT_VISIBILITY_SHORTCUT
                .parse()
                .expect("valid built-in shortcut");
            let mut desired = vec![emergency, answer, meeting];
            if visibility != emergency {
                desired.push(visibility);
            }
            let mut preferences = self
                .preferences
                .lock()
                .map_err(|_| "Cannot update the assistant shortcuts.")?;
            let shortcuts = app.global_shortcut();
            let mut added = Vec::new();
            // Register replacements before removing anything. A collision with
            // another app leaves the previous shortcuts and settings untouched.
            for shortcut in &desired {
                if !shortcuts.is_registered(*shortcut) {
                    if let Err(error) = shortcuts.register(*shortcut) {
                        for newly_registered in added {
                            let _ = shortcuts.unregister(newly_registered);
                        }
                        return Err(format!("That shortcut is unavailable. The previous shortcuts are still active: {error}"));
                    }
                    added.push(*shortcut);
                }
            }
            let mut removed = Vec::new();
            for shortcut in &preferences.registered {
                if !desired.contains(shortcut) {
                    if let Err(error) = shortcuts.unregister(*shortcut) {
                        for previous in removed {
                            let _ = shortcuts.register(previous);
                        }
                        for newly_registered in added {
                            let _ = shortcuts.unregister(newly_registered);
                        }
                        return Err(format!("Cannot change these shortcuts. The previous settings were retained: {error}"));
                    }
                    removed.push(*shortcut);
                }
            }
            if let Err(error) = apply_interaction(window, mode, click_through) {
                for previous in removed {
                    let _ = shortcuts.register(previous);
                }
                for newly_registered in added {
                    let _ = shortcuts.unregister(newly_registered);
                }
                let _ = apply_interaction(window, mode, preferences.click_through);
                return Err(format!("Cannot update click-through mode. The previous settings were retained: {error}"));
            }
            preferences.registered = desired;
            preferences.answer_shortcut = answer_name;
            preferences.visibility_shortcut = visibility_name;
            preferences.meeting_shortcut = meeting_name;
            preferences.shortcut_available = true;
            preferences.visibility_shortcut_available = true;
            preferences.meeting_shortcut_available = true;
            preferences.emergency_shortcut_available = true;
            preferences.capture_protection_enabled = capture_protection_enabled;
            preferences.click_through = click_through;
        }
        #[cfg(not(desktop))]
        {
            let _ = (app, answer_shortcut, visibility_shortcut, meeting_shortcut);
            let mut preferences = self
                .preferences
                .lock()
                .map_err(|_| "Cannot update the assistant settings.")?;
            if let Err(error) = apply_interaction(window, mode, click_through) {
                let _ = apply_interaction(window, mode, preferences.click_through);
                return Err(format!("Cannot update click-through mode. The previous settings were retained: {error}"));
            }
            preferences.capture_protection_enabled = capture_protection_enabled;
            preferences.click_through = click_through;
        }
        self.status(window)
    }

    pub fn status(&self, window: &WebviewWindow) -> Result<CompanionStatus, String> {
        let mode = self
            .state
            .lock()
            .map_err(|_| "Cannot access the floating assistant.")?
            .mode;
        let preferences = self
            .preferences
            .lock()
            .map_err(|_| "Cannot access the assistant settings.")?
            .clone();
        Ok(CompanionStatus {
            mode,
            capture_protection: capture_protection(
                window,
                mode,
                preferences.capture_protection_enabled,
            ),
            shortcut_available: preferences.shortcut_available,
            visibility_shortcut_available: preferences.visibility_shortcut_available,
            meeting_shortcut_available: preferences.meeting_shortcut_available,
            emergency_shortcut_available: preferences.emergency_shortcut_available,
            visible: window.is_visible().map_err(|error| error.to_string())?,
            answer_shortcut: preferences.answer_shortcut,
            visibility_shortcut: preferences.visibility_shortcut,
            meeting_shortcut: preferences.meeting_shortcut,
            capture_protection_enabled: preferences.capture_protection_enabled,
            click_through: preferences.click_through,
        })
    }

    pub fn set_tray_recovery(&self, available: bool) {
        self.tray_recovery_available
            .store(available, Ordering::SeqCst);
    }

    pub fn recovery_available(&self) -> bool {
        self.tray_recovery_available.load(Ordering::SeqCst)
            || self.preferences.lock().is_ok_and(|preferences| {
                preferences.visibility_shortcut_available
                    || preferences.emergency_shortcut_available
            })
    }

    pub fn set_visible(
        &self,
        window: &WebviewWindow,
        visible: bool,
    ) -> Result<CompanionStatus, String> {
        let _transition = self
            .transition
            .lock()
            .map_err(|_| "Cannot update the floating assistant.")?;
        self.set_visible_locked(window, visible, RevealIntent::Recovery)
    }

    pub fn reveal_for_answer(&self, window: &WebviewWindow) -> Result<CompanionStatus, String> {
        let _transition = self
            .transition
            .lock()
            .map_err(|_| "Cannot update the floating assistant.")?;
        self.set_visible_locked(window, true, RevealIntent::Answer)
    }

    fn set_visible_locked(
        &self,
        window: &WebviewWindow,
        visible: bool,
        intent: RevealIntent,
    ) -> Result<CompanionStatus, String> {
        let mut mode = self
            .state
            .lock()
            .map_err(|_| "Cannot access the floating assistant.")?
            .mode;
        let click_through = self
            .preferences
            .lock()
            .map_err(|_| "Cannot access the assistant settings.")?
            .click_through;
        if visible {
            let restored_mode = visibility_mode(mode, visible, click_through, intent);
            if restored_mode != mode {
                // The visibility shortcut must recover usable mouse controls
                // even when the tray is unavailable and the card ignores clicks.
                self.set_mode_locked(window, restored_mode)?;
                mode = restored_mode;
            }
            // Restore the workspace's Dock/taskbar and input behavior before
            // showing it, even when a click-through card was previously hidden.
            apply_presence(window, mode, true).map_err(|error| error.to_string())?;
            apply_interaction(window, mode, click_through).map_err(|error| error.to_string())?;
            if let Err(error) = window.show() {
                let _ = apply_presence(window, mode, false);
                return Err(error.to_string());
            }
            if mode == CompanionMode::Main {
                window.set_focus().map_err(|error| error.to_string())?;
            }
        } else {
            let preferences = self
                .preferences
                .lock()
                .map_err(|_| "Cannot access the assistant shortcuts.")?;
            if !self.tray_recovery_available.load(Ordering::SeqCst)
                && !preferences.visibility_shortcut_available
                && !preferences.emergency_shortcut_available
            {
                return Err(
                    "Set up an available hide/show shortcut or system tray before hiding the assistant.".into(),
                );
            }
            drop(preferences);
            window.hide().map_err(|error| error.to_string())?;
            if let Err(error) = apply_presence(window, mode, false) {
                let _ = apply_presence(window, mode, true);
                let _ = window.show();
                return Err(error.to_string());
            }
        }
        let status = self.status(window)?;
        let _ = window.emit("companion-visibility", &status);
        Ok(status)
    }

    fn remember(&self, anchor: Point) {
        // Position persistence is best effort; an unwritable data directory
        // should never keep a user from returning to the full workspace.
        let Some(directory) = self.position_file.parent() else {
            return;
        };
        if fs::create_dir_all(directory).is_err() {
            return;
        }
        if let Ok(bytes) = serde_json::to_vec(&anchor) {
            let temporary = self.position_file.with_extension("json.tmp");
            if fs::write(&temporary, bytes).is_ok() {
                let _ = fs::rename(temporary, &self.position_file);
            }
        }
    }

    pub fn record_position(&self, position: PhysicalPosition<i32>) {
        // Window movement may be emitted during a mode transition. Never block
        // the native event loop waiting for that transition's window operations.
        let Ok(mut state) = self.state.try_lock() else {
            return;
        };
        if state.mode == CompanionMode::Main {
            return;
        }
        let anchor = Point {
            x: position.x.saturating_add(state.anchor_offset.x),
            y: position.y.saturating_add(state.anchor_offset.y),
        };
        state.anchor = Some(anchor);
        self.remember(anchor);
    }

    pub fn set_mode(
        &self,
        window: &WebviewWindow,
        mode: CompanionMode,
    ) -> Result<CompanionStatus, String> {
        let _transition = self
            .transition
            .lock()
            .map_err(|_| "Cannot update the floating assistant.")?;
        self.set_mode_locked(window, mode)
    }

    fn set_mode_locked(
        &self,
        window: &WebviewWindow,
        mode: CompanionMode,
    ) -> Result<CompanionStatus, String> {
        let click_through = self
            .preferences
            .lock()
            .map_err(|_| "Cannot access the assistant settings.")?
            .click_through;
        if mode != CompanionMode::Main && click_through && !self.recovery_available() {
            return Err("Set up an available hide/show shortcut or system tray before making the floating assistant click-through.".into());
        }
        let visible = window.is_visible().map_err(|error| error.to_string())?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Cannot access the floating assistant.".to_string())?;
        if state.mode != CompanionMode::Main {
            let position = window.outer_position().map_err(|e| e.to_string())?;
            let anchor = Point {
                x: position.x.saturating_add(state.anchor_offset.x),
                y: position.y.saturating_add(state.anchor_offset.y),
            };
            state.anchor = Some(anchor);
            self.remember(anchor);
        }

        if mode == CompanionMode::Main {
            apply_presence(window, mode, visible).map_err(|error| error.to_string())?;
            apply_interaction(window, mode, click_through).map_err(|error| error.to_string())?;
            if let Some(workspace) = &state.workspace {
                workspace
                    .restore(window, visible)
                    .map_err(|e| e.to_string())?;
            }
            state.workspace = None;
            state.mode = mode;
            state.anchor_offset = Point::default();
        } else {
            let monitors = window.available_monitors().map_err(|e| e.to_string())?;
            let monitor = state
                .anchor
                .and_then(|anchor| {
                    monitors
                        .iter()
                        .find(|monitor| WorkArea::from(*monitor).contains(anchor))
                        .cloned()
                })
                .or(window.current_monitor().map_err(|e| e.to_string())?)
                .or(window.primary_monitor().map_err(|e| e.to_string())?)
                .ok_or("Cannot find a display for the floating assistant.")?;
            let area = WorkArea::from(&monitor);
            let scale = monitor.scale_factor();
            let pin_size = PIN_SIZE.to_physical::<u32>(scale);
            let preferred_size = if mode == CompanionMode::Pin {
                pin_size
            } else {
                ANSWER_SIZE.to_physical::<u32>(scale)
            };
            let size = PhysicalSize::new(
                preferred_size.width.min(area.size.width),
                preferred_size.height.min(area.size.height),
            );
            let default_anchor = Point {
                x: (i64::from(area.origin.x) + i64::from(area.size.width)
                    - i64::from(pin_size.width)
                    - (24.0 * scale) as i64) as i32,
                y: area.origin.y.saturating_add((24.0 * scale) as i32),
            };
            let anchor = clamp_position(state.anchor.unwrap_or(default_anchor), pin_size, area);
            let position = if mode == CompanionMode::Pin {
                clamp_position(anchor, size, area)
            } else {
                answer_position(anchor, pin_size, size, area)
            };
            if state.mode == CompanionMode::Main {
                state.workspace = Some(WorkspaceBounds::read(window)?);
            }

            let change_window = || -> tauri::Result<()> {
                apply_presence(window, mode, visible)?;
                if window.is_fullscreen()? {
                    window.set_fullscreen(false)?;
                }
                if window.is_maximized()? {
                    window.unmaximize()?;
                }
                // Floating answers never take keyboard focus. Optional pointer
                // passthrough lets the app underneath receive every click.
                apply_interaction(window, mode, click_through)?;
                window.set_min_size(None::<LogicalSize<f64>>)?;
                window.set_resizable(false)?;
                window.set_decorations(false)?;
                window.set_always_on_top(true)?;
                let _ = window.set_shadow(false);
                window.set_size(size)?;
                window.set_position(PhysicalPosition::from(position))?;
                Ok(())
            };
            if let Err(error) = change_window() {
                // Always provide an escape back to the workspace after a
                // platform rejects an operation midway through the change.
                let _ = apply_presence(window, CompanionMode::Main, visible);
                let _ = apply_interaction(window, CompanionMode::Main, false);
                if let Some(workspace) = &state.workspace {
                    let _ = workspace.restore(window, visible);
                }
                state.mode = CompanionMode::Main;
                state.workspace = None;
                state.anchor_offset = Point::default();
                return Err(format!("Cannot open the floating assistant: {error}"));
            }
            state.anchor = Some(anchor);
            state.anchor_offset = Point {
                x: anchor.x.saturating_sub(position.x),
                y: anchor.y.saturating_sub(position.y),
            };
            state.mode = mode;
            self.remember(anchor);
        }
        drop(state);
        self.status(window)
    }
}

fn capture_protection(
    window: &WebviewWindow,
    mode: CompanionMode,
    enabled: bool,
) -> CaptureProtection {
    #[cfg(target_os = "windows")]
    {
        // Windows requests WDA_EXCLUDEFROMCAPTURE. Capture methods vary, so this
        // is deliberately never reported as guaranteed hiding.
        if !enabled {
            return if window.set_content_protected(false).is_ok() {
                CaptureProtection::Disabled
            } else {
                CaptureProtection::Unavailable
            };
        } else if window.set_content_protected(true).is_ok() {
            return CaptureProtection::BestEffort;
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (window, mode);
    #[cfg(target_os = "windows")]
    let _ = mode;
    if enabled {
        CaptureProtection::Unavailable
    } else {
        CaptureProtection::Disabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(x: i32, y: i32, width: u32, height: u32) -> WorkArea {
        WorkArea {
            origin: Point { x, y },
            size: PhysicalSize::new(width, height),
        }
    }

    #[test]
    fn card_near_lower_right_keeps_pin_corner_and_avoids_taskbar() {
        let area = display(0, 0, 1920, 1040);
        let anchor = Point { x: 1824, y: 944 };
        let position = answer_position(
            anchor,
            PhysicalSize::new(72, 72),
            PhysicalSize::new(390, 480),
            area,
        );
        assert_eq!(position, Point { x: 1506, y: 536 });
        assert_eq!(position.x + 390, anchor.x + 72);
        assert_eq!(position.y + 480, anchor.y + 72);
    }

    #[test]
    fn card_on_negative_origin_monitor_remains_inside_its_work_area() {
        let area = display(-1920, -100, 1920, 1080);
        assert!(area.contains(Point { x: -1900, y: 20 }));
        assert!(!area.contains(Point { x: 0, y: 20 }));
        assert_eq!(
            answer_position(
                Point { x: -1910, y: -95 },
                PhysicalSize::new(144, 144),
                PhysicalSize::new(780, 960),
                area,
            ),
            Point { x: -1910, y: -95 }
        );
    }

    #[test]
    fn removed_monitor_position_is_clamped_into_current_display() {
        assert_eq!(
            clamp_position(
                Point { x: -1600, y: 1200 },
                PhysicalSize::new(72, 72),
                display(0, 24, 1440, 850),
            ),
            Point { x: 0, y: 802 }
        );
    }

    #[test]
    fn undersized_work_area_and_extreme_saved_positions_do_not_panic() {
        assert_eq!(
            clamp_position(
                Point {
                    x: i32::MAX,
                    y: i32::MIN,
                },
                PhysicalSize::new(390, 480),
                display(-200, -100, 200, 100),
            ),
            Point { x: -200, y: -100 }
        );
    }

    #[test]
    fn dragging_expanded_card_preserves_relative_pin_location() {
        let anchor = Point { x: 1824, y: 944 };
        let original = Point { x: 1506, y: 536 };
        let offset = Point {
            x: anchor.x - original.x,
            y: anchor.y - original.y,
        };
        let dragged = Point { x: 1200, y: 300 };
        let updated_anchor = Point {
            x: dragged.x + offset.x,
            y: dragged.y + offset.y,
        };
        assert_eq!(updated_anchor, Point { x: 1518, y: 708 });
    }

    #[test]
    fn accepted_modes_and_protection_status_match_frontend_contract() {
        assert_eq!(
            serde_json::from_str::<CompanionMode>("\"pin\"").unwrap(),
            CompanionMode::Pin
        );
        assert!(serde_json::from_str::<CompanionMode>("\"unknown\"").is_err());
        let status = serde_json::to_value(CompanionStatus {
            mode: CompanionMode::Answer,
            capture_protection: CaptureProtection::BestEffort,
            shortcut_available: true,
            visibility_shortcut_available: true,
            meeting_shortcut_available: true,
            emergency_shortcut_available: true,
            visible: true,
            answer_shortcut: DEFAULT_ANSWER_SHORTCUT.into(),
            visibility_shortcut: DEFAULT_VISIBILITY_SHORTCUT.into(),
            meeting_shortcut: DEFAULT_MEETING_SHORTCUT.into(),
            capture_protection_enabled: true,
            click_through: true,
        })
        .unwrap();
        assert_eq!(status["mode"], "answer");
        assert_eq!(status["captureProtection"], "best-effort");
        assert_eq!(status["shortcutAvailable"], true);
        assert_eq!(status["visible"], true);
        assert_eq!(status["clickThrough"], true);
        assert_eq!(status["visibilityShortcut"], DEFAULT_VISIBILITY_SHORTCUT);
        assert_eq!(
            serde_json::to_value(CaptureProtection::Disabled).unwrap(),
            "disabled"
        );
    }

    #[test]
    fn workspace_always_accepts_input_and_only_visible_main_has_app_presence() {
        assert!(!Preferences::default().click_through);
        for visible in [false, true] {
            for click_through in [false, true] {
                let main = window_policy(CompanionMode::Main, visible, click_through);
                assert!(main.focusable);
                assert!(!main.ignore_cursor_events);
                assert_eq!(main.application_visible, visible);
                for mode in [CompanionMode::Pin, CompanionMode::Answer] {
                    let floating = window_policy(mode, visible, click_through);
                    assert!(!floating.focusable);
                    assert_eq!(floating.ignore_cursor_events, click_through);
                    assert!(!floating.application_visible);
                }
            }
        }
    }

    #[test]
    fn visibility_shortcut_recovers_mouse_controls_from_click_through_cards() {
        for mode in [CompanionMode::Pin, CompanionMode::Answer] {
            assert_eq!(
                visibility_mode(mode, false, true, RevealIntent::Recovery),
                mode
            );
            assert_eq!(
                visibility_mode(mode, true, false, RevealIntent::Recovery),
                mode
            );
            let restored = visibility_mode(mode, true, true, RevealIntent::Recovery);
            assert_eq!(restored, CompanionMode::Main);
            let policy = window_policy(restored, true, true);
            assert!(!policy.ignore_cursor_events);
            assert!(policy.focusable && policy.application_visible);
            let answer = visibility_mode(mode, true, true, RevealIntent::Answer);
            assert_eq!(answer, mode);
            let answer_policy = window_policy(answer, true, true);
            assert!(answer_policy.ignore_cursor_events);
            assert!(!answer_policy.focusable && !answer_policy.application_visible);
        }
    }

    #[cfg(desktop)]
    #[test]
    fn shortcut_validation_keeps_typing_and_emergency_recovery_available() {
        for shortcut in [
            "Space",
            "Shift+Space",
            "CommandOrControl+Enter",
            "CommandOrControl+Shift+Enter",
            "Alt+Tab",
            "CommandOrControl+Q",
            "CommandOrControl+Space",
        ] {
            assert!(
                parse_shortcut(shortcut, DEFAULT_ANSWER_SHORTCUT).is_err(),
                "{shortcut} must stay available to other controls"
            );
        }
        assert!(shortcut_pair(DEFAULT_VISIBILITY_SHORTCUT, "Alt+Shift+K").is_err());
        assert!(shortcut_pair("Alt+Shift+K", "Alt+Shift+K").is_err());
        let (answer, _, visibility, _) = shortcut_pair("", "").unwrap();
        assert_eq!(answer, DEFAULT_ANSWER_SHORTCUT);
        assert_eq!(visibility, DEFAULT_VISIBILITY_SHORTCUT);
        assert!(shortcut_pair("CommandOrControl+Shift+A", "CommandOrControl+Shift+J").is_ok());
    }

    #[cfg(desktop)]
    #[test]
    fn meeting_shortcut_cannot_replace_answer_or_recovery_shortcuts() {
        for meeting in [
            DEFAULT_ANSWER_SHORTCUT,
            DEFAULT_VISIBILITY_SHORTCUT,
            "Alt+Shift+K",
        ] {
            assert!(shortcut_set(DEFAULT_ANSWER_SHORTCUT, "Alt+Shift+K", meeting).is_err());
        }
        let (_, _, _, _, name, _) = shortcut_set("", "", "").unwrap();
        assert_eq!(name, DEFAULT_MEETING_SHORTCUT);
        assert!(shortcut_set("Alt+Shift+J", "Alt+Shift+K", "Alt+Shift+M").is_ok());
    }
}
