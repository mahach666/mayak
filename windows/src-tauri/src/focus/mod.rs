//! Switching to the window of a session (sub-project 2, spec 2026-10-06-mayak-window-switching-design.md).
//! The pure part lives here: choosing a window from data, choosing a fallback, naming outcomes.
//! The Win32 part is in `win.rs`, Windows only: elsewhere no window is found and the fallback
//! opens the folder.

use std::process::Command;

use crate::observe::keys::HostKind;
use crate::{log, observe, platform};

#[cfg(windows)]
pub mod win;
pub mod msix;
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

    pub fn host_windows(_agent_pid: u32, _accept: impl Fn(&str) -> bool) -> Vec<WindowInfo> {
        Vec::new()
    }

    pub fn raise(_hwnd: isize) -> Raise {
        Raise::Failed
    }

    pub fn app_windows(_exe_name: &str) -> Vec<WindowInfo> {
        Vec::new()
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
/// launcher, which tells folders apart by their full path. Any other host (a terminal, an IDE
/// whose own terminal runs the agent) is the nearest ancestor of the agent that owns a window.
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
        HostKind::AntigravityIde | HostKind::AntigravityCli | HostKind::Unknown => pick_terminal(root, windows),
    }
}

/// Programs whose window an unknown host may be (any case): terminals, consoles, and editors
/// whose own terminal runs the agent. Anything else is the app that started a console, not it.
const TERMINAL_HOSTS: &[&str] = &[
    "WindowsTerminal.exe", "wezterm-gui.exe", "alacritty.exe", "Hyper.exe", "Tabby.exe", "ConEmu.exe",
    "ConEmu64.exe", "mintty.exe", "warp.exe", "cmd.exe", "powershell.exe", "pwsh.exe", "conhost.exe",
    "Code.exe", "Code - Insiders.exe", "Cursor.exe", "Windsurf.exe", "Zed.exe", "idea64.exe",
    "pycharm64.exe", "webstorm64.exe", "rider64.exe", "clion64.exe", "goland64.exe", "rustrover64.exe",
    "phpstorm64.exe",
];

/// Whether the image `exe` may host the window of an unknown session (see `TERMINAL_HOSTS`).
pub fn is_terminal_host(exe: &str) -> bool {
    TERMINAL_HOSTS.iter().any(|known| known.eq_ignore_ascii_case(exe))
}

/// A terminal or an editor the agent runs under: the topmost of its windows whose title names
/// the project folder, else its topmost window. Unlike an editor, two matches are not `Many`:
/// no launcher tells them apart, and either is closer than the Explorer fallback.
fn pick_terminal(root: &str, windows: &[WindowInfo]) -> Pick {
    let folder = folder_name(root).map(str::to_lowercase);
    let named = folder.and_then(|folder| windows.iter().find(|w| names_folder(&w.title.to_lowercase(), &folder)));
    named.or_else(|| windows.first()).map_or(Pick::None, |w| Pick::One(w.hwnd))
}

/// Whether `title` holds `folder` as a word of its own, both lower case: not inside another
/// name (`webapp`, `mapping`, `app-old` do not name `app`), but at the end of a path or between
/// spaces and dashes (`C:\src\app`, `Fix the bug — app`).
fn names_folder(title: &str, folder: &str) -> bool {
    let joins = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || "-_.".contains(c));
    title.match_indices(folder).any(|(at, _)| {
        !joins(title[..at].chars().next_back()) && !joins(title[at + folder.len()..].chars().next())
    })
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
    /// No window, but the link to the Claude Desktop chat went out.
    OpenedChat,
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Focused => "Focused",
            Outcome::Flashed => "Flashed",
            Outcome::Launched => "Launched",
            Outcome::OpenedExplorer => "OpenedExplorer",
            Outcome::OpenedChat => "OpenedChat",
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
        HostKind::ClaudeDesktop
        | HostKind::CodexDesktop
        | HostKind::AntigravityIde
        | HostKind::AntigravityCli
        | HostKind::Unknown => Fallback::Explorer,
    }
}

/// The image of Antigravity's window (apps.rs "antigravity").
pub const ANTIGRAVITY_EXE: &str = "Antigravity.exe";

