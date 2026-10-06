// Preferences, stored as plain JSON in settings.json under platform::config_dir().
// No secret ever lands here — API keys live in the OS keychain (see secrets.rs).

use serde::{Deserialize, Serialize};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

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
}

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
    match std::fs::read(settings_path()) {
        Ok(bytes) => serde_json::from_slice::<Settings>(&bytes).unwrap_or_default().migrated(),
        Err(_) => Settings::default(),
    }
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    let dir = config_dir();
    crate::platform::ensure_private_dir(&dir)?;
    let json = serde_json::to_vec_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(settings_path(), json)
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
}
