//! Launch at login, kept in step with Windows (Task Manager → Startup, Settings → Apps → Startup).
//! Windows is the source of truth: `settings.autostart` follows what the system reports, read at
//! start-up and every time the Settings window asks. The decisions are pure functions; the
//! registry is only touched through the autostart plugin and, for the stale-path check of release
//! builds, one read of the Run value. Tests never touch the registry.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;

use crate::{log, settings, Shared};

/// What went wrong with the startup entry, as Settings reports it under the toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AutostartError {
    /// The system state could not be read: the stored value is kept.
    Read,
    /// Turning it on or off failed: the toggle shows the state after the attempt.
    Write,
}

/// The Settings toggle: the real state and the error to show under it, if any. Never persisted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutostartStatus {
    pub enabled: bool,
    pub error: Option<AutostartError>,
}

/// The outcome of comparing the stored flag with the system's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reconciled {
    pub autostart: bool,
    pub persist: bool,
    pub error: Option<AutostartError>,
}

/// `stored` against what the system reports: the system wins; a read error keeps `stored`.
pub fn reconcile<E>(stored: bool, system: &Result<bool, E>) -> Reconciled {
    match system {
        Ok(on) => Reconciled { autostart: *on, persist: *on != stored, error: None },
        Err(_) => Reconciled { autostart: stored, persist: false, error: Some(AutostartError::Read) },
    }
}

/// The last change from Settings failed: reported once, by the next `status`. Never persisted.
static WRITE_FAILED: AtomicBool = AtomicBool::new(false);

/// The executable a Run command starts: the quoted path, or, unquoted, the whole command
/// (auto-launch writes `path args` without quotes, so a path may hold spaces).
#[cfg_attr(any(debug_assertions, not(windows)), allow(dead_code))]
fn command_points_to(command: &str, exe: &str) -> bool {
    let command = command.trim();
    let exe = exe.trim().trim_matches('"');
    if exe.is_empty() {
        return false;
    }
    if let Some(rest) = command.strip_prefix('"') {
        return match rest.find('"') {
            Some(end) => rest[..end].eq_ignore_ascii_case(exe),
            None => false,
        };
    }
    if command.len() < exe.len() || !command.is_char_boundary(exe.len()) {
        return false;
    }
    let (head, tail) = command.split_at(exe.len());
    head.eq_ignore_ascii_case(exe) && (tail.is_empty() || tail.starts_with(char::is_whitespace))
}

/// True when the Run command does not start `exe` (paths compared ignoring case and quotes).
#[cfg_attr(any(debug_assertions, not(windows)), allow(dead_code))]
pub fn is_stale(command: &str, exe: &str) -> bool {
    !command_points_to(command, exe)
}