/// What «Open» does for a session of Antigravity's own agent: the window of `Antigravity.exe`
/// whose title names the project folder, else its topmost window (`pick_terminal`); else
/// `launch` (the Apps tab's start of the built-in app); else Explorer on `root`. `raise` is
/// `win::raise` for one window.
pub fn open_antigravity(
    root: Option<&str>,
    windows: &[WindowInfo],
    raise: impl Fn(isize) -> Option<Outcome>,
    launch: impl FnOnce() -> bool,
    fallback: impl FnOnce() -> Outcome,
) -> Outcome {
    if let Pick::One(hwnd) = pick_terminal(root.unwrap_or(""), windows) {
        if let Some(outcome) = raise(hwnd) {
            return outcome;
        }
    }
    if launch() {
        return Outcome::Launched;
    }
    fallback()
}

/// Bring the window of the session forward; otherwise start the fallback. Blocking (a Win32 walk
/// and ~150 ms of settling): call it off the UI thread. `root` is an absolute folder that exists;
/// it is `None` only for a Claude Desktop session whose folder is not on this disk (a scratch
/// chat, quick wins §3.2), which has its chat link but nothing for the Explorer fallback.
///
/// `desktop_chat` is the validated chat id of a Claude Desktop session (quick wins §3.1): after
/// the raise its chat link is sent, and when no window was raised the link replaces the Explorer
/// fallback, which then runs only if the link did not go out. Without an id nothing changes.
pub fn switch_to(host: HostKind, root: Option<&str>, agent_pid: Option<u32>, desktop_chat: Option<&str>) -> Outcome {
    if host == HostKind::AntigravityIde {
        let outcome = open_antigravity(
            root,
            &win::app_windows(ANTIGRAVITY_EXE),
            |hwnd| match win::raise(hwnd) {
                win::Raise::Focused => Some(Outcome::Focused),
                win::Raise::Flashed => Some(Outcome::Flashed),
                win::Raise::Failed => None,
            },
            || crate::apps::open_builtin("antigravity").is_ok(),
            || fallback_outcome(root, |folder| run_fallback(host, folder)),
        );
        log::line(log_line(host, outcome));
        return outcome;
    }
    // An unknown host is raised only when it is a terminal or an editor: an agent whose console
    // Windows Terminal took over has the app that started it (a file manager…) as its nearest
    // window owner, and raising that would hide the Explorer fallback behind a wrong window. The
    // `agy` CLI runs in a terminal too.
    let accept = |exe: &str| !matches!(host, HostKind::Unknown | HostKind::AntigravityCli) || is_terminal_host(exe);
    let pick = agent_pid.map_or(Pick::None, |pid| {
        pick_window(host, root.unwrap_or(""), &win::host_windows(pid, accept), |names| workspace::load(host, names))
    });
    let raised = match pick {
        Pick::One(hwnd) => match win::raise(hwnd) {
            win::Raise::Focused => Some(Outcome::Focused),
            win::Raise::Flashed => Some(Outcome::Flashed),
            win::Raise::Failed => None,
        },
        Pick::Many | Pick::None => None,
    };
    let outcome = finish_switch(
        raised,
        desktop_chat.and_then(desktop_continue_uri),
        |uri| {
            let sent = send_link(uri);
            log::line(match sent {
                Ok(()) => "session_open: desktop chat link sent",
                Err(_) => "session_open: desktop chat link failed",
            });
            sent
        },
        || fallback_outcome(root, |folder| run_fallback(host, folder)),
    );
    log::line(log_line(host, outcome));
    outcome
}

/// What happens after the window step, in one place. `raised` is the outcome of a raised window,
/// `None` when no window was raised (none found, several, or the raise failed). `link` is the
/// Desktop chat link, present only for a session with a validated id.
/// - a raised window stays the outcome; the link, if any, is sent after it;
/// - no window and a link: the link is sent instead of the fallback, and only its failure
///   starts the fallback;
/// - no window and no link: the fallback, as before.
fn finish_switch(
    raised: Option<Outcome>,
    link: Option<String>,
    send: impl FnOnce(&str) -> Result<(), String>,
    fallback: impl FnOnce() -> Outcome,
) -> Outcome {
    match (raised, link) {
        (Some(outcome), Some(uri)) => {
            let _ = send(&uri); // already logged by the caller; the window is raised either way
            outcome
        }
        (Some(outcome), None) => outcome,
        (None, Some(uri)) => match send(&uri) {
            Ok(()) => Outcome::OpenedChat,
            Err(_) => fallback(),
        },
        (None, None) => fallback(),
    }
}

