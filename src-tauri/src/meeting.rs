//! Optional desktop-app presence hints. A running app does not prove that a call
//! has started. We inspect executable names only: no command-line arguments,
//! window titles, browser contents, microphone activity, or screen contents.

use serde::Serialize;
use std::{
    collections::{BTreeSet, VecDeque},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};

const MAX_PROCESS_OUTPUT_BYTES: u64 = 1_048_576;
const MAX_PROCESS_ROWS: usize = 20_000;
const PROCESS_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingHint {
    pub app: String,
    pub detail: String,
    pub kind: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum MeetingApp {
    Zoom,
    Teams,
}

impl MeetingApp {
    fn hint(self) -> MeetingHint {
        let app = match self {
            Self::Zoom => "Zoom",
            Self::Teams => "Microsoft Teams",
        };
        MeetingHint {
            app: app.into(),
            kind: "desktop-app".into(),
            detail: format!(
                "{app} opened. This is an app presence hint; NextCue cannot confirm that a meeting is active."
            ),
        }
    }
}

#[derive(Clone, Copy)]
// Each host constructs one variant; cross-platform classifier tests use all.
#[cfg_attr(not(test), allow(dead_code))]
enum Platform {
    Mac,
    Windows,
    Linux,
}

/// The service owning this detector should poll only while the setting is on.
/// The first successful sample establishes a baseline without triggering an
/// action, including after re-enabling detection. A newly opened supported app
/// then produces one hint until it has disappeared and opened again.
#[derive(Default)]
pub struct MeetingDetector {
    initialized: bool,
    previous: BTreeSet<MeetingApp>,
    pending: VecDeque<MeetingApp>,
}

impl MeetingDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Availability means the OS process-list utility is installed. A denied or
    /// failed inspection produces no hint and never creates a false transition.
    pub fn available() -> bool {
        process_command().is_some_and(|(path, _, _)| path.is_file())
    }

    pub async fn poll(&mut self, enabled: bool) -> Option<MeetingHint> {
        if !enabled {
            return self.observe(false, Ok(BTreeSet::new()));
        }
        self.observe(true, read_presence().await)
    }

    fn observe(
        &mut self,
        enabled: bool,
        sample: Result<BTreeSet<MeetingApp>, ()>,
    ) -> Option<MeetingHint> {
        if !enabled {
            self.initialized = false;
            self.previous.clear();
            self.pending.clear();
            return None;
        }

        // Preserve the last trustworthy observation across process-list errors.
        // Treating an error as an empty list would incorrectly re-trigger later.
        let current = sample.ok()?;
        if !self.initialized {
            self.initialized = true;
            self.previous = current;
            return None;
        }

        self.pending.retain(|app| current.contains(app));
        for app in current.difference(&self.previous) {
            if !self.pending.contains(app) {
                self.pending.push_back(*app);
            }
        }
        self.previous = current;
        self.pending.pop_front().map(MeetingApp::hint)
    }
}

