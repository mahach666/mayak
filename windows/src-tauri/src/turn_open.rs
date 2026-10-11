//! «Итог хода» (ideas §3): a file a turn changed, opened in an editor at its first changed line
//! (`cursor://file/<path>:<line>`, `vscode://file/<path>:<line>`). The island asks by session key
//! and file index, so only a path the snapshot holds is ever opened; it must be a local file on a
//! drive (no UNC share, no `..`). The editor is the session's host when it is Cursor or VS Code,
//! else the first of VS Code and Cursor whose URL protocol is registered. Log lines carry no path.

#![cfg_attr(not(windows), allow(dead_code))]

use crate::launcher::{encode_component, Editor};
use crate::observe::keys::HostKind;
use crate::observe::model::{SessionsSnapshot, TurnFileView};

/// The host and the `index`th changed file of the last turn of session `key`, if there is one and
/// it is still `path`: the list may have changed since the island drew it.
pub fn turn_file(snapshot: &SessionsSnapshot, key: &str, index: usize, path: &str) -> Option<(HostKind, TurnFileView)> {
    snapshot.projects.iter().find_map(|project| {
        let s = project.sessions.iter().find(|s| s.key == key)?;
        let file = s.turn.as_ref()?.files.get(index)?;
        (file.path == path).then(|| (s.host, file.clone()))
    })
}

/// The editor for a session's host: the host itself when it is an editor, else VS Code, then
/// Cursor, whichever has its protocol registered (`registered(scheme)`).
pub fn pick_editor(host: HostKind, registered: impl Fn(&str) -> bool) -> Option<Editor> {
    let order = match host {
        HostKind::Cursor => [Editor::Cursor, Editor::VsCode],
        _ => [Editor::VsCode, Editor::Cursor],
    };
    order.into_iter().find(|editor| registered(editor.scheme()))
}

/// `<scheme>://file/C:/dir/name.ext:<line>`, every path segment percent-encoded. None unless the
/// path is absolute on a drive letter (`C:\…` or `C:/…`), has no empty, `.` or `..` segment and no
/// control character: a UNC path would reach the network, a relative one depends on the editor.
pub fn file_uri(editor: Editor, path: &str, line: u32) -> Option<String> {
    let bytes = path.as_bytes();
    let drive_ok = bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/');
    if !drive_ok || path.chars().any(char::is_control) {
        return None;
    }
    let segments: Vec<&str> = path[3..].split(['\\', '/']).collect();
    if segments.iter().any(|s| s.is_empty() || *s == "." || *s == "..") {
        return None;
    }
    let encoded: Vec<String> = segments.iter().map(|s| encode_component(s)).collect();
    Some(format!("{}://file/{}:/{}:{}", editor.scheme(), &path[..1], encoded.join("/"), line.max(1)))
}

/// Starts opening `path` at `line` for a session of `host`; true when the link was handed to
/// Windows' protocol handler on its own thread. The caller checked that the path is a file.
#[cfg(windows)]
pub fn open(host: HostKind, path: &str, line: u32) -> bool {
    let Some(editor) = pick_editor(host, crate::launcher::win::protocol_registered) else {
        crate::log::line("session_file_open: no editor protocol registered");
        return false;
    };
    let Some(uri) = file_uri(editor, path, line) else {
        crate::log::line("session_file_open: path refused");
        return false;
    };
    std::thread::spawn(move || {
        // COM for the call: `ShellExecuteW` may hand the link to a shell extension.
        let _com = crate::launcher::win::Com::init();
        match crate::launcher::win::open_uri(&uri) {
            Ok(()) => crate::log::line(format!("session_file_open: opened in {}", editor.scheme())),
            Err(err) => crate::log::line(format!("session_file_open: {} link failed: {err}", editor.scheme())),
        }
    });
    true
}

