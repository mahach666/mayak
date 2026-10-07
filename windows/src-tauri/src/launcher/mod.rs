//! The chat tab as a launcher (spec docs/superpowers/specs/2026-10-07-mayak-launcher-design.md):
//! which agents are installed (§2, §3) and opening one with the question in its field (§4).
//! Everything but `open` and its flows is pure or reads only the folders given to it; the Win32
//! part is in `win.rs`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod win;

/// A target of the prompt view (spec §3). Wire names are camelCase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentId {
    ClaudeDesktop,
    ClaudeCursor,
    ClaudeVsCode,
    CodexDesktop,
    CodexCursor,
    CodexVsCode,
    /// The island chat over the saved Anthropic key; never launched.
    ClaudeApi,
}

/// The editors that host the Claude and Codex extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editor {
    Cursor,
    VsCode,
}

impl Editor {
    /// URL scheme of the editor (`cursor://…`, `vscode://…`), also its protocol key.
    pub fn scheme(self) -> &'static str {
        match self {
            Editor::Cursor => "cursor",
            Editor::VsCode => "vscode",
        }
    }

    /// `%USERPROFILE%\.cursor\extensions` / `%USERPROFILE%\.vscode\extensions` (spec §2).
    pub fn extensions_dir(self, home: &Path) -> PathBuf {
        let profile = match self {
            Editor::Cursor => ".cursor",
            Editor::VsCode => ".vscode",
        };
        home.join(profile).join("extensions")
    }
}

/// Folder name prefix of the Claude Code extension (`anthropic.claude-code-2.1.291-win32-x64`).
pub const CLAUDE_EXTENSION: &str = "anthropic.claude-code-";
/// Folder name prefix of the Codex extension (`openai.chatgpt-26.930.51102-win32-x64`).
pub const CODEX_EXTENSION: &str = "openai.chatgpt-";
/// AppX package name of Codex Desktop; its family is `OpenAI.Codex_<publisher id>`.
pub const CODEX_PACKAGE: &str = "OpenAI.Codex";
/// Credential Manager key of the Anthropic API key (secrets.rs).
pub const API_KEY: &str = "anthropic-api-key";

/// True when `dir` holds a folder of the extension `prefix` (`<prefix><digit>…`, any version)
/// that the editor has not marked `true` in `.obsolete` (uninstalled, folder not removed yet).
pub fn has_extension(dir: &Path, prefix: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    let obsolete = obsolete_folders(dir);
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { return false };
        is_extension_folder(name, prefix)
            && !obsolete.contains(name)
            && entry.file_type().is_ok_and(|kind| kind.is_dir())
    })
}

fn is_extension_folder(name: &str, prefix: &str) -> bool {
    name.len() > prefix.len()
        && name.is_char_boundary(prefix.len())
        && name[..prefix.len()].eq_ignore_ascii_case(prefix)
        && name.as_bytes()[prefix.len()].is_ascii_digit()
}

/// The folder names `.obsolete` marks `true`; empty when the file is missing or not a JSON object.
fn obsolete_folders(dir: &Path) -> HashSet<String> {
    let Ok(text) = std::fs::read_to_string(dir.join(".obsolete")) else { return HashSet::new() };
    let Ok(map) = serde_json::from_str::<serde_json::Map<String, Value>>(&text) else { return HashSet::new() };
    map.into_iter().filter(|(_, v)| v.as_bool() == Some(true)).map(|(k, _)| k).collect()
}

/// The package family folder of Codex Desktop under `%LOCALAPPDATA%\Packages`
/// (`OpenAI.Codex_2p2nqsd0c76g0`), as it is named on disk.
pub fn codex_family(packages: &Path) -> Option<String> {
    let entries = std::fs::read_dir(packages).ok()?;
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .find(|name| is_family_of(name, CODEX_PACKAGE))
}

