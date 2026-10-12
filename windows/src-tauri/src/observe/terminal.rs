// The terminal a session runs in, for the badge of its row: told from the image names of the
// agent's ancestors (`focus::win::ancestors`, nearest first). Only a session whose own image says
// nothing (`HostKind::Unknown`) is asked, once per process (`TerminalCache`): never per frame or
// per event. The rules are pure; the process walk is the Windows part at the bottom.

use std::collections::HashMap;

use super::keys::{HostKind, ProcId};

/// How a recognised ancestor counts: a program with a window of its own (a terminal, or an editor
/// whose integrated terminal runs the agent), or a shell, which has one only when nothing hosts it.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Window,
    Shell,
}

/// File names (lower case) and what they are. `bash.exe` is handled apart (`is_git_bash`).
#[cfg_attr(not(windows), allow(dead_code))]
const KNOWN: &[(&str, HostKind, Tier)] = &[
    ("windowsterminal.exe", HostKind::WindowsTerminal, Tier::Window),
    // Windows Terminal's console host; it is a parent only under Windows Terminal itself.
    ("openconsole.exe", HostKind::WindowsTerminal, Tier::Window),
    ("wezterm-gui.exe", HostKind::WezTerm, Tier::Window),
    ("wezterm.exe", HostKind::WezTerm, Tier::Window),
    ("alacritty.exe", HostKind::Alacritty, Tier::Window),
    ("tabby.exe", HostKind::Tabby, Tier::Window),
    ("hyper.exe", HostKind::Hyper, Tier::Window),
    // Cmder is ConEmu with a launcher.
    ("conemu.exe", HostKind::ConEmu, Tier::Window),
    ("conemu64.exe", HostKind::ConEmu, Tier::Window),
    ("conemuc.exe", HostKind::ConEmu, Tier::Window),
    ("conemuc64.exe", HostKind::ConEmu, Tier::Window),
    ("cmder.exe", HostKind::ConEmu, Tier::Window),
    ("warp.exe", HostKind::Warp, Tier::Window),
    // Git Bash's window.
    ("mintty.exe", HostKind::GitBash, Tier::Window),
    ("git-bash.exe", HostKind::GitBash, Tier::Window),
    // An integrated terminal: the session belongs to the editor, as its extension's would.
    ("code.exe", HostKind::VsCode, Tier::Window),
    ("code - insiders.exe", HostKind::VsCode, Tier::Window),
    ("cursor.exe", HostKind::Cursor, Tier::Window),
    // Editors whose integrated terminal hosts the shell but which have no host kind of their own:
    // their window shows the session, so the shell below them is not a standalone console and
    // the session stays Unknown (a letter). `focus::TERMINAL_HOSTS` lists the same editors.
    ("windsurf.exe", HostKind::Unknown, Tier::Window),
    ("zed.exe", HostKind::Unknown, Tier::Window),
    ("antigravity.exe", HostKind::Unknown, Tier::Window),
    ("idea64.exe", HostKind::Unknown, Tier::Window),
    ("pycharm64.exe", HostKind::Unknown, Tier::Window),
    ("webstorm64.exe", HostKind::Unknown, Tier::Window),
    ("rider64.exe", HostKind::Unknown, Tier::Window),
    ("clion64.exe", HostKind::Unknown, Tier::Window),
    ("goland64.exe", HostKind::Unknown, Tier::Window),
    ("rustrover64.exe", HostKind::Unknown, Tier::Window),
    ("phpstorm64.exe", HostKind::Unknown, Tier::Window),
    ("pwsh.exe", HostKind::PowerShell, Tier::Shell),
    ("powershell.exe", HostKind::PowerShell, Tier::Shell),
    ("cmd.exe", HostKind::CommandPrompt, Tier::Shell),
];

/// The file name of an image path or name, lower case.
#[cfg_attr(not(windows), allow(dead_code))]
fn file_name(image: &str) -> String {
    image.rsplit(['\\', '/']).next().unwrap_or_default().to_lowercase()
}

/// Git for Windows' `bash.exe` (`…\Git\usr\bin\bash.exe`, `…\PortableGit\bin\bash.exe`), not
/// WSL's (`C:\Windows\System32\bash.exe`): only a full path tells them apart, a bare name is neither.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_git_bash(image: &str) -> bool {
    let mut parts = image.split(['\\', '/']).rev();
    parts.next().is_some_and(|name| name.eq_ignore_ascii_case("bash.exe"))
        && parts.any(|dir| dir.eq_ignore_ascii_case("git") || dir.eq_ignore_ascii_case("portablegit"))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn kind_of(image: &str) -> Option<(HostKind, Tier)> {
    if is_git_bash(image) {
        return Some((HostKind::GitBash, Tier::Shell));
    }
    let name = file_name(image);
    KNOWN.iter().find(|(known, _, _)| *known == name).map(|&(_, kind, tier)| (kind, tier))
}

