// Preferences, stored as plain JSON in settings.json under platform::config_dir().
// No secret ever lands here — API keys live in the OS keychain (see secrets.rs).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::ident;

const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub sound_enabled: bool,
    pub sound_volume: f64,
    pub auto_close_interval: f64,
    pub absence_interval: f64,
    pub active_integrations: Vec<String>,
    /// "primary" = the main display, "cursor" = whichever display the mouse is on.
    pub screen: String,
    pub autostart: bool,
    /// Claude model used by the chat. Changeable in the settings window.
    /// Defaulted explicitly so a settings.json written by an older build still loads.
    #[serde(default = "default_model")]
    pub model: String,
    /// Alert kinds, switched separately (spec §2). A settings.json from before
    /// they existed — or copied from Coucou — loads with all three on.
    #[serde(default = "default_true")]
    pub alert_waiting: bool,
    #[serde(default = "default_true")]
    pub alert_finished: bool,
    #[serde(default = "default_true")]
    pub alert_error: bool,
    /// Global shortcut that shows and hides the island (hotkey spec §2): `Alt+M` unless the
    /// user changed it. A settings.json from before it existed loads with the default.
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// The shortcut's on/off switch in Settings. Off keeps `hotkey`, so turning it on again
    /// brings the same shortcut back.
    #[serde(default = "default_true")]
    pub hotkey_enabled: bool,
    /// The prompt view's last chosen agent (launcher spec §3), a wire name of `launcher::AgentId`;
    /// "" until one is chosen. Kept as text so a name this build does not know still loads.
    #[serde(default)]
    pub launcher_agent: String,
    /// Unused since the pomodoro replaced the one-shot focus; kept so older settings files load and round-trip.
    #[serde(default = "default_focus_minutes")]
    pub focus_minutes: u32,
    /// During a muted pomodoro work interval, permission and question waits still open the island (pomodoro spec §2.3).
    #[serde(default = "default_true")]
    pub focus_let_waits_through: bool,
    /// Pomodoro lengths in minutes and the work intervals before a long break (pomodoro spec §3).
    /// The island only offers the listed values and falls back to the default for anything else.
    /// A settings.json from before the pomodoro loads with 25 / 5 / 15 / 4.
    #[serde(default = "default_pomodoro_work")]
    pub pomodoro_work: u32,
    #[serde(default = "default_pomodoro_short")]
    pub pomodoro_short: u32,
    #[serde(default = "default_pomodoro_long")]
    pub pomodoro_long: u32,
    #[serde(default = "default_pomodoro_cycles")]
    pub pomodoro_cycles: u32,
    /// Agent alerts are held back during work intervals (pomodoro spec §2.3).
    #[serde(default = "default_true")]
    pub pomodoro_mute: bool,
    /// The player row in the footer of the main tabs (night spec §B). Off, the media thread drops its
    /// subscriptions. A settings.json from before the player loads with it on.
    #[serde(default = "default_true")]
    pub show_media: bool,
    /// UI language (batch spec §C): "system" follows the OS language, else a code the island knows
    /// (en ru de fr es zh ja). Rust only stores it. A settings.json from before it loads with "system".
    #[serde(default = "default_language")]
    pub language: String,
    /// The page the island opens on when the user opens it (quick-wins spec §2): "sessions",
    /// "overview", "stats", "focus", "apps" or "last". Rust only stores it; a value outside that
    /// list loads as "sessions". A settings.json from before it loads with "sessions".
    #[serde(default = "default_open_on_click")]
    pub open_on_click: String,
    /// The home page reads Codex's limits from the Codex CLI's app server when the rollout ones are
    /// stale (`codex_limits`). A settings.json from before it loads with it on.
    #[serde(default = "default_true")]
    pub codex_fresh_limits: bool,
}

/// The values of `open_on_click` the island understands.
const OPEN_ON_CLICK: [&str; 6] = ["sessions", "overview", "stats", "focus", "apps", "last"];

impl Settings {
    /// What gets registered: the shortcut while it is switched on, "" (nothing) while off.
    pub fn active_hotkey(&self) -> &str {
        if self.hotkey_enabled { &self.hotkey } else { "" }
    }

    /// The first shortcut build stored "off" as an empty `hotkey`: that is now the switch off
    /// with the default shortcut kept for when it is turned on again.
    fn migrated(mut self) -> Self {
        if self.hotkey.trim().is_empty() {
            self.hotkey = default_hotkey();
            self.hotkey_enabled = false;
        }
        if !OPEN_ON_CLICK.contains(&self.open_on_click.as_str()) {
            self.open_on_click = default_open_on_click();
        }
        self
    }
}

fn default_model() -> String {
    crate::claude::DEFAULT_MODEL.to_string()
}

fn default_true() -> bool {
    true
}

fn default_hotkey() -> String {
    crate::hotkey::DEFAULT_COMBO.to_string()
}

fn default_focus_minutes() -> u32 {
    25
}

fn default_pomodoro_work() -> u32 {
    25
}

