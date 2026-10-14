// Everything that differs between operating systems, behind one set of names.
//
// The rest of the app calls `platform::…` and never touches Win32 or a Linux
// API directly. Each OS file exposes the same functions; the compiler picks one.

use std::path::PathBuf;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use self::linux::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

/// Wall-clock time in the user's time zone, for log lines and backup names.
pub struct LocalTime {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// An agent's CLI by name: on PATH, and on a Mac, failing that, the one inside the app that ships
/// it (Claude Desktop, ChatGPT). The chat, Resume and the limits all look it up through here.
pub fn find_cli(stem: &str) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    return find_on_path(stem).or_else(|| app_cli(stem));
    #[cfg(not(target_os = "macos"))]
    find_on_path(stem)
}

/// The user's home directory, where `.claude/settings.json` lives.
pub fn home_dir() -> PathBuf {
    std::env::var_os(HOME_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Whether folding a hotkey-opened island hands the keyboard back to the window it came from
/// (`restore_foreground`): only while the island still holds the focus (the user did not click
/// into another app meanwhile), only to a window that still exists, and never after the island
/// opened a session (that terminal or editor has the focus now).
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
pub fn should_restore_focus(island_has_focus: bool, target_alive: bool, opened_something: bool) -> bool {
    island_has_focus && target_alive && !opened_something
}

#[cfg(test)]
mod focus_tests {
    use super::should_restore_focus;

    #[test]
    fn the_focus_goes_back_only_from_the_island_to_a_live_window_when_nothing_was_opened() {
        assert!(should_restore_focus(true, true, false));
        // The user clicked into another app: theirs to keep.
        assert!(!should_restore_focus(false, true, false));
        // The window the hotkey was pressed in is gone.
        assert!(!should_restore_focus(true, false, false));
        // Enter opened a session: its terminal or editor keeps the focus.
        assert!(!should_restore_focus(true, true, true));
        assert!(!should_restore_focus(false, false, true));
    }
}