/// The terminal of an agent from its ancestors' images (nearest first; full paths or file names,
/// any case). The nearest program with a window wins: that window shows the session (pwsh in
/// Windows Terminal is Windows Terminal, the integrated terminal of VS Code is VS Code). Without
/// one, the outermost shell, which started the console window (cmd under its conhost is Command
/// Prompt). Nothing recognised → Unknown.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn classify<S: AsRef<str>>(chain: &[S]) -> HostKind {
    let kinds: Vec<(HostKind, Tier)> = chain.iter().filter_map(|image| kind_of(image.as_ref())).collect();
    if let Some(&(kind, _)) = kinds.iter().find(|(_, tier)| *tier == Tier::Window) {
        return kind;
    }
    kinds.last().map_or(HostKind::Unknown, |&(kind, _)| kind)
}

/// Entries kept; past it the cache starts again (a process is asked again at most once more).
const MAX_CACHED: usize = 256;

/// The terminal of each agent process, asked once: a pid with its creation time never moves to
/// another terminal, and a reused pid has another creation time.
#[derive(Debug, Default)]
pub struct TerminalCache {
    by_proc: HashMap<(u32, u64), HostKind>,
}

impl TerminalCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The terminal of `proc`, from `lookup(pid)` the first time only.
    pub fn host(&mut self, proc: &ProcId, lookup: impl FnOnce(u32) -> HostKind) -> HostKind {
        let id = (proc.pid, proc.created_ft);
        if let Some(&kind) = self.by_proc.get(&id) {
            return kind;
        }
        if self.by_proc.len() >= MAX_CACHED {
            self.by_proc.clear();
        }
        let kind = lookup(proc.pid);
        self.by_proc.insert(id, kind);
        kind
    }
}

/// The terminal of the live process `pid`: one process snapshot, the ancestor walk of the window
/// switch, and the full image path of a `bash.exe` ancestor only (Git's or WSL's).
#[cfg(windows)]
pub fn of_process(pid: u32) -> HostKind {
    use crate::focus::win::{ancestors, creation_ft, process_table, MAX_ANCESTORS};

    let rows = process_table();
    let chain: Vec<String> = ancestors(&rows, creation_ft, pid, MAX_ANCESTORS)
        .into_iter()
        .filter_map(|ancestor| rows.iter().find(|r| r.pid == ancestor))
        .map(|row| {
            if row.exe_name.eq_ignore_ascii_case("bash.exe") {
                if let Ok((id, _handle)) = super::win::proc::identify(row.pid) {
                    return id.image;
                }
            }
            row.exe_name.clone()
        })
        .collect();
    classify(&chain)
}

/// macOS: the app bundles (by name) that show a session, and what they are. Editors whose
/// integrated terminal runs the agent count as their editor, or as Unknown when they have no host
/// kind of their own (their window shows the session, so nothing further up does).
#[cfg(any(target_os = "macos", test))]
const MAC_APPS: &[(&str, HostKind)] = &[
    ("Terminal", HostKind::MacTerminal),
    ("iTerm", HostKind::ITerm),
    ("iTerm2", HostKind::ITerm),
    ("Ghostty", HostKind::Ghostty),
    ("kitty", HostKind::Kitty),
    ("WezTerm", HostKind::WezTerm),
    ("Alacritty", HostKind::Alacritty),
    ("Hyper", HostKind::Hyper),
    ("Tabby", HostKind::Tabby),
    ("Warp", HostKind::Warp),
    ("Visual Studio Code", HostKind::VsCode),
    ("Visual Studio Code - Insiders", HostKind::VsCode),
    ("Cursor", HostKind::Cursor),
    ("Windsurf", HostKind::Unknown),
    ("Zed", HostKind::Unknown),
    ("Antigravity", HostKind::Unknown),
];

/// The host of an agent from the app bundles of its ancestors, nearest first (`app_bundle`
/// names): the nearest known app wins. Nothing known → Unknown.
#[cfg(any(target_os = "macos", test))]
pub fn classify_mac<S: AsRef<str>>(bundles: &[S]) -> HostKind {
    bundles
        .iter()
        .find_map(|b| MAC_APPS.iter().find(|(name, _)| name.eq_ignore_ascii_case(b.as_ref())).map(|&(_, k)| k))
        .unwrap_or(HostKind::Unknown)
}