fn default_pomodoro_short() -> u32 {
    5
}

fn default_pomodoro_long() -> u32 {
    15
}

fn default_pomodoro_cycles() -> u32 {
    4
}

fn default_language() -> String {
    "system".to_string()
}

fn default_open_on_click() -> String {
    "sessions".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sound_enabled: true,
            sound_volume: 0.12,
            auto_close_interval: 15.0,
            absence_interval: 180.0,
            active_integrations: vec![
                "integration_resend".into(),
                "integration_n8n".into(),
                "integration_vercel".into(),
                "integration_github".into(),
            ],
            screen: "primary".into(),
            autostart: false,
            model: default_model(),
            alert_waiting: true,
            alert_finished: true,
            alert_error: true,
            hotkey: default_hotkey(),
            hotkey_enabled: true,
            launcher_agent: String::new(),
            focus_minutes: default_focus_minutes(),
            focus_let_waits_through: true,
            pomodoro_work: default_pomodoro_work(),
            pomodoro_short: default_pomodoro_short(),
            pomodoro_long: default_pomodoro_long(),
            pomodoro_cycles: default_pomodoro_cycles(),
            pomodoro_mute: true,
            show_media: true,
            language: default_language(),
            open_on_click: default_open_on_click(),
            codex_fresh_limits: true,
        }
    }
}

pub use crate::platform::{config_dir, local_dir};

pub fn hook_exe_path() -> PathBuf {
    local_dir().join("bin").join(crate::platform::HOOK_EXE)
}

fn settings_path() -> PathBuf {
    config_dir().join(SETTINGS_FILE)
}

/// %APPDATA%, the parent of `config_dir()`; only `load()` needs it.
fn appdata_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// One-time migration from Coucou: copies `<appdata>\Coucou\settings.json` to
/// `<appdata>\Mayak\settings.json` when Mayak has no settings file yet and the
/// Coucou one exists. The Coucou file and folder are left exactly as they are.
/// `Ok(true)` when a copy was made; `Ok(false)` when there was nothing to do.
pub fn copy_legacy_settings(appdata: &Path) -> std::io::Result<bool> {
    let target = appdata.join(ident::APP_DIR).join(SETTINGS_FILE);
    match std::fs::metadata(&target) {
        Ok(_) => return Ok(false),
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        // Unknown state of our own file: refuse rather than risk overwriting it.
        Err(err) => return Err(err),
    }
    let source = appdata.join(ident::LEGACY_DIR).join(SETTINGS_FILE);
    match std::fs::metadata(&source) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Ok(false),
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    }
    std::fs::create_dir_all(appdata.join(ident::APP_DIR))?;
    std::fs::copy(&source, &target)?;
    Ok(true)
}

pub fn load() -> Settings {
    match copy_legacy_settings(&appdata_dir()) {
        Ok(true) => crate::log::line("settings: copied the Coucou settings.json"),
        Ok(false) => {}
        Err(err) => crate::log::line(format!("settings: Coucou settings.json not copied: {err}")),
    }
    load_from(&settings_path())
}

/// `load` for the file at `path` (tests use a temp folder). One bad value never costs the
/// others: a field of the wrong type takes its default and the rest are kept. A file that is
/// not a JSON object gives the defaults and is copied beside itself first, so the next save
/// cannot destroy it. Log lines are fixed text: never the file's content.
fn load_from(path: &Path) -> Settings {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound => return Settings::default(),
        Err(err) => {
            crate::log::line(format!("settings: settings.json not read ({:?}), using the defaults", err.kind()));
            mark_unread(path);
            return Settings::default();
        }
    };
    let settings = match fields(&bytes) {
        Some(fields) => from_fields(fields),
        None => {
            match keep_aside(path, "broken", &bytes) {
                Ok(_) => crate::log::line(
                    "settings: settings.json is not a JSON object, a copy is kept beside it; using the defaults",
                ),
                Err(err) => crate::log::line(format!(
                    "settings: settings.json is not a JSON object and no copy could be kept ({:?}); using the defaults",
                    err.kind()
                )),
            }
            Settings::default()
        }
    };
    settings.migrated()
}

/// The fields of a settings.json (a UTF-8 BOM, as Notepad and PowerShell 5 write it, is
/// skipped; an empty file has none), or None when it is not a JSON object at all.
fn fields(bytes: &[u8]) -> Option<Map<String, Value>> {
    let text = crate::hookfile::strip_bom(bytes);
    if text.iter().all(u8::is_ascii_whitespace) {
        return Some(Map::new());
    }
    match serde_json::from_slice::<Value>(text) {
        Ok(Value::Object(fields)) => Some(fields),
        _ => None,
    }
}