/// The link that opens the Claude Desktop chat `host_session_id` (quick wins §3.1). `None` unless
/// the id has the shape Desktop accepts, so nothing else ever reaches the address.
pub fn desktop_continue_uri(host_session_id: &str) -> Option<String> {
    observe::claude::is_host_session_id(host_session_id)
        .then(|| format!("claude://code/continue?session={host_session_id}"))
}

/// Hands the link to the `claude:` protocol handler. The error carries no id.
#[cfg(windows)]
fn send_link(uri: &str) -> Result<(), String> {
    // COM for the call: `ShellExecuteW` may hand the link to a shell extension.
    let _com = crate::launcher::win::Com::init();
    crate::launcher::win::open_uri(uri)
}

/// Desktop chat ids come only from Windows sessions; elsewhere there is no handler to call.
#[cfg(not(windows))]
fn send_link(_uri: &str) -> Result<(), String> {
    Err("no protocol handler on this platform".to_string())
}

/// The fallback needs a folder: without one nothing is started and the switch has failed.
/// The line carries no path.
fn fallback_outcome(root: Option<&str>, run: impl FnOnce(&str) -> Outcome) -> Outcome {
    match root {
        Some(folder) => run(folder),
        None => {
            log::line("session_open: no folder for the fallback");
            Outcome::Failed
        }
    }
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

    #[test]
    fn a_valid_desktop_chat_id_gives_the_continue_link() {
        assert_eq!(
            desktop_continue_uri("local_da7117cd-3665-4872-af75-dd782d0be925").as_deref(),
            Some("claude://code/continue?session=local_da7117cd-3665-4872-af75-dd782d0be925")
        );
    }

    #[test]
    fn an_invalid_desktop_chat_id_gives_no_link() {
        let too_long = format!("local_{}", "a".repeat(65));
        for id in ["", "local_", "remote_abc", "local_a&b=1", "local_a b", "local_a#x", "local_a/../b", too_long.as_str()] {
            assert_eq!(desktop_continue_uri(id), None, "{id:?}");
        }
    }

    fn w(hwnd: isize, title: &str) -> WindowInfo {
        WindowInfo { hwnd, title: title.into(), minimized: false }
    }

    #[test]
    fn antigravity_raises_its_project_window_else_starts_else_opens_the_folder() {
        let wins = [w(1, "other - Antigravity"), w(2, "demo - Antigravity")];
        let raised = |hwnd: isize| (hwnd == 2).then_some(Outcome::Focused);
        let never = || -> bool { panic!("a window was raised") };
        let no_fallback = || -> Outcome { panic!("no fallback") };
        assert_eq!(open_antigravity(Some(r"C:\src\demo"), &wins, raised, never, no_fallback), Outcome::Focused);
        // No folder: the topmost window.
        assert_eq!(open_antigravity(None, &wins, |h| (h == 1).then_some(Outcome::Flashed), never, no_fallback), Outcome::Flashed);
        // No window (or none raised): Antigravity starts.
        assert_eq!(open_antigravity(Some(r"C:\src\demo"), &[], |_| None, || true, no_fallback), Outcome::Launched);
        assert_eq!(open_antigravity(None, &wins, |_| None, || true, no_fallback), Outcome::Launched);
        // Not installed: the fallback.
        assert_eq!(open_antigravity(None, &[], |_| None, || false, || Outcome::OpenedExplorer), Outcome::OpenedExplorer);
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
        assert_eq!(pick_window(HostKind::Unknown, ROOT, &wins, |_| panic!("not an editor")), Pick::One(5));
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
    fn only_terminals_consoles_and_editors_host_an_unknown_session() {
        for exe in ["WindowsTerminal.exe", "windowsterminal.EXE", "pwsh.exe", "Code.exe", "idea64.exe"] {
            assert!(is_terminal_host(exe), "{exe}");
        }
        for exe in ["totalcmd64.exe", "Taskmgr.exe", "explorer.exe", "mayak.exe", ""] {
            assert!(!is_terminal_host(exe), "{exe}");
        }
    }

    #[test]
    fn a_terminal_takes_its_window_that_names_the_folder_else_its_topmost() {
        let wins = [w(1, "PowerShell"), w(2, "✳ Fix the bug — APP"), w(3, r"C:\src\app")];
        assert_eq!(pick_window(HostKind::Unknown, r"C:\src\app", &wins, none), Pick::One(2));
        let wins = [w(1, "✳ Fix the mapping bug"), w(2, r"C:\src\webapp"), w(3, "app-old"), w(4, r"C:\src\app")];
        assert_eq!(pick_window(HostKind::Unknown, r"C:\src\app", &wins, none), Pick::One(4), "a word of its own");
        let wins = [w(1, "x"), w(2, "my-app: claude")];
        assert_eq!(pick_window(HostKind::Unknown, r"C:\src\my-app", &wins, none), Pick::One(2));
        let wins = [w(1, "PowerShell"), w(2, "cmd")];
        assert_eq!(pick_window(HostKind::Unknown, r"C:\src\app", &wins, none), Pick::One(1));
        assert_eq!(pick_window(HostKind::Unknown, r"C:\", &wins, none), Pick::One(1), "no folder name: the topmost");
        assert_eq!(pick_window(HostKind::Unknown, "", &wins, none), Pick::One(1));
        assert_eq!(pick_window(HostKind::Unknown, r"C:\src\app", &[], none), Pick::None, "no window: the fallback");
    }

    #[test]
    fn outcomes_have_stable_names() {
        let all = [Outcome::Focused, Outcome::Flashed, Outcome::Launched, Outcome::OpenedExplorer, Outcome::OpenedChat, Outcome::Failed];
        let names: Vec<_> = all.iter().map(|o| o.as_str()).collect();
        assert_eq!(names, ["Focused", "Flashed", "Launched", "OpenedExplorer", "OpenedChat", "Failed"]);
    }

    const LINK: &str = "claude://code/continue?session=local_abc";

    /// Runs `finish_switch` and reports the outcome with the calls made: the links sent and
    /// whether the fallback ran. `sends` is the result of the link.
    fn finish(raised: Option<Outcome>, link: Option<&str>, sends: Result<(), String>) -> (Outcome, Vec<String>, bool) {
        let sent = std::cell::RefCell::new(Vec::new());
        let fell_back = std::cell::Cell::new(false);
        let outcome = finish_switch(
            raised,
            link.map(str::to_string),
            |uri| {
                sent.borrow_mut().push(uri.to_string());
                sends
            },
            || {
                fell_back.set(true);
                Outcome::OpenedExplorer
            },
        );
        (outcome, sent.into_inner(), fell_back.get())
    }

    #[test]
    fn an_id_and_a_raised_window_raise_and_send_the_link() {
        for raised in [Outcome::Focused, Outcome::Flashed] {
            assert_eq!(finish(Some(raised), Some(LINK), Ok(())), (raised, vec![LINK.to_string()], false));
        }
        // a failed link leaves the raised window as it is, and never starts Explorer
        assert_eq!(finish(Some(Outcome::Focused), Some(LINK), Err("x".into())), (Outcome::Focused, vec![LINK.to_string()], false));
    }

    #[test]
    fn an_id_without_a_window_sends_only_the_link() {
        assert_eq!(finish(None, Some(LINK), Ok(())), (Outcome::OpenedChat, vec![LINK.to_string()], false));
    }

    #[test]
    fn an_id_without_a_window_opens_explorer_only_when_the_link_fails() {
        assert_eq!(finish(None, Some(LINK), Err("x".into())), (Outcome::OpenedExplorer, vec![LINK.to_string()], true));
    }

    #[test]
    fn without_an_id_the_path_is_the_one_it_always_was() {
        assert_eq!(finish(Some(Outcome::Focused), None, Ok(())), (Outcome::Focused, vec![], false));
        assert_eq!(finish(None, None, Ok(())), (Outcome::OpenedExplorer, vec![], true));
    }

    #[test]
    fn without_a_folder_the_fallback_fails_and_never_runs() {
        assert_eq!(fallback_outcome(None, |_| panic!("no folder to open")), Outcome::Failed);
        assert_eq!(fallback_outcome(Some(r"C:\src\app"), |folder| {
            assert_eq!(folder, r"C:\src\app");
            Outcome::OpenedExplorer
        }), Outcome::OpenedExplorer);
    }

    #[test]
    fn a_link_that_fails_without_a_folder_is_failed() {
        let outcome = finish_switch(None, Some(LINK.to_string()), |_| Err("x".into()), || fallback_outcome(None, |_| panic!("no folder")));
        assert_eq!(outcome, Outcome::Failed);
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
