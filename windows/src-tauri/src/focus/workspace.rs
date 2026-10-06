//! Multi-root workspace windows of Cursor and VS Code. Their title names the `.code-workspace`
//! file (`<chat or file> - <name> (Workspace) - Cursor`), not a folder, so the folders come from
//! that file. The editor's `storage.json` only tells where those files are; which window is which
//! still comes from the live titles. Everything here is pure except `load`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::log;
use crate::observe::keys::HostKind;
use crate::observe::paths::{normalize, NormPath};

/// Lowercase workspace name (the file stem) → normalized folder keys of that workspace.
pub type Workspaces = HashMap<String, Vec<String>>;

const WORKSPACE_MARK: &str = " (Workspace)";

/// The workspace name of a title segment (`anlax-web-ws (Workspace)` → `anlax-web-ws`), `None`
/// for a folder segment. The mark is ASCII: its comparison ignores case on bytes.
pub fn workspace_name(segment: &str) -> Option<&str> {
    let cut = segment.len().checked_sub(WORKSPACE_MARK.len())?;
    if !segment.is_char_boundary(cut) || !segment[cut..].eq_ignore_ascii_case(WORKSPACE_MARK) {
        return None;
    }
    Some(&segment[..cut]).filter(|name| !name.trim().is_empty())
}

/// JSON with comments (`.code-workspace`) to plain JSON: `//` and `/* */` comments outside
/// strings removed, trailing commas before `]` or `}` dropped.
pub fn strip_jsonc(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            plain.push(c);
            if c == '\\' {
                if let Some(escaped) = chars.next() {
                    plain.push(escaped);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                plain.push(c);
            }
            ('/', Some('/')) => {
                while chars.next_if(|n| *n != '\n').is_some() {}
            }
            ('/', Some('*')) => {
                chars.next();
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
                // a comment between two tokens still separates them
                plain.push(' ');
            }
            _ => plain.push(c),
        }
    }
    drop_trailing_commas(&plain)
}

/// Commas outside strings whose next token is `]` or `}`. Comments are already gone.
fn drop_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
        } else if c == ',' && text[i + 1..].trim_start().starts_with([']', '}']) {
            continue;
        }
        out.push(c);
    }
    out
}

/// The folders of a workspace file, as normalized keys. `dir` is the folder of the file: a
/// relative `path` resolves against it. An entry that is not a usable `path` or `file:///` `uri`
/// is skipped. `None` when the text is not a workspace (not JSON, or no `folders` array).
pub fn workspace_folders(text: &str, dir: &str) -> Option<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let value: Value = serde_json::from_str(&strip_jsonc(text)).ok()?;
    let entries = value.get("folders")?.as_array()?;
    let folders = entries
        .iter()
        .filter_map(|entry| {
            if let Some(path) = entry.get("path").and_then(Value::as_str) {
                return Some(folder_of_path(path, dir));
            }
            entry.get("uri").and_then(Value::as_str).and_then(file_uri_path)
        })
        .map(|norm| norm.key)
        .filter(|key| !key.is_empty())
        .collect();
    Some(folders)
}