/// The settings in `fields`: all of them when they load as a whole, else field by field on
/// top of the defaults, each one kept only when it deserializes. A missing field takes its
/// default; an unknown one is ignored, as in a whole load.
fn from_fields(fields: Map<String, Value>) -> Settings {
    if let Ok(settings) = serde_json::from_value::<Settings>(Value::Object(fields.clone())) {
        return settings;
    }
    let mut settings = Settings::default();
    // A struct of plain fields always serializes to an object; the else is never taken.
    let Ok(Value::Object(mut kept)) = serde_json::to_value(&settings) else {
        return settings;
    };
    let mut dropped = 0;
    for (key, value) in fields {
        let mut candidate = kept.clone();
        candidate.insert(key, value);
        match serde_json::from_value::<Settings>(Value::Object(candidate.clone())) {
            Ok(next) => {
                settings = next;
                kept = candidate;
            }
            Err(_) => dropped += 1,
        }
    }
    if dropped > 0 {
        crate::log::line(format!("settings: {dropped} field(s) of settings.json not usable, their defaults are used"));
    }
    settings
}

/// The settings.json files `load_from` could not read (an error other than NotFound): what is
/// in memory for them is the defaults, so the first save over one keeps a copy of it first,
/// whatever it holds by then. A list, not one path, so tests in temp folders stay apart.
static UNREAD: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn unread() -> MutexGuard<'static, Vec<PathBuf>> {
    UNREAD.lock().unwrap_or_else(PoisonError::into_inner)
}

fn mark_unread(path: &Path) {
    let mut list = unread();
    if !list.iter().any(|p| p == path) {
        list.push(path.to_path_buf());
    }
}

/// Removes a file this module made and must not leave behind; a failure is logged (fixed
/// text, the error kind), and one that is already gone is nothing to report.
fn remove_leftover(path: &Path, what: &str) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => crate::log::line(format!("settings: {what} could not be removed ({:?})", err.kind())),
    }
}

/// Copies `bytes`, the content of `path`, to `<name>.<kind>-<unix secs>` beside it (`-1`,
/// `-2`… while that name is taken), never over an existing file. Once: when a copy of that
/// kind with the same bytes is already there, nothing is written and `Ok(false)` is returned.
fn keep_aside(path: &Path, kind: &str, bytes: &[u8]) -> std::io::Result<bool> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let prefix = format!("{name}.{kind}-");
    if let Some(dir) = path.parent() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with(&prefix)
                && entry.metadata().is_ok_and(|m| m.len() == bytes.len() as u64)
                && std::fs::read(entry.path()).is_ok_and(|held| held == bytes)
            {
                return Ok(false);
            }
        }
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    for n in 0..100 {
        let copy = match n {
            0 => path.with_file_name(format!("{prefix}{secs}")),
            n => path.with_file_name(format!("{prefix}{secs}-{n}")),
        };
        let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&copy) {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        };
        let written = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        if written.is_err() {
            // A copy that is not whole must not look like a faithful one.
            remove_leftover(&copy, "an incomplete copy of settings.json");
        }
        return written.map(|()| true);
    }
    Err(std::io::Error::new(ErrorKind::AlreadyExists, "too many copies of settings.json in one second"))
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    let dir = config_dir();
    crate::platform::ensure_private_dir(&dir)?;
    save_to(&settings_path(), settings)
}

