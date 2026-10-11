//! macOS: the "window" of a session is its host app. Reading other apps' window titles needs
//! Screen Recording, raising one window of them needs Accessibility; without either, an app is
//! the finest thing Mayak can bring forward. So a `WindowInfo` here is an app: `hwnd` holds the
//! pid of its main process and the title stays empty, which makes the pure part of `focus` pick
//! it as a terminal's topmost window, and send an editor to its CLI launcher (`cursor <folder>`,
//! `code <folder>`), which raises the window of exactly that folder. Terminal and iTerm2 go
//! further: they name their tabs by terminal device, so AppleScript can bring forward the very tab
//! of the session (`raise_tab`), once the user lets Mayak control them.

use super::WindowInfo;
use crate::observe::keys::HostKind;
use crate::platform::{all_pids, ancestors, app_bundle, bring_app_forward, image_path};

/// How far above the agent its host app is looked for.
const MAX_ANCESTORS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raise {
    Focused,
    #[allow(dead_code)] // the same variants as on Windows, so `switch_to` matches them alike
    Flashed,
    Failed,
}

fn app_window(pid: u32) -> WindowInfo {
    WindowInfo { hwnd: pid as isize, title: String::new(), minimized: false }
}

/// The host app of the agent `agent_pid`: the nearest ancestor inside an app bundle, when
/// `accept` takes the bundle's name. Empty otherwise (no app above it: tmux, ssh, launchd).
pub fn host_windows(agent_pid: u32, accept: impl Fn(&str) -> bool) -> Vec<WindowInfo> {
    ancestors(agent_pid, MAX_ANCESTORS)
        .into_iter()
        .find_map(|(pid, image)| app_bundle(&image).map(|(_, name)| (pid, name)))
        .filter(|(_, name)| accept(name))
        .map(|(pid, _)| vec![app_window(pid)])
        .unwrap_or_default()
}

/// The running app whose bundle is called `name` (`Antigravity`, `Cursor`…), as one window.
pub fn app_windows(name: &str) -> Vec<WindowInfo> {
    all_pids()
        .into_iter()
        .find(|pid| {
            image_path(*pid).and_then(|image| app_bundle(&image)).is_some_and(|(bundle, bundle_name)| {
                // The app itself, not a helper of another app that happens to share the name.
                bundle_name.eq_ignore_ascii_case(name)
                    && image_path(*pid).is_some_and(|i| i.starts_with(&format!("{}/Contents/MacOS/", bundle.display())))
            })
        })
        .map(|pid| vec![app_window(pid)])
        .unwrap_or_default()
}

/// Brings the app of the process `hwnd` forward. `Failed` when it is gone or not in a bundle.
pub fn raise(hwnd: isize) -> Raise {
    let Some((bundle, _)) = image_path(hwnd as u32).and_then(|image| app_bundle(&image)) else {
        return Raise::Failed;
    };
    if bring_app_forward(&bundle) { Raise::Focused } else { Raise::Failed }
}

