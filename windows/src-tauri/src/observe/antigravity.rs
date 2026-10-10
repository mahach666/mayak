// Google Antigravity sessions (IDE agent and `agy` CLI), fed only by Mayak's hooks in
// ~/.gemini/config/hooks.json (antigravity_hooks.rs). The relay sends one line per
// PreInvocation, PostToolUse and Stop (hook/src/antigravity.rs); codex-io parses it here
// and the store applies it (model.rs `antigravity_hook`). There is no file to tail and no
// process to prove: a session is its `conversationId`.

use serde::Deserialize;
use serde_json::Value;

/// One relay line of Antigravity, as `mayak-hook --agent antigravity` writes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AntigravityEvent {
    pub hook_event_name: String,
    /// Antigravity's `conversationId`.
    pub session_id: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<Value>,
    /// PostToolUse: the tool failed; Stop: the run ended on an error.
    #[serde(default)]
    pub error: Option<String>,
    /// Relay clock, ms since the Unix epoch (UTC).
    pub relay_ts: i64,
}

/// The relay line as an event. The error names the problem and never repeats a payload value,
/// so it can be logged.
pub fn parse(payload: Value) -> Result<AntigravityEvent, String> {
    if !payload.is_object() {
        return Err("not an object".to_string());
    }
    let event: AntigravityEvent = serde_json::from_value(payload).map_err(|e| {
        let message = e.to_string();
        message.split(':').next().unwrap_or_default().trim().to_string()
    })?;
    if event.session_id.trim().is_empty() {
        return Err("empty session_id".to_string());
    }
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_relay_line_becomes_an_event() {
        let ev = parse(json!({
            "agent": "antigravity", "hook_event_name": "PostToolUse", "relay": 2, "relay_ts": 9,
            "session_id": "conv-1", "cwd": r"C:\p", "tool_name": "run_command",
            "tool_input": { "command": "cargo test" }, "error": "exit 1"
        }))
        .unwrap();
        assert_eq!(ev.hook_event_name, "PostToolUse");
        assert_eq!(ev.session_id, "conv-1");
        assert_eq!(ev.cwd, r"C:\p");
        assert_eq!(ev.tool_name.as_deref(), Some("run_command"));
        assert_eq!(ev.tool_input, Some(json!({ "command": "cargo test" })));
        assert_eq!(ev.error.as_deref(), Some("exit 1"));
        assert_eq!(ev.relay_ts, 9);
        let bare = parse(json!({ "hook_event_name": "Stop", "session_id": "c", "relay_ts": 1 })).unwrap();
        assert_eq!((bare.cwd.as_str(), bare.tool_name, bare.error), ("", None, None));
    }

    #[test]
    fn broken_lines_name_the_problem_without_their_values() {
        assert_eq!(parse(json!([1])).unwrap_err(), "not an object");
        assert_eq!(parse(json!({ "hook_event_name": "Stop", "session_id": " ", "relay_ts": 1 })).unwrap_err(), "empty session_id");
        let missing = parse(json!({ "hook_event_name": "Stop", "relay_ts": 1, "cwd": "secret-path" })).unwrap_err();
        assert!(missing.contains("session_id") && !missing.contains("secret"), "{missing}");
    }
}