/// `save` for the file at `path`, whose folder exists. Before it is replaced, the file is
/// copied aside (see `keep_aside`) when the launch could not read it (`.unread-`) or when it
/// is not a JSON object (`.broken-`); a file that cannot be read now is not replaced at all.
/// The new content goes to `<name>.tmp`, flushed to disk, then renamed over the target: a
/// crash or a full disk leaves the previous file whole, and no temp file stays behind.
fn save_to(path: &Path, settings: &Settings) -> std::io::Result<()> {
    let was_unread = unread().iter().any(|p| p == path);
    match std::fs::read(path) {
        Ok(bytes) if was_unread => {
            keep_aside(path, "unread", &bytes)?;
            crate::log::line("settings: settings.json could not be read at launch, a copy is kept beside it before saving");
        }
        Ok(bytes) => {
            if fields(&bytes).is_none() && keep_aside(path, "broken", &bytes)? {
                crate::log::line("settings: settings.json was not a JSON object, a copy is kept beside it before saving");
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    if was_unread {
        // Kept, or gone since: nothing is left to protect.
        unread().retain(|p| p != path);
    }
    let json = serde_json::to_vec_pretty(settings).map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // A fixed temp name is safe because `save_settings` is a synchronous command, which Tauri runs
    // on the main thread, so two saves never overlap. If it ever becomes async, make this name unique.
    let temp = path.with_file_name(format!("{name}.tmp"));
    let written = crate::hookfile::write_synced(&temp, &json).and_then(|()| std::fs::rename(&temp, path));
    if written.is_err() {
        remove_leftover(&temp, "settings.json.tmp");
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn coucou_settings_are_copied_when_mayak_has_none() {
        let tmp = TempDir::new("settings-copy");
        let coucou = tmp.path.join("Coucou");
        std::fs::create_dir_all(&coucou).unwrap();
        let original = br#"{"soundEnabled":false,"activeIntegrations":["integration_github"]}"#;
        std::fs::write(coucou.join("settings.json"), original).unwrap();

        assert!(copy_legacy_settings(&tmp.path).unwrap(), "the copy must be reported");

        let copied = std::fs::read(tmp.path.join("Mayak").join("settings.json")).unwrap();
        assert_eq!(copied, original, "the copy must be byte for byte");
        let left = std::fs::read(coucou.join("settings.json")).unwrap();
        assert_eq!(left, original, "the Coucou file must stay untouched");
    }

    #[test]
    fn existing_mayak_settings_are_never_overwritten() {
        let tmp = TempDir::new("settings-keep");
        std::fs::create_dir_all(tmp.path.join("Coucou")).unwrap();
        std::fs::create_dir_all(tmp.path.join("Mayak")).unwrap();
        std::fs::write(tmp.path.join("Coucou").join("settings.json"), b"{\"from\":\"coucou\"}").unwrap();
        std::fs::write(tmp.path.join("Mayak").join("settings.json"), b"{\"from\":\"mayak\"}").unwrap();

        assert!(!copy_legacy_settings(&tmp.path).unwrap());

        let mayak = std::fs::read(tmp.path.join("Mayak").join("settings.json")).unwrap();
        assert_eq!(mayak, b"{\"from\":\"mayak\"}");
    }

    #[test]
    fn nothing_to_copy_creates_nothing() {
        let tmp = TempDir::new("settings-none");

        assert!(!copy_legacy_settings(&tmp.path).unwrap());

        assert!(!tmp.path.join("Mayak").exists(), "no Coucou file, so no Mayak folder either");
    }

    #[test]
    fn alert_preferences_default_to_on_for_an_older_file() {
        // Exactly the keys a Coucou-era settings.json has, hooksInstalled included.
        let old = r#"{"soundEnabled":true,"soundVolume":0.12,"autoCloseInterval":15,"absenceInterval":180,"activeIntegrations":["integration_github"],"screen":"primary","autostart":false,"hooksInstalled":true,"model":"claude-opus-5"}"#;
        let s: super::Settings = serde_json::from_str(old).expect("an older settings.json must still load");
        assert!(s.alert_waiting && s.alert_finished && s.alert_error);
        assert_eq!(s.active_integrations, vec!["integration_github".to_string()]);
    }

    #[test]
    fn alert_preferences_round_trip_in_camel_case() {
        let mut s = super::Settings::default();
        assert!(s.alert_waiting && s.alert_finished && s.alert_error);
        s.alert_finished = false;
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["alertWaiting"], true);
        assert_eq!(v["alertFinished"], false);
        assert_eq!(v["alertError"], true);
        let back: super::Settings = serde_json::from_value(v).unwrap();
        assert!(back.alert_waiting && !back.alert_finished && back.alert_error);
    }

    #[test]
    fn hooks_installed_is_no_longer_stored() {
        // Spec §8: the field is gone; an old file that has it still loads (above).
        let v = serde_json::to_value(super::Settings::default()).unwrap();
        assert!(v.get("hooksInstalled").is_none(), "got: {v}");
    }

    #[test]
    fn an_older_settings_file_gets_alt_m_switched_on() {
        // Exactly the keys a settings.json from before the shortcut has.
        let old = r#"{"soundEnabled":true,"soundVolume":0.12,"autoCloseInterval":15,"absenceInterval":180,"activeIntegrations":["integration_github"],"screen":"primary","autostart":false,"model":"claude-opus-5","alertWaiting":true,"alertFinished":true,"alertError":true}"#;
        let s = serde_json::from_str::<super::Settings>(old).expect("an older settings.json must still load").migrated();
        assert_eq!(s.hotkey, "Alt+M");
        assert!(s.hotkey_enabled);
        assert_eq!(s.active_hotkey(), "Alt+M");
        let d = super::Settings::default();
        assert_eq!((d.hotkey.as_str(), d.hotkey_enabled), ("Alt+M", true));
    }

    #[test]
    fn the_first_builds_empty_shortcut_is_the_switch_off_with_alt_m_kept() {
        let first = r#"{"soundEnabled":true,"soundVolume":0.12,"autoCloseInterval":15,"absenceInterval":180,"activeIntegrations":[],"screen":"primary","autostart":false,"model":"claude-opus-5","alertWaiting":true,"alertFinished":true,"alertError":true,"hotkey":""}"#;
        let s = serde_json::from_str::<super::Settings>(first).unwrap().migrated();
        assert_eq!(s.hotkey, "Alt+M");
        assert!(!s.hotkey_enabled);
        assert_eq!(s.active_hotkey(), "");
    }

    #[test]
    fn the_switch_off_keeps_the_shortcut_through_a_save_and_a_load() {
        let mut off = super::Settings::default();
        off.hotkey = "Ctrl+Alt+K".into();
        off.hotkey_enabled = false;
        let v = serde_json::to_value(&off).unwrap();
        assert_eq!((v["hotkey"].as_str(), v["hotkeyEnabled"].as_bool()), (Some("Ctrl+Alt+K"), Some(false)));
        let back = serde_json::from_value::<super::Settings>(v).unwrap().migrated();
        assert_eq!((back.hotkey.as_str(), back.hotkey_enabled, back.active_hotkey()), ("Ctrl+Alt+K", false, ""));
    }

    #[test]
    fn an_older_settings_file_loads_with_no_launcher_agent() {
        // Exactly the keys a settings.json from before the launcher has.
        let old = r#"{"soundEnabled":true,"soundVolume":0.12,"autoCloseInterval":15,"absenceInterval":180,"activeIntegrations":[],"screen":"primary","autostart":false,"model":"claude-opus-5","alertWaiting":true,"alertFinished":true,"alertError":true,"hotkey":"Alt+M","hotkeyEnabled":true}"#;
        let s = serde_json::from_str::<super::Settings>(old).expect("an older settings.json must still load");
        assert_eq!(s.launcher_agent, "");
        assert_eq!(super::Settings::default().launcher_agent, "");
    }

    #[test]
    fn the_launcher_agent_round_trips_in_camel_case() {
        let mut s = super::Settings::default();
        s.launcher_agent = "codexDesktop".into();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["launcherAgent"], "codexDesktop");
        let back: super::Settings = serde_json::from_value(v).unwrap();
        assert_eq!(back.launcher_agent, "codexDesktop");
    }

    #[test]
    fn an_older_settings_file_loads_with_a_25_minute_focus_that_lets_waits_through() {
        // Exactly the keys a settings.json from before focus mode has.
        let old = r#"{"soundEnabled":true,"soundVolume":0.12,"autoCloseInterval":15,"absenceInterval":180,"activeIntegrations":[],"screen":"primary","autostart":false,"model":"claude-opus-5","alertWaiting":true,"alertFinished":true,"alertError":true,"hotkey":"Alt+M","hotkeyEnabled":true,"launcherAgent":""}"#;
        let s = serde_json::from_str::<super::Settings>(old).expect("an older settings.json must still load");
        assert_eq!((s.focus_minutes, s.focus_let_waits_through), (25, true));
        let d = super::Settings::default();
        assert_eq!((d.focus_minutes, d.focus_let_waits_through), (25, true));
    }

    #[test]
    fn the_focus_settings_round_trip_in_camel_case() {
        let mut s = super::Settings::default();
        s.focus_minutes = 90;
        s.focus_let_waits_through = false;
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!((v["focusMinutes"].as_u64(), v["focusLetWaitsThrough"].as_bool()), (Some(90), Some(false)));
        let back: super::Settings = serde_json::from_value(v).unwrap();
        assert_eq!((back.focus_minutes, back.focus_let_waits_through), (90, false));
    }

    #[test]
    fn an_older_settings_file_loads_with_the_default_pomodoro() {
        // A settings.json from before the pomodoro: it has focusMinutes and no pomodoro keys.
        let old = r#"{"soundEnabled":true,"soundVolume":0.12,"autoCloseInterval":15,"absenceInterval":180,"activeIntegrations":[],"screen":"primary","autostart":false,"model":"claude-opus-5","focusMinutes":50,"focusLetWaitsThrough":false}"#;
        let s: super::Settings = serde_json::from_str(old).unwrap();
        assert_eq!(
            (s.pomodoro_work, s.pomodoro_short, s.pomodoro_long, s.pomodoro_cycles, s.pomodoro_mute),
            (25, 5, 15, 4, true)
        );
        assert!(!s.focus_let_waits_through);
        let d = super::Settings::default();
        assert_eq!(
            (d.pomodoro_work, d.pomodoro_short, d.pomodoro_long, d.pomodoro_cycles, d.pomodoro_mute),
            (25, 5, 15, 4, true)
        );
    }

    #[test]
    fn the_pomodoro_settings_round_trip_in_camel_case() {
        let mut s = super::Settings::default();
        s.pomodoro_work = 50;
        s.pomodoro_short = 10;
        s.pomodoro_long = 30;
        s.pomodoro_cycles = 2;
        s.pomodoro_mute = false;
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["pomodoroWork"].as_u64(), Some(50));
        assert_eq!(v["pomodoroShort"].as_u64(), Some(10));
        assert_eq!(v["pomodoroLong"].as_u64(), Some(30));
        assert_eq!(v["pomodoroCycles"].as_u64(), Some(2));
        assert_eq!(v["pomodoroMute"].as_bool(), Some(false));
        let back: super::Settings = serde_json::from_value(v).unwrap();
        assert_eq!(
            (back.pomodoro_work, back.pomodoro_short, back.pomodoro_long, back.pomodoro_cycles, back.pomodoro_mute),
            (50, 10, 30, 2, false)
        );
    }

    #[test]
    fn an_older_settings_file_loads_with_the_player_shown() {
        // Exactly the keys a settings.json from before the player has.
        let old = r#"{"soundEnabled":true,"soundVolume":0.12,"autoCloseInterval":15,"absenceInterval":180,"activeIntegrations":[],"screen":"primary","autostart":false,"model":"claude-opus-5","alertWaiting":true,"alertFinished":true,"alertError":true,"hotkey":"Alt+M","hotkeyEnabled":true,"launcherAgent":"","focusMinutes":25,"focusLetWaitsThrough":true}"#;
        let s = serde_json::from_str::<super::Settings>(old).expect("an older settings.json must still load");
        assert!(s.show_media);
        assert!(super::Settings::default().show_media);
    }

    #[test]
    fn the_player_setting_round_trips_in_camel_case() {
        let mut s = super::Settings::default();
        s.show_media = false;
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["showMedia"], false);
        let back: super::Settings = serde_json::from_value(v).unwrap();
        assert!(!back.show_media);
    }

    #[test]
    fn an_older_settings_file_loads_with_the_system_language() {
        // Exactly the keys a settings.json from before the language setting has.
        let old = r#"{"soundEnabled":true,"soundVolume":0.12,"autoCloseInterval":15,"absenceInterval":180,"activeIntegrations":[],"screen":"primary","autostart":false,"model":"claude-opus-5","alertWaiting":true,"alertFinished":true,"alertError":true,"hotkey":"Alt+M","hotkeyEnabled":true,"launcherAgent":"","focusMinutes":25,"focusLetWaitsThrough":true,"showMedia":true}"#;
        let s = serde_json::from_str::<super::Settings>(old).expect("an older settings.json must still load");
        assert_eq!(s.language, "system");
        assert_eq!(super::Settings::default().language, "system");
    }

    #[test]
    fn an_older_settings_file_loads_with_fresh_codex_limits_on() {
        // Exactly the keys a settings.json from before the switch has.
        let old = r#"{"soundEnabled":true,"soundVolume":0.12,"autoCloseInterval":15,"absenceInterval":180,"activeIntegrations":[],"screen":"primary","autostart":false,"model":"claude-opus-5","alertWaiting":true,"alertFinished":true,"alertError":true,"hotkey":"Alt+M","hotkeyEnabled":true,"launcherAgent":"","focusMinutes":25,"focusLetWaitsThrough":true,"showMedia":true,"language":"system","openOnClick":"sessions"}"#;
        let s = serde_json::from_str::<super::Settings>(old).expect("an older settings.json must still load");
        assert!(s.codex_fresh_limits);
        assert!(super::Settings::default().codex_fresh_limits);
        let mut off = super::Settings::default();
        off.codex_fresh_limits = false;
        let v = serde_json::to_value(&off).unwrap();
        assert_eq!(v["codexFreshLimits"], false);
        assert!(!serde_json::from_value::<super::Settings>(v).unwrap().codex_fresh_limits);
    }

    // ── Loading a damaged file, saving atomically ─────────────────────────────

    /// A settings.json in which the values the brief names are not the defaults.
    const CUSTOM: &str = r#"{"soundEnabled":false,"soundVolume":0.5,"autoCloseInterval":30,"absenceInterval":60,"activeIntegrations":["integration_github"],"screen":"cursor","autostart":true,"model":"some-model","hotkey":"Ctrl+Alt+K","hotkeyEnabled":true,"pomodoroWork":50,"pomodoroShort":10,"pomodoroLong":30,"pomodoroCycles":2,"language":"de"}"#;

    fn file_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn assert_custom(s: &Settings) {
        assert_eq!(s.language, "de");
        assert_eq!((s.hotkey.as_str(), s.hotkey_enabled), ("Ctrl+Alt+K", true));
        assert_eq!((s.pomodoro_work, s.pomodoro_short, s.pomodoro_long, s.pomodoro_cycles), (50, 10, 30, 2));
        assert_eq!((s.sound_enabled, s.screen.as_str(), s.autostart), (false, "cursor", true));
        assert_eq!(s.active_integrations, vec!["integration_github".to_string()]);
    }

    #[test]
    fn a_good_file_loads_unchanged_and_is_left_alone() {
        let tmp = TempDir::new("settings-good");
        let file = tmp.path.join("settings.json");
        std::fs::write(&file, CUSTOM).unwrap();

        let s = load_from(&file);

        assert_custom(&s);
        assert_eq!(s.sound_volume, 0.5);
        assert_eq!(std::fs::read(&file).unwrap(), CUSTOM.as_bytes());
        assert_eq!(file_names(&tmp.path), ["settings.json"]);
    }

    #[test]
    fn a_file_with_a_utf8_bom_loads() {
        let tmp = TempDir::new("settings-bom");
        let file = tmp.path.join("settings.json");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(CUSTOM.as_bytes());
        std::fs::write(&file, &bytes).unwrap();

        assert_custom(&load_from(&file));
        assert_eq!(file_names(&tmp.path), ["settings.json"], "a BOM is not damage");
    }

    #[test]
    fn one_wrong_typed_field_takes_its_default_and_keeps_the_others() {
        let tmp = TempDir::new("settings-field");
        let file = tmp.path.join("settings.json");
        for (key, wrong) in [
            ("soundVolume", serde_json::json!("loud")),
            ("autostart", serde_json::json!(1)),
            ("activeIntegrations", serde_json::json!("integration_github")),
            ("screen", serde_json::Value::Null),
        ] {
            let mut v: serde_json::Value = serde_json::from_str(CUSTOM).unwrap();
            v[key] = wrong;
            std::fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();

            let s = load_from(&file);

            let shown = serde_json::to_value(&s).unwrap();
            assert_eq!(shown[key], serde_json::to_value(Settings::default()).unwrap()[key], "{key} takes its default");
            assert_eq!(s.language, "de", "{key}");
            assert_eq!((s.hotkey.as_str(), s.hotkey_enabled), ("Ctrl+Alt+K", true), "{key}");
            assert_eq!((s.pomodoro_work, s.pomodoro_short, s.pomodoro_long, s.pomodoro_cycles), (50, 10, 30, 2), "{key}");
            assert_eq!(s.model, "some-model", "{key}");
        }
        assert_eq!(file_names(&tmp.path), ["settings.json"], "a bad field is not a broken file");
    }

    #[test]
    fn a_rescued_file_still_goes_through_the_migration() {
        // The first shortcut build's empty hotkey, next to a field of the wrong type.
        let tmp = TempDir::new("settings-rescue-migrate");
        let file = tmp.path.join("settings.json");
        std::fs::write(&file, br#"{"soundVolume":"loud","hotkey":"","language":"fr"}"#).unwrap();

        let s = load_from(&file);

        assert_eq!((s.hotkey.as_str(), s.hotkey_enabled, s.language.as_str()), ("Alt+M", false, "fr"));
        assert_eq!(s.sound_volume, Settings::default().sound_volume);
    }

    #[test]
    fn a_file_that_is_not_json_gives_the_defaults_and_is_kept_once() {
        let tmp = TempDir::new("settings-broken");
        let file = tmp.path.join("settings.json");
        let broken = b"{\"language\":\"de\", oops";
        std::fs::write(&file, broken).unwrap();

        let s = load_from(&file);

        assert_eq!(serde_json::to_value(&s).unwrap(), serde_json::to_value(Settings::default()).unwrap());
        let names = file_names(&tmp.path);
        assert_eq!(names.len(), 2, "{names:?}");
        let copy = &names[1];
        let secs = copy.strip_prefix("settings.json.broken-").unwrap_or_else(|| panic!("{copy}"));
        assert!(!secs.is_empty() && secs.chars().all(|c| c.is_ascii_digit()), "{copy}");
        assert_eq!(std::fs::read(tmp.path.join(copy)).unwrap(), broken);
        assert_eq!(std::fs::read(&file).unwrap(), broken, "loading never rewrites the file");

        // A second launch with the same file makes no second copy, and neither does the save.
        load_from(&file);
        save_to(&file, &s).unwrap();
        assert_eq!(file_names(&tmp.path), ["settings.json", copy.as_str()]);
        assert_eq!(std::fs::read(tmp.path.join(copy)).unwrap(), broken);
    }

    #[test]
    fn json_that_is_not_an_object_is_kept_too_and_an_empty_file_is_not() {
        let tmp = TempDir::new("settings-not-object");
        let file = tmp.path.join("settings.json");
        std::fs::write(&file, b"[1, 2]").unwrap();
        load_from(&file);
        assert_eq!(file_names(&tmp.path).len(), 2);

        let tmp = TempDir::new("settings-empty");
        let file = tmp.path.join("settings.json");
        std::fs::write(&file, b" \r\n").unwrap();
        let s = load_from(&file);
        assert_eq!(serde_json::to_value(&s).unwrap(), serde_json::to_value(Settings::default()).unwrap());
        assert_eq!(file_names(&tmp.path), ["settings.json"]);
    }

    #[test]
    fn a_save_over_a_file_broken_since_the_load_keeps_a_copy_first() {
        let tmp = TempDir::new("settings-broken-later");
        let file = tmp.path.join("settings.json");
        std::fs::write(&file, b"not json at all").unwrap();

        save_to(&file, &Settings::default()).unwrap();

        let names = file_names(&tmp.path);
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names[1].starts_with("settings.json.broken-"), "{names:?}");
        assert_eq!(std::fs::read(tmp.path.join(&names[1])).unwrap(), b"not json at all");
        assert!(serde_json::from_slice::<Settings>(&std::fs::read(&file).unwrap()).is_ok(), "then the save is written");
    }

    #[test]
    fn a_file_without_a_required_field_is_rescued_with_its_other_values() {
        // A Coucou-era file (hooksInstalled and all) that lacks "screen".
        let tmp = TempDir::new("settings-missing-field");
        let file = tmp.path.join("settings.json");
        std::fs::write(&file, br#"{"soundEnabled":false,"soundVolume":0.3,"autoCloseInterval":20,"absenceInterval":90,"activeIntegrations":["integration_github"],"autostart":true,"hooksInstalled":true,"model":"claude-opus-5"}"#).unwrap();

        let s = load_from(&file);

        assert_eq!(s.screen, "primary", "the missing field takes its default");
        assert_eq!((s.sound_enabled, s.sound_volume, s.auto_close_interval, s.absence_interval), (false, 0.3, 20.0, 90.0));
        assert_eq!((s.autostart, s.model.as_str()), (true, "claude-opus-5"));
        assert_eq!(s.active_integrations, vec!["integration_github".to_string()]);
        assert_eq!(file_names(&tmp.path), ["settings.json"]);
    }

    #[test]
    fn a_file_unreadable_at_launch_is_copied_before_the_first_save_over_it() {
        let tmp = TempDir::new("settings-unread");
        let file = tmp.path.join("settings.json");
        // A folder in its place: reading it fails with an error that is not NotFound.
        std::fs::create_dir(&file).unwrap();
        let s = load_from(&file);
        assert_eq!(serde_json::to_value(&s).unwrap(), serde_json::to_value(Settings::default()).unwrap());
        assert!(save_to(&file, &s).is_err(), "what can't be read is not replaced");

        // Readable again by the next save, holding what the user had.
        std::fs::remove_dir(&file).unwrap();
        std::fs::write(&file, CUSTOM).unwrap();
        let mut changed = s.clone();
        changed.language = "ja".into();
        save_to(&file, &changed).unwrap();

        let names = file_names(&tmp.path);
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names[1].starts_with("settings.json.unread-"), "{names:?}");
        assert_custom(&serde_json::from_slice::<Settings>(&std::fs::read(tmp.path.join(&names[1])).unwrap()).unwrap());
        assert_eq!(load_from(&file).language, "ja", "the save went through");

        // Kept once: the next save is an ordinary one.
        save_to(&file, &changed).unwrap();
        assert_eq!(file_names(&tmp.path), names);
    }

    #[test]
    fn a_save_is_atomic_leaves_no_temp_file_and_round_trips() {
        let tmp = TempDir::new("settings-save");
        let file = tmp.path.join("settings.json");
        std::fs::write(&file, CUSTOM).unwrap();
        let mut s = load_from(&file);
        s.language = "ja".into();
        s.pomodoro_cycles = 3;

        save_to(&file, &s).unwrap();

        assert_eq!(file_names(&tmp.path), ["settings.json"], "no settings.json.tmp is left");
        let back = load_from(&file);
        assert_eq!(serde_json::to_value(&back).unwrap(), serde_json::to_value(&s).unwrap());
        assert_eq!((back.language.as_str(), back.pomodoro_cycles), ("ja", 3));

        // A save into a folder that is gone fails, and leaves no temp file either.
        let gone = tmp.path.join("missing").join("settings.json");
        assert!(save_to(&gone, &s).is_err());
        assert_eq!(file_names(&tmp.path), ["settings.json"]);
    }

    #[test]
    fn an_older_settings_file_loads_opening_on_the_sessions() {
        // The keys of a settings.json from before the "open on click" setting: it has no `openOnClick`.
        let old = r#"{"soundEnabled":true,"soundVolume":0.12,"autoCloseInterval":15,"absenceInterval":180,"activeIntegrations":[],"screen":"primary","autostart":false,"model":"claude-opus-5","hotkey":"Alt+M","hotkeyEnabled":true,"language":"de"}"#;
        let s = serde_json::from_str::<super::Settings>(old).expect("an older settings.json must still load");
        assert_eq!(s.open_on_click, "sessions");
        assert_eq!(s.language, "de", "the other values are kept");
        assert_eq!(super::Settings::default().open_on_click, "sessions");
    }

    #[test]
    fn open_on_click_round_trips_in_camel_case() {
        for page in super::OPEN_ON_CLICK {
            let mut s = super::Settings::default();
            s.open_on_click = page.into();
            let v = serde_json::to_value(&s).unwrap();
            assert_eq!(v["openOnClick"], page);
            let back: super::Settings = serde_json::from_value(v).unwrap();
            assert_eq!(back.open_on_click, page);
        }
    }

    #[test]
    fn an_unknown_open_on_click_loads_as_sessions_and_the_rest_is_kept() {
        let tmp = TempDir::new("settings-open-on-click");
        let file = tmp.path.join("settings.json");
        std::fs::write(&file, br#"{"openOnClick":"somewhere","language":"fr"}"#).unwrap();
        let s = load_from(&file);
        assert_eq!((s.open_on_click.as_str(), s.language.as_str()), ("sessions", "fr"));

        std::fs::write(&file, br#"{"openOnClick":"last"}"#).unwrap();
        assert_eq!(load_from(&file).open_on_click, "last");
    }

    #[test]
    fn the_language_round_trips_in_camel_case() {
        let mut s = super::Settings::default();
        s.language = "de".into();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["language"], "de");
        let back: super::Settings = serde_json::from_value(v).unwrap();
        assert_eq!(back.language, "de");
    }
}