/// `<package>_<publisher id>`, the publisher id ASCII letters and digits only.
fn is_family_of(name: &str, package: &str) -> bool {
    let Some((head, publisher)) = name.split_once('_') else { return false };
    head.eq_ignore_ascii_case(package) && !publisher.is_empty() && publisher.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The installed targets in the spec's order (§3). `protocol` tells whether a URL scheme is
/// registered (`win::protocol_registered`); `api_key` whether the Anthropic key is saved.
pub fn installed_targets(home: &Path, localappdata: &Path, protocol: impl Fn(&str) -> bool, api_key: bool) -> Vec<AgentId> {
    let in_editor = |editor: Editor, prefix: &str| {
        protocol(editor.scheme()) && has_extension(&editor.extensions_dir(home), prefix)
    };
    let mut found = Vec::new();
    if protocol("claude") {
        found.push(AgentId::ClaudeDesktop);
    }
    if in_editor(Editor::Cursor, CLAUDE_EXTENSION) {
        found.push(AgentId::ClaudeCursor);
    }
    if in_editor(Editor::VsCode, CLAUDE_EXTENSION) {
        found.push(AgentId::ClaudeVsCode);
    }
    if protocol("codex") && codex_family(&localappdata.join("Packages")).is_some() {
        found.push(AgentId::CodexDesktop);
    }
    if in_editor(Editor::Cursor, CODEX_EXTENSION) {
        found.push(AgentId::CodexCursor);
    }
    if in_editor(Editor::VsCode, CODEX_EXTENSION) {
        found.push(AgentId::CodexVsCode);
    }
    if api_key {
        found.push(AgentId::ClaudeApi);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::AgentId::*;
    use super::*;
    use crate::testutil::TempDir;
    use serde_json::json;

    const ALL: [AgentId; 7] = [ClaudeDesktop, ClaudeCursor, ClaudeVsCode, CodexDesktop, CodexCursor, CodexVsCode, ClaudeApi];

    fn mkdirs(root: &Path, dirs: &[&str]) {
        for dir in dirs {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
    }

    /// A profile with both extensions in both editors and the Codex package: (home, localappdata).
    fn machine(tmp: &TempDir) -> (PathBuf, PathBuf) {
        let home = tmp.path.join("home");
        let local = tmp.path.join("local");
        mkdirs(
            &home,
            &[
                r".cursor\extensions\anthropic.claude-code-2.1.291-win32-x64",
                r".cursor\extensions\openai.chatgpt-26.721.30844-win32-x64",
                r".vscode\extensions\anthropic.claude-code-2.1.289-win32-x64",
                r".vscode\extensions\openai.chatgpt-26.930.51102-win32-x64",
            ],
        );
        mkdirs(&local, &[r"Packages\OpenAI.Codex_2p2nqsd0c76g0"]);
        (home, local)
    }

    #[test]
    fn agent_ids_have_stable_wire_names() {
        let names: Vec<_> = ALL.iter().map(|a| serde_json::to_value(a).unwrap()).collect();
        assert_eq!(
            names,
            [
                json!("claudeDesktop"),
                json!("claudeCursor"),
                json!("claudeVsCode"),
                json!("codexDesktop"),
                json!("codexCursor"),
                json!("codexVsCode"),
                json!("claudeApi"),
            ]
        );
        assert_eq!(serde_json::from_value::<AgentId>(json!("claudeVsCode")).unwrap(), ClaudeVsCode);
        assert!(serde_json::from_value::<AgentId>(json!("cursor")).is_err());
    }

    #[test]
    fn an_extension_folder_of_any_version_is_found() {
        let tmp = TempDir::new("launcher-ext");
        mkdirs(&tmp.path, &["anthropic.claude-code-2.1.288-win32-x64", "anthropic.claude-code-2.1.291-win32-x64"]);
        assert!(has_extension(&tmp.path, CLAUDE_EXTENSION));
        assert!(!has_extension(&tmp.path, CODEX_EXTENSION));
    }

    #[test]
    fn no_folder_another_extension_or_a_file_is_no_target() {
        let tmp = TempDir::new("launcher-ext-none");
        assert!(!has_extension(&tmp.path.join("missing"), CLAUDE_EXTENSION), "no extensions folder");
        mkdirs(&tmp.path, &["openai.codex-audio-26.930.41038", "anthropic.claude-code-insiders", "anthropic.claude-code-"]);
        std::fs::write(tmp.path.join("openai.chatgpt-26.930.51102-win32-x64"), b"").unwrap();
        assert!(!has_extension(&tmp.path, CLAUDE_EXTENSION), "no version after the prefix");
        assert!(!has_extension(&tmp.path, CODEX_EXTENSION), "codex-audio is another extension, a file is no folder");
    }

    #[test]
    fn a_folder_listed_as_obsolete_does_not_count() {
        // Both editors keep the folders of uninstalled versions and list them in `.obsolete`.
        let tmp = TempDir::new("launcher-ext-obsolete");
        mkdirs(&tmp.path, &["anthropic.claude-code-2.1.288-win32-x64", "openai.chatgpt-26.623.31443-win32-x64"]);
        std::fs::write(
            tmp.path.join(".obsolete"),
            r#"{"anthropic.claude-code-2.1.288-win32-x64":true,"openai.chatgpt-26.623.31443-win32-x64":false}"#,
        )
        .unwrap();
        assert!(!has_extension(&tmp.path, CLAUDE_EXTENSION), "uninstalled, its folder not yet removed");
        assert!(has_extension(&tmp.path, CODEX_EXTENSION), "false is not obsolete");
        mkdirs(&tmp.path, &["anthropic.claude-code-2.1.291-win32-x64"]);
        assert!(has_extension(&tmp.path, CLAUDE_EXTENSION), "a newer version is installed");
        std::fs::remove_dir_all(tmp.path.join("anthropic.claude-code-2.1.291-win32-x64")).unwrap();
        std::fs::write(tmp.path.join(".obsolete"), b"{ torn").unwrap();
        assert!(has_extension(&tmp.path, CLAUDE_EXTENSION), "an unreadable list hides nothing");
    }

    #[test]
    fn codex_family_is_the_package_folder_name() {
        let tmp = TempDir::new("launcher-packages");
        assert_eq!(codex_family(&tmp.path.join("missing")), None);
        mkdirs(&tmp.path, &["Claude_pzs8sxrjxfjjc", "OpenAI.CodexBeta_2p2nqsd0c76g0", "OpenAI.Codex_", "OpenAI.Codex_a_b"]);
        std::fs::write(tmp.path.join("OpenAI.Codex_filenotfolder"), b"").unwrap();
        assert_eq!(codex_family(&tmp.path), None);
        mkdirs(&tmp.path, &["OpenAI.Codex_2p2nqsd0c76g0"]);
        assert_eq!(codex_family(&tmp.path).as_deref(), Some("OpenAI.Codex_2p2nqsd0c76g0"));
    }

    #[test]
    fn installed_targets_follow_the_spec_order() {
        let tmp = TempDir::new("launcher-targets");
        let (home, local) = machine(&tmp);
        assert_eq!(installed_targets(&home, &local, |_| true, true), ALL);
    }

    #[test]
    fn a_target_needs_its_protocol_and_the_api_needs_the_key() {
        let tmp = TempDir::new("launcher-targets-partial");
        let (home, local) = machine(&tmp);
        assert_eq!(
            installed_targets(&home, &local, |s| s != "claude", false),
            [ClaudeCursor, ClaudeVsCode, CodexDesktop, CodexCursor, CodexVsCode],
            "no claude protocol, no key"
        );
        assert_eq!(
            installed_targets(&home, &local, |s| s != "codex", true),
            [ClaudeDesktop, ClaudeCursor, ClaudeVsCode, CodexCursor, CodexVsCode, ClaudeApi],
            "a Codex package folder without its protocol"
        );
        assert_eq!(
            installed_targets(&home, &local, |s| s != "vscode", false),
            [ClaudeDesktop, ClaudeCursor, CodexDesktop, CodexCursor],
            "a profile folder left by an uninstalled VS Code"
        );
        let nobody = tmp.path.join("nobody");
        assert_eq!(installed_targets(&nobody, &nobody, |_| true, false), [ClaudeDesktop], "only the protocol");
    }
}