/// The Run value auto-launch writes for this app, `None` when it is absent or unreadable.
#[cfg(all(windows, not(debug_assertions)))]
fn run_command(name: &str) -> Option<String> {
    use ::windows::core::HSTRING;
    use ::windows::Win32::Foundation::ERROR_SUCCESS;
    use ::windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

    let key = HSTRING::from("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run");
    let value = HSTRING::from(name);
    let mut size = 0u32;
    unsafe {
        if RegGetValueW(HKEY_CURRENT_USER, &key, &value, RRF_RT_REG_SZ, None, None, Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        if RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &value,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

/// Release builds only: an enabled entry that starts another exe (Mayak moved or reinstalled
/// elsewhere) is written again for this one. A dev build never rewrites it: it would take the
/// installed app's startup entry.
#[cfg(all(windows, not(debug_assertions)))]
fn repair_stale_entry(app: &AppHandle) {
    let manager = app.autolaunch();
    if !matches!(manager.is_enabled(), Ok(true)) {
        return;
    }
    let Some(command) = run_command(&app.package_info().name) else { return };
    let exe = match std::env::current_exe() {
        Ok(exe) => exe.display().to_string(),
        Err(err) => {
            log::line(format!("autostart: current exe unknown ({:?})", err.kind()));
            return;
        }
    };
    if !is_stale(&command, &exe) {
        return;
    }
    match manager.enable() {
        Ok(()) => log::line("autostart: the startup entry pointed to another exe, rewritten"),
        Err(err) => log::line(format!("autostart: could not rewrite the stale startup entry ({err})")),
    }
}

#[cfg(not(all(windows, not(debug_assertions))))]
fn repair_stale_entry(_app: &AppHandle) {}

/// Reads the system state, makes `settings.autostart` follow it (saved and broadcast when it
/// changed) and returns the state for the Settings toggle.
pub fn sync(app: &AppHandle) -> AutostartStatus {
    let system = app.autolaunch().is_enabled();
    if let Err(err) = &system {
        log::line(format!("autostart: could not read the startup entry ({err})"));
    }
    let shared = app.state::<Shared>();
    let (outcome, changed) = {
        let mut current = shared.settings.lock().unwrap();
        let outcome = reconcile(current.autostart, &system);
        let changed = outcome.persist.then(|| {
            current.autostart = outcome.autostart;
            current.clone()
        });
        (outcome, changed)
    };
    if let Some(updated) = changed {
        log::line(format!("autostart: follows Windows, now {}", if updated.autostart { "on" } else { "off" }));
        if let Err(err) = settings::save(&updated) {
            log::line(format!("settings: could not save settings.json ({:?})", err.kind()));
        }
        let _ = app.emit("settings-changed", updated);
    }
    AutostartStatus { enabled: outcome.autostart, error: outcome.error }
}

/// What Settings asks every time it is shown: the flag follows Windows, then the toggle is drawn.
#[tauri::command]
pub fn autostart_status(app: AppHandle) -> AutostartStatus {
    status(&app)
}

/// Start-up: the stale entry of a release build is repaired, then the flag follows Windows.
pub fn startup(app: &AppHandle) {
    repair_stale_entry(app);
    sync(app);
}

/// The Settings toggle: the state after `sync`, with a failed change reported once.
pub fn status(app: &AppHandle) -> AutostartStatus {
    let mut status = sync(app);
    if WRITE_FAILED.swap(false, Ordering::Relaxed) && status.error.is_none() {
        status.error = Some(AutostartError::Write);
    }
    status
}

/// Turns the entry on or off for a Settings change. On failure the error is logged, kept for the
/// next `status`, and the state after the attempt is returned (`previous` when even that cannot
/// be read).
pub fn apply(app: &AppHandle, on: bool, previous: bool) -> Result<(), bool> {
    let manager = app.autolaunch();
    let result = if on { manager.enable() } else { manager.disable() };
    match result {
        Ok(()) => {
            WRITE_FAILED.store(false, Ordering::Relaxed);
            Ok(())
        }
        Err(err) => {
            log::line(format!("autostart: could not turn {} ({err})", if on { "on" } else { "off" }));
            WRITE_FAILED.store(true, Ordering::Relaxed);
            Err(match manager.is_enabled() {
                Ok(real) => real,
                Err(_) => previous,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_on_stored_off_turns_on_and_saves() {
        let r = reconcile(false, &Ok::<_, ()>(true));
        assert_eq!(r, Reconciled { autostart: true, persist: true, error: None });
    }

    #[test]
    fn system_off_stored_on_turns_off_and_saves() {
        let r = reconcile(true, &Ok::<_, ()>(false));
        assert_eq!(r, Reconciled { autostart: false, persist: true, error: None });
    }

    #[test]
    fn equal_states_change_nothing() {
        for on in [true, false] {
            let r = reconcile(on, &Ok::<_, ()>(on));
            assert_eq!(r, Reconciled { autostart: on, persist: false, error: None });
        }
    }

    #[test]
    fn read_error_keeps_the_stored_value_and_reports_it() {
        for on in [true, false] {
            let r = reconcile(on, &Err::<bool, _>("denied"));
            assert_eq!(r, Reconciled { autostart: on, persist: false, error: Some(AutostartError::Read) });
        }
    }

    const EXE: &str = r"C:\Users\Me\AppData\Local\Mayak\mayak.exe";

    #[test]
    fn the_entry_auto_launch_writes_is_current() {
        // auto-launch writes `path args`: no quotes, a trailing space with no args.
        assert!(!is_stale(&format!("{EXE} "), EXE));
        assert!(!is_stale(EXE, EXE));
        assert!(!is_stale(&format!("{EXE} --minimized"), EXE));
    }

    #[test]
    fn paths_compare_ignoring_case_and_quotes() {
        assert!(!is_stale(r"c:\users\me\appdata\local\MAYAK\Mayak.EXE", EXE));
        assert!(!is_stale(&format!("\"{EXE}\""), EXE));
        assert!(!is_stale(&format!("\"{EXE}\" --flag"), EXE));
        assert!(!is_stale(&format!("  {EXE}  "), &format!("\"{EXE}\"")));
    }

    #[test]
    fn spaces_in_the_path_are_tolerated() {
        let exe = r"C:\Program Files\Mayak\mayak.exe";
        assert!(!is_stale(&format!("{exe} "), exe));
        assert!(!is_stale(&format!("\"{exe}\""), exe));
        assert!(is_stale(r"C:\Program Files\Other\mayak.exe ", exe));
    }

    #[test]
    fn another_exe_is_stale() {
        assert!(is_stale(r"C:\Old\Mayak\mayak.exe ", EXE));
        assert!(is_stale(&format!("{EXE}x"), EXE));
        assert!(is_stale(r"C:\Users\Me\AppData\Local\Mayak", EXE));
        assert!(is_stale(&format!("\"{EXE}"), EXE));
        assert!(is_stale("", EXE));
        assert!(is_stale(EXE, ""));
    }
}
