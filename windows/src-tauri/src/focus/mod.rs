//! Switching to the window of a session (sub-project 2, spec 2026-10-06-mayak-window-switching-design.md).
//! The pure part lives here: choosing a window from data, choosing a fallback, naming outcomes.
//! The Win32 part is in `win.rs`, Windows only: elsewhere no window is found and the fallback
//! opens the folder.

use std::process::Command;

use crate::observe::keys::HostKind;
use crate::{log, observe, platform};

#[cfg(windows)]
pub mod win;
pub mod workspace;

/// Elsewhere there is no window to raise: `switch_to` always takes the fallback.
#[cfg(not(windows))]
mod win {
    use super::WindowInfo;

    #[allow(dead_code)] // the same variants as on Windows, so `switch_to` matches them alike
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Raise {
        Focused,
        Flashed,
        Failed,
    }

    pub fn host_windows(_agent_pid: u32) -> Vec<WindowInfo> {
        Vec::new()
    }

    pub fn raise(_hwnd: isize) -> Raise {
        Raise::Failed
    }
}

/// What opens a folder in the file manager when no launcher is available.
const FOLDER_OPENER: &str = if cfg!(windows) {
    "explorer"
} else if cfg!(target_os = "macos") {
    "open"
} else {
    "xdg-open"
};

use workspace::Workspaces;

/// A top-level window of the host process, as `focus::win` reports it (Z-order, topmost first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub title: String,
    pub minimized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
///
/// Only when no folder window matches, the multi-root workspace windows (`<name> (Workspace)`)
/// are tried: `workspaces` gets their lowercase names and gives the folders of each, and a window
/// matches when the root is one of them or lies inside one. It is called at most once, and never
/// when a folder window decides or no title names a workspace.
pub fn pick_window(
    host: HostKind,
    root: &str,
    windows: &[WindowInfo],
    workspaces: impl FnOnce(&[String]) -> Workspaces,
) -> Pick {
    match host {
        HostKind::Cursor => pick_editor(CURSOR_SUFFIX, root, windows, workspaces),
        HostKind::VsCode => pick_editor(VS_CODE_SUFFIX, root, windows, workspaces),
        HostKind::ClaudeDesktop | HostKind::CodexDesktop => {
            windows.first().map_or(Pick::None, |w| Pick::One(w.hwnd))
        }
        HostKind::Unknown => Pick::None,
    }
}

fn pick_editor(
    suffix: &str,
    root: &str,
    windows: &[WindowInfo],
    workspaces: impl FnOnce(&[String]) -> Workspaces,
) -> Pick {
    match pick_folder_window(suffix, root, windows) {
        Pick::None => pick_workspace_window(suffix, root, windows, workspaces),
        decided => decided,
    }
}

fn pick_folder_window(suffix: &str, root: &str, windows: &[WindowInfo]) -> Pick {
    let Some(folder) = folder_name(root) else { return Pick::None };
    let folder = folder.to_lowercase();
    one_of(
        windows
            .iter()
            .filter(|w| title_folder(&w.title, suffix).is_some_and(|f| f.to_lowercase() == folder))
            .map(|w| w.hwnd),
    )
}

fn pick_workspace_window(
    suffix: &str,
    root: &str,
    windows: &[WindowInfo],
    workspaces: impl FnOnce(&[String]) -> Workspaces,
) -> Pick {
    let named: Vec<(isize, String)> = windows
        .iter()
        .filter_map(|w| {
            let name = title_folder(&w.title, suffix).and_then(workspace::workspace_name)?;
            Some((w.hwnd, name.to_lowercase()))
        })
        .collect();
    let root = observe::paths::normalize(root).key;
    if named.is_empty() || root.is_empty() {
        return Pick::None;
    }
    let mut names: Vec<String> = named.iter().map(|(_, name)| name.clone()).collect();
    names.sort();
    names.dedup();
    let known = workspaces(&names);
    one_of(
        named
            .iter()
            .filter(|(_, name)| known.get(name).is_some_and(|folders| workspace::root_in_folders(&root, folders)))
            .map(|(hwnd, _)| *hwnd),
    )
}

