//! The adapter trait: one small, pure description of a CLI (how to start a turn, how to read its
//! output, how to answer it), and the types adapters and the shared runner talk in. Nothing here
//! starts a process or touches the disk; `runner.rs` and `mod.rs` do that for every adapter alike.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use super::mcp::McpApply;
use super::mcp::McpAttach;

/// The CLIs Mayak can drive. Wire names are camelCase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentKind {
    ClaudeCode,
    Codex,
    GeminiCli,
    CursorAgent,
}

impl AgentKind {
    pub const ALL: [AgentKind; 4] = [AgentKind::ClaudeCode, AgentKind::Codex, AgentKind::GeminiCli, AgentKind::CursorAgent];

    pub fn adapter(self) -> &'static dyn Adapter {
        match self {
            AgentKind::ClaudeCode => &super::claude_code::ClaudeCode,
            AgentKind::Codex => &super::codex::Codex,
            AgentKind::GeminiCli => &super::gemini::GeminiCli,
            AgentKind::CursorAgent => &super::cursor::CursorAgent,
        }
    }
}

/// Whether an adapter was checked against the real CLI, and with which version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Verification {
    pub verified: bool,
    /// The CLI version it was checked with; None for an adapter written from documentation.
    pub with: Option<&'static str>,
}

impl Verification {
    pub const fn checked(with: &'static str) -> Self {
        Self { verified: true, with: Some(with) }
    }

    pub const fn from_docs() -> Self {
        Self { verified: false, with: None }
    }
}

/// A model the user can pick. An empty `id` is the CLI's own default (no flag is passed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelChoice {
    pub id: String,
    /// A product name, never translated; empty for the default (the page says "Default").
    pub label: String,
}

impl ModelChoice {
    pub fn new(id: &str, label: &str) -> Self {
        Self { id: id.into(), label: label.into() }
    }
}

/// Whether the CLI is signed in, as far as it can tell without a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Login {
    Ready,
    NotLoggedIn,
    /// The CLI has no command that says; a run that fails shows the CLI's own message.
    Unknown,
}

/// A short command whose output answers a question about the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub args: Vec<String>,
    pub timeout: Duration,
}

impl Probe {
    pub fn new(args: &[&str], timeout_s: u64) -> Self {
        Self { args: args.iter().map(|a| a.to_string()).collect(), timeout: Duration::from_secs(timeout_s) }
    }
}

/// What the page can show and Rust can act on: an error with a stable code (the page translates
/// it) and the English sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatError {
    pub code: &'static str,
    pub message: String,
}

impl ChatError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl std::fmt::Display for ChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Everything a turn is built from.
pub struct TurnSpec<'a> {
    pub cwd: &'a Path,
    pub prompt: &'a str,
    /// "" is the CLI's default.
    pub model: &'a str,
    /// One of the adapter's `modes()`.
    pub mode: &'a str,
    /// The CLI's own id of the conversation, once it has told it; the turn then resumes it.
    pub resume: Option<&'a str>,
    /// What the Mayak MCP server hands this CLI (mcp.rs); None: nothing.
    pub mcp: Option<&'a McpAttach>,
}

/// How to start the process of a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Written to the process's stdin right after it starts.
    pub stdin: Vec<u8>,
    /// Close stdin after `stdin` (one-shot CLIs read their prompt to its end); false keeps it open
    /// for the answers and the next turns of a persistent process.
    pub close_stdin: bool,
    /// What happened to what the MCP server handed over.
    pub mcp: McpApply,
}

/// A permission request from the CLI, as the page shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionAsk {
    /// The CLI's own id of the request, which its answer carries back.
    pub request_id: String,
    pub tool: String,
    pub input: Value,
    pub description: Option<String>,
    /// The tool call it is about, which the log already shows as a tool line.
    pub tool_use_id: Option<String>,
    pub blocked_path: Option<String>,
}

/// How a turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Stopped by an interrupt.
    Aborted,
    /// `login`: the CLI said it is not signed in.
    Failed { text: String, login: bool },
}

