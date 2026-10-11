// The session list's row menu (idea #8): which items a row offers and the text each copy item puts
// on the clipboard. Pure: the commands in lib.rs look the row up in the snapshot, check the folder
// on disk and pass the facts in. Nothing here is logged.

use serde::{Deserialize, Serialize};

use crate::observe::model::Source;

/// One item of the menu, in the menu's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MenuItem {
    /// The agent's last reply, whole.
    CopyReply,
    /// The project folder's path.
    CopyPath,
    /// `claude --resume <id>` / `codex resume <id>`.
    CopyResume,
    /// The project folder in the file manager.
    OpenFolder,
}

/// What one row has to offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowFacts<'a> {
    pub agent: Source,
    /// Claude sessionId / Codex session id; empty when unknown.
    pub logical_id: &'a str,
    /// The agent's last reply (`text::reply`), when one was read.
    pub reply: Option<&'a str>,
    /// The project folder as an absolute path: the folder found on disk, or else the project root.
    pub folder: Option<&'a str>,
    /// `folder` is a folder on this disk.
    pub folder_exists: bool,
}

/// The items the row offers, in the menu's order; an item without its data is left out.
pub fn items(f: &RowFacts) -> Vec<MenuItem> {
    let mut out = Vec::new();
    if f.reply.is_some_and(|r| !r.is_empty()) {
        out.push(MenuItem::CopyReply);
    }
    if f.folder.is_some() {
        out.push(MenuItem::CopyPath);
    }
    if resume_command(f.agent, f.logical_id).is_some() {
        out.push(MenuItem::CopyResume);
    }
    if f.folder.is_some() && f.folder_exists {
        out.push(MenuItem::OpenFolder);
    }
    out
}

/// The text a copy item puts on the clipboard; None for OpenFolder and for an item the row does not
/// offer.
pub fn copy_text(item: MenuItem, f: &RowFacts) -> Option<String> {
    match item {
        MenuItem::CopyReply => f.reply.filter(|r| !r.is_empty()).map(str::to_owned),
        MenuItem::CopyPath => f.folder.map(str::to_owned),
        MenuItem::CopyResume => resume_command(f.agent, f.logical_id),
        MenuItem::OpenFolder => None,
    }
}

/// The command that resumes the session in its CLI, run from the project folder: Claude Code
/// `claude --resume <sessionId>`, Codex `codex resume <session id>`. None for Antigravity (no
/// resume command) and for an id that is not a plain token (it goes into a shell line as it is).
pub fn resume_command(agent: Source, logical_id: &str) -> Option<String> {
    if !is_plain_id(logical_id) {
        return None;
    }
    match agent {
        Source::Claude => Some(format!("claude --resume {logical_id}")),
        Source::Codex => Some(format!("codex resume {logical_id}")),
        Source::Antigravity => None,
    }
}

/// 1 to 128 ASCII letters, digits, '-' or '_' (session ids are UUIDs): safe in any shell as it is.
fn is_plain_id(id: &str) -> bool {
    (1..=128).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "0f8b1d2e-0000-4000-8000-000000000001";

    fn facts<'a>(agent: Source, reply: Option<&'a str>, folder: Option<&'a str>, exists: bool) -> RowFacts<'a> {
        RowFacts { agent, logical_id: UUID, reply, folder, folder_exists: exists }
    }

    #[test]
    fn a_full_row_offers_the_four_items_in_order() {
        let f = facts(Source::Claude, Some("Done."), Some(r"C:\work\app"), true);
        assert_eq!(items(&f), [MenuItem::CopyReply, MenuItem::CopyPath, MenuItem::CopyResume, MenuItem::OpenFolder]);
    }

    #[test]
    fn items_without_their_data_are_left_out() {
        // No reply read yet, folder gone from the disk: the path is still a fact, opening it is not.
        let f = facts(Source::Codex, None, Some(r"C:\gone"), false);
        assert_eq!(items(&f), [MenuItem::CopyPath, MenuItem::CopyResume]);
        // No project folder (Antigravity without a workspace), no resume command.
        let f = facts(Source::Antigravity, Some("ok"), None, false);
        assert_eq!(items(&f), [MenuItem::CopyReply]);
        // An empty reply is no reply; an unknown id gives no command.
        let f = RowFacts { logical_id: "", ..facts(Source::Claude, Some(""), None, false) };
        assert!(items(&f).is_empty());
    }

    #[test]
    fn copy_texts_are_the_reply_the_path_and_the_command() {
        let f = facts(Source::Claude, Some("Line 1\n\nLine 2"), Some(r"C:\work\my app"), true);
        assert_eq!(copy_text(MenuItem::CopyReply, &f).as_deref(), Some("Line 1\n\nLine 2"));
        assert_eq!(copy_text(MenuItem::CopyPath, &f).as_deref(), Some(r"C:\work\my app"));
        assert_eq!(copy_text(MenuItem::CopyResume, &f), Some(format!("claude --resume {UUID}")));
        assert_eq!(copy_text(MenuItem::OpenFolder, &f), None);
        let none = facts(Source::Codex, None, None, false);
        assert_eq!(copy_text(MenuItem::CopyReply, &none), None);
        assert_eq!(copy_text(MenuItem::CopyPath, &none), None);
    }

    #[test]
    fn resume_commands_per_agent() {
        assert_eq!(resume_command(Source::Claude, UUID), Some(format!("claude --resume {UUID}")));
        assert_eq!(resume_command(Source::Codex, "019a8c4e-0000-7000-8000-00000000c0de").as_deref(),
            Some("codex resume 019a8c4e-0000-7000-8000-00000000c0de"));
        assert_eq!(resume_command(Source::Antigravity, UUID), None);
    }

    #[test]
    fn an_id_that_is_not_a_plain_token_gives_no_command() {
        for bad in ["", "a b", "x;rm -rf /", "id&calc", "$(whoami)", "ид", "\"q\"", &"a".repeat(129)] {
            assert_eq!(resume_command(Source::Claude, bad), None, "{bad:?}");
        }
        assert!(resume_command(Source::Codex, &"a".repeat(128)).is_some());
        assert!(resume_command(Source::Codex, "abc_DEF-123").is_some());
    }

    #[test]
    fn items_travel_as_camel_case_names() {
        assert_eq!(serde_json::to_value(MenuItem::CopyReply).unwrap(), "copyReply");
        assert_eq!(serde_json::from_value::<MenuItem>("openFolder".into()).unwrap(), MenuItem::OpenFolder);
        assert!(serde_json::from_value::<MenuItem>("deleteAll".into()).is_err());
    }
}
