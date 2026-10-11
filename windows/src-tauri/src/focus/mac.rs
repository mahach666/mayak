//! macOS: the "window" of a session is its host app. Reading other apps' window titles needs
//! Screen Recording, raising one window of them needs Accessibility; without either, an app is
//! the finest thing Mayak can bring forward. So a `WindowInfo` here is an app: `hwnd` holds the
//! pid of its main process and the title stays empty, which makes the pure part of `focus` pick
//! it as a terminal's topmost window, and send an editor to its CLI launcher (`cursor <folder>`,
//! `code <folder>`), which raises the window of exactly that folder.

use super::WindowInfo;
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
    fn nothing_accepted_gives_no_window() {
        assert!(host_windows(std::process::id(), |_| false).is_empty());
        assert!(app_windows("No Such App Mayak").is_empty());
    }
}
