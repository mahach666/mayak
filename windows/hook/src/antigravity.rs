//! Google Antigravity's hooks (`~/.gemini/config/hooks.json`, written by Mayak's
//! Settings → Agents → Antigravity): the IDE agent and the `agy` CLI run
//! `mayak-hook --agent antigravity <Event>` and send camelCase JSON on stdin
//! (antigravity.google/docs/hooks): `conversationId`, `workspacePaths`,
//! `toolCall {name, args}`, `error`, `terminationReason`, `fullyIdle`…
//!
//! Mayak only observes Antigravity. It installs no `PreToolUse` hook, so it
//! never takes part in Antigravity's permission decisions, and every event it
//! does install gets `{}` on stdout — "nothing to add" — whether Mayak runs or
//! not, at once.

use serde_json::{Map, Value};

use crate::RELAY_VERSION;

/// The events Mayak installs, in the order of hooks.json. The app's installer
/// (`antigravity_hooks.rs`) writes the same list.
pub const EVENTS: &[&str] = &["PreInvocation", "PostToolUse", "Stop"];

/// What Antigravity reads back from one of `EVENTS`: an empty object adds
/// nothing (no injected step, no continue on `Stop`). None for any other
/// event: Mayak never installed it and stays silent.
pub fn stdout(event: &str) -> Option<&'static str> {
    EVENTS.contains(&event).then_some("{}")
}

/// Keys of `toolCall.args` the observer reads under its own name (`text::TARGET_FIELDS`).
/// Only these go to Mayak, not the rest of the arguments (file contents, diffs).
const ARG_ALIASES: &[(&str, &str)] = &[
    ("CommandLine", "command"),
    ("FilePath", "file_path"),
    ("Path", "path"),
    ("Url", "url"),
    ("Query", "query"),
    ("Pattern", "pattern"),
];

fn non_empty_str<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty())
}