/// The terminal of the live process `pid`: the app bundles above it, nearest first.
#[cfg(target_os = "macos")]
pub fn of_process(pid: u32) -> HostKind {
    let bundles: Vec<String> = crate::platform::ancestors(pid, 16)
        .into_iter()
        .filter_map(|(_, image)| crate::platform::app_bundle(&image).map(|(_, name)| name))
        .collect();
    classify_mac(&bundles)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn of_process(_pid: u32) -> HostKind {
    HostKind::Unknown
}

#[cfg(test)]
mod mac_tests {
    use super::*;

    #[test]
    #[cfg(not(windows))]
    fn every_known_mac_app_may_host_a_window() {
        for (name, _) in MAC_APPS {
            assert!(crate::focus::is_terminal_host(name), "{name} missing from focus::TERMINAL_HOSTS");
        }
    }

    #[test]
    fn the_nearest_known_app_is_the_host() {
        assert_eq!(classify_mac(&["Terminal"]), HostKind::MacTerminal);
        assert_eq!(classify_mac(&["iTerm2", "Terminal"]), HostKind::ITerm);
        assert_eq!(classify_mac(&["Raycast", "Ghostty"]), HostKind::Ghostty);
        assert_eq!(classify_mac(&["Cursor", "Terminal"]), HostKind::Cursor, "Cursor's own terminal");
        assert_eq!(classify_mac(&["Zed", "Terminal"]), HostKind::Unknown, "an editor without a kind stops the walk");
        assert_eq!(classify_mac::<&str>(&[]), HostKind::Unknown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIT_BASH: &str = r"C:\Program Files\Git\usr\bin\bash.exe";

    #[test]
    fn a_shell_inside_a_terminal_is_the_terminal() {
        assert_eq!(classify(&["pwsh.exe", "WindowsTerminal.exe"]), HostKind::WindowsTerminal);
        assert_eq!(classify(&["node.exe", "cmd.exe", "powershell.exe", "WINDOWSTERMINAL.EXE"]), HostKind::WindowsTerminal);
        assert_eq!(classify(&["cmd.exe", "OpenConsole.exe", "WindowsTerminal.exe"]), HostKind::WindowsTerminal);
        assert_eq!(classify(&["pwsh.exe", "wezterm-gui.exe"]), HostKind::WezTerm);
        assert_eq!(classify(&["pwsh.exe", "alacritty.exe"]), HostKind::Alacritty);
        assert_eq!(classify(&["cmd.exe", "Tabby.exe"]), HostKind::Tabby);
        assert_eq!(classify(&["pwsh.exe", "Hyper.exe"]), HostKind::Hyper);
        assert_eq!(classify(&["cmd.exe", "ConEmuC64.exe", "ConEmu64.exe", "Cmder.exe"]), HostKind::ConEmu);
        assert_eq!(classify(&["pwsh.exe", "warp.exe"]), HostKind::Warp);
        assert_eq!(classify(&[GIT_BASH, "mintty.exe"]), HostKind::GitBash);
    }

    #[test]
    fn a_shell_alone_is_the_outermost_shell() {
        assert_eq!(classify(&["pwsh.exe"]), HostKind::PowerShell);
        assert_eq!(classify(&["powershell.exe"]), HostKind::PowerShell);
        assert_eq!(classify(&["cmd.exe"]), HostKind::CommandPrompt);
        assert_eq!(classify(&["node.exe", "cmd.exe", "pwsh.exe"]), HostKind::PowerShell, "codex.cmd run from pwsh");
        assert_eq!(classify(&["pwsh.exe", "cmd.exe"]), HostKind::CommandPrompt, "pwsh started from cmd");
        assert_eq!(classify(&[GIT_BASH]), HostKind::GitBash);
        assert_eq!(classify(&["node.exe", r"D:\PortableGit\bin\bash.exe", "cmd.exe"]), HostKind::CommandPrompt);
    }

    #[test]
    fn the_integrated_terminal_of_an_editor_is_the_editor() {
        assert_eq!(classify(&["pwsh.exe", "Code.exe", "Code.exe"]), HostKind::VsCode);
        assert_eq!(classify(&["cmd.exe", "Code - Insiders.exe"]), HostKind::VsCode);
        assert_eq!(classify(&["pwsh.exe", "Cursor.exe"]), HostKind::Cursor);
        assert_eq!(classify(&[GIT_BASH, "Cursor.exe"]), HostKind::Cursor);
        // the window nearest the agent shows it: an editor started from a terminal is its own window
        assert_eq!(classify(&["pwsh.exe", "Cursor.exe", "pwsh.exe", "WindowsTerminal.exe"]), HostKind::Cursor);
        assert_eq!(classify(&["cmd.exe", "WindowsTerminal.exe", "Code.exe"]), HostKind::WindowsTerminal);
    }

    #[test]
    fn a_shell_in_an_editor_without_a_host_kind_stays_unknown() {
        assert_eq!(classify(&["pwsh.exe", "idea64.exe"]), HostKind::Unknown);
        assert_eq!(classify(&["node.exe", "cmd.exe", "pwsh.exe", "idea64.exe"]), HostKind::Unknown);
        assert_eq!(classify(&["cmd.exe", "Antigravity.exe"]), HostKind::Unknown);
        assert_eq!(classify(&["claude.exe", "pwsh.exe", "Antigravity.exe"]), HostKind::Unknown);
        assert_eq!(classify(&["pwsh.exe", "Windsurf.exe"]), HostKind::Unknown);
        assert_eq!(classify(&[GIT_BASH, "Zed.exe"]), HostKind::Unknown);
        // the nearest window wins: an editor started from Windows Terminal is its own window
        assert_eq!(classify(&["pwsh.exe", "Windsurf.exe", "WindowsTerminal.exe"]), HostKind::Unknown);
        // an app that only launches a console is not a window of the session
        assert_eq!(classify(&["cmd.exe", "totalcmd64.exe"]), HostKind::CommandPrompt);
    }

    #[test]
    #[cfg(windows)]
    fn every_terminal_host_of_the_window_switch_is_recognised() {
        // conhost.exe hosts every console, so it tells nothing about the terminal
        for exe in crate::focus::TERMINAL_HOSTS.iter().filter(|exe| **exe != "conhost.exe") {
            assert!(kind_of(exe).is_some(), "{exe} missing from KNOWN");
        }
    }

    #[test]
    fn full_paths_and_any_case_are_read_by_their_file_name() {
        let wt = r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal_1.21\WindowsTerminal.exe";
        assert_eq!(classify(&[r"C:\Program Files\PowerShell\7\pwsh.exe", wt]), HostKind::WindowsTerminal);
        assert_eq!(classify(&[r"C:\WINDOWS\system32\CMD.EXE"]), HostKind::CommandPrompt);
        assert_eq!(classify(&["/usr/bin/alacritty.exe"]), HostKind::Alacritty);
    }

    #[test]
    fn nothing_recognised_stays_unknown() {
        assert_eq!(classify::<&str>(&[]), HostKind::Unknown);
        assert_eq!(classify(&["node.exe", "Claude.exe", "svchost.exe"]), HostKind::Unknown);
        assert_eq!(classify(&["conhost.exe"]), HostKind::Unknown);
        assert_eq!(classify(&["Windsurf.exe", "idea64.exe"]), HostKind::Unknown, "editors without a host kind");
        // WSL's bash, or a bare name whose folder is unknown, is not Git Bash
        assert_eq!(classify(&[r"C:\Windows\System32\bash.exe"]), HostKind::Unknown);
        assert_eq!(classify(&["bash.exe"]), HostKind::Unknown);
        assert_eq!(classify(&[r"C:\tools\gitlab\bash.exe"]), HostKind::Unknown);
        assert_eq!(classify(&["pwsh.exe.bak", "xcmd.exe", "WindowsTerminal"]), HostKind::Unknown);
    }

    #[test]
    fn the_cache_asks_each_process_once() {
        let mut cache = TerminalCache::new();
        let proc = |pid: u32, created_ft: u64| ProcId { pid, created_ft, image: String::new() };
        let mut calls = 0;
        let mut ask = |p: &ProcId, kind: HostKind| {
            cache.host(p, |_| {
                calls += 1;
                kind
            })
        };
        assert_eq!(ask(&proc(7, 1), HostKind::PowerShell), HostKind::PowerShell);
        assert_eq!(ask(&proc(7, 1), HostKind::CommandPrompt), HostKind::PowerShell, "cached");
        // a reused pid is another process
        assert_eq!(ask(&proc(7, 2), HostKind::CommandPrompt), HostKind::CommandPrompt);
        assert_eq!(ask(&proc(8, 1), HostKind::Unknown), HostKind::Unknown);
        assert_eq!(ask(&proc(8, 1), HostKind::GitBash), HostKind::Unknown, "Unknown is cached too");
        assert_eq!(calls, 3);
    }

    #[test]
    fn the_cache_stays_bounded() {
        let mut cache = TerminalCache::new();
        for pid in 0..(MAX_CACHED as u32 * 2 + 1) {
            cache.host(&ProcId { pid, created_ft: 1, image: String::new() }, |_| HostKind::PowerShell);
            assert!(cache.by_proc.len() <= MAX_CACHED);
        }
    }
}
