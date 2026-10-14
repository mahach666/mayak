//! The conversations kept across restarts: one JSON file in the app's folder (`chats.json`) with
//! what shows a conversation again and what resumes it through the CLI's own resume: the agent, the
//! folder, the model and mode, the title, the CLI's session id, and the visible transcript, cut to
//! the caps below. No key or token is ever part of a conversation (the MCP token goes to the CLI in
//! its environment, never into the chat). Pure: building, reading and writing the file; the manager
//! decides when.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::agent::{clip, AgentKind};
use super::convo::{Body, Convo};

/// The file in the app's folder.
pub const FILE_NAME: &str = "chats.json";
/// The format; a file of another version is not read (the next save replaces it).
const VERSION: u32 = 1;
/// Conversations kept, the most recently opened first; older ones are dropped.
pub const MAX_CONVOS: usize = 20;
/// Messages kept per conversation, the last ones.
pub const MAX_MESSAGES: usize = 200;
/// Characters kept of one message's text.
pub const MAX_TEXT_CHARS: usize = 20_000;
/// The whole file at most; past it the oldest conversations go.
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub agent: AgentKind,
    pub cwd: String,
    pub model: String,
    pub mode: String,
    pub title: String,
    /// The CLI's own id of the conversation: what the next turn resumes.
    pub session: Option<String>,
    pub used_model: Option<String>,
    pub context_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub messages: Vec<Body>,
}

#[derive(Debug, Serialize, Deserialize)]
struct File {
    version: u32,
    convos: Vec<Saved>,
}

/// What is kept of a conversation; None for one where nothing was said (it has nothing to resume).
pub fn of(c: &Convo) -> Option<Saved> {
    if c.msgs.is_empty() {
        return None;
    }
    let from = c.msgs.len().saturating_sub(MAX_MESSAGES);
    let messages = c.msgs[from..].iter().map(|m| clipped(&m.body)).collect();
    Some(Saved {
        agent: c.agent,
        cwd: c.cwd.clone(),
        model: c.model.clone(),
        mode: c.mode.clone(),
        title: c.title.clone(),
        session: c.session.clone(),
        used_model: c.used_model.clone(),
        context_tokens: c.context_tokens,
        context_window: c.context_window,
        messages,
    })
}

fn clipped(b: &Body) -> Body {
    let cut = |s: &str| clip(s, MAX_TEXT_CHARS);
    match b {
        Body::User { text } => Body::User { text: cut(text) },
        Body::Assistant { text } => Body::Assistant { text: cut(text) },
        Body::Tool { tool_id, name, summary, status, output } => Body::Tool {
            tool_id: tool_id.clone(),
            name: name.clone(),
            summary: cut(summary),
            status: *status,
            output: output.as_deref().map(cut),
        },
        Body::Permission { request_id, tool, summary, detail, tool_use_id, status } => Body::Permission {
            request_id: request_id.clone(),
            tool: tool.clone(),
            summary: cut(summary),
            detail: detail.as_deref().map(cut),
            tool_use_id: tool_use_id.clone(),
            status: *status,
        },
        Body::Notice { code, text } => Body::Notice { code: code.clone(), text: cut(text) },
        Body::Error { code, text } => Body::Error { code: code.clone(), text: cut(text) },
    }
}

/// The file's text for `convos` (newest first): at most MAX_CONVOS, and the oldest dropped until it
/// fits MAX_FILE_BYTES.
pub fn encode(mut convos: Vec<Saved>) -> String {
    convos.truncate(MAX_CONVOS);
    loop {
        let text = serde_json::to_string(&File { version: VERSION, convos: convos.clone() }).unwrap_or_default();
        if text.len() <= MAX_FILE_BYTES || convos.is_empty() {
            return text;
        }
        convos.pop();
    }
}

/// The conversations of a file's text, newest first. An empty file reads as none; anything that is
/// not this format is an error (the caller says so in the log; the next save replaces the file).
pub fn decode(text: &str) -> Result<Vec<Saved>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let file: File = serde_json::from_str(text).map_err(|e| format!("not a chats file: {e}"))?;
    if file.version != VERSION {
        return Err(format!("chats file version {} (this build reads {VERSION})", file.version));
    }
    Ok(file.convos)
}

/// Reads the file; a missing file is no conversation.
pub fn read(path: &Path) -> Result<Vec<Saved>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => decode(&text),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(format!("cannot read {}: {err}", path.display())),
    }
}

/// Writes the file whole: next to it first, then in its place, so a crash never leaves half a file.
pub fn write(path: &Path, convos: Vec<Saved>) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, encode(convos))?;
    std::fs::rename(&tmp, path)
}