/// The line Mayak gets for an Antigravity event: snake_case fields the
/// observer reads, nothing else of the payload (no transcript path, no model
/// name, no other tool arguments). None — nothing is sent — for an event Mayak
/// does not install or a payload without a `conversationId`.
pub fn payload(
    raw: &Map<String, Value>,
    event: &str,
    relay_ts: i64,
    cwd_fallback: Option<&str>,
) -> Option<Map<String, Value>> {
    if !EVENTS.contains(&event) {
        return None;
    }
    let session_id = non_empty_str(raw, "conversationId")?;
    let mut out = Map::new();
    out.insert("agent".into(), Value::from("antigravity"));
    out.insert("hook_event_name".into(), Value::from(event));
    out.insert("relay".into(), Value::from(RELAY_VERSION));
    out.insert("relay_ts".into(), Value::from(relay_ts));
    out.insert("session_id".into(), Value::from(session_id));
    let workspace = raw
        .get("workspacePaths")
        .and_then(Value::as_array)
        .and_then(|paths| paths.iter().filter_map(Value::as_str).find(|p| !p.trim().is_empty()));
    if let Some(cwd) = workspace.or(cwd_fallback) {
        out.insert("cwd".into(), Value::from(cwd));
    }
    let tool = raw.get("toolCall").and_then(Value::as_object);
    if let Some(name) = tool.and_then(|t| non_empty_str(t, "name")) {
        out.insert("tool_name".into(), Value::from(name));
    }
    if let Some(args) = tool.and_then(|t| t.get("args")).and_then(Value::as_object) {
        let mut input = Map::new();
        for (from, to) in ARG_ALIASES {
            let value = args.get(*from).or_else(|| args.get(*to));
            if let Some(Value::String(s)) = value {
                input.insert((*to).to_string(), Value::String(s.clone()));
            }
        }
        if !input.is_empty() {
            out.insert("tool_input".into(), Value::Object(input));
        }
    }
    if let Some(error) = non_empty_str(raw, "error") {
        out.insert("error".into(), Value::from(error));
    }
    if let Some(reason) = non_empty_str(raw, "terminationReason") {
        out.insert("termination_reason".into(), Value::from(reason));
    }
    if let Some(idle) = raw.get("fullyIdle").and_then(Value::as_bool) {
        out.insert("fully_idle".into(), Value::from(idle));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Shaped after the fields antigravity.google/docs/hooks lists for each event;
    // the values are made up.
    const PRE_INVOCATION: &str = include_str!("../fixtures/antigravity/pre-invocation.json");
    const POST_TOOL_USE: &str = include_str!("../fixtures/antigravity/post-tool-use.json");
    const STOP: &str = include_str!("../fixtures/antigravity/stop.json");
    const STOP_ERROR: &str = include_str!("../fixtures/antigravity/stop-error.json");

    fn fixture(text: &str) -> Map<String, Value> {
        serde_json::from_str::<Value>(text).unwrap().as_object().cloned().unwrap()
    }

    fn line(text: &str, event: &str) -> Value {
        Value::Object(payload(&fixture(text), event, 1_700, Some(r"C:\fallback")).expect("relayed"))
    }

    #[test]
    fn pre_invocation_names_the_conversation_and_its_workspace() {
        assert_eq!(
            line(PRE_INVOCATION, "PreInvocation"),
            json!({
                "agent": "antigravity", "hook_event_name": "PreInvocation", "relay": RELAY_VERSION,
                "relay_ts": 1_700, "session_id": "conv-7f3a", "cwd": r"C:\Projects\demo",
            })
        );
    }

    #[test]
    fn a_tool_call_keeps_its_name_and_only_the_arguments_the_observer_reads() {
        let got = line(POST_TOOL_USE, "PostToolUse");
        assert_eq!(got["tool_name"], "run_command");
        assert_eq!(got["tool_input"], json!({ "command": "cargo test" }));
        // No transcript path, model name, step index or other argument goes to Mayak.
        let text = got.to_string();
        for absent in ["transcript", "modelName", "stepIdx", "Cwd", "artifact", "WaitMsBeforeAsync"] {
            assert!(!text.contains(absent), "{absent} in {text}");
        }
        assert!(got.get("error").is_none(), "an empty error is no error");
    }

    #[test]
    fn stop_carries_its_reason_error_and_idle_flag() {
        let got = line(STOP, "Stop");
        assert_eq!(got["termination_reason"], "TERMINATION_REASON_MODEL_STOP");
        assert_eq!(got["fully_idle"], true);
        assert!(got.get("error").is_none());
        let failed = line(STOP_ERROR, "Stop");
        assert_eq!(failed["error"], "quota exceeded");
        assert_eq!(failed["fully_idle"], false);
    }

    #[test]
    fn without_a_workspace_the_relay_folder_is_the_cwd() {
        let mut raw = fixture(PRE_INVOCATION);
        raw.insert("workspacePaths".into(), json!([]));
        assert_eq!(payload(&raw, "PreInvocation", 1, Some(r"C:\here")).unwrap()["cwd"], r"C:\here");
        assert!(payload(&raw, "PreInvocation", 1, None).unwrap().get("cwd").is_none());
    }

    #[test]
    fn other_events_and_payloads_without_a_conversation_send_nothing() {
        let raw = fixture(POST_TOOL_USE);
        for event in ["PreToolUse", "PostInvocation", "", "Something"] {
            assert!(payload(&raw, event, 1, None).is_none(), "{event}");
        }
        for missing in [json!({}), json!({ "conversationId": "" }), json!({ "conversationId": 4 })] {
            assert!(payload(missing.as_object().unwrap(), "Stop", 1, None).is_none(), "{missing}");
        }
    }

    #[test]
    fn installed_events_get_an_empty_object_and_nothing_else_gets_a_word() {
        for event in EVENTS {
            assert_eq!(stdout(event), Some("{}"), "{event}");
        }
        // PreToolUse needs a decision Mayak never gives: it is not installed and stays silent.
        for event in ["PreToolUse", "PostInvocation", "", "stop"] {
            assert_eq!(stdout(event), None, "{event}");
        }
    }
}