fn process_command() -> Option<(PathBuf, &'static [&'static str], Platform)> {
    #[cfg(target_os = "macos")]
    {
        Some((
            PathBuf::from("/bin/ps"),
            &["-x", "-o", "comm="],
            Platform::Mac,
        ))
    }
    #[cfg(target_os = "windows")]
    {
        let windows_directory = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        Some((
            windows_directory.join("System32").join("tasklist.exe"),
            &["/FO", "CSV", "/NH"],
            Platform::Windows,
        ))
    }
    #[cfg(target_os = "linux")]
    {
        ["/usr/bin/ps", "/bin/ps"]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
            .map(|path| (path, &["-x", "-o", "comm="][..], Platform::Linux))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

async fn read_presence() -> Result<BTreeSet<MeetingApp>, ()> {
    let (executable, args, platform) = process_command().ok_or(())?;
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(target_os = "windows")]
    {
        // Inspect the current desktop user's apps without requesting verbose
        // output, which would unnecessarily include window titles.
        let username = std::env::var("USERNAME").map_err(|_| ())?;
        if username.is_empty()
            || username.len() > 256
            || username
                .chars()
                .any(|character| character.is_control() || character == '*')
        {
            return Err(());
        }
        command.args(["/FI", &format!("USERNAME eq {username}")]);
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    // Keep the child inside the timed future: timeout drops and terminates it.
    // take() also prevents an unexpected process list from using unbounded RAM.
    tokio::time::timeout(PROCESS_TIMEOUT, async {
        let mut child = command.spawn().map_err(|_| ())?;
        let stdout = child.stdout.take().ok_or(())?;
        let mut output = Vec::new();
        stdout
            .take(MAX_PROCESS_OUTPUT_BYTES + 1)
            .read_to_end(&mut output)
            .await
            .map_err(|_| ())?;
        if output.len() as u64 > MAX_PROCESS_OUTPUT_BYTES {
            return Err(());
        }
        if !child.wait().await.map_err(|_| ())?.success() {
            return Err(());
        }
        // tasklist may use the Windows local code page. Main executable names
        // in our exact allowlist are ASCII; unrelated non-UTF8 rows must not
        // prevent those names from being recognized.
        let text = String::from_utf8_lossy(&output);
        parse_presence(&text, platform)
    })
    .await
    .map_err(|_| ())?
}

fn parse_presence(text: &str, platform: Platform) -> Result<BTreeSet<MeetingApp>, ()> {
    let mut apps = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        if index >= MAX_PROCESS_ROWS {
            return Err(());
        }
        let image = match platform {
            Platform::Windows => csv_first_field(line),
            Platform::Mac | Platform::Linux => Some(line.trim().to_owned()),
        };
        if let Some(app) = image.and_then(|image| classify_image(&image, platform)) {
            apps.insert(app);
        }
    }
    Ok(apps)
}

fn classify_image(image: &str, platform: Platform) -> Option<MeetingApp> {
    // comm on macOS may return an executable path. Basename comparison avoids
    // matching helpers or similarly named programs by an imprecise substring.
    let name = image.rsplit(['/', '\\']).next()?.trim();
    match platform {
        Platform::Mac => match name {
            "zoom.us" => Some(MeetingApp::Zoom),
            "MSTeams" | "Teams" => Some(MeetingApp::Teams),
            _ => None,
        },
        Platform::Windows => {
            if name.eq_ignore_ascii_case("Zoom.exe") {
                Some(MeetingApp::Zoom)
            } else if name.eq_ignore_ascii_case("ms-teams.exe")
                || name.eq_ignore_ascii_case("Teams.exe")
            {
                Some(MeetingApp::Teams)
            } else {
                None
            }
        }
        // Microsoft's current Linux Teams app is a browser/PWA. Browser process
        // names cannot establish which site is open; those calls remain manual.
        Platform::Linux => (name == "zoom").then_some(MeetingApp::Zoom),
    }
}

/// tasklist CSV can quote image names and use doubled quote escapes. Only the
/// image-name column is consumed; IDs, sessions, and memory values are ignored.
fn csv_first_field(line: &str) -> Option<String> {
    let line = line.trim();
    let rest = line.strip_prefix('"')?;
    let mut chars = rest.chars().peekable();
    let mut field = String::new();
    while let Some(character) = chars.next() {
        if character == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
                field.push('"');
            } else {
                return matches!(chars.peek(), Some(',') | None).then_some(field);
            }
        } else {
            field.push(character);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(apps: &[MeetingApp]) -> Result<BTreeSet<MeetingApp>, ()> {
        Ok(apps.iter().copied().collect())
    }

    #[test]
    fn matches_only_main_executables_and_never_helpers_or_browser_processes() {
        assert_eq!(
            classify_image(
                "/Applications/zoom.us.app/Contents/MacOS/zoom.us",
                Platform::Mac
            ),
            Some(MeetingApp::Zoom)
        );
        assert_eq!(
            classify_image("MSTeams", Platform::Mac),
            Some(MeetingApp::Teams)
        );
        assert_eq!(
            classify_image("Teams", Platform::Mac),
            Some(MeetingApp::Teams)
        );
        assert_eq!(
            classify_image("ZOOM.EXE", Platform::Windows),
            Some(MeetingApp::Zoom)
        );
        assert_eq!(
            classify_image("ms-teams.exe", Platform::Windows),
            Some(MeetingApp::Teams)
        );
        assert_eq!(
            classify_image("zoom", Platform::Linux),
            Some(MeetingApp::Zoom)
        );
        for image in [
            "NextCue",
            "ZoomCefHelper",
            "ZoomAutoUpdater",
            "zoom.us --join",
            "not-zoom.us",
        ] {
            assert_eq!(classify_image(image, Platform::Mac), None);
        }
        for image in [
            "ms-teamsupdate.exe",
            "Zoom.exe.backup",
            "chrome.exe",
            "msedge.exe",
        ] {
            assert_eq!(classify_image(image, Platform::Windows), None);
        }
        for image in [
            "chrome",
            "chromium",
            "firefox",
            "teams",
            "teams-for-linux",
            "ZoomWebviewHost",
        ] {
            assert_eq!(classify_image(image, Platform::Linux), None);
        }
    }

    #[test]
    fn csv_consumes_only_exact_image_name_and_rejects_malformed_rows() {
        assert_eq!(
            csv_first_field(r#""Zoom.exe","42","Console","1","8,192 K""#),
            Some("Zoom.exe".into())
        );
        assert_eq!(
            csv_first_field(r#""a""b.exe","42""#),
            Some("a\"b.exe".into())
        );
        assert_eq!(csv_first_field(r#""Zoom.exe"suffix,"42""#), None);
        assert_eq!(csv_first_field("Zoom.exe"), None);
        assert_eq!(csv_first_field("\"Zoom.exe"), None);
        assert_eq!(
            parse_presence(
                "\"Zoom.exe\",\"1\"\n\"Zoom.exe\",\"2\"\n\"ms-teamsupdate.exe\",\"3\"",
                Platform::Windows
            ),
            sample(&[MeetingApp::Zoom])
        );
    }

    #[test]
    fn first_observation_is_a_baseline_not_a_meeting_trigger() {
        let mut detector = MeetingDetector::new();
        assert!(detector
            .observe(true, sample(&[MeetingApp::Zoom]))
            .is_none());
        assert!(detector
            .observe(true, sample(&[MeetingApp::Zoom]))
            .is_none());
    }

    #[test]
    fn hints_once_per_absent_to_present_transition() {
        let mut detector = MeetingDetector::new();
        assert!(detector.observe(true, sample(&[])).is_none());
        let hint = detector.observe(true, sample(&[MeetingApp::Zoom])).unwrap();
        assert_eq!(hint.app, "Zoom");
        assert!(hint.detail.contains("cannot confirm"));
        assert!(detector
            .observe(true, sample(&[MeetingApp::Zoom]))
            .is_none());
        assert!(detector.observe(true, sample(&[])).is_none());
        assert_eq!(
            detector
                .observe(true, sample(&[MeetingApp::Zoom]))
                .unwrap()
                .app,
            "Zoom"
        );
    }

    #[test]
    fn disable_clears_queue_and_reenable_requires_fresh_baseline() {
        let mut detector = MeetingDetector::new();
        detector.observe(true, sample(&[]));
        detector.observe(true, sample(&[MeetingApp::Zoom, MeetingApp::Teams]));
        assert!(detector.observe(false, sample(&[])).is_none());
        assert!(detector
            .observe(true, sample(&[MeetingApp::Teams]))
            .is_none());
        assert!(detector
            .observe(true, sample(&[MeetingApp::Teams]))
            .is_none());
    }

    #[test]
    fn failed_inspection_does_not_reset_baseline_or_retrigger() {
        let mut detector = MeetingDetector::new();
        assert!(detector.observe(true, Err(())).is_none());
        assert!(detector
            .observe(true, sample(&[MeetingApp::Zoom]))
            .is_none());
        assert!(detector.observe(true, Err(())).is_none());
        assert!(detector
            .observe(true, sample(&[MeetingApp::Zoom]))
            .is_none());
    }

    #[test]
    fn concurrent_apps_queue_once_and_closed_pending_app_is_not_reported() {
        let mut detector = MeetingDetector::new();
        detector.observe(true, sample(&[]));
        assert_eq!(
            detector
                .observe(true, sample(&[MeetingApp::Zoom, MeetingApp::Teams]))
                .unwrap()
                .app,
            "Zoom"
        );
        assert_eq!(
            detector
                .observe(true, sample(&[MeetingApp::Zoom, MeetingApp::Teams]))
                .unwrap()
                .app,
            "Microsoft Teams"
        );
        assert!(detector
            .observe(true, sample(&[MeetingApp::Zoom, MeetingApp::Teams]))
            .is_none());

        let mut detector = MeetingDetector::new();
        detector.observe(true, sample(&[]));
        detector.observe(true, sample(&[MeetingApp::Zoom, MeetingApp::Teams]));
        assert!(detector
            .observe(true, sample(&[MeetingApp::Zoom]))
            .is_none());
    }

    #[test]
    fn process_output_row_limit_rejects_incomplete_observations() {
        let too_many = "other\n".repeat(MAX_PROCESS_ROWS + 1);
        assert_eq!(parse_presence(&too_many, Platform::Mac), Err(()));
    }
}