/// One of the plan's usage windows, as the CLI reports it (Claude Code's `rate_limit_event`:
/// `unifiedWindows.five_hour` / `seven_day`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanWindow {
    /// The CLI's own name of the window ("five_hour", "seven_day").
    pub kind: String,
    /// How much of it is used, 0 to 1.
    pub used: f64,
    /// When it resets (seconds since the Unix epoch), when the CLI says.
    pub resets_at: Option<u64>,
}

/// What a line of a CLI's output means, in words that fit every CLI.
#[derive(Debug, Clone, PartialEq)]
pub enum Ev {
    /// The CLI's id of the conversation (what a later turn resumes), the model it runs and where
    /// its credentials come from, when it says.
    Session { id: String, model: Option<String>, api_key_source: Option<String> },
    /// More of the answer being written.
    TextDelta(String),
    /// A whole block of the answer, from a CLI that does not stream it.
    Text(String),
    ToolStart { id: String, name: String, summary: String },
    ToolEnd { id: String, ok: bool, output: Option<String> },
    Permission(PermissionAsk),
    /// A line to show while the turn goes on (a reconnect), not part of the conversation.
    Status(String),
    /// A warning the CLI printed that the user may want to see.
    Notice(String),
    /// What the CLI said of the conversation's size: the tokens its last request sent (input and
    /// cache), and the model's context window. Either may come alone.
    Usage { context_tokens: Option<u64>, context_window: Option<u64> },
    /// The plan's usage windows, as the CLI last reported them.
    Limits(Vec<PlanWindow>),
    TurnEnd(Outcome),
}

/// Reads a CLI's stdout one line at a time.
pub trait Parser: Send {
    /// A line without its newline. A line that is not JSON, or JSON this adapter does not know,
    /// gives nothing.
    fn line(&mut self, line: &str) -> Vec<Ev>;
}

pub trait Adapter: Sync {
    fn label(&self) -> &'static str;
    /// The command looked up on PATH ("claude", "codex", "gemini", "cursor-agent").
    fn stem(&self) -> &'static str;
    fn verification(&self) -> Verification;
    /// The models the picker offers when the CLI cannot list them (or cannot be asked).
    fn models(&self) -> Vec<ModelChoice>;
    /// What the user may pick for how much the agent may do; the first is the default. Never a
    /// mode in which the CLI approves a tool call itself, and never one that leaves a sandbox.
    /// The one stated exception to "every request is a click": Codex's `exec` has no approval
    /// prompts at all, so its modes are the sandbox the run stays in (codex.rs).
    fn modes(&self) -> &'static [&'static str];
    /// One process keeps the conversation between turns (turns go to its stdin).
    fn persistent(&self) -> bool {
        false
    }
    fn build(&self, turn: &TurnSpec<'_>) -> Result<Invocation, ChatError>;
    fn parser(&self) -> Box<dyn Parser>;
    /// A persistent process: the line that sends the next prompt.
    fn user_line(&self, _prompt: &str) -> Option<String> {
        None
    }
    /// The line that answers `ask` on stdin; None when this CLI never asks.
    fn permission_reply(&self, _ask: &PermissionAsk, _allow: bool) -> Option<String> {
        None
    }
    /// The line that stops the running turn gracefully; None: the process is killed.
    fn interrupt_line(&self) -> Option<String> {
        None
    }
    fn login_probe(&self) -> Option<Probe> {
        None
    }
    /// `output` is stdout and stderr together.
    fn parse_login(&self, _exit: Option<i32>, _output: &str) -> Login {
        Login::Unknown
    }
    fn models_probe(&self) -> Option<Probe> {
        None
    }
    fn parse_models(&self, _output: &str) -> Option<Vec<ModelChoice>> {
        None
    }
}

// ── Helpers adapters share ─────────────────────────────────────────────────────

/// The failure says the account's plan limit is used up. Codex words it "You’ve hit your usage limit.
/// … try again at 1:23 AM." (real output, `fixtures/codex_usage_limit.jsonl`); Claude Code's
/// wording was never captured here, so only this phrase counts. The page shows it as the rate-limit
/// state; the text itself is the CLI's own and is shown as it is.
pub fn is_usage_limit(text: &str) -> bool {
    text.to_ascii_lowercase().contains("usage limit")
}