/// A `path` entry: absolute as it is, relative against the folder of the workspace file.
fn folder_of_path(path: &str, dir: &str) -> NormPath {
    let path = path.trim();
    let b = path.as_bytes();
    let absolute = (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') || path.starts_with(['\\', '/']);
    if absolute {
        normalize(path)
    } else {
        normalize(&format!("{dir}\\{path}"))
    }
}

/// A local `file:///` URI (`file:///c%3A/Users/x`) as a drive path; `None` for another scheme,
/// a broken percent escape, or a path that is not on a drive.
fn file_uri_path(uri: &str) -> Option<NormPath> {
    let head = uri.get(..8)?;
    if !head.eq_ignore_ascii_case("file:///") {
        return None;
    }
    let norm = normalize(&percent_decode(&uri[8..])?);
    let b = norm.display.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\').then_some(norm)
}

/// `%XX` escapes to bytes, read as UTF-8; `None` for a broken escape or bytes that are not UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The session root (a normalized key) is one of `folders` or lies inside one of them.
pub fn root_in_folders(root: &str, folders: &[String]) -> bool {
    !root.is_empty()
        && folders.iter().filter(|f| !f.is_empty()).any(|folder| match root.strip_prefix(folder.as_str()) {
            Some(rest) => rest.is_empty() || rest.starts_with('\\') || folder.ends_with('\\'),
            None => false,
        })
}

/// The `.code-workspace` files named in an editor's `storage.json` (display paths, each once).
/// `None` when the text is not JSON; missing keys give an empty list.
pub fn storage_workspace_files(text: &str) -> Option<Vec<String>> {
    let value: Value = serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(text)).ok()?;
    let state = value.get("windowsState");
    let windows = state
        .and_then(|s| s.get("lastActiveWindow"))
        .into_iter()
        .chain(state.and_then(|s| s.get("openedWindows")).and_then(Value::as_array).into_iter().flatten())
        .filter_map(|w| w.get("workspaceIdentifier"));
    let backups = value
        .get("backupWorkspaces")
        .and_then(|b| b.get("workspaces"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let mut seen = std::collections::HashSet::new();
    let files = windows
        .chain(backups)
        .filter_map(|ws| ws.get("configURIPath").and_then(Value::as_str))
        .filter_map(file_uri_path)
        .filter(|norm| seen.insert(norm.key.clone()))
        .map(|norm| norm.display)
        .collect();
    Some(files)
}

/// The files among `files` whose stem is one of `names` (lowercase), as `(name, path)`. A name
/// carried by two different files is ambiguous and left out: the title cannot tell them apart.
/// `files` holds each file once (`storage_workspace_files`).
pub fn files_for_names(files: &[String], names: &[String]) -> Vec<(String, String)> {
    let stem = |file: &String| Path::new(file).file_stem().and_then(|s| s.to_str()).map(str::to_lowercase);
    let mut chosen: Vec<(String, String)> = Vec::new();
    let mut ambiguous: Vec<String> = Vec::new();
    for file in files {
        let Some(name) = stem(file).filter(|n| names.contains(n)) else { continue };
        if chosen.iter().any(|(n, _)| *n == name) {
            ambiguous.push(name);
        } else {
            chosen.push((name, file.clone()));
        }
    }
    chosen.retain(|(n, _)| !ambiguous.contains(n));
    chosen
}

/// `storage.json` of the editor under `%APPDATA%`; `None` for a host that is not an editor.
pub fn storage_path(host: HostKind, appdata: &Path) -> Option<PathBuf> {
    let app = match host {
        HostKind::Cursor => "Cursor",
        HostKind::VsCode => "Code",
        HostKind::ClaudeDesktop | HostKind::CodexDesktop | HostKind::Unknown => return None,
    };
    Some(appdata.join(app).join("User").join("globalStorage").join("storage.json"))
}

/// The folders of the workspaces called `names` (lowercase) for `host`, read from disk. A missing
/// or broken file only leaves its workspace out, with one log line that names the error kind and
/// never a path or content.
pub fn load(host: HostKind, names: &[String]) -> Workspaces {
    let Some(appdata) = std::env::var_os("APPDATA") else { return Workspaces::new() };
    let Some(storage) = storage_path(host, Path::new(&appdata)) else { return Workspaces::new() };
    let text = match std::fs::read_to_string(storage) {
        Ok(text) => text,
        Err(err) => {
            log::line(format!("session_open: editor storage unreadable: {:?}", err.kind()));
            return Workspaces::new();
        }
    };
    let Some(files) = storage_workspace_files(&text) else {
        log::line("session_open: editor storage invalid");
        return Workspaces::new();
    };
    read_workspaces(&files, names)
}

/// The folders of the workspaces called `names` among `files`.
fn read_workspaces(files: &[String], names: &[String]) -> Workspaces {
    let mut found = Workspaces::new();
    for (name, file) in files_for_names(files, names) {
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(err) => {
                log::line(format!("session_open: workspace file unreadable: {:?}", err.kind()));
                continue;
            }
        };
        let dir = Path::new(&file).parent().and_then(Path::to_str).unwrap_or("");
        match workspace_folders(&text, dir) {
            Some(folders) => {
                found.insert(name, folders);
            }
            None => log::line("session_open: workspace file invalid"),
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_workspace_name_is_the_segment_without_its_mark() {
        assert_eq!(workspace_name("anlax-web-ws (Workspace)"), Some("anlax-web-ws"));
        assert_eq!(workspace_name("Anlax-Web-WS (workspace)"), Some("Anlax-Web-WS"));
        assert_eq!(workspace_name("anlax-web-ws"), None);
        assert_eq!(workspace_name(" (Workspace)"), None);
        assert_eq!(workspace_name("Проект (Workspace)"), Some("Проект"));
        assert_eq!(workspace_name("(Workspace)"), None);
    }

    #[test]
    fn comments_outside_strings_are_removed() {
        let text = "{\n  // a line comment\n  \"a\": 1, /* block\n comment */ \"b\": \"x//y/*z*/\"\n}";
        let v: Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
        assert_eq!(v["a"], 1);
        assert_eq!(v["b"], "x//y/*z*/");
    }

    #[test]
    fn escaped_quotes_keep_the_string_open() {
        let text = r#"{"a": "q\"//not a comment", "b": "c:\\"} // end"#;
        let v: Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
        assert_eq!(v["a"], "q\"//not a comment");
        assert_eq!(v["b"], "c:\\");
    }

    #[test]
    fn trailing_commas_are_dropped_but_not_inside_strings() {
        let text = "{\"folders\": [{\"path\": \"a,]\",}, {\"path\": \"b\"} , // last\n ],\n}";
        let v: Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
        assert_eq!(v["folders"][0]["path"], "a,]");
        assert_eq!(v["folders"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn folders_resolve_relative_paths_absolute_paths_and_file_uris() {
        let text = r#"{
            // the projects
            "folders": [
                { "path": "AnlaxCollisionService" },
                { "path": "../shared/lib/" },
                { "path": "D:\\Abs\\Path" },
                { "path": "e:/forward/slashes" },
                { "uri": "file:///c%3A/Users/HP/Some%20Dir" },
                { "uri": "vscode-remote://ssh-remote+host/home/x" },
                { "uri": "file:///c%3A/bad%zz" },
                { "name": "no path" },
                { "path": 7 },
                "garbage",
            ],
        }"#;
        let folders = workspace_folders(text, r"C:\Users\HP\source\repos\anlaxtech").unwrap();
        assert_eq!(
            folders,
            keys(&[
                r"c:\users\hp\source\repos\anlaxtech\anlaxcollisionservice",
                r"c:\users\hp\source\repos\shared\lib",
                r"d:\abs\path",
                r"e:\forward\slashes",
                r"c:\users\hp\some dir",
            ])
        );
    }

    #[test]
    fn a_text_that_is_not_a_workspace_has_no_folders() {
        assert_eq!(workspace_folders("not json", r"C:\x"), None);
        assert_eq!(workspace_folders(r#"{"settings": {}}"#, r"C:\x"), None);
        assert_eq!(workspace_folders(r#"{"folders": {}}"#, r"C:\x"), None);
        assert_eq!(workspace_folders("\u{feff}{\"folders\": []}", r"C:\x"), Some(Vec::new()));
    }

    #[test]
    fn the_root_is_a_folder_or_lies_inside_one() {
        let folders = keys(&[r"c:\repos\anlaxtech", r"d:\"]);
        assert!(root_in_folders(r"c:\repos\anlaxtech", &folders));
        assert!(root_in_folders(r"c:\repos\anlaxtech\service", &folders));
        assert!(root_in_folders(r"d:\anything", &folders));
        assert!(!root_in_folders(r"c:\repos\anlax", &folders));
        assert!(!root_in_folders(r"c:\repos\anlaxtech2", &folders));
        assert!(!root_in_folders(r"c:\repos", &folders));
        assert!(!root_in_folders("", &folders));
        assert!(!root_in_folders(r"c:\x", &keys(&[""])));
        let sibling = keys(&[r"c:\repos\anlax"]);
        assert!(!root_in_folders(r"c:\repos\anlaxtech", &sibling));
    }

    #[test]
    fn storage_lists_the_workspace_files_of_windows_and_backups_once() {
        let text = r#"{
            "windowsState": {
                "lastActiveWindow": { "workspaceIdentifier": { "id": "1", "configURIPath": "file:///c%3A/r/Anlax-web-ws.code-workspace" } },
                "openedWindows": [
                    { "folder": "file:///c%3A/Users/HP/source/repos/tasq" },
                    { "workspaceIdentifier": { "id": "1", "configURIPath": "file:///c%3A/r/Anlax-web-ws.code-workspace" } },
                    { "workspaceIdentifier": { "id": "2", "configURIPath": "file:///d%3A/w/Other.code-workspace" } },
                    { "workspaceIdentifier": { "id": "3" } },
                    7
                ]
            },
            "backupWorkspaces": {
                "workspaces": [ { "id": "4", "configURIPath": "file:///e%3A/b/Back.code-workspace" }, "x" ],
                "folders": [ { "folderUri": "file:///c%3A/f" } ]
            }
        }"#;
        assert_eq!(
            storage_workspace_files(text).unwrap(),
            keys(&[
                r"C:\r\Anlax-web-ws.code-workspace",
                r"D:\w\Other.code-workspace",
                r"E:\b\Back.code-workspace",
            ])
        );
    }

    #[test]
    fn storage_without_the_keys_or_of_another_shape_lists_nothing() {
        assert_eq!(storage_workspace_files("{}"), Some(Vec::new()));
        assert_eq!(storage_workspace_files(r#"{"windowsState": 3, "backupWorkspaces": []}"#), Some(Vec::new()));
        assert_eq!(storage_workspace_files(r#"{"windowsState": {"openedWindows": {}}}"#), Some(Vec::new()));
        assert_eq!(storage_workspace_files("[1, 2]"), Some(Vec::new()));
        assert_eq!(storage_workspace_files("garbage {"), None);
        assert_eq!(storage_workspace_files(""), None);
    }

    #[test]
    fn files_are_chosen_by_their_stem_and_an_ambiguous_name_is_left_out() {
        let files = keys(&[
            r"C:\repos\Anlax-web-ws.code-workspace",
            r"C:\a\Twin.code-workspace",
            r"C:\b\twin.code-workspace",
            r"C:\c\my.project.code-workspace",
            r"C:\d\Unwanted.code-workspace",
        ]);
        let names = keys(&["anlax-web-ws", "twin", "my.project", "missing"]);
        assert_eq!(
            files_for_names(&files, &names),
            vec![
                ("anlax-web-ws".to_string(), r"C:\repos\Anlax-web-ws.code-workspace".to_string()),
                ("my.project".to_string(), r"C:\c\my.project.code-workspace".to_string()),
            ]
        );
        assert_eq!(files_for_names(&files, &[]), Vec::new());
    }

    #[test]
    fn each_editor_has_its_storage_under_appdata() {
        let appdata = Path::new(r"C:\Users\HP\AppData\Roaming");
        assert_eq!(
            storage_path(HostKind::Cursor, appdata),
            Some(appdata.join(r"Cursor\User\globalStorage\storage.json"))
        );
        assert_eq!(
            storage_path(HostKind::VsCode, appdata),
            Some(appdata.join(r"Code\User\globalStorage\storage.json"))
        );
        for host in [HostKind::ClaudeDesktop, HostKind::CodexDesktop, HostKind::Unknown] {
            assert_eq!(storage_path(host, appdata), None);
        }
    }

    #[test]
    fn load_reads_the_named_workspace_and_skips_what_is_missing() {
        let tmp = crate::testutil::TempDir::new("workspace-load");
        let dir = tmp.path.join("anlaxtech");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Anlax-web-ws.code-workspace");
        std::fs::write(&file, "{\"folders\": [{\"path\": \"Service\"},], // c\n}").unwrap();
        let broken = dir.join("Broken.code-workspace");
        std::fs::write(&broken, "{ not json").unwrap();
        let files = vec![
            file.to_str().unwrap().to_string(),
            broken.to_str().unwrap().to_string(),
            dir.join("Gone.code-workspace").to_str().unwrap().to_string(),
        ];
        let names = keys(&["anlax-web-ws", "broken", "gone"]);
        let got = read_workspaces(&files, &names);
        let service = normalize(dir.join("Service").to_str().unwrap()).key;
        assert_eq!(got.len(), 1);
        assert_eq!(got.get("anlax-web-ws"), Some(&vec![service]));
    }
}
