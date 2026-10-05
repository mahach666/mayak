//! Switching to the window of a session (sub-project 2, spec 2026-10-06-mayak-window-switching-design.md).
//! The pure part lives here: choosing a window from data, choosing a fallback, naming outcomes.
//! The Win32 part is in `win.rs`.

use crate::observe::keys::HostKind;

/// A top-level window of the host process, as `focus::win` reports it (Z-order, topmost first).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // used from Task 4
pub struct WindowInfo {
    pub hwnd: isize,
    pub title: String,
    pub minimized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // used from Task 4
pub enum Pick {
    One(isize),
    Many,
    None,
}

const CURSOR_SUFFIX: &str = " - Cursor";
const VS_CODE_SUFFIX: &str = " - Visual Studio Code";
const SEPARATOR: &str = " - ";

/// The window of a session among the windows of its host (spec §4). Editors are told apart by the
/// folder name just before the app suffix of the title; Desktop apps have one window, take the
/// topmost. Two editor windows of one folder name are `Many`: the caller falls back to the
/// launcher, which tells folders apart by their full path.
#[allow(dead_code)] // used from Task 4
pub fn pick_window(host: HostKind, root: &str, windows: &[WindowInfo]) -> Pick {
    match host {
        HostKind::Cursor => pick_editor(CURSOR_SUFFIX, root, windows),
        HostKind::VsCode => pick_editor(VS_CODE_SUFFIX, root, windows),
        HostKind::ClaudeDesktop | HostKind::CodexDesktop => {
            windows.first().map_or(Pick::None, |w| Pick::One(w.hwnd))
        }
        HostKind::Unknown => Pick::None,
    }
}

fn pick_editor(suffix: &str, root: &str, windows: &[WindowInfo]) -> Pick {
    let Some(folder) = folder_name(root) else { return Pick::None };
    let folder = folder.to_lowercase();
    let mut hits = windows
        .iter()
        .filter(|w| title_folder(&w.title, suffix).is_some_and(|f| f.to_lowercase() == folder))
        .map(|w| w.hwnd);
    match (hits.next(), hits.next()) {
        (None, _) => Pick::None,
        (Some(hwnd), None) => Pick::One(hwnd),
        (Some(_), Some(_)) => Pick::Many,
    }
}

/// Last path component of the project root; `None` for `C:\` and for an empty root.
fn folder_name(root: &str) -> Option<&str> {
    std::path::Path::new(root).file_name()?.to_str().filter(|n| !n.is_empty())
}

/// The segment of `title` right before the editor suffix (`<chat or file> - <folder> - Cursor`).
/// `None` when the title does not end with the suffix. The suffix is ASCII, so its comparison
/// ignores case on bytes and never splits a character.
fn title_folder<'a>(title: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = title.len().checked_sub(suffix.len())?;
    if !title.is_char_boundary(cut) || !title[cut..].eq_ignore_ascii_case(suffix) {
        return None;
    }
    title[..cut].rsplit(SEPARATOR).next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(hwnd: isize, title: &str) -> WindowInfo {
        WindowInfo { hwnd, title: title.into(), minimized: false }
    }

    #[test]
    fn an_editor_window_is_chosen_by_the_folder_before_the_app_suffix() {
        let wins = [w(1, "Refactor filters - other - Cursor"), w(2, "Fix the bug - app - Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins), Pick::One(2));
        let vs = [w(7, "main.rs - app - Visual Studio Code"), w(8, "lib.rs - other - Visual Studio Code")];
        assert_eq!(pick_window(HostKind::VsCode, r"C:\src\app", &vs), Pick::One(7));
    }

    #[test]
    fn the_folder_name_is_matched_whole_and_without_case() {
        let wins = [w(1, "app2 - Cursor"), w(2, "myapp - Cursor"), w(3, "x - app (Workspace) - Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins), Pick::None);
        let wins = [w(1, "Задача - ПРОЕКТ - Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\проект", &wins), Pick::One(1));
        let wins = [w(1, "a - APP - cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins), Pick::One(1));
    }

    #[test]
    fn a_name_elsewhere_in_the_title_does_not_match() {
        let wins = [w(1, "app - notes.md - other - Cursor"), w(2, "Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins), Pick::None);
    }

    #[test]
    fn two_windows_of_one_folder_are_never_guessed() {
        let wins = [w(1, "a.rs - app - Cursor"), w(2, "b.rs - app - Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins), Pick::Many);
    }

    #[test]
    fn a_root_without_a_folder_name_matches_nothing() {
        let wins = [w(1, "- Cursor"), w(2, "Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\", &wins), Pick::None);
        assert_eq!(pick_window(HostKind::Cursor, "", &wins), Pick::None);
    }

    #[test]
    fn a_suffix_of_another_app_is_not_this_editor() {
        let wins = [w(1, "x - app - Visual Studio Code")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins), Pick::None);
    }

    #[test]
    fn desktop_apps_take_their_topmost_window() {
        let wins = [w(5, "Claude"), w(6, "Claude")];
        assert_eq!(pick_window(HostKind::ClaudeDesktop, r"C:\src\app", &wins), Pick::One(5));
        assert_eq!(pick_window(HostKind::CodexDesktop, r"C:\src\app", &wins), Pick::One(5));
        assert_eq!(pick_window(HostKind::CodexDesktop, r"C:\src\app", &[]), Pick::None);
    }

    #[test]
    fn an_unknown_host_picks_nothing() {
        assert_eq!(pick_window(HostKind::Unknown, r"C:\src\app", &[w(1, "app - Cursor")]), Pick::None);
    }
}
