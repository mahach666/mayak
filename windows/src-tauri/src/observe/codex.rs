// Codex source (spec §5.6): hook events forwarded by the relay, rollout records (legacy and
// paginated mode), subagent meta, host classification and session_index titles. These
// functions are pure: payloads, lines, paths and %LOCALAPPDATA% come in as arguments.

use std::path::Path;

use serde::Deserialize;
use serde_json::{Map, Value};

use super::keys::HostKind;
use super::paths::{glob_match, normalize};
use super::text::{collapse, tool_label, SUMMARY_MAX};

/// A Codex hook payload as the relay forwards it (spec §5.4 `HookEvent`, §5.2). Keys are
/// snake_case as Codex and the relay write them; unknown keys (`model`, `permission_mode`,
/// `agent`, `relay`, …) are ignored; a missing or null optional field is None.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HookEvent {
    pub hook_event_name: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub cwd: String,
    pub transcript_path: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<Value>,
    pub last_assistant_message: Option<String>,
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
    /// Relay clock, ms since the Unix epoch (UTC).
    pub relay_ts: i64,
    pub agent_pid: Option<u32>,
    /// Creation FILETIME of `agent_pid`.
    pub agent_created: Option<u64>,
    pub agent_image: Option<String>,
}

/// The relay payload as a HookEvent. The error names the problem ("not an object",
/// "missing field `cwd`", "invalid type", …) and never repeats a payload value, so codex-io
/// can log it (logs carry no payload content).
pub fn parse_hook(payload: Value) -> Result<HookEvent, String> {
    if !payload.is_object() {
        return Err("not an object".to_string());
    }
    serde_json::from_value(payload).map_err(|e| {
        let message = e.to_string();
        message.split(':').next().unwrap_or_default().trim().to_string()
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RolloutRecord {
    Meta { id: String, cwd: Option<String>, parent_thread_id: Option<String>, agent_role: Option<String> },
    TaskStarted { turn_id: Option<String> },
    TaskComplete { turn_id: Option<String>, last_agent_message: Option<String>, error: Option<String> },
    TurnAborted { turn_id: Option<String> },
    Call { call_id: String, label: String },
    /// A `Call` that asks the user for permissions: a function_call or custom_tool_call named
    /// `request_permissions`, or the `exec` custom tool whose string `input` mentions it. Codex runs no
    /// PermissionRequest hook for that dialog (spec §7, §15).
    PermissionAsk { call_id: String, label: String },
    /// The output of a call (`function_call_output` / `custom_tool_call_output`), paired by `call_id`.
    CallDone { call_id: String },
    /// A record of a known shape (legacy or paginated) that is not interesting.
    Other,
    /// A record of a shape this parser does not know → source health Degraded.
    Unknown,
}

/// One rollout line (spec §5.6), legacy mode or, after spike S3, paginated mode (D70). Top level
/// `{timestamp, type, payload}`; a paginated line also has `ordinal` (u64 or null, else Unknown):
/// - `session_meta` with a string `payload.id` → Meta (cwd, parent_thread_id, agent_role);
///   parent_thread_id/agent_role from the top-level payload keys, else from
///   `payload.source.subagent.thread_spawn` (Codex `SubAgentSource::ThreadSpawn`);
/// - `event_msg` + `payload.type` task_started | task_complete | turn_aborted → the variants;
///   `task_complete.error`: string → itself; object → its "message" string, else compact JSON;
///   null or absent → None; any other JSON → compact JSON;
/// - `response_item` + `payload.type` function_call (label `tool_label(name, arguments parsed
///   as JSON)`), custom_tool_call (label `tool_label(name, None)`, i.e. the name), local_shell_call
///   (label `tool_label("shell", payload.action)`) → Call; `call_id` "" when absent. A function_call or
///   custom_tool_call named `request_permissions`, or a custom_tool_call named `exec` whose `input` is a
///   string containing that word, is a PermissionAsk instead, with the same `call_id` and label; only that
///   boolean is computed, the text is neither kept nor logged. The `arguments` of a function_call never
///   count, nor does the `input` of any other custom tool: a command that mentions the word stays a Call;
/// - `response_item` + `payload.type` function_call_output | custom_tool_call_output with a string
///   `call_id` → CallDone; with any other `call_id` → Other;
/// - other `payload.type` values of event_msg/response_item, and top-level turn_context |
///   world_state | compacted | token_usage_record | inter_agent_communication_metadata
///   (the last two from Codex 0.160, spike S3) → Other;
/// - anything else → Unknown: unknown or missing top-level type, non-object, event_msg or
///   response_item without a string `payload.type`, session_meta without `payload.id`,
///   function_call or custom_tool_call without `name`.
pub fn parse_rollout_line(v: &Value) -> RolloutRecord {
    // Paginated mode (spike S3, D70): Codex reads `ordinal` as an unsigned integer or null.
    if v.get("ordinal").is_some_and(|ordinal| !(ordinal.is_null() || ordinal.is_u64())) {
        return RolloutRecord::Unknown;
    }
    let Some(kind) = v.get("type").and_then(Value::as_str) else {
        return RolloutRecord::Unknown;
    };
    let payload = v.get("payload").and_then(Value::as_object);
    let payload_type = payload.and_then(|p| p.get("type")).and_then(Value::as_str);
    match (kind, payload, payload_type) {
        ("session_meta", Some(p), _) => match string(p, "id") {
            Some(id) => RolloutRecord::Meta {
                id,
                cwd: string(p, "cwd"),
                parent_thread_id: string(p, "parent_thread_id").or_else(|| thread_spawn(v, "parent_thread_id")),
                agent_role: string(p, "agent_role").or_else(|| thread_spawn(v, "agent_role")),
            },
            None => RolloutRecord::Unknown,
        },
        ("event_msg", Some(p), Some("task_started")) => RolloutRecord::TaskStarted { turn_id: string(p, "turn_id") },
        ("event_msg", Some(p), Some("task_complete")) => RolloutRecord::TaskComplete {
            turn_id: string(p, "turn_id"),
            last_agent_message: string(p, "last_agent_message"),
            error: error_text(p.get("error")),
        },
        ("event_msg", Some(p), Some("turn_aborted")) => RolloutRecord::TurnAborted { turn_id: string(p, "turn_id") },
        ("response_item", Some(p), Some("function_call")) => match string(p, "name") {
            Some(name) => {
                let label = tool_label(&name, call_arguments(p).as_ref());
                call_record(p, name == REQUEST_PERMISSIONS, label)
            }
            None => RolloutRecord::Unknown,
        },
        ("response_item", Some(p), Some("custom_tool_call")) => match string(p, "name") {
            Some(name) => {
                let label = tool_label(&name, None);
                let input = p.get("input").and_then(Value::as_str);
                let asks = name == REQUEST_PERMISSIONS || (name == EXEC && input.is_some_and(|text| text.contains(REQUEST_PERMISSIONS)));
                call_record(p, asks, label)
            }
            None => RolloutRecord::Unknown,
        },
        ("response_item", Some(p), Some("local_shell_call")) => {
            RolloutRecord::Call { call_id: call_id(p), label: tool_label("shell", p.get("action")) }
        }
        ("response_item", Some(p), Some("function_call_output" | "custom_tool_call_output")) => match string(p, "call_id") {
            Some(id) => RolloutRecord::CallDone { call_id: id },
            None => RolloutRecord::Other,
        },
        ("event_msg" | "response_item", Some(_), Some(_)) => RolloutRecord::Other,
        ("turn_context" | "world_state" | "compacted" | "token_usage_record" | "inter_agent_communication_metadata", _, _) => {
            RolloutRecord::Other
        }
        _ => RolloutRecord::Unknown,
    }
}

fn string(o: &Map<String, Value>, key: &str) -> Option<String> {
    o.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// A string field of `payload.source.subagent.thread_spawn`: where Codex serializes
/// `SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. })`.
fn thread_spawn(v: &Value, key: &str) -> Option<String> {
    v.pointer(&format!("/payload/source/subagent/thread_spawn/{key}")).and_then(Value::as_str).map(str::to_owned)
}

fn call_id(p: &Map<String, Value>) -> String {
    string(p, "call_id").unwrap_or_default()
}

/// The tool Codex asks the user for filesystem permissions with (its «Разрешения» dialog).
pub const REQUEST_PERMISSIONS: &str = "request_permissions";

/// The custom tool of Codex Desktop threads without a project (code mode): its `input` is JS text, and
/// `tools.request_permissions(..)` inside it opens the same dialog.
const EXEC: &str = "exec";

/// PermissionAsk when `asks`, else Call; the same `call_id` and label either way.
fn call_record(p: &Map<String, Value>, asks: bool, label: String) -> RolloutRecord {
    let id = call_id(p);
    if asks {
        RolloutRecord::PermissionAsk { call_id: id, label }
    } else {
        RolloutRecord::Call { call_id: id, label }
    }
}

/// `function_call.arguments` is a JSON document in a string; an object is taken as is.
fn call_arguments(p: &Map<String, Value>) -> Option<Value> {
    match p.get("arguments") {
        Some(Value::String(s)) => serde_json::from_str(s).ok(),
        Some(object @ Value::Object(_)) => Some(object.clone()),
        _ => None,
    }
}

fn error_text(v: Option<&Value>) -> Option<String> {
    let v = v?;
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => Some(o.get("message").and_then(Value::as_str).map_or_else(|| v.to_string(), str::to_owned)),
        other => Some(other.to_string()),
    }
}

const CURSOR_EXTENSION: &str = r"**\.cursor\extensions\openai.chatgpt-*\**";
const VSCODE_EXTENSION: &str = r"**\.vscode\extensions\openai.chatgpt-*\**";
const DESKTOP_MSIX: &str = r"**\windowsapps\openai.codex_*\**";

/// Host of a Codex process from the relay's `agent_image`, compared as
/// `paths::normalize(image).key` (spec §5.6). Desktop: the MSIX package folder or a copy under
/// `<localappdata>\OpenAI\Codex\bin\<ver>\codex.exe`. None or anything else → Unknown.
pub fn classify_host(agent_image: Option<&str>, localappdata: &Path) -> HostKind {
    let Some(image) = agent_image else {
        return HostKind::Unknown;
    };
    let key = normalize(image).key;
    let local_copy = !localappdata.as_os_str().is_empty()
        && glob_match(
            &key,
            &format!(r"{}\openai\codex\bin\*\codex.exe", normalize(&localappdata.to_string_lossy()).key),
        );
    if glob_match(&key, CURSOR_EXTENSION) {
        HostKind::Cursor
    } else if glob_match(&key, VSCODE_EXTENSION) {
        HostKind::VsCode
    } else if glob_match(&key, DESKTOP_MSIX) || local_copy {
        HostKind::CodexDesktop
    } else {
        HostKind::Unknown
    }
}

/// Title from `~/.codex/session_index.jsonl` text: the `thread_name` of the last line whose
/// `id` equals `session_id` and whose name is not blank, as `collapse(_, 160)`. Lines that are
/// not JSON are skipped.
pub fn index_title(index_text: &str, session_id: &str) -> Option<String> {
    index_text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| v.get("id").and_then(Value::as_str) == Some(session_id))
        .filter_map(|v| v.get("thread_name").and_then(Value::as_str).map(|name| collapse(name, SUMMARY_MAX)))
        .filter(|name| !name.is_empty())
        .last()
}

/// A rollout path codex-io may tail: not blank and not compressed (".zst", ASCII
/// case-insensitive).
pub fn is_readable_rollout(path: &str) -> bool {
    !path.trim().is_empty() && !path.to_ascii_lowercase().ends_with(".zst")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LEGACY: &str = include_str!("fixtures/codex/rollout_legacy.jsonl");
    const SUBAGENT_META: &str = include_str!("fixtures/codex/rollout_subagent_meta.jsonl");
    const UNKNOWN: &str = include_str!("fixtures/codex/rollout_unknown.jsonl");
    const HOOK_PERMISSION: &str = include_str!("fixtures/codex/hook_permission_request.json");
    const HOOK_STOP: &str = include_str!("fixtures/codex/hook_stop.json");
    const SESSION_INDEX: &str = include_str!("fixtures/codex/session_index.jsonl");

    const SESSION: &str = "22222222-2222-4222-8222-222222222222";
    const LOCALAPPDATA: &str = r"C:\Users\dev\AppData\Local";

    fn records(src: &str) -> Vec<RolloutRecord> {
        src.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| parse_rollout_line(&serde_json::from_str(line).expect("fixture lines are JSON")))
            .collect()
    }

    fn turn(n: u32) -> Option<String> {
        Some(format!("33333333-3333-4333-8333-33333333333{n}"))
    }

    fn error_of(r: RolloutRecord) -> Option<String> {
        match r {
            RolloutRecord::TaskComplete { error, .. } => error,
            other => panic!("not a task_complete: {other:?}"),
        }
    }

    #[test]
    fn parses_a_permission_request_hook() {
        let event = parse_hook(serde_json::from_str(HOOK_PERMISSION).unwrap()).unwrap();
        let expected = HookEvent {
            hook_event_name: "PermissionRequest".into(),
            session_id: SESSION.into(),
            turn_id: turn(1),
            cwd: r"C:\Users\dev\src\demo".into(),
            transcript_path: Some(
                r"C:\Users\dev\.codex\sessions\2026\10\01\rollout-2026-10-01T10-00-00-22222222-2222-4222-8222-222222222222.jsonl".into(),
            ),
            tool_name: Some("Bash".into()),
            tool_input: Some(json!({ "command": "cargo test --offline" })),
            last_assistant_message: None,
            agent_id: None,
            agent_type: None,
            relay_ts: 1_790_000_001_000,
            agent_pid: Some(5150),
            agent_created: Some(134_040_000_000_005_150),
            agent_image: Some(r"c:\Users\dev\.cursor\extensions\openai.chatgpt-0.146.0-win32-x64\bin\windows-x86_64\codex.exe".into()),
        };
        assert_eq!(event, expected);
    }

    #[test]
    fn parses_a_stop_hook_with_null_fields() {
        let event = parse_hook(serde_json::from_str(HOOK_STOP).unwrap()).unwrap();
        assert_eq!(event.hook_event_name, "Stop");
        assert_eq!(event.turn_id, turn(1));
        assert_eq!(event.last_assistant_message.as_deref(), Some("Tests pass.\nNothing else changed."));
        assert_eq!(event.transcript_path, None);
        assert_eq!((event.agent_pid, event.agent_created, event.agent_image), (None, None, None));
        assert_eq!(event.relay_ts, 1_790_000_009_000);
    }

    #[test]
    fn absent_optional_fields_are_none() {
        let event = parse_hook(json!({
            "hook_event_name": "SessionEnd", "session_id": SESSION, "cwd": r"C:\x", "relay_ts": 5,
            "agent_id": "agent-1", "agent_type": "explorer"
        }))
        .unwrap();
        assert_eq!(event.turn_id, None);
        assert_eq!(event.transcript_path, None);
        assert_eq!(event.tool_name, None);
        assert_eq!(event.tool_input, None);
        assert_eq!(event.agent_pid, None);
        assert_eq!(event.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(event.agent_type.as_deref(), Some("explorer"));
    }

    #[test]
    fn hook_errors_name_the_problem_without_payload_values() {
        assert_eq!(parse_hook(json!([1])), Err("not an object".to_string()));
        let missing = parse_hook(json!({ "hook_event_name": "Stop", "cwd": r"C:\x", "relay_ts": 5 }));
        assert_eq!(missing, Err("missing field `session_id`".to_string()));
        let wrong = parse_hook(json!({ "hook_event_name": "Stop", "session_id": "s", "cwd": r"C:\x", "relay_ts": "secret-value" }));
        assert_eq!(wrong, Err("invalid type".to_string()));
        let negative = parse_hook(json!({ "hook_event_name": "Stop", "session_id": "s", "cwd": r"C:\x", "relay_ts": 5, "agent_pid": -1 }));
        assert_eq!(negative, Err("invalid value".to_string()));
    }

    #[test]
    fn legacy_rollout_records() {
        use RolloutRecord::*;
        let call = |id: &str, label: &str| Call { call_id: id.into(), label: label.into() };
        let done = |id: &str| CallDone { call_id: id.into() };
        assert_eq!(
            records(LEGACY),
            vec![
                Meta { id: SESSION.into(), cwd: Some(r"C:\Users\dev\src\demo".into()), parent_thread_id: None, agent_role: None },
                TaskStarted { turn_id: turn(1) },
                Other, // turn_context
                Other, // event_msg user_message
                Other, // response_item message
                Other, // response_item reasoning
                call("call_1", "shell · cargo test"),
                done("call_1"), // function_call_output
                call("call_2", "exec"),
                done("call_2"), // custom_tool_call_output
                call("call_3", "shell · git status"),
                call("call_4", "update_plan"),
                Other, // agent_message
                Other, // token_count
                TaskComplete { turn_id: turn(1), last_agent_message: Some("Tests pass.".into()), error: None },
                TaskStarted { turn_id: turn(2) },
                TurnAborted { turn_id: turn(2) },
                TaskStarted { turn_id: turn(3) },
                TaskComplete { turn_id: turn(3), last_agent_message: None, error: Some("stream disconnected before completion".into()) },
                TaskStarted { turn_id: turn(4) },
                TaskComplete { turn_id: turn(4), last_agent_message: None, error: Some("Usage limit reached".into()) },
                TurnAborted { turn_id: None },
                Other, // world_state
                Other, // compacted
                Other, // event_msg context_compacted
            ]
        );
    }

    #[test]
    fn task_complete_error_forms() {
        let complete = |error: Value| {
            parse_rollout_line(&json!({ "type": "event_msg", "payload": { "type": "task_complete", "turn_id": "t", "error": error } }))
        };
        assert_eq!(error_of(complete(json!("boom"))), Some("boom".to_string()));
        assert_eq!(error_of(complete(json!({ "message": "Usage limit reached", "code": "x" }))), Some("Usage limit reached".to_string()));
        assert_eq!(error_of(complete(json!({ "code": "x" }))), Some(r#"{"code":"x"}"#.to_string()));
        assert_eq!(error_of(complete(json!(42))), Some("42".to_string()));
        assert_eq!(error_of(complete(Value::Null)), None);
        let absent = json!({ "type": "event_msg", "payload": { "type": "task_complete" } });
        assert_eq!(error_of(parse_rollout_line(&absent)), None);
    }

    #[test]
    fn subagent_meta_fields_are_parsed() {
        let meta = |n: u32, parent_thread_id: Option<&str>, agent_role: Option<&str>| RolloutRecord::Meta {
            id: format!("44444444-4444-4444-8444-44444444444{n}"),
            cwd: Some(r"C:\Users\dev\src\demo".into()),
            parent_thread_id: parent_thread_id.map(str::to_owned),
            agent_role: agent_role.map(str::to_owned),
        };
        assert_eq!(
            records(SUBAGENT_META),
            vec![
                meta(1, Some(SESSION), Some("explorer")), // top-level keys
                meta(2, None, Some("worker")), // agent_role only
                meta(3, Some(SESSION), None), // agent_role null
                meta(4, Some(SESSION), Some("explorer")), // only under source.subagent.thread_spawn
            ]
        );
    }

    #[test]
    fn records_that_are_not_legacy_shaped_are_unknown() {
        assert_eq!(records(UNKNOWN), vec![RolloutRecord::Unknown; 6]);
        assert_eq!(parse_rollout_line(&json!("text")), RolloutRecord::Unknown);
    }

    #[test]
    fn codex_hosts_come_from_the_agent_image() {
        let local = Path::new(LOCALAPPDATA);
        let cases = [
            (r"c:\Users\dev\.cursor\extensions\openai.chatgpt-0.146.0-win32-x64\bin\windows-x86_64\codex.exe", HostKind::Cursor),
            (r"C:\Users\dev\.vscode\extensions\openai.chatgpt-0.155.0-win32-x64\bin\windows-x86_64\codex.exe", HostKind::VsCode),
            (r"C:\Program Files\WindowsApps\OpenAI.Codex_26.924.0.0_x64__0000000000000\app\resources\codex.exe", HostKind::CodexDesktop),
            (r"C:\Users\dev\AppData\Local\OpenAI\Codex\bin\0.158.0\codex.exe", HostKind::CodexDesktop),
            (r"C:\Users\dev\AppData\Local\OpenAI\Codex\bin\codex.exe", HostKind::Unknown),
            (r"C:\tools\codex.exe", HostKind::Unknown),
        ];
        for (image, host) in cases {
            assert_eq!(classify_host(Some(image), local), host, "{image}");
        }
        assert_eq!(classify_host(None, local), HostKind::Unknown);
        let copy = r"C:\Users\dev\AppData\Local\OpenAI\Codex\bin\0.158.0\codex.exe";
        assert_eq!(classify_host(Some(copy), Path::new("")), HostKind::Unknown, "without %LOCALAPPDATA%");
    }

    #[test]
    fn index_title_takes_the_last_non_blank_name() {
        assert_eq!(index_title(SESSION_INDEX, SESSION).as_deref(), Some("Run the tests offline"));
        assert_eq!(index_title(SESSION_INDEX, "55555555-5555-4555-8555-555555555555").as_deref(), Some("Other thread"));
        assert_eq!(index_title(SESSION_INDEX, "66666666-6666-4666-8666-666666666666"), None, "blank name");
        assert_eq!(index_title(SESSION_INDEX, "77777777-7777-4777-8777-777777777777"), None, "unknown id");
        assert_eq!(index_title("", SESSION), None);
    }

    #[test]
    fn only_plain_rollout_paths_are_read() {
        assert!(is_readable_rollout(r"C:\Users\dev\.codex\sessions\2026\10\01\rollout-a.jsonl"));
        assert!(!is_readable_rollout(r"C:\Users\dev\.codex\sessions\2026\10\01\rollout-a.jsonl.zst"));
        assert!(!is_readable_rollout(r"C:\Users\dev\.codex\sessions\ROLLOUT-A.JSONL.ZST"));
        assert!(!is_readable_rollout(""));
        assert!(!is_readable_rollout("   "));
    }

    #[test]
    fn paginated_rollout_records() {
        // Spike S3 (Step 9a): paginated lines are the legacy records plus a top-level `ordinal`.
        use RolloutRecord::*;
        let paged_turn = |n: u32| Some(format!("99999999-9999-4999-8999-99999999999{n}"));
        assert_eq!(
            records(include_str!("fixtures/codex/rollout_paginated.jsonl")),
            vec![
                Meta {
                    id: "88888888-8888-4888-8888-888888888888".into(),
                    cwd: Some(r"C:\Users\dev\src\demo".into()),
                    parent_thread_id: None,
                    agent_role: None,
                },
                TaskStarted { turn_id: paged_turn(1) },
                Other, // turn_context
                Call { call_id: "call_1".into(), label: "shell · cargo test".into() },
                CallDone { call_id: "call_1".into() }, // function_call_output
                TaskComplete { turn_id: paged_turn(1), last_agent_message: Some("Tests pass.".into()), error: None },
                TaskStarted { turn_id: paged_turn(2) },
                TurnAborted { turn_id: paged_turn(2) },
                Other, // token_count with "ordinal": null
            ]
        );
    }

    #[test]
    fn the_codex_0_160_usage_and_agent_message_records_are_known() {
        // Spike S3 (2026-10-04, Codex 0.160.0): two more paginated top-level types, neither with a
        // `payload.type`; the payload keys below are the ones Codex wrote, the values are generic.
        let usage = json!({
            "ordinal": 12, "timestamp": "2026-10-04T10:00:00.000Z", "type": "token_usage_record",
            "payload": {
                "session_id": "s", "thread_id": "t", "root_turn_id": "r", "turn_id": "u", "response_id": "p",
                "usage": { "total_tokens": 3 }, "turn_token_usage": { "total_tokens": 3 }, "thread_token_usage": { "total_tokens": 3 }
            }
        });
        let metadata = json!({
            "ordinal": 13, "timestamp": "2026-10-04T10:00:01.000Z", "type": "inter_agent_communication_metadata",
            "payload": { "trigger_turn": true }
        });
        assert_eq!(parse_rollout_line(&usage), RolloutRecord::Other);
        assert_eq!(parse_rollout_line(&metadata), RolloutRecord::Other);
        // Any other unknown type stays Unknown (source health Degraded).
        let other = json!({ "ordinal": 14, "timestamp": "2026-10-04T10:00:02.000Z", "type": "token_usage_records", "payload": {} });
        assert_eq!(parse_rollout_line(&other), RolloutRecord::Unknown);
    }

    #[test]
    fn a_paginated_line_with_a_bad_ordinal_is_unknown() {
        // Spike S3 (Step 9a): Codex reads `ordinal` as an unsigned integer or null.
        let line = |ordinal: Value| {
            json!({ "ordinal": ordinal, "type": "event_msg", "payload": { "type": "task_started", "turn_id": "t" } })
        };
        for ordinal in [json!(-1), json!(1.5), json!("7"), json!([7]), json!({ "n": 7 })] {
            assert_eq!(parse_rollout_line(&line(ordinal.clone())), RolloutRecord::Unknown, "ordinal {ordinal}");
        }
        let started = RolloutRecord::TaskStarted { turn_id: Some("t".into()) };
        assert_eq!(parse_rollout_line(&line(json!(7))), started);
        assert_eq!(parse_rollout_line(&line(Value::Null)), started);
    }

    /// A paginated-mode response_item with the given payload.
    fn response_item(payload: Value) -> RolloutRecord {
        parse_rollout_line(&json!({
            "ordinal": 3, "timestamp": "2026-10-05T10:00:00.000Z", "type": "response_item", "payload": payload
        }))
    }

    fn ask(call_id: &str, label: &str) -> RolloutRecord {
        RolloutRecord::PermissionAsk { call_id: call_id.into(), label: label.into() }
    }

    fn call(call_id: &str, label: &str) -> RolloutRecord {
        RolloutRecord::Call { call_id: call_id.into(), label: label.into() }
    }

    #[test]
    fn an_exec_call_that_mentions_request_permissions_is_a_permission_ask() {
        // Projectless (code mode) threads: `exec` is a custom_tool_call whose `input` is JS text.
        let exec = |input: Value| {
            response_item(json!({ "type": "custom_tool_call", "name": "exec", "call_id": "call_1", "input": input }))
        };
        assert_eq!(exec(json!("const r = await tools.request_permissions({ permissions: {} });")), ask("call_1", "exec"));
        assert_eq!(exec(json!("text('request_permissions')")), ask("call_1", "exec"), "a mention is enough");
        assert_eq!(exec(json!("text('hello')")), call("call_1", "exec"));
        assert_eq!(exec(json!("REQUEST_PERMISSIONS")), call("call_1", "exec"), "the match is case-sensitive");
        assert_eq!(exec(json!({ "code": "request_permissions" })), call("call_1", "exec"), "only a string input is read");
        assert_eq!(exec(Value::Null), call("call_1", "exec"));
        // The name alone is enough, even without any text.
        let named = response_item(json!({ "type": "custom_tool_call", "name": "request_permissions", "call_id": "call_2" }));
        assert_eq!(named, ask("call_2", "request_permissions"));
    }

    #[test]
    fn a_function_call_named_request_permissions_is_a_permission_ask() {
        // Project threads: a function_call named request_permissions, `arguments` a JSON string.
        let named = response_item(json!({
            "type": "function_call", "name": "request_permissions", "call_id": "call_2",
            "arguments": r#"{"permissions":{"file_system":{"write":["C:\\Users\\dev\\out"]}},"reason":"Write the report"}"#
        }));
        assert_eq!(named, ask("call_2", "request_permissions"));
        // The `arguments` text of any other function never counts: a shell command that merely
        // mentions the word stays a plain Call.
        let shell = |arguments: Value| {
            response_item(json!({ "type": "function_call", "name": "shell", "call_id": "call_3", "arguments": arguments }))
        };
        assert_eq!(
            shell(json!(r#"{"command":["echo","request_permissions"]}"#)),
            call("call_3", "shell · echo request_permissions")
        );
        assert_eq!(shell(json!(r#"{"command":["echo","hi"]}"#)), call("call_3", "shell · echo hi"));
        assert_eq!(shell(json!({ "command": "request_permissions" })), call("call_3", "shell · request_permissions"));
        // A name is still required; a local_shell_call is never a permission ask.
        let nameless = response_item(json!({ "type": "function_call", "call_id": "call_4", "arguments": "request_permissions" }));
        assert_eq!(nameless, RolloutRecord::Unknown);
        let local = response_item(json!({
            "type": "local_shell_call", "call_id": "call_5", "action": { "command": ["echo", "request_permissions"] }
        }));
        assert_eq!(local, call("call_5", "shell · echo request_permissions"));
    }

    #[test]
    fn a_mention_in_a_custom_tool_input_counts_only_for_exec() {
        let custom = |name: &str, input: Value| {
            response_item(json!({ "type": "custom_tool_call", "name": name, "call_id": "call_6", "input": input }))
        };
        let text = json!("tools.request_permissions({ permissions: {} })");
        // Another custom tool (a patch, a note, a shell wrapper) that mentions the word is a plain Call.
        assert_eq!(custom("apply_patch", text.clone()), call("call_6", "apply_patch"));
        assert_eq!(custom("exec_command", text.clone()), call("call_6", "exec_command"));
        assert_eq!(custom("Exec", text.clone()), call("call_6", "Exec"), "the name is matched exactly");
        assert_eq!(custom("exec", text), ask("call_6", "exec"));
        // The name alone decides for request_permissions itself; its input is not needed.
        assert_eq!(custom("request_permissions", json!("{}")), ask("call_6", "request_permissions"));
    }

    #[test]
    fn call_outputs_with_a_string_call_id_are_call_done() {
        for kind in ["function_call_output", "custom_tool_call_output"] {
            let done = response_item(json!({ "type": kind, "call_id": "call_7", "output": "ok" }));
            assert_eq!(done, RolloutRecord::CallDone { call_id: "call_7".into() }, "{kind}");
            for call_id in [json!(7), Value::Null, json!(["call_7"])] {
                let other = response_item(json!({ "type": kind, "call_id": call_id, "output": "ok" }));
                assert_eq!(other, RolloutRecord::Other, "{kind} with call_id {call_id}");
            }
            assert_eq!(response_item(json!({ "type": kind, "output": "ok" })), RolloutRecord::Other, "{kind} without call_id");
        }
    }
}

// ── codex-io thread (T12, spec §5.6) ──────────────────────────────────────────
//
// A private child module keeps these imports apart from the parsing part above; the public
// items are re-exported as `observe::codex::{spawn, CodexConfig, CodexInput}`.

pub use io_thread::{spawn, CodexConfig, CodexInput};

mod io_thread {
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::io;
    use std::os::windows::io::OwnedHandle;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
    use std::time::{Duration, Instant};

    use serde_json::Value;

    use super::{
        classify_host, index_title, is_readable_rollout, parse_hook, parse_rollout_line, HookEvent,
        RolloutRecord,
    };
    use crate::log;
    use crate::observe::keys::{codex_key, HostKind, ProcId, SessionKey};
    use crate::observe::model::{Effect, Health, Signal, Source};
    use crate::observe::paths::{self, ProjectCache, ProjectRef};
    use crate::observe::silent::{host_of, parse_session_meta};
    use crate::observe::tail::JsonlTail;
    use crate::observe::win::proc;
    use crate::observe::win::{DirWatcher, FileAction, WatchEvent, MASK_NAME_WRITE_SIZE};

    /// D9: tracked rollouts that produced an Unknown record.
    fn codex_health(unknown_rollouts: usize) -> Health {
        if unknown_rollouts == 0 {
            Health::Ok
        } else {
            Health::Degraded { unreadable: u32::try_from(unknown_rollouts).unwrap_or(u32::MAX) }
        }
    }

    /// Codex names its rollouts `rollout-<time>-<id>.jsonl`. Any other file a hook names (for
    /// example `auth.json` or `config.toml`, spec §8) is never opened. `path_key` is a
    /// `NormPath.key` (lowercase).
    fn is_rollout_name(path_key: &str) -> bool {
        let name = path_key.rsplit('\\').next().unwrap_or(path_key);
        name.starts_with("rollout-") && name.ends_with(".jsonl")
    }

    /// First 8 characters of a session id for the log, "-" when empty.
    fn short(session_id: &str) -> String {
        if session_id.is_empty() {
            "-".into()
        } else {
            session_id.chars().take(8).collect()
        }
    }

    /// One JSON value per line; unparsable lines are skipped.
    fn parse_records(lines: Vec<(u64, String)>) -> Vec<(u64, Value)> {
        lines
            .into_iter()
            .filter_map(|(offset, text)| serde_json::from_str(&text).ok().map(|v| (offset, v)))
            .collect()
    }

    pub struct CodexConfig {
        pub codex_home: PathBuf,
        pub localappdata: PathBuf,
        /// false in tests: no DirWatcher, `Sessions` inputs are fed by hand.
        pub watch: bool,
    }

    /// The first line of a rollout is read up to this size; longer first lines are not silent rows.
    const FIRST_LINE_CAP: usize = 256 * 1024;

    /// Rollout files remembered by the sessions watcher; the oldest inserted is dropped first.
    const SILENT_FILES_MAX: usize = 256;

    /// One rollout file the watcher has looked at (kept so its first line is read once).
    enum SilentFile {
        /// Subagent, unreadable or not a session: never a row.
        Skip,
        Session { id: String, project: ProjectRef, host: HostKind, folder_exists: bool },
    }

    /// What reading the first line of a rollout gave.
    enum FirstLine {
        /// A complete line (newline found within the cap).
        Complete(String),
        /// Nothing, a partial line (no newline yet) or an I/O error: the file may still be
        /// written to, so nothing is decided and the next event reads again.
        Incomplete,
        /// No newline within `FIRST_LINE_CAP` bytes: never a silent row.
        TooLong,
    }

    /// First line of `path`, reading at most `FIRST_LINE_CAP` + 1 bytes (short lines read no more
    /// than they are long). An error is logged by kind only.
    fn read_first_line(path: &Path) -> FirstLine {
        use std::io::{BufRead, BufReader, Read};
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(err) => {
                log::line(format!("codex rollout first line unreadable: {:?}", err.kind()));
                return FirstLine::Incomplete;
            }
        };
        let mut buf = Vec::with_capacity(8 * 1024);
        if let Err(err) = BufReader::new(file.take(FIRST_LINE_CAP as u64 + 1)).read_until(b'\n', &mut buf) {
            log::line(format!("codex rollout first line unreadable: {:?}", err.kind()));
            return FirstLine::Incomplete;
        }
        if buf.last() == Some(&b'\n') {
            buf.pop();
            FirstLine::Complete(String::from_utf8_lossy(&buf).trim_end_matches('\r').to_owned())
        } else if buf.len() > FIRST_LINE_CAP {
            FirstLine::TooLong
        } else {
            FirstLine::Incomplete
        }
    }

    #[derive(Debug)]
    pub enum CodexInput {
        Hook(Value),
        Effect(Effect),
        /// The island was shown: start the sessions watch if it is not running.
        Retry,
        /// A file-system event under `<codex_home>\sessions`.
        Sessions(WatchEvent),
    }

    /// Tail state of one rollout file, shared by every session whose main rollout it is.
    struct Rollout {
        tail: JsonlTail,
        /// NormPath.display, sent as CodexRollout.path.
        display: String,
        /// Produced an `Unknown` record while tracked (health, D9).
        unknown: bool,
    }

    struct CodexIo {
        cfg: CodexConfig,
        signals: Sender<Signal>,
        send_failed: bool,
        registry: proc::Registry,
        cache: ProjectCache,
        /// By normalized path key (spec §5.6: tail state is kept per path).
        rollouts: HashMap<String, Rollout>,
        /// Session → its main rollout's path key.
        main_rollout: HashMap<SessionKey, String>,
        /// Keys whose title was looked up in this run.
        seen: HashSet<SessionKey>,
        /// CodexTicker(true) is in force: `run` reads every rollout once per TICK.
        ticker: bool,
        /// Last health sent; the store starts at Ok.
        health: Health,
        /// Rollouts seen by the sessions watcher, by normalized path key, and their insertion order.
        silent_files: HashMap<String, SilentFile>,
        silent_order: VecDeque<String>,
        /// Rollouts whose first line cannot be read as a session: health (spec silent-codex section 5).
        silent_unreadable: HashSet<String>,
        /// Sender for the sessions watcher (None without `watch`) and whether the watch runs.
        watcher_tx: Option<Sender<CodexInput>>,
        watching: bool,
    }

    impl CodexIo {
        fn new(cfg: CodexConfig, signals: Sender<Signal>, registry: proc::Registry) -> Self {
            CodexIo {
                cfg,
                signals,
                send_failed: false,
                registry,
                cache: ProjectCache::new(),
                rollouts: HashMap::new(),
                main_rollout: HashMap::new(),
                seen: HashSet::new(),
                ticker: false,
                health: Health::Ok,
                silent_files: HashMap::new(),
                silent_order: VecDeque::new(),
                silent_unreadable: HashSet::new(),
                watcher_tx: None,
                watching: false,
            }
        }

        /// Never blocks; the first failed send (store gone) is logged once.
        fn emit(&mut self, signal: Signal) {
            if self.signals.send(signal).is_err() && !self.send_failed {
                self.send_failed = true;
                log::line("codex-io: store is gone, signals are dropped");
            }
        }

        fn handle(&mut self, input: CodexInput) {
            match input {
                CodexInput::Hook(payload) => self.hook(payload),
                CodexInput::Effect(Effect::UntrackRollout { key }) => self.untrack(&key),
                CodexInput::Effect(Effect::CodexTicker(on)) => {
                    if on != self.ticker {
                        log::line(format!("codex-io: ticker {}", if on { "on" } else { "off" }));
                    }
                    self.ticker = on;
                }
                CodexInput::Effect(other) => log::line(format!("codex-io: ignored effect {other:?}")),
                CodexInput::Retry => self.ensure_watch(),
                CodexInput::Sessions(WatchEvent::Changes(changes)) => self.sessions_changed(&changes),
                CodexInput::Sessions(WatchEvent::Overflow) => log::line("codex-io: sessions watcher overflow"),
                CodexInput::Sessions(WatchEvent::Error(err)) => {
                    log::line(format!("codex-io: sessions watcher error: {:?}", err.kind()));
                }
            }
            self.report_health();
        }

        fn hook(&mut self, payload: Value) {
            let len = payload.to_string().len();
            let event = match parse_hook(payload) {
                Ok(event) => event,
                Err(reason) => {
                    log::line(format!("codex hook dropped: len={len} reason={reason}"));
                    return;
                }
            };
            let key = codex_key(&event.session_id);
            let proven = self.prove(&event);
            let host = classify_host(event.agent_image.as_deref(), &self.cfg.localappdata);
            let project = self.cache.get(&event.cwd, paths::git_probe);
            let title = if self.seen.insert(key.clone()) { self.read_title(&event.session_id) } else { None };
            let rollout_eof = self.main_rollout_eof(&key, &event);
            log::line(format!(
                "codex {} s={} host={host:?} proc={}",
                event.hook_event_name,
                short(&event.session_id),
                if proven.is_some() { "yes" } else { "no" }
            ));
            let proc = proven.as_ref().map(|(id, _)| id.clone());
            self.emit(Signal::CodexHook { key, event, proc, host, project, rollout_eof, title });
            if let Some((id, handle)) = proven {
                // Registered after CodexHook, so ProcExit can never overtake the hook that created
                // the session. `false` needs nothing more: the process is already watched, or its
                // waiter thread could not be started, and then the Registry has removed the entry
                // and logged it, so the next proven hook registers it again.
                let signals = self.signals.clone();
                let _ = self.registry.watch(&id, handle, move |pid, created_ft| {
                    if signals.send(Signal::ProcExit { pid, created_ft }).is_err() {
                        log::line(format!("proc-exit {pid}: store is gone"));
                    }
                });
            }
        }

        /// identify(agent_pid) with a creation time equal to agent_created (spec §5.6).
        fn prove(&self, event: &HookEvent) -> Option<(ProcId, OwnedHandle)> {
            let (pid, created) = (event.agent_pid?, event.agent_created?);
            match proc::identify(pid) {
                Ok((id, handle)) if id.created_ft == created => Some((id, handle)),
                Ok(_) => {
                    log::line(format!("codex agent {pid}: creation time differs, not proved"));
                    None
                }
                Err(err) => {
                    log::line(format!("codex agent {pid} not identified: {err:?}"));
                    None
                }
            }
        }

        fn read_title(&self, session_id: &str) -> Option<String> {
            let path = self.cfg.codex_home.join("session_index.jsonl");
            match std::fs::read(&path) {
                Ok(bytes) => index_title(&String::from_utf8_lossy(&bytes), session_id),
                Err(err) if err.kind() == io::ErrorKind::NotFound => None,
                Err(err) => {
                    log::line(format!("codex session index unreadable: {:?}", err.kind()));
                    None
                }
            }
        }
    }

    impl CodexIo {
        /// A rollout written under `sessions\` that no hook tracks is a silent session (spec
        /// silent-codex section 2). Its first line is read once; every later event is answered
        /// from the cache.
        fn sessions_changed(&mut self, changes: &[(FileAction, String)]) {
            for (action, name) in changes {
                if !matches!(action, FileAction::Added | FileAction::Modified | FileAction::RenamedNew) {
                    continue;
                }
                let full = self.cfg.codex_home.join("sessions").join(name);
                let raw = full.to_string_lossy();
                if !is_readable_rollout(&raw) {
                    continue;
                }
                let norm = paths::normalize(&raw);
                if !is_rollout_name(&norm.key) || self.rollouts.contains_key(&norm.key) {
                    continue;
                }
                if !self.silent_files.contains_key(&norm.key) {
                    match self.read_silent_file(&full) {
                        Some((file, unreadable)) => self.remember_silent(&norm.key, file, unreadable),
                        None => continue,
                    }
                }
                let Some(SilentFile::Session { id, project, host, folder_exists }) = self.silent_files.get(&norm.key)
                else {
                    continue;
                };
                let signal = Signal::CodexSilent {
                    id: id.clone(),
                    project: project.clone(),
                    host: *host,
                    folder_exists: *folder_exists,
                    written_ms: crate::observe::now_ms(),
                };
                self.emit(signal);
            }
        }

        /// Bounded: the oldest inserted key is dropped first.
        fn remember_silent(&mut self, key: &str, file: SilentFile, unreadable: bool) {
            if !self.silent_files.contains_key(key) {
                if self.silent_order.len() >= SILENT_FILES_MAX {
                    if let Some(oldest) = self.silent_order.pop_front() {
                        self.silent_files.remove(&oldest);
                        self.silent_unreadable.remove(&oldest);
                    }
                }
                self.silent_order.push_back(key.to_owned());
            }
            self.silent_files.insert(key.to_owned(), file);
            if unreadable {
                self.silent_unreadable.insert(key.to_owned());
            } else {
                self.silent_unreadable.remove(key);
            }
        }

        /// Starts the sessions watch when `watch` is on and it is not running. A missing folder
        /// is not an error: the next Retry (island shown) tries again.
        fn ensure_watch(&mut self) {
            if self.watching {
                return;
            }
            let Some(tx) = self.watcher_tx.clone() else { return };
            let dir = self.cfg.codex_home.join("sessions");
            match DirWatcher::spawn(&dir, true, MASK_NAME_WRITE_SIZE, move |ev| {
                tx.send(CodexInput::Sessions(ev)).is_ok()
            }) {
                Ok(()) => self.watching = true,
                Err(err) => log::line(format!("observe: codex sessions watcher not started: {:?}", err.kind())),
            }
        }

        /// `None` when the file has no complete first line yet (nothing is cached); otherwise the
        /// verdict and whether the file counts as unreadable for health.
        fn read_silent_file(&mut self, path: &Path) -> Option<(SilentFile, bool)> {
            let line = match read_first_line(path) {
                FirstLine::Complete(line) => line,
                FirstLine::Incomplete => return None,
                FirstLine::TooLong => return Some((SilentFile::Skip, true)),
            };
            let Some(meta) = parse_session_meta(&line) else {
                return Some((SilentFile::Skip, true));
            };
            if meta.subagent {
                return Some((SilentFile::Skip, false));
            }
            let cwd = meta.cwd.as_deref().unwrap_or("");
            let folder_exists = !cwd.is_empty() && Path::new(cwd).is_dir();
            let project = if folder_exists {
                self.cache.get(cwd, paths::git_probe)
            } else {
                // No folder to probe: the project is the cwd itself, as for a session with no repo.
                let norm = paths::normalize(cwd);
                ProjectRef { key: norm.key, root: norm.display }
            };
            let host = host_of(meta.originator.as_deref());
            Some((SilentFile::Session { id: meta.id, project, host, folder_exists }, false))
        }

        /// Main rollout = transcript_path of events without agent_id (spec §5.6). Drains it
        /// (CodexRollout first) and returns its EOF for CodexHook; None when the session has no
        /// main rollout.
        fn main_rollout_eof(&mut self, key: &SessionKey, event: &HookEvent) -> Option<u64> {
            if event.agent_id.is_none() {
                if let Some(path_key) = event.transcript_path.as_deref().and_then(|raw| self.open_rollout(raw)) {
                    if self.main_rollout.insert(key.clone(), path_key).is_some() {
                        self.drop_unused_rollouts();
                    }
                }
            }
            let path_key = self.main_rollout.get(key)?.clone();
            self.drain_path(&path_key);
            self.rollouts.get(&path_key).map(|r| r.tail.eof())
        }

        /// Path key of a usable rollout: a readable `rollout-*.jsonl` (else the path is not used
        /// at all), whose tail starts at its EOF on first sight (history is not replayed). A
        /// file that cannot be opened at its EOF is not tracked and never read from offset 0:
        /// the session's next hook tries again.
        fn open_rollout(&mut self, raw: &str) -> Option<String> {
            if !is_readable_rollout(raw) {
                return None;
            }
            let norm = paths::normalize(raw);
            if !is_rollout_name(&norm.key) {
                log::line("codex rollout path rejected");
                return None;
            }
            if !self.rollouts.contains_key(&norm.key) {
                let mut tail = JsonlTail::new(&norm.display);
                if let Err(err) = tail.start_at_eof() {
                    log::line(format!("codex rollout start failed: {:?}", err.kind()));
                    return None;
                }
                self.rollouts.insert(norm.key.clone(), Rollout { tail, display: norm.display, unknown: false });
            }
            Some(norm.key)
        }

        /// New records of one rollout, sent to every session that uses it as main rollout.
        fn drain_path(&mut self, path_key: &str) {
            let Some(rollout) = self.rollouts.get_mut(path_key) else { return };
            let (records, eof) = match rollout.tail.read_new() {
                Ok((lines, eof)) => (parse_records(lines), eof),
                Err(err) => {
                    log::line(format!("codex rollout read failed: {:?}", err.kind()));
                    return;
                }
            };
            if records.is_empty() {
                return;
            }
            let unknown = records.iter().any(|(_, v)| matches!(parse_rollout_line(v), RolloutRecord::Unknown));
            if unknown && !rollout.unknown {
                log::line("codex rollout has records of an unknown shape");
                rollout.unknown = true;
            }
            let path = rollout.display.clone();
            let mut keys: Vec<SessionKey> = self
                .main_rollout
                .iter()
                .filter(|(_, p)| p.as_str() == path_key)
                .map(|(k, _)| k.clone())
                .collect();
            keys.sort();
            for key in keys {
                self.emit(Signal::CodexRollout { key, path: path.clone(), records: records.clone(), eof });
            }
        }

        fn tick(&mut self) {
            let mut path_keys: Vec<String> = self.rollouts.keys().cloned().collect();
            path_keys.sort();
            for path_key in path_keys {
                self.drain_path(&path_key);
            }
            self.report_health();
        }

        /// Drops the key → path mapping, the tail when no other session uses it, and the "seen"
        /// mark (a re-created session reads its title again).
        fn untrack(&mut self, key: &str) {
            self.seen.remove(key);
            if self.main_rollout.remove(key).is_some() {
                self.drop_unused_rollouts();
            }
        }

        fn drop_unused_rollouts(&mut self) {
            let used: HashSet<&String> = self.main_rollout.values().collect();
            let dropped: Vec<String> =
                self.rollouts.keys().filter(|path_key| !used.contains(path_key)).cloned().collect();
            self.rollouts.retain(|path_key, _| used.contains(path_key));
            // A rollout a hook reported is never a silent row, also after its session is removed.
            for key in dropped {
                self.remember_silent(&key, SilentFile::Skip, false);
            }
        }

        /// Sent on change only.
        fn report_health(&mut self) {
            let unknown = self.rollouts.values().filter(|r| r.unknown).count() + self.silent_unreadable.len();
            let health = codex_health(unknown);
            if health != self.health {
                log::line(format!("codex-io: health {health:?}"));
                self.health = health.clone();
                self.emit(Signal::SourceHealth { source: Source::Codex, health });
            }
        }
    }

    pub const TICK: Duration = Duration::from_secs(1);

    /// Thread "codex-io". The ticker is a recv_timeout against a 1 s deadline while
    /// CodexTicker(true) is in force; a plain recv otherwise (no extra thread, D27).
    pub fn spawn(
        cfg: CodexConfig,
        signals: Sender<Signal>,
        registry: proc::Registry,
    ) -> io::Result<Sender<CodexInput>> {
        let (tx, rx) = mpsc::channel();
        // Without `watch` (tests) the thread keeps no sender, so it ends when the caller drops theirs.
        let watcher_tx = cfg.watch.then(|| tx.clone());
        std::thread::Builder::new().name("codex-io".into()).spawn(move || {
            let mut io = CodexIo::new(cfg, signals, registry);
            io.watcher_tx = watcher_tx;
            io.ensure_watch();
            run(rx, io)
        })?;
        Ok(tx)
    }

    fn run(rx: Receiver<CodexInput>, mut io: CodexIo) {
        let mut deadline = Instant::now();
        loop {
            let input = if io.ticker {
                let now = Instant::now();
                if now >= deadline {
                    io.tick();
                    deadline = now + TICK;
                    continue;
                }
                match rx.recv_timeout(deadline - now) {
                    Ok(input) => input,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match rx.recv() {
                    Ok(input) => input,
                    Err(_) => return,
                }
            };
            let was_on = io.ticker;
            io.handle(input);
            if io.ticker && !was_on {
                deadline = Instant::now() + TICK;
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::testutil::TempDir;
        use std::path::Path;
        use std::sync::mpsc::Receiver;
        use std::time::Duration;

        #[test]
        fn health_counts_rollouts_with_unknown_records() {
            assert_eq!(codex_health(0), Health::Ok);
            assert_eq!(codex_health(2), Health::Degraded { unreadable: 2 });
        }

        #[test]
        fn only_rollout_jsonl_files_are_main_rollouts() {
            assert!(is_rollout_name(r"c:\users\dev\.codex\sessions\2026\10\01\rollout-2026-10-01t10-00-00-019a.jsonl"));
            for key in [
                r"c:\users\dev\.codex\auth.json",
                r"c:\users\dev\.codex\config.toml",
                r"c:\users\dev\.codex\sessions\notes.jsonl",
                r"c:\users\dev\.codex\sessions\rollout-a.jsonl.zst",
                "",
            ] {
                assert!(!is_rollout_name(key), "{key}");
            }
        }

        #[test]
        fn log_ids_are_short() {
            assert_eq!(short("019a0000-0000-7000-8000-00000000c0de"), "019a0000");
            assert_eq!(short(""), "-");
        }

        #[test]
        fn unparsable_rollout_lines_are_skipped() {
            let lines = vec![(0, r#"{"a":1}"#.to_string()), (8, "{not json".to_string())];
            let offsets: Vec<u64> = parse_records(lines).iter().map(|(o, _)| *o).collect();
            assert_eq!(offsets, [0]);
        }

        const SID: &str = "019a0000-0000-7000-8000-00000000c0de";
        const TASK_STARTED: &str = r#"{"type":"event_msg","payload":{"type":"task_started","turn_id":"t2"}}"#;
        const TASK_COMPLETE: &str = r#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"t2","last_agent_message":"done"}}"#;
        const MYSTERY: &str = r#"{"type":"mystery","payload":{}}"#;

        fn me() -> ProcId {
            proc::identify(std::process::id()).expect("the test process identifies itself").0
        }

        struct Rig {
            dir: TempDir,
            io: CodexIo,
            rx: Receiver<Signal>,
            me: ProcId,
        }

        fn rig(tag: &str) -> Rig {
            let dir = TempDir::new(tag);
            let home = dir.path.join("codex");
            std::fs::create_dir_all(home.join("sessions")).unwrap();
            // The project root must not depend on a `.git` above %TEMP% (git_probe walks up).
            std::fs::create_dir_all(dir.path.join("work").join(".git")).unwrap();
            std::fs::write(
                home.join("session_index.jsonl"),
                format!("{{\"id\":\"{SID}\",\"thread_name\":\"Fix the login page\"}}\n"),
            )
            .unwrap();
            let rollout = home.join("sessions").join("rollout-a.jsonl");
            std::fs::write(&rollout, format!("{TASK_STARTED}\n")).unwrap();
            let (tx, rx) = mpsc::channel();
            let cfg = CodexConfig { codex_home: home, localappdata: dir.path.join("local"), watch: false };
            let io = CodexIo::new(cfg, tx, proc::Registry::new());
            Rig { dir, io, rx, me: me() }
        }

        impl Rig {
            fn rollout(&self) -> PathBuf {
                self.dir.path.join("codex").join("sessions").join("rollout-a.jsonl")
            }
            fn cwd(&self) -> String {
                self.dir.path.join("work").to_string_lossy().into_owned()
            }
            fn payload(&self, event: &str, transcript: Option<&Path>) -> Value {
                serde_json::json!({
                    "hook_event_name": event, "session_id": SID, "turn_id": "t2",
                    "cwd": self.cwd(), "model": "gpt-5-codex",
                    "transcript_path": transcript.map(|p| p.to_string_lossy().into_owned()),
                    "agent": "codex", "relay": 2, "relay_ts": 1_000,
                    "agent_pid": self.me.pid, "agent_created": self.me.created_ft,
                    "agent_image": self.me.image,
                })
            }
            fn hook(&mut self, payload: Value) {
                self.io.handle(CodexInput::Hook(payload));
            }
            fn append(&self, lines: &[&str]) {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new().append(true).open(self.rollout()).unwrap();
                for line in lines {
                    writeln!(f, "{line}").unwrap();
                }
            }
            fn len(&self) -> u64 {
                std::fs::metadata(self.rollout()).unwrap().len()
            }
            fn signals(&self) -> Vec<Signal> {
                self.rx.try_iter().collect()
            }
        }

        fn hook_of(signals: &[Signal]) -> (Option<ProcId>, Option<u64>, Option<String>) {
            match signals.last() {
                Some(Signal::CodexHook { proc, rollout_eof, title, .. }) => (proc.clone(), *rollout_eof, title.clone()),
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn an_invalid_payload_is_dropped() {
            let mut rig = rig("xio-invalid");
            rig.hook(serde_json::json!({ "hook_event_name": "Stop" }));
            assert_eq!(rig.signals(), []);
        }

        #[test]
        fn first_hook_proves_reads_the_title_and_starts_the_rollout_at_eof() {
            let mut rig = rig("xio-first");
            let rollout = rig.rollout();
            rig.hook(rig.payload("UserPromptSubmit", Some(&rollout)));
            let signals = rig.signals();
            assert_eq!(signals.len(), 1, "history is not replayed: {signals:?}");
            match &signals[0] {
                Signal::CodexHook { key, event, proc, project, rollout_eof, title, .. } => {
                    assert_eq!(key, &codex_key(SID));
                    assert_eq!(event.hook_event_name, "UserPromptSubmit");
                    assert_eq!(proc.as_ref(), Some(&rig.me));
                    assert_eq!(project.root, paths::normalize(&rig.cwd()).display);
                    assert_eq!(*rollout_eof, Some(rig.len()));
                    assert_eq!(title.as_deref(), Some("Fix the login page"));
                }
                other => panic!("{other:?}"),
            }
            rig.hook(rig.payload("Stop", Some(&rollout)));
            assert_eq!(hook_of(&rig.signals()).2, None, "the title is read on first sight only");
        }

        #[test]
        fn appended_rollout_records_go_out_before_the_hook() {
            let mut rig = rig("xio-order");
            let rollout = rig.rollout();
            rig.hook(rig.payload("UserPromptSubmit", Some(&rollout)));
            rig.signals();
            let start = rig.len();
            rig.append(&[TASK_COMPLETE, "{not json"]);
            rig.hook(rig.payload("Stop", Some(&rollout)));
            let signals = rig.signals();
            assert_eq!(signals.len(), 2, "{signals:?}");
            match &signals[0] {
                Signal::CodexRollout { key, path, records, eof } => {
                    assert_eq!(key, &codex_key(SID));
                    assert_eq!(path, &paths::normalize(&rollout.to_string_lossy()).display);
                    assert_eq!(records.iter().map(|(o, _)| *o).collect::<Vec<_>>(), [start]);
                    assert_eq!(*eof, rig.len());
                }
                other => panic!("{other:?}"),
            }
            assert_eq!(hook_of(&signals).1, Some(rig.len()));
        }

        #[test]
        fn subagent_and_zst_paths_are_not_main_rollouts() {
            let mut rig = rig("xio-skip");
            let rollout = rig.rollout();
            let mut sub = rig.payload("PermissionRequest", Some(&rollout));
            sub["agent_id"] = "a-1".into();
            sub["agent_type"] = "explorer".into();
            rig.hook(sub);
            assert_eq!(hook_of(&rig.signals()).1, None);
            let zst = rig.dir.path.join("codex").join("sessions").join("rollout-b.jsonl.zst");
            rig.hook(rig.payload("Stop", Some(&zst)));
            assert_eq!(hook_of(&rig.signals()).1, None);
            rig.append(&[TASK_COMPLETE]);
            rig.io.tick();
            assert_eq!(rig.signals(), []);
        }

        #[test]
        fn a_transcript_path_that_is_not_a_rollout_file_is_never_opened() {
            let mut rig = rig("xio-reject");
            let auth = rig.dir.path.join("codex").join("auth.json");
            std::fs::write(&auth, "{}\n").unwrap();
            rig.hook(rig.payload("UserPromptSubmit", Some(&auth)));
            assert_eq!(hook_of(&rig.signals()).1, None);
            std::fs::write(&auth, "{}\n{}\n").unwrap();
            rig.io.tick();
            assert_eq!(rig.signals(), []);
        }

        #[test]
        fn a_rollout_that_cannot_be_opened_is_never_read_from_offset_zero() {
            let mut rig = rig("xio-unopened");
            // A directory cannot be opened as a file, so start_at_eof fails.
            let later = rig.dir.path.join("codex").join("sessions").join("rollout-later.jsonl");
            std::fs::create_dir(&later).unwrap();
            rig.hook(rig.payload("UserPromptSubmit", Some(&later)));
            assert_eq!(hook_of(&rig.signals()).1, None);
            std::fs::remove_dir(&later).unwrap();
            std::fs::write(&later, format!("{TASK_STARTED}\n")).unwrap();
            rig.hook(rig.payload("Stop", Some(&later)));
            let signals = rig.signals();
            assert_eq!(signals.len(), 1, "the history is not replayed: {signals:?}");
            assert_eq!(hook_of(&signals).1, Some(std::fs::metadata(&later).unwrap().len()));
        }

        #[test]
        fn a_different_creation_time_gives_no_proc() {
            let mut rig = rig("xio-proof");
            let mut payload = rig.payload("UserPromptSubmit", None);
            payload["agent_created"] = (rig.me.created_ft + 1).into();
            rig.hook(payload);
            assert_eq!(hook_of(&rig.signals()).0, None);
            let mut payload = rig.payload("Stop", None);
            payload["agent_pid"] = Value::Null;
            rig.hook(payload);
            assert_eq!(hook_of(&rig.signals()).0, None);
        }

        #[test]
        fn an_exited_agent_gets_proc_exit_after_its_hook() {
            let mut rig = rig("xio-exit");
            let mut child = std::process::Command::new("cmd")
                .args(["/c", "ping -n 3 127.0.0.1 >nul"])
                .spawn()
                .expect("cmd starts");
            let id = proc::identify(child.id()).expect("child is running").0;
            let mut payload = rig.payload("UserPromptSubmit", None);
            payload["agent_pid"] = id.pid.into();
            payload["agent_created"] = id.created_ft.into();
            rig.hook(payload);
            assert_eq!(hook_of(&rig.signals()).0, Some(id.clone()), "the hook comes first");
            child.wait().unwrap();
            let exit = rig.rx.recv_timeout(Duration::from_secs(10)).expect("ProcExit");
            assert_eq!(exit, Signal::ProcExit { pid: id.pid, created_ft: id.created_ft });
        }

        #[test]
        fn tick_reads_every_tracked_rollout() {
            let mut rig = rig("xio-tick");
            let rollout = rig.rollout();
            rig.hook(rig.payload("UserPromptSubmit", Some(&rollout)));
            rig.signals();
            rig.io.tick();
            assert_eq!(rig.signals(), []);
            rig.append(&[TASK_COMPLETE]);
            rig.io.tick();
            match rig.signals().as_slice() {
                [Signal::CodexRollout { key, records, .. }] => {
                    assert_eq!(key, &codex_key(SID));
                    assert_eq!(records.len(), 1);
                }
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn untrack_drops_the_tail_and_the_seen_mark() {
            let mut rig = rig("xio-untrack");
            let rollout = rig.rollout();
            rig.hook(rig.payload("UserPromptSubmit", Some(&rollout)));
            rig.signals();
            rig.io.handle(CodexInput::Effect(Effect::UntrackRollout { key: codex_key(SID) }));
            rig.append(&[TASK_COMPLETE]);
            rig.io.tick();
            assert_eq!(rig.signals(), []);
            rig.hook(rig.payload("UserPromptSubmit", Some(&rollout)));
            let signals = rig.signals();
            assert_eq!(signals.len(), 1, "restarted at EOF: {signals:?}");
            let (_, eof, title) = hook_of(&signals);
            assert_eq!(eof, Some(rig.len()));
            assert_eq!(title.as_deref(), Some("Fix the login page"));
        }

        #[test]
        fn unknown_records_degrade_health_until_untracked() {
            let mut rig = rig("xio-health");
            let rollout = rig.rollout();
            rig.hook(rig.payload("UserPromptSubmit", Some(&rollout)));
            rig.signals();
            rig.append(&[MYSTERY]);
            rig.io.tick();
            let signals = rig.signals();
            assert_eq!(signals.len(), 2, "{signals:?}");
            assert_eq!(signals[1], Signal::SourceHealth { source: Source::Codex, health: Health::Degraded { unreadable: 1 } });
            rig.io.handle(CodexInput::Effect(Effect::UntrackRollout { key: codex_key(SID) }));
            assert_eq!(rig.signals(), [Signal::SourceHealth { source: Source::Codex, health: Health::Ok }]);
        }

        #[test]
        fn the_ticker_reads_rollouts_only_while_on() {
            let rig = rig("xio-ticker");
            let rollout = rig.rollout();
            let (tx, rx) = mpsc::channel();
            let cfg = CodexConfig { codex_home: rig.dir.path.join("codex"), localappdata: rig.dir.path.clone(), watch: false };
            let input = spawn(cfg, tx, proc::Registry::new()).expect("thread starts");
            let wait = Duration::from_secs(5);
            input.send(CodexInput::Hook(rig.payload("UserPromptSubmit", Some(&rollout)))).unwrap();
            assert!(matches!(rx.recv_timeout(wait), Ok(Signal::CodexHook { .. })));
            input.send(CodexInput::Effect(Effect::CodexTicker(true))).unwrap();
            rig.append(&[TASK_COMPLETE]);
            assert!(matches!(rx.recv_timeout(wait), Ok(Signal::CodexRollout { .. })), "the ticker read the rollout");
            input.send(CodexInput::Effect(Effect::CodexTicker(false))).unwrap();
            // Barrier: a hook without rollout is answered only after the ticker-off was handled.
            let mut other = rig.payload("UserPromptSubmit", None);
            other["session_id"] = "other-session".into();
            input.send(CodexInput::Hook(other)).unwrap();
            assert!(matches!(rx.recv_timeout(wait), Ok(Signal::CodexHook { .. })));
            rig.append(&[TASK_STARTED]);
            assert!(rx.recv_timeout(Duration::from_millis(1500)).is_err(), "no tick while the ticker is off");
        }

        // ---- silent rollouts (the sessions watcher) ----

        const SESSION_META: &str = r#"{"type":"session_meta","payload":{"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","cwd":"C:\\no\\such\\folder","originator":"Codex Desktop"}}"#;

        fn rollout_file(dir: &Path, name: &str, first_line: &str) -> PathBuf {
            let day = dir.join("sessions").join("2026").join("10").join("06");
            std::fs::create_dir_all(&day).unwrap();
            let path = day.join(name);
            std::fs::write(&path, format!("{first_line}\n{{\"type\":\"event_msg\"}}\n")).unwrap();
            path
        }

        fn change(name: &str, action: FileAction) -> CodexInput {
            CodexInput::Sessions(WatchEvent::Changes(vec![(action, format!(r"2026\10\06\{name}"))]))
        }

        fn silent_rig(tag: &str) -> Rig {
            let mut rig = rig(tag);
            // codex_home = the temp dir itself, so `rollout_file` lands under <codex_home>\sessions.
            rig.io = CodexIo::new(
                CodexConfig { codex_home: rig.dir.path.clone(), localappdata: rig.dir.path.join("local"), watch: false },
                mpsc::channel().0,
                proc::Registry::new(),
            );
            let (tx, rx) = mpsc::channel();
            rig.io.signals = tx;
            rig.rx = rx;
            rig
        }

        fn silent_of(signals: Vec<Signal>) -> Vec<Signal> {
            signals.into_iter().filter(|s| matches!(s, Signal::CodexSilent { .. })).collect()
        }

        #[test]
        fn a_new_untracked_rollout_gives_one_silent_signal_and_is_read_once() {
            let mut rig = silent_rig("xio-silent-new");
            let name = "rollout-2026-10-06T10-00-00-aaaa.jsonl";
            let path = rollout_file(&rig.dir.path, name, SESSION_META);
            rig.io.handle(change(name, FileAction::Added));
            // The file is replaced by garbage: a second event must come from the cache, not a re-read.
            std::fs::write(&path, "garbage\n").unwrap();
            rig.io.handle(change(name, FileAction::Modified));
            let silent = silent_of(rig.signals());
            assert_eq!(silent.len(), 2, "two events, one id");
            for s in &silent {
                let Signal::CodexSilent { id, folder_exists, host, .. } = s else { unreachable!() };
                assert_eq!(id, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
                assert!(!folder_exists);
                assert_eq!(*host, HostKind::CodexDesktop);
            }
        }

        #[test]
        fn a_rollout_that_a_hook_already_tracks_is_never_silent() {
            let mut rig = silent_rig("xio-silent-tracked");
            let name = "rollout-2026-10-06T10-00-00-aaaa.jsonl";
            let path = rollout_file(&rig.dir.path, name, SESSION_META);
            rig.hook(rig.payload("UserPromptSubmit", Some(&path)));
            assert!(matches!(rig.signals().last(), Some(Signal::CodexHook { .. })));
            rig.io.handle(change(name, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty());
        }

        #[test]
        fn subagents_zst_and_unreadable_first_lines_give_no_signal() {
            let mut rig = silent_rig("xio-silent-skip");
            let sub = SESSION_META.replace(r#""originator""#, r#""parent_thread_id":"bbbb","originator""#);
            rollout_file(&rig.dir.path, "rollout-sub.jsonl", &sub);
            rollout_file(&rig.dir.path, "rollout-bad.jsonl", "not json");
            rollout_file(&rig.dir.path, "rollout-noid.jsonl", r#"{"type":"session_meta","payload":{"cwd":"C:\\x"}}"#);
            rollout_file(&rig.dir.path, "rollout-z.jsonl.zst", SESSION_META);
            for name in ["rollout-sub.jsonl", "rollout-bad.jsonl", "rollout-noid.jsonl", "rollout-z.jsonl.zst"] {
                rig.io.handle(change(name, FileAction::Added));
            }
            assert!(silent_of(rig.signals()).is_empty());
        }

        #[test]
        fn removals_overflow_and_other_files_are_ignored() {
            let mut rig = silent_rig("xio-silent-ignored");
            rollout_file(&rig.dir.path, "rollout-r.jsonl", SESSION_META);
            rig.io.handle(change("rollout-r.jsonl", FileAction::Removed));
            rig.io.handle(CodexInput::Sessions(WatchEvent::Overflow));
            rig.io.handle(change("session_index.jsonl", FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty());
        }

        #[test]
        fn only_the_first_line_up_to_the_cap_is_read() {
            let mut rig = silent_rig("xio-silent-cap");
            let huge = format!(r#"{{"type":"session_meta","payload":{{"id":"x","pad":"{}"}}}}"#, "a".repeat(300 * 1024));
            rollout_file(&rig.dir.path, "rollout-huge.jsonl", &huge);
            rig.io.handle(change("rollout-huge.jsonl", FileAction::Added));
            assert!(silent_of(rig.signals()).is_empty());
        }

        #[test]
        fn an_empty_or_partial_first_line_is_read_again_on_the_next_event() {
            let mut rig = silent_rig("xio-silent-partial");
            let name = "rollout-late.jsonl";
            let path = rollout_file(&rig.dir.path, name, "");
            std::fs::write(&path, "").unwrap();
            rig.io.handle(change(name, FileAction::Added));
            std::fs::write(&path, &SESSION_META[..40]).unwrap();
            rig.io.handle(change(name, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty());
            std::fs::write(&path, format!("{SESSION_META}\n")).unwrap();
            rig.io.handle(change(name, FileAction::Modified));
            assert_eq!(silent_of(rig.signals()).len(), 1);
        }

        #[test]
        fn a_session_removed_after_its_hook_never_comes_back_as_silent() {
            let mut rig = silent_rig("xio-silent-ghost");
            let name = "rollout-2026-10-06T10-00-00-aaaa.jsonl";
            let path = rollout_file(&rig.dir.path, name, SESSION_META);
            rig.hook(rig.payload("UserPromptSubmit", Some(&path)));
            rig.io.handle(CodexInput::Effect(Effect::UntrackRollout { key: codex_key(SID) }));
            rig.signals();
            rig.io.handle(change(name, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty());
        }

        #[test]
        fn a_first_line_without_an_id_degrades_health_and_subagents_do_not() {
            let mut rig = silent_rig("xio-silent-health");
            let sub = SESSION_META.replace(r#""originator""#, r#""parent_thread_id":"bbbb","originator""#);
            rollout_file(&rig.dir.path, "rollout-sub.jsonl", &sub);
            rollout_file(&rig.dir.path, "rollout-z.jsonl.zst", SESSION_META);
            rig.io.handle(change("rollout-sub.jsonl", FileAction::Added));
            rig.io.handle(change("rollout-z.jsonl.zst", FileAction::Added));
            assert!(rig.signals().iter().all(|s| !matches!(s, Signal::SourceHealth { .. })));
            rollout_file(&rig.dir.path, "rollout-noid.jsonl", r#"{"type":"session_meta","payload":{}}"#);
            rig.io.handle(change("rollout-noid.jsonl", FileAction::Added));
            assert_eq!(
                rig.signals(),
                [Signal::SourceHealth { source: Source::Codex, health: Health::Degraded { unreadable: 1 } }]
            );
        }

        #[test]
        fn retry_starts_the_sessions_watch_once_the_folder_exists() {
            let mut rig = silent_rig("xio-silent-retry");
            let sessions = rig.dir.path.join("sessions");
            let (tx, rx) = mpsc::channel();
            rig.io.watcher_tx = Some(tx);
            rig.io.handle(CodexInput::Retry);
            assert!(!rig.io.watching, "no folder, no watch");
            std::fs::create_dir_all(&sessions).unwrap();
            rig.io.handle(CodexInput::Retry);
            assert!(rig.io.watching);
            std::fs::write(sessions.join("rollout-x.jsonl"), "x\n").unwrap();
            let got = rx.recv_timeout(Duration::from_secs(5));
            assert!(matches!(got, Ok(CodexInput::Sessions(WatchEvent::Changes(_)))), "{got:?}");
        }

        #[test]
        fn the_silent_cache_is_bounded() {
            let mut rig = silent_rig("xio-silent-bound");
            for i in 0..(SILENT_FILES_MAX + 5) {
                let name = format!("rollout-{i}.jsonl");
                rollout_file(&rig.dir.path, &name, SESSION_META);
                rig.io.handle(change(&name, FileAction::Added));
            }
            assert_eq!(rig.io.silent_files.len(), SILENT_FILES_MAX);
        }
    }
}