/// Deletes the file; a missing one is already cleared.
pub fn remove(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::agent::{Ev, Outcome, PermissionAsk};
    use crate::chat::convo::{PermissionStatus, State, ToolStatus};
    use crate::testutil::TempDir;
    use serde_json::json;

    /// A conversation with a finished turn, then one cut in the middle: a tool running, a request waiting.
    fn talked() -> Convo {
        let mut c = Convo::new(3, AgentKind::ClaudeCode, "C:/work/app".into(), "haiku".into(), "ask".into(), None);
        c.user("Why does the build fail?");
        c.apply(&Ev::Session { id: "sess-1".into(), model: Some("claude-haiku-5-5".into()), api_key_source: None });
        c.apply(&Ev::Usage { context_tokens: Some(26_155), context_window: Some(1_000_000) });
        c.apply(&Ev::Text("Node 20 is pinned.".into()));
        c.apply(&Ev::TurnEnd(Outcome::Done));
        c.user("Fix it");
        c.apply(&Ev::ToolStart { id: "t1".into(), name: "Bash".into(), summary: "npm ci".into() });
        c.apply(&Ev::Permission(PermissionAsk {
            request_id: "cli-1".into(),
            tool: "Bash".into(),
            input: json!({ "command": "npm ci" }),
            description: None,
            tool_use_id: Some("t1".into()),
            blocked_path: None,
        }));
        c
    }

    #[test]
    fn a_conversation_comes_back_as_it_was_shown_and_nothing_runs_in_it() {
        let saved = of(&talked()).expect("something was said");
        assert_eq!((saved.session.as_deref(), saved.title.as_str()), (Some("sess-1"), "Why does the build fail?"));
        let back = Convo::restored(11, &saved);
        assert_eq!((back.id, back.state, back.agent), (11, State::Idle, AgentKind::ClaudeCode));
        assert_eq!((back.session.as_deref(), back.model.as_str(), back.mode.as_str()), (Some("sess-1"), "haiku", "ask"));
        assert_eq!((back.context_tokens, back.context_window), (Some(26_155), Some(1_000_000)));
        assert_eq!(back.msgs.len(), 5);
        assert!(matches!(&back.msgs[3].body, Body::Tool { status: ToolStatus::Failed, .. }), "the tool that ran when the app stopped");
        let Body::Permission { status, request_id, .. } = &back.msgs[4].body else { panic!() };
        assert_eq!(*status, PermissionStatus::Cancelled, "a request nobody answered is not answerable any more");
        assert_eq!(request_id, "11.r4", "an id no new request can take");
        assert!(!back.has_pending_permission());
        assert!(back.turn_started.is_none() && back.turn_ended.is_none());
    }

    #[test]
    fn an_empty_conversation_is_not_kept() {
        let c = Convo::new(1, AgentKind::Codex, "C:/x".into(), String::new(), "readOnly".into(), None);
        assert!(of(&c).is_none());
    }

    #[test]
    fn the_caps_hold() {
        let mut c = Convo::new(1, AgentKind::ClaudeCode, "C:/x".into(), String::new(), "ask".into(), None);
        for i in 0..MAX_MESSAGES + 30 {
            c.user(&format!("message {i}"));
            c.apply(&Ev::TurnEnd(Outcome::Done));
        }
        c.user(&"x".repeat(MAX_TEXT_CHARS + 500));
        let saved = of(&c).unwrap();
        assert_eq!(saved.messages.len(), MAX_MESSAGES, "the last ones");
        let Body::User { text } = saved.messages.last().unwrap() else { panic!() };
        assert_eq!(text.chars().count(), MAX_TEXT_CHARS + 1, "cut, with the ellipsis");

        let many: Vec<Saved> = (0..MAX_CONVOS + 5).map(|_| saved.clone()).collect();
        let kept = decode(&encode(many)).unwrap();
        assert_eq!(kept.len(), MAX_CONVOS, "the newest MAX_CONVOS");

        // Past the size cap the oldest (last) ones go.
        let big = Saved { messages: vec![Body::User { text: "y".repeat(MAX_FILE_BYTES / 3) }], ..saved.clone() };
        let kept = decode(&encode(vec![saved.clone(), big.clone(), big.clone(), big])).unwrap();
        assert!(encode(kept.clone()).len() <= MAX_FILE_BYTES);
        assert_eq!(kept[0], saved, "the newest is kept");
        assert!(kept.len() < 4);
    }

    #[test]
    fn a_file_is_read_back_and_a_bad_one_is_an_error() {
        let dir = TempDir::new("chat-saved");
        let path = dir.path.join(FILE_NAME);
        assert_eq!(read(&path).unwrap(), Vec::<Saved>::new(), "no file: no conversation");
        let saved = of(&talked()).unwrap();
        write(&path, vec![saved.clone()]).unwrap();
        assert_eq!(read(&path).unwrap(), vec![saved]);
        assert!(!path.with_extension("json.tmp").exists(), "the temporary file is renamed");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(read(&path).is_err());
        std::fs::write(&path, r#"{"version":99,"convos":[]}"#).unwrap();
        assert!(read(&path).unwrap_err().contains("version 99"));
        std::fs::write(&path, "").unwrap();
        assert_eq!(read(&path).unwrap(), Vec::<Saved>::new());
        remove(&path).unwrap();
        assert!(!path.exists());
        remove(&path).unwrap();
    }

    #[test]
    fn nothing_but_the_conversation_is_written() {
        // The page's own fields (rev, state, status) and the manager's are not part of the file.
        let text = encode(vec![of(&talked()).unwrap()]);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut keys: Vec<&str> = v["convos"][0].as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["agent", "contextTokens", "contextWindow", "cwd", "messages", "mode", "model", "session", "title", "usedModel"]);
        assert_eq!(v["version"], 1);
    }
}