fn one_of(mut hits: impl Iterator<Item = isize>) -> Pick {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The window is in the foreground.
    Focused,
    /// Windows refused the foreground; the taskbar button flashes.
    Flashed,
    /// No usable window; the launcher of the host started.
    Launched,
    /// No window and no launcher: the project folder opened in Explorer.
    OpenedExplorer,
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Focused => "Focused",
            Outcome::Flashed => "Flashed",
            Outcome::Launched => "Launched",
            Outcome::OpenedExplorer => "OpenedExplorer",
            Outcome::Failed => "Failed",
        }
    }
}

/// The only thing the switch writes to `mayak.log`: no title, no folder, no path (spec §6).
pub fn log_line(host: HostKind, outcome: Outcome) -> String {
    format!("session_open: host={host:?} outcome={}", outcome.as_str())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    Launcher(&'static str),
    Explorer,
}

/// What starts when no window of the host can be raised (D32; protocols may join after spike S3).
pub fn fallback_for(host: HostKind) -> Fallback {
    match host {
        HostKind::Cursor => Fallback::Launcher("cursor"),
        HostKind::VsCode => Fallback::Launcher("code"),
        HostKind::ClaudeDesktop | HostKind::CodexDesktop | HostKind::Unknown => Fallback::Explorer,
    }
}

/// Bring the window of the session forward; otherwise start the fallback. Blocking (a Win32 walk
/// and ~150 ms of settling): call it off the UI thread. `root` must be an absolute folder.
pub fn switch_to(host: HostKind, root: &str, agent_pid: Option<u32>) -> Outcome {
    // an unknown host has no window to pick: skip the process walk and the enumeration
    let pick = if host == HostKind::Unknown {
        Pick::None
    } else {
        agent_pid.map_or(Pick::None, |pid| {
            pick_window(host, root, &win::host_windows(pid), |names| workspace::load(host, names))
        })
    };
    let outcome = match pick {
        Pick::One(hwnd) => match win::raise(hwnd) {
            win::Raise::Focused => Outcome::Focused,
            win::Raise::Flashed => Outcome::Flashed,
            win::Raise::Failed => run_fallback(host, root),
        },
        Pick::Many | Pick::None => run_fallback(host, root),
    };
    log::line(log_line(host, outcome));
    outcome
}

fn run_fallback(host: HostKind, root: &str) -> Outcome {
    if let Fallback::Launcher(stem) = fallback_for(host) {
        if let Some(launcher) = platform::find_on_path(stem) {
            let mut cmd = Command::new(launcher);
            cmd.arg(root);
            match platform::no_console(&mut cmd).spawn() {
                Ok(_) => return Outcome::Launched,
                Err(err) => log::line(format!("session_open: launcher did not start: {err}")),
            }
        }
    }
    match Command::new(FOLDER_OPENER).arg(root).spawn() {
        Ok(_) => Outcome::OpenedExplorer,
        Err(err) => {
            log::line(format!("session_open: explorer did not start: {err}"));
            Outcome::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(hwnd: isize, title: &str) -> WindowInfo {
        WindowInfo { hwnd, title: title.into(), minimized: false }
    }

    /// No workspace is known; a test that reaches the workspace step gets nothing.
    fn none(_: &[String]) -> Workspaces {
        Workspaces::new()
    }

    /// Workspaces from `(lowercase name, folder keys)`.
    fn known(list: &[(&str, &[&str])]) -> Workspaces {
        list.iter().map(|(n, f)| (n.to_string(), f.iter().map(|s| s.to_string()).collect())).collect()
    }

    const ROOT: &str = r"C:\Users\HP\source\repos\anlaxtech\AnlaxCollisionService";
    const ANLAX: &[&str] = &[r"c:\users\hp\source\repos\anlaxtech\anlaxcollisionservice", r"c:\other"];

    #[test]
    fn a_workspace_window_holding_the_root_is_picked() {
        let wins = [w(1, "notes.md - tasq - Cursor"), w(2, "Fix the bug - anlax-web-ws (Workspace) - Cursor")];
        let asked = std::cell::RefCell::new(Vec::new());
        let pick = pick_window(HostKind::Cursor, ROOT, &wins, |names| {
            asked.borrow_mut().extend_from_slice(names);
            known(&[("anlax-web-ws", ANLAX)])
        });
        assert_eq!(pick, Pick::One(2));
        assert_eq!(*asked.borrow(), vec!["anlax-web-ws".to_string()]);
    }

    #[test]
    fn a_root_inside_a_workspace_folder_matches_and_a_sibling_prefix_does_not() {
        let wins = [w(3, "x - Anlax-Web-WS (workspace) - Cursor")];
        let ws = || known(&[("anlax-web-ws", &[r"c:\repos\anlaxtech"])]);
        assert_eq!(pick_window(HostKind::Cursor, r"C:\repos\anlaxtech\svc\api", &wins, |_| ws()), Pick::One(3));
        assert_eq!(pick_window(HostKind::Cursor, r"c:/REPOS/AnlaxTech/", &wins, |_| ws()), Pick::One(3));
        assert_eq!(pick_window(HostKind::Cursor, r"C:\repos\anlax", &wins, |_| ws()), Pick::None);
        assert_eq!(pick_window(HostKind::Cursor, r"C:\repos\anlaxtech2", &wins, |_| ws()), Pick::None);
    }

    #[test]
    fn the_folder_window_wins_over_the_workspace_window() {
        let wins = [
            w(1, "a - anlax-web-ws (Workspace) - Cursor"),
            w(2, "b - AnlaxCollisionService - Cursor"),
        ];
        let pick = pick_window(HostKind::Cursor, ROOT, &wins, |_| panic!("a folder window decides alone"));
        assert_eq!(pick, Pick::One(2));
        let many = [w(1, "a - AnlaxCollisionService - Cursor"), w(2, "b - AnlaxCollisionService - Cursor")];
        let pick = pick_window(HostKind::Cursor, ROOT, &many, |_| panic!("Many is decided by the folders"));
        assert_eq!(pick, Pick::Many);
    }

    #[test]
    fn two_workspace_windows_holding_the_root_are_many() {
        let wins = [w(1, "a - anlax-web-ws (Workspace) - Cursor"), w(2, "b - all (Workspace) - Cursor")];
        let ws = known(&[("anlax-web-ws", ANLAX), ("all", &[r"c:\users\hp\source\repos"])]);
        assert_eq!(pick_window(HostKind::Cursor, ROOT, &wins, |_| ws), Pick::Many);
    }

    #[test]
    fn a_workspace_without_the_root_or_unknown_gives_none() {
        let wins = [w(1, "a - anlax-web-ws (Workspace) - Cursor"), w(2, "b - unknown (Workspace) - Cursor")];
        let ws = known(&[("anlax-web-ws", &[r"c:\elsewhere"])]);
        assert_eq!(pick_window(HostKind::Cursor, ROOT, &wins, |_| ws), Pick::None);
        assert_eq!(pick_window(HostKind::Cursor, ROOT, &wins, none), Pick::None);
    }

    #[test]
    fn without_a_workspace_title_nothing_is_read() {
        let wins = [w(1, "a - tasq - Cursor"), w(2, "Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, ROOT, &wins, |_| panic!("no workspace title")), Pick::None);
        let ws = [w(1, "a - x (Workspace) - Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, "", &ws, |_| known(&[("x", &[""])])), Pick::None);
    }

    #[test]
    fn the_vs_code_workspace_title_works_the_same() {
        let wins = [
            w(1, "main.rs - anlax-web-ws (Workspace) - Cursor"),
            w(2, "main.rs - anlax-web-ws (Workspace) - Visual Studio Code"),
        ];
        let ws = || known(&[("anlax-web-ws", ANLAX)]);
        assert_eq!(pick_window(HostKind::VsCode, ROOT, &wins, |_| ws()), Pick::One(2));
        assert_eq!(pick_window(HostKind::Cursor, ROOT, &wins, |_| ws()), Pick::One(1));
    }

    #[test]
    fn desktop_apps_never_read_workspaces() {
        let wins = [w(5, "x - a (Workspace) - Cursor")];
        assert_eq!(pick_window(HostKind::ClaudeDesktop, ROOT, &wins, |_| panic!("not an editor")), Pick::One(5));
        assert_eq!(pick_window(HostKind::Unknown, ROOT, &wins, |_| panic!("not an editor")), Pick::None);
    }

    #[test]
    fn an_editor_window_is_chosen_by_the_folder_before_the_app_suffix() {
        let wins = [w(1, "Refactor filters - other - Cursor"), w(2, "Fix the bug - app - Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins, none), Pick::One(2));
        let vs = [w(7, "main.rs - app - Visual Studio Code"), w(8, "lib.rs - other - Visual Studio Code")];
        assert_eq!(pick_window(HostKind::VsCode, r"C:\src\app", &vs, none), Pick::One(7));
    }

    #[test]
    fn the_folder_name_is_matched_whole_and_without_case() {
        let wins = [w(1, "app2 - Cursor"), w(2, "myapp - Cursor"), w(3, "x - app (Workspace) - Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins, none), Pick::None);
        let wins = [w(1, "Задача - ПРОЕКТ - Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\проект", &wins, none), Pick::One(1));
        let wins = [w(1, "a - APP - cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins, none), Pick::One(1));
    }

    #[test]
    fn a_name_elsewhere_in_the_title_does_not_match() {
        let wins = [w(1, "app - notes.md - other - Cursor"), w(2, "Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins, none), Pick::None);
    }

    #[test]
    fn two_windows_of_one_folder_are_never_guessed() {
        let wins = [w(1, "a.rs - app - Cursor"), w(2, "b.rs - app - Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins, none), Pick::Many);
    }

    #[test]
    fn a_root_without_a_folder_name_matches_nothing() {
        let wins = [w(1, "- Cursor"), w(2, "Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\", &wins, none), Pick::None);
        assert_eq!(pick_window(HostKind::Cursor, "", &wins, none), Pick::None);
    }

    #[test]
    fn a_suffix_of_another_app_is_not_this_editor() {
        let wins = [w(1, "x - app - Visual Studio Code")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins, none), Pick::None);
    }

    #[test]
    fn desktop_apps_take_their_topmost_window() {
        let wins = [w(5, "Claude"), w(6, "Claude")];
        assert_eq!(pick_window(HostKind::ClaudeDesktop, r"C:\src\app", &wins, none), Pick::One(5));
        assert_eq!(pick_window(HostKind::CodexDesktop, r"C:\src\app", &wins, none), Pick::One(5));
        assert_eq!(pick_window(HostKind::CodexDesktop, r"C:\src\app", &[], none), Pick::None);
    }

    #[test]
    fn an_unknown_host_picks_nothing() {
        assert_eq!(pick_window(HostKind::Unknown, r"C:\src\app", &[w(1, "app - Cursor")], none), Pick::None);
    }

    #[test]
    fn outcomes_have_stable_names() {
        let all = [Outcome::Focused, Outcome::Flashed, Outcome::Launched, Outcome::OpenedExplorer, Outcome::Failed];
        let names: Vec<_> = all.iter().map(|o| o.as_str()).collect();
        assert_eq!(names, ["Focused", "Flashed", "Launched", "OpenedExplorer", "Failed"]);
    }

    #[test]
    fn the_log_line_has_only_the_host_and_the_outcome() {
        let line = log_line(HostKind::Cursor, Outcome::Flashed);
        assert_eq!(line, "session_open: host=Cursor outcome=Flashed");
    }

    #[test]
    fn the_fallback_is_the_launcher_of_the_editors_and_explorer_otherwise() {
        assert_eq!(fallback_for(HostKind::Cursor), Fallback::Launcher("cursor"));
        assert_eq!(fallback_for(HostKind::VsCode), Fallback::Launcher("code"));
        for host in [HostKind::ClaudeDesktop, HostKind::CodexDesktop, HostKind::Unknown] {
            assert_eq!(fallback_for(host), Fallback::Explorer);
        }
    }
}