/// `text` cut to `max` characters, with an ellipsis when it was longer.
pub fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

/// The field of a tool's input that says what it does, in the order the island's cards use
/// (`APPROVAL_FIELDS` in island/hooks.ts), so a tool shows what it will touch, not just its name.
pub fn summarize(tool: &str, input: &Value) -> String {
    if tool == "AskUserQuestion" {
        if let Some(q) = input.pointer("/questions/0/question").and_then(Value::as_str) {
            return clip(q.trim(), 160);
        }
    }
    for field in ["command", "file_path", "path", "url", "query", "pattern", "prompt", "question"] {
        if let Some(text) = input.get(field).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) {
            return clip(text, 160);
        }
    }
    String::new()
}

/// A model id the page picked: it becomes one command-line argument, so it must not look like an
/// option or hold anything but the characters model ids use.
pub fn check_model(model: &str) -> Result<(), ChatError> {
    let ok = model.is_empty()
        || (model.len() <= 100
            && model.as_bytes()[0].is_ascii_alphanumeric()
            && model.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b':' | b'/' | b'[' | b']')));
    if ok {
        Ok(())
    } else {
        Err(ChatError::new("badModel", "That model name is not usable."))
    }
}

/// A conversation id the CLI gave: the same shape `resume.rs` accepts for a resume command.
pub fn check_resume(id: &str) -> Result<(), ChatError> {
    if crate::resume::valid_id(id) {
        Ok(())
    } else {
        Err(ChatError::new("badResume", "That conversation id is not usable."))
    }
}

pub fn bad_mode(mode: &str) -> ChatError {
    ChatError::new("badMode", format!("Unknown mode: {mode}"))
}

/// The text of a JSON value that holds either a string or an array of `{type: "text", text}` blocks.
pub fn text_of(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let parts: Vec<&str> = items.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn clip_cuts_on_characters_not_bytes() {
        assert_eq!(clip("héllo wörld", 5), "héllo…");
        assert_eq!(clip("short", 5), "short");
        assert_eq!(clip("", 3), "");
    }

    #[test]
    fn a_tool_shows_the_field_that_says_what_it_touches() {
        assert_eq!(summarize("Bash", &json!({ "command": " git status ", "description": "x" })), "git status");
        assert_eq!(summarize("Write", &json!({ "file_path": r"C:\a\b.txt", "content": "zzz" })), r"C:\a\b.txt");
        assert_eq!(summarize("WebSearch", &json!({ "query": "tauri" })), "tauri");
        assert_eq!(summarize("Other", &json!({ "n": 1 })), "");
        // The question an agent asks through the Mayak MCP server's ask_user.
        assert_eq!(summarize("mcp__mayak__ask_user", &json!({ "question": "Red or blue?", "options": ["Red", "Blue"] })), "Red or blue?");
        assert_eq!(
            summarize("AskUserQuestion", &json!({ "questions": [{ "question": "Which color?" }] })),
            "Which color?"
        );
        assert_eq!(summarize("Bash", &json!({ "command": "x".repeat(300) })).chars().count(), 161);
    }

    #[test]
    fn model_ids_cannot_pose_as_options() {
        for ok in ["", "haiku", "gpt-6.1-sol", "claude-opus-5-5", "claude-haiku-5-5[1m]", "org/model:v1"] {
            assert!(check_model(ok).is_ok(), "{ok}");
        }
        for bad in ["--help", "-m", " x", "a b", "a;b", "a\"b", "a\nb", &"m".repeat(101)] {
            assert!(check_model(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn text_comes_from_a_string_or_text_blocks() {
        assert_eq!(text_of(&json!("plain")).as_deref(), Some("plain"));
        assert_eq!(text_of(&json!([{ "type": "text", "text": "a" }, { "type": "image" }, { "type": "text", "text": "b" }])).as_deref(), Some("a\nb"));
        assert_eq!(text_of(&json!([{ "type": "image" }])), None);
        assert_eq!(text_of(&json!(null)), None);
    }
}