/// Editor links go through Windows' protocol handlers; elsewhere nothing is opened.
#[cfg(not(windows))]
pub fn open(_host: HostKind, _path: &str, _line: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::model::{PhaseView, ProjectView, SessionView, Source, Store, TurnView};

    #[test]
    fn the_host_editor_comes_first_then_vs_code_then_cursor() {
        let all = |_: &str| true;
        assert_eq!(pick_editor(HostKind::Cursor, all), Some(Editor::Cursor));
        assert_eq!(pick_editor(HostKind::VsCode, all), Some(Editor::VsCode));
        assert_eq!(pick_editor(HostKind::WindowsTerminal, all), Some(Editor::VsCode));
        assert_eq!(pick_editor(HostKind::ClaudeDesktop, |s| s == "cursor"), Some(Editor::Cursor));
        assert_eq!(pick_editor(HostKind::Cursor, |s| s == "vscode"), Some(Editor::VsCode));
        assert_eq!(pick_editor(HostKind::Unknown, |_| false), None);
    }

    #[test]
    fn a_drive_path_becomes_an_encoded_file_link_at_the_line() {
        assert_eq!(
            file_uri(Editor::Cursor, r"C:\Users\dev\src\demo\src\lib.rs", 12).as_deref(),
            Some("cursor://file/C:/Users/dev/src/demo/src/lib.rs:12")
        );
        assert_eq!(
            file_uri(Editor::VsCode, "d:/My Projects/app #1/файл.ts", 0).as_deref(),
            Some("vscode://file/d:/My%20Projects/app%20%231/%D1%84%D0%B0%D0%B9%D0%BB.ts:1"),
            "spaces, '#' and non-ASCII are encoded; line 0 becomes 1"
        );
    }

    #[test]
    fn paths_that_are_not_plain_local_files_are_refused() {
        for path in [
            r"\\server\share\a.rs",
            r"\\?\C:\a.rs",
            "relative/a.rs",
            r"C:",
            r"C:\",
            r"C:a.rs",
            r"C:\src\..\Windows\a.rs",
            r"C:\src\.\a.rs",
            r"C:\src\\a.rs",
            "C:\\src\\a\u{0}.rs",
            "C:\\src\\a\n.rs",
            "1:\\a.rs",
        ] {
            assert_eq!(file_uri(Editor::VsCode, path, 1), None, "{path:?}");
        }
    }

    fn session_with_turn(key: &str, host: HostKind, files: &[&str]) -> SessionView {
        SessionView {
            key: key.into(),
            agent: Source::Claude,
            host,
            title: None,
            phase: PhaseView::Idle,
            since: 0,
            activity: None,
            detail: None,
            outcome: None,
            attention: None,
            acked: true,
            silent: None,
            context: None,
            model: None,
            compacted: None,
            turn: (!files.is_empty()).then(|| TurnView {
                files: files.iter().map(|p| TurnFileView { path: p.to_string(), added: 1, removed: 0, line: 3 }).collect(),
                file_count: files.len() as u32,
                added: files.len() as u64,
                removed: 0,
                partial: false,
                ms: None,
            }),
            agent_pid: None,
            host_session_id: None,
            logical_id: None,
            cwd: None,
            last_reply: None,
        }
    }

    #[test]
    fn the_file_is_looked_up_by_session_key_and_index() {
        let mut snapshot = Store::new().snapshot(0);
        snapshot.projects.push(ProjectView {
            key: r"c:\work\app".into(),
            name: "app".into(),
            root: r"C:\work\app".into(),
            sessions: vec![
                session_with_turn("claude:1:2", HostKind::Cursor, &[r"C:\work\app\a.rs", r"C:\work\app\b.rs"]),
                session_with_turn("codex:t", HostKind::CodexDesktop, &[]),
            ],
        });
        let (host, file) = turn_file(&snapshot, "claude:1:2", 1, r"C:\work\app\b.rs").unwrap();
        assert_eq!((host, file.path.as_str(), file.line), (HostKind::Cursor, r"C:\work\app\b.rs", 3));
        assert_eq!(turn_file(&snapshot, "claude:1:2", 1, r"C:\work\app\a.rs"), None, "the list changed since it was drawn");
        assert_eq!(turn_file(&snapshot, "claude:1:2", 2, r"C:\work\app\b.rs"), None, "past the list");
        assert_eq!(turn_file(&snapshot, "codex:t", 0, r"C:\work\app\a.rs"), None, "no turn summary");
        assert_eq!(turn_file(&snapshot, "claude:9:9", 0, r"C:\work\app\a.rs"), None, "unknown session");
    }
}