/// A terminal device as AppleScript may be given it: `/dev/ttys` and digits, nothing else.
fn plain_tty(tty: &str) -> bool {
    tty.strip_prefix("/dev/ttys").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The AppleScript that brings forward the tab of `tty` in `host`, Terminal or iTerm2 (the two
/// that name their tabs by device); it answers "ok" when it found the tab, and never starts the
/// app. None for any other host or a device that is not plain.
pub fn tab_script(host: HostKind, tty: &str) -> Option<String> {
    if !plain_tty(tty) {
        return None;
    }
    let body = match host {
        HostKind::MacTerminal => format!(
            "tell application \"Terminal\"
  repeat with w in windows
    repeat with t in tabs of w
      if tty of t is \"{tty}\" then
        set miniaturized of w to false
        set selected of t to true
        set index of w to 1
        activate
        return \"ok\"
      end if
    end repeat
  end repeat
end tell"
        ),
        HostKind::ITerm => format!(
            "tell application \"iTerm2\"
  repeat with w in windows
    repeat with t in tabs of w
      repeat with s in sessions of t
        if tty of s is \"{tty}\" then
          select w
          tell t to select
          tell s to select
          activate
          return \"ok\"
        end if
      end repeat
    end repeat
  end repeat
end tell"
        ),
        _ => return None,
    };
    let app = if host == HostKind::ITerm { "iTerm2" } else { "Terminal" };
    Some(format!("if application \"{app}\" is running then\n{body}\nend if\nreturn \"none\""))
}

/// The WezTerm pane whose terminal is `tty`, from `wezterm cli list --format json`.
pub fn wezterm_pane(list_json: &str, tty: &str) -> Option<u64> {
    let panes: Vec<serde_json::Value> = serde_json::from_str(list_json).ok()?;
    panes
        .iter()
        .find(|p| p.get("tty_name").and_then(|t| t.as_str()) == Some(tty))
        .and_then(|p| p.get("pane_id")?.as_u64())
}

/// WezTerm's own CLI: the one in its bundle, else on PATH.
fn wezterm_cli() -> Option<std::path::PathBuf> {
    let home = crate::platform::home_dir();
    [std::path::PathBuf::from("/Applications"), home.join("Applications")]
        .iter()
        .map(|dir| dir.join("WezTerm.app/Contents/MacOS/wezterm"))
        .find(|p| p.is_file())
        .or_else(|| crate::platform::find_on_path("wezterm"))
}

/// WezTerm has no AppleScript, but its CLI lists panes with their terminal and activates one:
/// that pane's tab comes forward, then the app.
fn raise_wezterm_pane(tty: &str) -> Option<Raise> {
    let cli = wezterm_cli()?;
    let out = std::process::Command::new(&cli).args(["cli", "list", "--format", "json"]).output().ok()?;
    let pane = wezterm_pane(&String::from_utf8_lossy(&out.stdout), tty)?;
    let activated = std::process::Command::new(&cli)
        .args(["cli", "activate-pane", "--pane-id", &pane.to_string()])
        .status()
        .ok()?
        .success();
    // The bundle the CLI lives in, else the running app (a CLI from Homebrew is outside it).
    match cli.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")) {
        Some(bundle) => {
            bring_app_forward(bundle);
        }
        None => {
            if let Some(app) = app_windows("WezTerm").first() {
                raise(app.hwnd);
            }
        }
    }
    activated.then_some(Raise::Focused)
}

/// Brings forward the tab the agent `agent_pid` runs in, for a Terminal, iTerm2 or WezTerm
/// session. For the first two, the first time macOS asks the user to let Mayak control that app (Automation); refused, or no tab
/// found, it is `None` and the caller raises the app as a whole.
pub fn raise_tab(host: HostKind, agent_pid: u32) -> Option<Raise> {
    let tty = crate::platform::tty_of(agent_pid)?;
    if host == HostKind::WezTerm {
        return raise_wezterm_pane(&tty);
    }
    let script = tab_script(host, &tty)?;
    let out = std::process::Command::new("/usr/bin/osascript").args(["-e", &script]).output().ok()?;
    (out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "ok").then_some(Raise::Focused)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_outside_any_app_cannot_be_raised() {
        let mut child = crate::testutil::sleeper(2);
        assert_eq!(raise(child.id() as isize), Raise::Failed);
        assert_eq!(raise(i32::MAX as isize), Raise::Failed);
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn a_tab_is_looked_for_by_a_plain_device_only() {
        let script = tab_script(HostKind::MacTerminal, "/dev/ttys003").unwrap();
        assert!(script.starts_with("if application \"Terminal\" is running then"), "{script}");
        assert!(script.contains("if tty of t is \"/dev/ttys003\" then"));
        assert!(tab_script(HostKind::ITerm, "/dev/ttys12").unwrap().contains("tell application \"iTerm2\""));
        for bad in ["/dev/ttys", "/dev/ttys1\" & do shell script \"x", "/dev/console", "ttys003"] {
            assert_eq!(tab_script(HostKind::MacTerminal, bad), None, "{bad}");
        }
        assert_eq!(tab_script(HostKind::Ghostty, "/dev/ttys003"), None, "no tab API");
    }

    #[test]
    fn a_wezterm_pane_is_found_by_its_terminal() {
        let list = r#"[{"window_id":0,"tab_id":0,"pane_id":3,"tty_name":"/dev/ttys004","title":"zsh"},
                       {"window_id":0,"tab_id":1,"pane_id":7,"tty_name":"/dev/ttys009","title":"claude"}]"#;
        assert_eq!(wezterm_pane(list, "/dev/ttys009"), Some(7));
        assert_eq!(wezterm_pane(list, "/dev/ttys001"), None);
        assert_eq!(wezterm_pane("not json", "/dev/ttys009"), None);
    }

    #[test]
    fn a_process_without_a_terminal_has_no_tab() {
        let mut child = crate::testutil::sleeper(2);
        assert_eq!(crate::platform::tty_of(child.id()), None);
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn nothing_accepted_gives_no_window() {
        assert!(host_windows(std::process::id(), |_| false).is_empty());
        assert!(app_windows("No Such App Mayak").is_empty());
    }
}
