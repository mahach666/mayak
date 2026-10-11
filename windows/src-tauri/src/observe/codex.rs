// Codex source (spec §5.6): hook events forwarded by the relay, rollout records (legacy and
// paginated mode), subagent meta, host classification and session_index titles. These
// functions are pure: payloads, lines, paths and %LOCALAPPDATA% come in as arguments.

use std::path::Path;

use serde::Deserialize;
use serde_json::{Map, Value};

use super::keys::HostKind;
use super::model::{CodexWindow, ContextFill};
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

#[derive(Debug, Clone, PartialEq)] // no Eq: CodexWindow carries an f64 percent
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
    /// A `response_item` `message` whose `role` is "user": the turn is a user turn. Nothing of the
    /// message is kept.
    UserMessage,
    /// `event_msg` `token_count`: how full the context is (`info.last_token_usage.total_tokens`,
    /// `window` = `info.model_context_window` when it is an integer > 0; context-fill spec §2.1) and
    /// the account's limit windows (`rate_limits.primary` / `secondary` of the `codex` bucket;
    /// usage-home spec §2.2). At least one of the two is Some. Only these numbers are read.
    Tokens { context: Option<ContextFill>, limits: Option<Vec<CodexWindow>> },
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
/// - `event_msg` + `payload.type` token_count → Tokens when it has a context fill (an unsigned integer
///   `info.last_token_usage.total_tokens`; `window`: `info.model_context_window` when it is an integer
///   > 0) or at least one limit window (`rate_limits`); any other token_count → Other;
/// - `response_item` + `payload.type` function_call (label `tool_label(name, arguments parsed
///   as JSON)`), custom_tool_call (label `tool_label(name, None)`, i.e. the name), local_shell_call
///   (label `tool_label("shell", payload.action)`) → Call; `call_id` "" when absent. A function_call or
///   custom_tool_call named `request_permissions`, or a custom_tool_call named `exec` whose `input` is a
///   string containing that word, is a PermissionAsk instead, with the same `call_id` and label; only that
///   boolean is computed, the text is neither kept nor logged. The `arguments` of a function_call never
///   count, nor does the `input` of any other custom tool: a command that mentions the word stays a Call;
/// - `response_item` + `payload.type` function_call_output | custom_tool_call_output with a string
///   `call_id` → CallDone; with any other `call_id` → Other;
/// - `response_item` + `payload.type` message with `payload.role` "user" → UserMessage (only the
///   role is looked at, never the text); a message of any other role → Other;
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
        ("event_msg", Some(p), Some("token_count")) => match (context_fill(p), rate_limits(p)) {
            (None, None) => RolloutRecord::Other,
            (context, limits) => RolloutRecord::Tokens { context, limits },
        },
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
        ("response_item", Some(p), Some("message")) if p.get("role").and_then(Value::as_str) == Some("user") => {
            RolloutRecord::UserMessage
        }
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

/// The context fill of a `token_count` payload: `info.last_token_usage.total_tokens` as a u64 and
/// `info.model_context_window` when it is an integer > 0. None when `info` is null or absent or has
/// no such `total_tokens`.
fn context_fill(p: &Map<String, Value>) -> Option<ContextFill> {
    let info = p.get("info")?.as_object()?;
    let used = info.get("last_token_usage")?.get("total_tokens")?.as_u64()?;
    let window = info.get("model_context_window").and_then(Value::as_u64).filter(|w| *w > 0);
    Some(ContextFill { used, window })
}

/// The `rate_limits.limit_id` of the account's own limits. Codex also writes per-model buckets
/// (for example "codex_bengalfox") and empty ones ("premium") into the same rollout.
const CODEX_LIMIT_ID: &str = "codex";

/// The account's limit windows of a `token_count` payload (usage-home spec §2.2): `primary`, then
/// `secondary`, each kept by `codex_window`. None when `rate_limits` is no object, when its
/// `limit_id` is a string other than CODEX_LIMIT_ID, or when no window is kept. Only `limit_id`,
/// `primary` and `secondary` are looked at.
fn rate_limits(p: &Map<String, Value>) -> Option<Vec<CodexWindow>> {
    let limits = p.get("rate_limits")?.as_object()?;
    if limits.get("limit_id").and_then(Value::as_str).is_some_and(|id| id != CODEX_LIMIT_ID) {
        return None;
    }
    let windows: Vec<CodexWindow> =
        ["primary", "secondary"].into_iter().filter_map(|k| limits.get(k)).filter_map(codex_window).collect();
    (!windows.is_empty()).then_some(windows)
}

/// One window: a number `used_percent` and an integer `window_minutes` > 0, else None; `resets_at`
/// (seconds since the epoch) becomes milliseconds when it is an integer.
fn codex_window(v: &Value) -> Option<CodexWindow> {
    let percent = v.get("used_percent")?.as_f64()?;
    let minutes = v.get("window_minutes")?.as_u64().filter(|m| *m > 0)?;
    let resets_at_ms = v.get("resets_at").and_then(Value::as_i64).and_then(|s| s.checked_mul(1_000));
    Some(CodexWindow { minutes, percent, resets_at_ms })
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
                UserMessage, // response_item message, role user
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
    fn only_a_user_role_message_is_a_user_message() {
        let message = |role: Value| response_item(json!({ "type": "message", "role": role, "content": [] }));
        assert_eq!(message(json!("user")), RolloutRecord::UserMessage);
        for role in [json!("assistant"), json!("developer"), json!("User"), Value::Null, json!(["user"])] {
            assert_eq!(message(role.clone()), RolloutRecord::Other, "role {role}");
        }
        assert_eq!(response_item(json!({ "type": "message", "content": [] })), RolloutRecord::Other, "no role");
        // The legacy event_msg user_message is not used.
        let legacy = parse_rollout_line(&json!({ "type": "event_msg", "payload": { "type": "user_message", "role": "user" } }));
        assert_eq!(legacy, RolloutRecord::Other);
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

    /// A paginated-mode `token_count` event with the given `info`.
    fn token_count(info: Value) -> RolloutRecord {
        parse_rollout_line(&json!({
            "ordinal": 7, "timestamp": "2026-10-06T10:00:00.000Z", "type": "event_msg",
            "payload": { "type": "token_count", "info": info, "rate_limits": {} }
        }))
    }

    /// The record of a token_count with this context fill and no limits.
    fn fill(used: u64, window: Option<u64>) -> RolloutRecord {
        RolloutRecord::Tokens { context: Some(ContextFill { used, window }), limits: None }
    }

    #[test]
    fn a_token_count_with_info_gives_the_fill_and_the_window() {
        // The keys Codex 0.160 writes (numbers generic): only the last request is the fill.
        let info = json!({
            "total_token_usage": { "total_tokens": 900_000 },
            "last_token_usage": {
                "input_tokens": 61_740, "cached_input_tokens": 61_568, "cache_write_input_tokens": 0,
                "output_tokens": 27, "reasoning_output_tokens": 0, "total_tokens": 61_767
            },
            "model_context_window": 258_400
        });
        assert_eq!(token_count(info), fill(61_767, Some(258_400)));
    }

    #[test]
    fn a_token_count_without_info_or_total_tokens_stays_other() {
        assert_eq!(token_count(Value::Null), RolloutRecord::Other, "info null: before the first answer");
        assert_eq!(token_count(json!({})), RolloutRecord::Other, "the empty info of the fixtures");
        assert_eq!(token_count(json!({ "last_token_usage": {}, "model_context_window": 258_400 })), RolloutRecord::Other);
        assert_eq!(
            token_count(json!({ "total_token_usage": { "total_tokens": 5 } })),
            RolloutRecord::Other,
            "the whole-thread total is no fill"
        );
        let absent = parse_rollout_line(&json!({ "type": "event_msg", "payload": { "type": "token_count" } }));
        assert_eq!(absent, RolloutRecord::Other);
    }

    #[test]
    fn a_token_count_window_is_kept_only_when_it_is_a_positive_integer() {
        let with_window = |window: Value| {
            token_count(json!({ "last_token_usage": { "total_tokens": 10 }, "model_context_window": window }))
        };
        let no_window = fill(10, None);
        assert_eq!(token_count(json!({ "last_token_usage": { "total_tokens": 10 } })), no_window);
        for window in [json!(0), json!(-1), json!(1.5), json!("258400"), Value::Null] {
            assert_eq!(with_window(window.clone()), no_window, "window {window}");
        }
        assert_eq!(with_window(json!(1)), fill(10, Some(1)));
    }

    #[test]
    fn a_token_count_whose_total_is_not_an_unsigned_integer_stays_other() {
        for total in [json!(-5), json!(2.5), json!("61767"), Value::Null, json!([1])] {
            let record = token_count(json!({ "last_token_usage": { "total_tokens": total }, "model_context_window": 258_400 }));
            assert_eq!(record, RolloutRecord::Other, "total_tokens {total}");
        }
        assert_eq!(token_count(json!({ "last_token_usage": { "total_tokens": 0 } })), fill(0, None));
    }

    /// A paginated-mode `token_count` event before the first answer, with the given `rate_limits`.
    fn limits(rate_limits: Value) -> RolloutRecord {
        parse_rollout_line(&json!({
            "ordinal": 8, "timestamp": "2026-10-06T10:00:00.000Z", "type": "event_msg",
            "payload": { "type": "token_count", "info": null, "rate_limits": rate_limits }
        }))
    }

    fn window(minutes: u64, percent: f64, resets_at_ms: Option<i64>) -> CodexWindow {
        CodexWindow { minutes, percent, resets_at_ms }
    }

    #[test]
    fn a_token_count_gives_the_codex_limit_windows() {
        // The keys Codex 0.160 writes (values generic); only primary and secondary are read.
        let rate_limits = json!({
            "limit_id": "codex", "limit_name": null,
            "primary": { "used_percent": 12, "window_minutes": 300, "resets_at": 1_791_300_000 },
            "secondary": { "used_percent": 37.5, "window_minutes": 10_080, "resets_at": 1_791_800_000 },
            "credits": { "has_credits": true, "unlimited": false, "balance": "5" },
            "individual_limit": null, "spend_control_reached": null, "plan_type": "plus", "rate_limit_reached_type": null
        });
        let expected = vec![window(300, 12.0, Some(1_791_300_000_000)), window(10_080, 37.5, Some(1_791_800_000_000))];
        assert_eq!(limits(rate_limits.clone()), RolloutRecord::Tokens { context: None, limits: Some(expected.clone()) });
        // with a context fill the record carries both
        let both = parse_rollout_line(&json!({ "type": "event_msg", "payload": {
            "type": "token_count",
            "info": { "last_token_usage": { "total_tokens": 61_767 }, "model_context_window": 258_400 },
            "rate_limits": rate_limits
        } }));
        assert_eq!(
            both,
            RolloutRecord::Tokens { context: Some(ContextFill { used: 61_767, window: Some(258_400) }), limits: Some(expected) }
        );
    }

    #[test]
    fn a_null_or_malformed_limit_window_is_skipped() {
        let week = json!({ "used_percent": 40, "window_minutes": 10_080, "resets_at": 1_791_800_000 });
        let only_week = RolloutRecord::Tokens { context: None, limits: Some(vec![window(10_080, 40.0, Some(1_791_800_000_000))]) };
        assert_eq!(limits(json!({ "primary": null, "secondary": week })), only_week);
        for bad in [
            json!({ "used_percent": "12", "window_minutes": 300 }),
            json!({ "window_minutes": 300 }),
            json!({ "used_percent": 12, "window_minutes": 0 }),
            json!({ "used_percent": 12, "window_minutes": -300 }),
            json!({ "used_percent": 12, "window_minutes": 1.5 }),
            json!({ "used_percent": 12 }),
            json!(12),
        ] {
            assert_eq!(limits(json!({ "primary": bad, "secondary": week })), only_week, "primary {bad}");
        }
        for resets_at in [json!("1791800000"), json!(1.5), Value::Null] {
            let w = json!({ "used_percent": 40, "window_minutes": 10_080, "resets_at": resets_at });
            let no_reset = RolloutRecord::Tokens { context: None, limits: Some(vec![window(10_080, 40.0, None)]) };
            assert_eq!(limits(json!({ "primary": w })), no_reset, "resets_at {resets_at}");
        }
        // no usable window: a token_count before the first answer stays Other
        assert_eq!(limits(json!({ "primary": null, "secondary": null })), RolloutRecord::Other);
        assert_eq!(limits(json!({})), RolloutRecord::Other);
        assert_eq!(limits(Value::Null), RolloutRecord::Other);
        let absent = parse_rollout_line(&json!({ "type": "event_msg", "payload": { "type": "token_count", "info": null } }));
        assert_eq!(absent, RolloutRecord::Other, "no rate_limits");
    }

    #[test]
    fn only_the_codex_limit_bucket_counts() {
        // Codex also writes per-model buckets (`limit_id` "codex_bengalfox") and empty ones ("premium")
        // into the same rollout; only the account's `codex` bucket, or a Codex that writes no
        // `limit_id`, is the home page's.
        let primary = json!({ "used_percent": 12, "window_minutes": 300, "resets_at": 1_791_300_000 });
        let kept = RolloutRecord::Tokens { context: None, limits: Some(vec![window(300, 12.0, Some(1_791_300_000_000))]) };
        assert_eq!(limits(json!({ "limit_id": "codex", "primary": primary })), kept);
        assert_eq!(limits(json!({ "primary": primary })), kept, "no limit_id");
        assert_eq!(limits(json!({ "limit_id": null, "primary": primary })), kept, "limit_id null");
        for other in ["codex_bengalfox", "premium", "Codex", ""] {
            let record = limits(json!({ "limit_id": other, "limit_name": "GPT-5.3-Codex-Spark", "primary": primary }));
            assert_eq!(record, RolloutRecord::Other, "limit_id {other}");
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
    use crate::observe::keys::{antigravity_key, codex_key, HostKind, ProcId, SessionKey};
    use crate::observe::claude_usage::parse_rfc3339_ms;
    use crate::observe::model::{CodexUsage, CodexWindow, Effect, Health, Signal, Source};
    use crate::observe::paths::{self, ProjectCache, ProjectRef};
    use crate::observe::silent::{host_of, parse_session_meta};
    use crate::observe::tail::JsonlTail;
    use crate::observe::terminal::TerminalCache;
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
        /// The terminal of a proved agent process whose image names no host
        /// (`terminal::of_process`; tests pass a fixed answer).
        pub terminal_of: fn(u32) -> HostKind,
    }

    /// The first line of a rollout is read up to this size; longer first lines are not silent rows.
    const FIRST_LINE_CAP: usize = 256 * 1024;

    /// Rollout files remembered by the sessions watcher; the oldest inserted is dropped first.
    const SILENT_FILES_MAX: usize = 256;

    /// One rollout file the watcher has looked at (kept so its first line is read once).
    enum SilentFile {
        /// Subagent, unreadable or not a session: never a row.
        Skip,
        /// `offset`: where the next read of the rollout starts (the end of the last complete line
        /// already looked at).
        Session { id: String, project: ProjectRef, host: HostKind, folder_exists: bool, offset: u64, turn: TurnScan },
    }

    /// The turn of an untracked rollout as its lines are read, kept between events (a turn's start
    /// and its end may fall in different events).
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    struct TurnScan {
        /// A `task_started` was read and its turn has not ended.
        started: bool,
        /// A user-role message was read in that turn: it is a user turn.
        user: bool,
    }

    impl TurnScan {
        /// Feeds one record; true when it ends a started user turn.
        fn feed(&mut self, record: &RolloutRecord) -> bool {
            match record {
                RolloutRecord::TaskStarted { .. } => {
                    *self = TurnScan { started: true, user: false };
                    false
                }
                RolloutRecord::UserMessage => {
                    self.user = true;
                    false
                }
                RolloutRecord::TaskComplete { .. } | RolloutRecord::TurnAborted { .. } => {
                    let silent = self.started && self.user;
                    *self = TurnScan::default();
                    silent
                }
                _ => false,
            }
        }
    }

    /// At most this much of a rollout is read per event; more pending than that is skipped.
    const SCAN_CAP: u64 = 1024 * 1024;

    /// The record of a complete rollout line, when it can matter to `TurnScan` (a turn start or end,
    /// or a user-role message); `Other` for the rest. A byte check skips parsing the other lines.
    /// Only the record kind is kept, never the text.
    fn turn_record(line: &[u8]) -> RolloutRecord {
        let mentions = |needle: &[u8]| line.windows(needle.len()).any(|w| w == needle);
        let candidate = [b"task_started".as_slice(), b"task_complete", b"turn_aborted", b"\"user\""]
            .into_iter()
            .any(mentions);
        if !candidate {
            return RolloutRecord::Other;
        }
        match serde_json::from_slice::<Value>(line) {
            Ok(value) => parse_rollout_line(&value),
            Err(_) => RolloutRecord::Other,
        }
    }

    /// Reads the complete lines of `path` after `*offset` and moves `*offset` past them, feeding
    /// `turn`; true when one of them ends a user turn whose start was read (`TurnScan::feed`). A
    /// partial last line is left for the next event; a truncated file resets the offset to its end
    /// and more than `SCAN_CAP` pending is skipped, both with the turn state cleared. Never blocks.
    fn scan_turn_end(path: &Path, offset: &mut u64, turn: &mut TurnScan) -> bool {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(err) => {
                log::line(format!("codex rollout unreadable: {:?}", err.kind()));
                return false;
            }
        };
        let len = match file.metadata() {
            Ok(meta) => meta.len(),
            Err(err) => {
                log::line(format!("codex rollout unreadable: {:?}", err.kind()));
                return false;
            }
        };
        if len == *offset {
            return false;
        }
        if len < *offset || len - *offset > SCAN_CAP {
            // Truncated or too much pending: the lines in between are not looked at.
            *offset = len;
            *turn = TurnScan::default();
            return false;
        }
        let mut buf = Vec::with_capacity((len - *offset) as usize);
        let read = file.seek(SeekFrom::Start(*offset)).and_then(|_| file.take(len - *offset).read_to_end(&mut buf));
        if let Err(err) = read {
            log::line(format!("codex rollout unreadable: {:?}", err.kind()));
            return false;
        }
        let Some(end) = buf.iter().rposition(|b| *b == b'\n') else { return false };
        *offset += end as u64 + 1;
        // Every line is fed (no short-circuit): the turn state must follow the whole batch.
        buf[..end].split(|b| *b == b'\n').fold(false, |silent, line| turn.feed(&turn_record(line)) || silent)
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

    /// At most this much of the end of the newest rollout is read to seed the usage at start.
    const SEED_TAIL: u64 = 256 * 1024;

    /// The `rollout-*.jsonl` under `dir` (recursive) with the latest modified time; on a tie the
    /// greater path. `.zst` and any other file are not looked at. One walk, no file is opened; an
    /// unreadable folder or entry is logged by kind only and skipped.
    fn newest_rollout(dir: &Path) -> Option<(PathBuf, std::time::SystemTime)> {
        let mut newest: Option<(PathBuf, std::time::SystemTime)> = None;
        let mut folders = vec![dir.to_path_buf()];
        while let Some(folder) = folders.pop() {
            let entries = match std::fs::read_dir(&folder) {
                Ok(entries) => entries,
                Err(err) => {
                    log::line(format!("codex usage seed: folder unreadable: {:?}", err.kind()));
                    continue;
                }
            };
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(err) => {
                        log::line(format!("codex usage seed: entry unreadable: {:?}", err.kind()));
                        continue;
                    }
                };
                let path = entry.path();
                let kind = match entry.file_type() {
                    Ok(kind) => kind,
                    Err(err) => {
                        log::line(format!("codex usage seed: entry unreadable: {:?}", err.kind()));
                        continue;
                    }
                };
                if kind.is_dir() {
                    folders.push(path);
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if !kind.is_file() || !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                    continue;
                }
                let modified = match entry.metadata().and_then(|meta| meta.modified()) {
                    Ok(modified) => modified,
                    Err(err) => {
                        log::line(format!("codex usage seed: entry unreadable: {:?}", err.kind()));
                        continue;
                    }
                };
                if newest.as_ref().map_or(true, |(best, at)| (modified, &path) > (*at, best)) {
                    newest = Some((path, modified));
                }
            }
        }
        newest
    }

    /// The last 256 KiB of `path` as its complete lines: when the read starts inside the file, the
    /// first (partial) line is dropped.
    fn read_tail(path: &Path) -> io::Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        let start = len.saturating_sub(SEED_TAIL);
        // One byte before the window tells whether its first line starts at a line boundary.
        file.seek(SeekFrom::Start(start.saturating_sub(1)))?;
        let mut buf = Vec::new();
        file.take(len - start.saturating_sub(1)).read_to_end(&mut buf)?;
        if start > 0 {
            match buf.iter().position(|b| *b == b'\n') {
                Some(end) => {
                    buf.drain(..=end);
                }
                None => buf.clear(),
            }
        }
        Ok(buf)
    }

    /// The limit windows of the last line of `lines` (read from the end) that holds some, with the
    /// `timestamp` of that line in ms since the epoch when it has a valid one. Only numbers are
    /// kept; a line that does not parse is skipped.
    fn last_limits(lines: &[u8]) -> Option<(Vec<CodexWindow>, Option<i64>)> {
        lines.split(|b| *b == b'\n').rev().find_map(|line| {
            let value: Value = serde_json::from_slice(line.strip_suffix(b"\r").unwrap_or(line)).ok()?;
            match parse_rollout_line(&value) {
                RolloutRecord::Tokens { limits: Some(windows), .. } => {
                    let at = value.get("timestamp").and_then(Value::as_str).and_then(parse_rfc3339_ms);
                    Some((windows, at))
                }
                _ => None,
            }
        })
    }

    /// The Codex usage of the newest rollout under `sessions`, for a start with no live record yet:
    /// its last limits, stamped with the time of their record (the file's modified time when the
    /// record has no valid timestamp). None when there is no rollout or no limits in its tail.
    /// A seed stamped after `now_ms` (a clock skew in the rollout) is stamped `now_ms`: a stamp in the
    /// future would hide its age on the home page.
    fn not_after(mut usage: CodexUsage, now_ms: i64) -> CodexUsage {
        usage.seen_at_ms = usage.seen_at_ms.min(now_ms);
        usage
    }

    fn seed_usage(sessions: &Path) -> Option<CodexUsage> {
        let (path, modified) = newest_rollout(sessions)?;
        let tail = match read_tail(&path) {
            Ok(tail) => tail,
            Err(err) => {
                log::line(format!("codex usage seed: rollout unreadable: {:?}", err.kind()));
                return None;
            }
        };
        let (windows, at) = last_limits(&tail)?;
        let seen_at_ms = match at {
            Some(ms) => ms,
            None => match modified.duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_millis()).ok()) {
                Some(ms) => ms,
                None => {
                    log::line("codex usage seed: modified time out of range");
                    return None;
                }
            },
        };
        Some(CodexUsage { seen_at_ms, windows })
    }

    #[derive(Debug)]
    pub enum CodexInput {
        Hook(Value),
        /// An Antigravity relay line: only its project is resolved here (observe/antigravity.rs).
        Antigravity(Value),
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
        /// The `cwd` of the rollout's `session_meta` line.
        cwd: MetaCwd,
    }

    /// The `session_meta` cwd of a tracked rollout, read from its first line once.
    enum MetaCwd {
        /// Not read yet, or the first line was not complete: read again with the next batch.
        Unread,
        /// The first line was read: its cwd; None when it is no session_meta or has no cwd.
        Read(Option<String>),
    }

    impl Rollout {
        /// Whether the session's folder exists now: the meta cwd is a directory. The first line is
        /// read once; the folder is checked on every call, since it can be deleted mid-session.
        fn folder_exists(&mut self) -> bool {
            if matches!(self.cwd, MetaCwd::Unread) {
                self.cwd = match read_first_line(Path::new(&self.display)) {
                    FirstLine::Complete(line) => MetaCwd::Read(parse_session_meta(&line).and_then(|meta| meta.cwd)),
                    FirstLine::TooLong => MetaCwd::Read(None),
                    FirstLine::Incomplete => MetaCwd::Unread,
                };
            }
            matches!(&self.cwd, MetaCwd::Read(Some(cwd)) if !cwd.is_empty() && Path::new(cwd).is_dir())
        }
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
        /// The terminal of each proved agent process whose image names no host, walked once.
        terminals: TerminalCache,
        /// CodexTicker(true) is in force: `run` reads every rollout once per TICK.
        ticker: bool,
        /// Last health sent; the store starts at Ok.
        health: Health,
        /// Rollouts seen by the sessions watcher, by normalized path key, and their insertion order.
        silent_files: HashMap<String, SilentFile>,
        silent_order: VecDeque<String>,
        /// Rollouts whose first line cannot be read as a session: health (spec silent-codex section 5).
        silent_unreadable: HashSet<String>,
        /// Rollouts first seen through `Added` whose first line was not complete yet: when it is,
        /// they are read from offset 0 (a new rollout), not from their end.
        silent_new: HashSet<String>,
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
                terminals: TerminalCache::new(),
                ticker: false,
                health: Health::Ok,
                silent_files: HashMap::new(),
                silent_order: VecDeque::new(),
                silent_unreadable: HashSet::new(),
                silent_new: HashSet::new(),
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
                CodexInput::Antigravity(payload) => self.antigravity(payload),
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
                    // The watcher thread has ended; let the next Retry start it again.
                    self.watching = false;
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
            let host = match (classify_host(event.agent_image.as_deref(), &self.cfg.localappdata), &proven) {
                (HostKind::Unknown, Some((id, _))) => self.terminals.host(id, self.cfg.terminal_of),
                (host, _) => host,
            };
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

        /// An Antigravity hook: its session key, project, host and process; on Stop the last answer
        /// of its transcript (never logged). Then to the store, and the process is watched as a
        /// Codex one is, so the sessions go when Antigravity exits.
        fn antigravity(&mut self, payload: Value) {
            use crate::observe::antigravity as ag;

            let len = payload.to_string().len();
            let mut event = match ag::parse(payload) {
                Ok(event) => event,
                Err(reason) => {
                    log::line(format!("antigravity hook dropped: len={len} reason={reason}"));
                    return;
                }
            };
            let key = antigravity_key(&event.session_id);
            let proven = self.prove_pid("antigravity", event.agent_pid, event.agent_created);
            let host = ag::host_of(event.transcript_path.as_deref(), event.agent_image.as_deref());
            let project = if event.cwd.trim().is_empty() {
                ag::no_workspace_project()
            } else {
                self.cache.get(&event.cwd, paths::git_probe)
            };
            if event.hook_event_name == "Stop" {
                if let Some(path) = event.transcript_path.as_deref() {
                    match ag::read_last_answer(path) {
                        Ok(answer) => event.last_assistant_message = answer,
                        Err(reason) => log::line(format!("antigravity transcript not read: {reason}")),
                    }
                }
            }
            log::line(format!(
                "antigravity {} s={} host={host:?} proc={}",
                event.hook_event_name,
                short(&event.session_id),
                if proven.is_some() { "yes" } else { "no" }
            ));
            let proc = proven.as_ref().map(|(id, _)| id.clone());
            self.emit(Signal::AntigravityHook { key, event, project, proc, host });
            if let Some((id, handle)) = proven {
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
            self.prove_pid("codex", event.agent_pid, event.agent_created)
        }

        /// identify(pid) with a creation time equal to `created`; `who` names the agent in the log.
        fn prove_pid(&self, who: &str, pid: Option<u32>, created: Option<u64>) -> Option<(ProcId, OwnedHandle)> {
            let (pid, created) = (pid?, created?);
            match proc::identify(pid) {
                Ok((id, handle)) if id.created_ft == created => Some((id, handle)),
                Ok(_) => {
                    log::line(format!("{who} agent {pid}: creation time differs, not proved"));
                    None
                }
                Err(err) => {
                    log::line(format!("{who} agent {pid} not identified: {err:?}"));
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
        /// A rollout under `sessions\` that no hook tracks is a silent session when a user turn of it
        /// ends (task_complete / turn_aborted after a task_started and a user-role message): a
        /// hooked session has always delivered its hooks by then. Its first line is read once and cached; every event reads only the complete lines
        /// appended since the last one. One signal per event, however many turn ends it holds. A
        /// rollout a hook tracks gets its new records read instead (`drain_path`).
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
                if !is_rollout_name(&norm.key) {
                    continue;
                }
                if self.rollouts.contains_key(&norm.key) {
                    // The rollout of a hooked session: its new records go to the model through the
                    // same tail as the ticker's and the hooks' reads, so a turn that ends without
                    // any hook is seen there (silent-codex Task 7).
                    self.drain_path(&norm.key);
                    continue;
                }
                if !self.silent_files.contains_key(&norm.key) {
                    // A new rollout is read from its start, an existing one (Mayak restarted) from
                    // its end: old history never fires.
                    let from_start = *action == FileAction::Added || self.silent_new.contains(&norm.key);
                    let Some((mut file, unreadable)) = self.read_silent_file(&full) else {
                        if from_start {
                            if self.silent_new.len() >= SILENT_FILES_MAX {
                                self.silent_new.clear();
                            }
                            self.silent_new.insert(norm.key.clone());
                        }
                        continue;
                    };
                    if let SilentFile::Session { offset, .. } = &mut file {
                        if !from_start {
                            match std::fs::metadata(&full) {
                                Ok(meta) => *offset = meta.len(),
                                Err(err) => {
                                    log::line(format!("codex rollout unreadable: {:?}", err.kind()));
                                    continue;
                                }
                            }
                        }
                    }
                    self.silent_new.remove(&norm.key);
                    self.remember_silent(&norm.key, file, unreadable);
                }
                let Some(SilentFile::Session { id, project, host, folder_exists, offset, turn }) =
                    self.silent_files.get_mut(&norm.key)
                else {
                    continue;
                };
                if !scan_turn_end(&full, offset, turn) {
                    continue;
                }
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

        /// Once at start: the limits of the newest rollout on disk, so that the home page has a value
        /// before Codex writes a new one. Any live record the store gets later replaces it.
        fn seed_usage(&mut self) {
            if let Some(usage) = seed_usage(&self.cfg.codex_home.join("sessions")) {
                log::line("codex-io: usage seeded from the newest rollout");
                self.emit(Signal::CodexUsageSeed(not_after(usage, crate::observe::now_ms())));
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
            let file = SilentFile::Session { id: meta.id, project, host, folder_exists, offset: 0, turn: TurnScan::default() };
            Some((file, false))
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
                let rollout = Rollout { tail, display: norm.display, unknown: false, cwd: MetaCwd::Unread };
                self.rollouts.insert(norm.key.clone(), rollout);
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
            let folder_exists = rollout.folder_exists();
            let mut keys: Vec<SessionKey> = self
                .main_rollout
                .iter()
                .filter(|(_, p)| p.as_str() == path_key)
                .map(|(k, _)| k.clone())
                .collect();
            keys.sort();
            for key in keys {
                self.emit(Signal::CodexRollout { key, path: path.clone(), records: records.clone(), eof, folder_exists });
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
                log::line(format!("codex-io: health {}", health.log_label()));
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
            io.seed_usage();
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
        fn a_seed_stamped_in_the_future_is_stamped_now() {
            let usage = CodexUsage { seen_at_ms: 2_000, windows: vec![] };
            assert_eq!(not_after(usage.clone(), 1_500).seen_at_ms, 1_500);
            assert_eq!(not_after(usage, 3_000).seen_at_ms, 2_000);
        }

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
            rig_with(tag, |_| HostKind::Unknown)
        }

        fn rig_with(tag: &str, terminal_of: fn(u32) -> HostKind) -> Rig {
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
            let cfg = CodexConfig { codex_home: home, localappdata: dir.path.join("local"), watch: false, terminal_of };
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
                Signal::CodexRollout { key, path, records, eof, .. } => {
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
        fn only_a_proved_agent_whose_image_names_no_host_is_asked_its_terminal() {
            let host_of = |signals: &[Signal]| match signals.last() {
                Some(Signal::CodexHook { host, .. }) => *host,
                other => panic!("{other:?}"),
            };
            let mut rig = rig_with("xio-terminal", |_| HostKind::PowerShell);
            rig.hook(rig.payload("UserPromptSubmit", None));
            assert_eq!(host_of(&rig.signals()), HostKind::PowerShell, "proved, the test binary names no host");
            let mut rig = rig_with("xio-terminal-unproved", |_| HostKind::PowerShell);
            let mut payload = rig.payload("UserPromptSubmit", None);
            payload["agent_created"] = (rig.me.created_ft + 1).into();
            rig.hook(payload);
            assert_eq!(host_of(&rig.signals()), HostKind::Unknown, "an unproved pid is never walked");
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
        fn an_antigravity_line_gets_its_project_and_goes_to_the_store() {
            let mut rig = rig("xio-antigravity");
            let sub = rig.dir.path.join("work").join("src");
            std::fs::create_dir_all(&sub).unwrap();
            rig.io.handle(CodexInput::Antigravity(serde_json::json!({
                "agent": "antigravity", "hook_event_name": "PreInvocation", "relay": 2, "relay_ts": 7,
                "session_id": "conv-1", "cwd": sub.to_string_lossy(),
            })));
            let hooks: Vec<Signal> =
                rig.signals().into_iter().filter(|s| matches!(s, Signal::AntigravityHook { .. })).collect();
            let [Signal::AntigravityHook { key, event, project, proc, host }] = hooks.as_slice() else { panic!("{hooks:?}") };
            assert_eq!(key, "antigravity:conv-1");
            assert_eq!((event.hook_event_name.as_str(), event.relay_ts), ("PreInvocation", 7));
            assert_eq!(project.root, paths::normalize(&rig.cwd()).display, "the git root above the workspace");
            assert_eq!((proc, host), (&None, &HostKind::Unknown));

            // Stop: the process above the relay is proved, the host is the IDE (its transcript
            // folder), and the last answer is read from the transcript.
            let logs = rig.dir.path.join(".gemini").join("antigravity").join("brain").join("conv-1").join("logs");
            std::fs::create_dir_all(&logs).unwrap();
            let transcript = logs.join("transcript.jsonl");
            std::fs::write(
                &transcript,
                "{\"source\":\"USER_EXPLICIT\",\"type\":\"USER_INPUT\",\"content\":\"q\"}\n\
                 {\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"content\":\"Renamed the file.\"}\n",
            )
            .unwrap();
            rig.io.handle(CodexInput::Antigravity(serde_json::json!({
                "agent": "antigravity", "hook_event_name": "Stop", "relay": 2, "relay_ts": 8,
                "session_id": "conv-1", "transcript_path": transcript.to_string_lossy(),
                "agent_pid": rig.me.pid, "agent_created": rig.me.created_ft, "agent_image": rig.me.image,
            })));
            let hooks: Vec<Signal> =
                rig.signals().into_iter().filter(|s| matches!(s, Signal::AntigravityHook { .. })).collect();
            let [Signal::AntigravityHook { event, project, proc, host, .. }] = hooks.as_slice() else { panic!("{hooks:?}") };
            assert_eq!(event.last_assistant_message.as_deref(), Some("Renamed the file."));
            assert_eq!(proc.as_ref().map(|p| p.pid), Some(rig.me.pid));
            assert_eq!(*host, HostKind::AntigravityIde);
            assert_eq!(*project, crate::observe::antigravity::no_workspace_project(), "no workspace, never the relay's folder");
            // A line without a conversation is dropped.
            rig.io.handle(CodexInput::Antigravity(serde_json::json!({ "hook_event_name": "Stop", "relay_ts": 8 })));
            assert!(!rig.signals().iter().any(|s| matches!(s, Signal::AntigravityHook { .. })));
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
            let cfg = CodexConfig { codex_home: rig.dir.path.join("codex"), localappdata: rig.dir.path.clone(), watch: false, terminal_of: |_| HostKind::Unknown };
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
            std::fs::write(&path, format!("{first_line}\n{SETTINGS}\n")).unwrap();
            path
        }

        const SETTINGS: &str = r#"{"type":"event_msg","payload":{"type":"thread_settings_applied"}}"#;
        const ABORTED: &str = r#"{"type":"event_msg","payload":{"type":"turn_aborted","turn_id":"t2"}}"#;
        const USER_MSG: &str = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}}"#;

        fn append_raw(path: &Path, text: &str) {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
            file.write_all(text.as_bytes()).unwrap();
        }

        fn append_lines(path: &Path, lines: &[&str]) {
            append_raw(path, &lines.iter().map(|l| format!("{l}\n")).collect::<String>());
        }

        fn change(name: &str, action: FileAction) -> CodexInput {
            CodexInput::Sessions(WatchEvent::Changes(vec![(action, format!(r"2026\10\06\{name}"))]))
        }

        fn silent_rig(tag: &str) -> Rig {
            let mut rig = rig(tag);
            // codex_home = the temp dir itself, so `rollout_file` lands under <codex_home>\sessions.
            rig.io = CodexIo::new(
                CodexConfig { codex_home: rig.dir.path.clone(), localappdata: rig.dir.path.join("local"), watch: false, terminal_of: |_| HostKind::Unknown },
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

        const NAME: &str = "rollout-2026-10-06T10-00-00-aaaa.jsonl";

        #[test]
        fn settings_and_a_started_turn_give_no_signal_and_a_turn_end_gives_one() {
            let mut rig = silent_rig("xio-silent-new");
            let path = rollout_file(&rig.dir.path, NAME, SESSION_META);
            rig.io.handle(change(NAME, FileAction::Added));
            assert!(silent_of(rig.signals()).is_empty(), "meta + thread_settings_applied");
            append_lines(&path, &[TASK_STARTED, USER_MSG]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty(), "task_started and the user message");
            append_lines(&path, &[TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            let silent = silent_of(rig.signals());
            assert_eq!(silent.len(), 1);
            let Signal::CodexSilent { id, folder_exists, host, .. } = &silent[0] else { unreachable!() };
            assert_eq!(id, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
            assert!(!folder_exists);
            assert_eq!(*host, HostKind::CodexDesktop);
            // Nothing new was written: the same turn end never fires twice.
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty());
        }

        #[test]
        fn the_first_line_is_read_once() {
            let mut rig = silent_rig("xio-silent-once");
            let path = rollout_file(&rig.dir.path, NAME, SESSION_META);
            let before = std::fs::metadata(&path).unwrap().len() as usize;
            rig.io.handle(change(NAME, FileAction::Added));
            // The first line is now garbage of the same length: only the cache still knows the id.
            std::fs::write(&path, format!("{}\n{TASK_STARTED}\n{USER_MSG}\n{TASK_COMPLETE}\n", "x".repeat(before - 1))).unwrap();
            rig.io.handle(change(NAME, FileAction::Modified));
            assert_eq!(silent_of(rig.signals()).len(), 1);
        }

        #[test]
        fn an_aborted_turn_gives_one_signal() {
            let mut rig = silent_rig("xio-silent-abort");
            let path = rollout_file(&rig.dir.path, NAME, SESSION_META);
            rig.io.handle(change(NAME, FileAction::Added));
            append_lines(&path, &[TASK_STARTED, USER_MSG, ABORTED]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert_eq!(silent_of(rig.signals()).len(), 1);
        }

        #[test]
        fn two_turn_ends_in_one_batch_give_one_signal() {
            let mut rig = silent_rig("xio-silent-batch");
            let path = rollout_file(&rig.dir.path, NAME, SESSION_META);
            rig.io.handle(change(NAME, FileAction::Added));
            append_lines(&path, &[TASK_STARTED, USER_MSG, TASK_COMPLETE, TASK_STARTED, USER_MSG, ABORTED]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert_eq!(silent_of(rig.signals()).len(), 1);
        }

        #[test]
        fn a_rollout_first_seen_through_modified_ignores_its_history() {
            let mut rig = silent_rig("xio-silent-history");
            let path = rollout_file(&rig.dir.path, NAME, SESSION_META);
            append_lines(&path, &[TASK_STARTED, USER_MSG]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty(), "old history");
            rig.io.handle(change(NAME, FileAction::RenamedNew));
            assert!(silent_of(rig.signals()).is_empty());
            // The turn that was running at first sight is not reported (its start was not read).
            append_lines(&path, &[USER_MSG, TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty(), "a turn already running");
            append_lines(&path, &[TASK_STARTED, USER_MSG, TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert_eq!(silent_of(rig.signals()).len(), 1);
        }

        #[test]
        fn a_partial_line_is_read_again_once_it_is_complete() {
            let mut rig = silent_rig("xio-silent-halfline");
            let path = rollout_file(&rig.dir.path, NAME, SESSION_META);
            rig.io.handle(change(NAME, FileAction::Added));
            append_lines(&path, &[TASK_STARTED, USER_MSG]);
            let (head, tail) = TASK_COMPLETE.split_at(30);
            append_raw(&path, head);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty());
            append_raw(&path, tail);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty(), "still no newline");
            append_raw(&path, "\n");
            rig.io.handle(change(NAME, FileAction::Modified));
            assert_eq!(silent_of(rig.signals()).len(), 1);
        }

        #[test]
        fn a_truncated_rollout_resets_the_offset_and_a_huge_write_is_skipped() {
            let mut rig = silent_rig("xio-silent-trunc");
            let path = rollout_file(&rig.dir.path, NAME, SESSION_META);
            append_lines(&path, &[TASK_STARTED]);
            rig.io.handle(change(NAME, FileAction::Added));
            std::fs::write(&path, format!("{SESSION_META}\n")).unwrap();
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty(), "truncated");
            append_lines(&path, &[TASK_STARTED, USER_MSG, TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert_eq!(silent_of(rig.signals()).len(), 1, "read from the new end");
            let pad = format!(r#"{{"type":"response_item","payload":{{"type":"message","text":"{}"}}}}"#, "a".repeat(SCAN_CAP as usize));
            append_lines(&path, &[TASK_STARTED, USER_MSG, &pad, TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty(), "more than 1 MiB pending is not looked at");
            append_lines(&path, &[TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty(), "the skip forgets the turn that was open");
            append_lines(&path, &[TASK_STARTED, USER_MSG, TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert_eq!(silent_of(rig.signals()).len(), 1);
        }

        #[test]
        fn only_a_top_level_event_msg_turn_end_counts() {
            let mut rig = silent_rig("xio-silent-shapes");
            let path = rollout_file(&rig.dir.path, NAME, SESSION_META);
            rig.io.handle(change(NAME, FileAction::Added));
            let quoted = r#"{"type":"response_item","payload":{"type":"message","text":"task_complete turn_aborted"}}"#;
            let wrong = r#"{"type":"response_item","payload":{"type":"task_complete"}}"#;
            append_lines(&path, &[TASK_STARTED, USER_MSG, quoted, wrong, "task_complete {not json"]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty());
            // None of them ended the turn: the real end still counts.
            append_lines(&path, &[TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert_eq!(silent_of(rig.signals()).len(), 1);
        }

        #[test]
        fn a_turn_without_a_user_message_gives_no_signal() {
            let mut rig = silent_rig("xio-silent-nouser");
            let path = rollout_file(&rig.dir.path, NAME, SESSION_META);
            rig.io.handle(change(NAME, FileAction::Added));
            let assistant = r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[]}}"#;
            append_lines(&path, &[TASK_STARTED, assistant, TASK_COMPLETE, TASK_STARTED, ABORTED]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty());
            // A user message outside a turn does not count for the next one.
            append_lines(&path, &[USER_MSG, TASK_STARTED, TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty());
        }

        #[test]
        fn a_user_turn_whose_end_comes_in_a_later_event_gives_one_signal() {
            let mut rig = silent_rig("xio-silent-split");
            let path = rollout_file(&rig.dir.path, NAME, SESSION_META);
            rig.io.handle(change(NAME, FileAction::Added));
            append_lines(&path, &[TASK_STARTED, USER_MSG]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty());
            append_lines(&path, &[SETTINGS]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty());
            append_lines(&path, &[TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert_eq!(silent_of(rig.signals()).len(), 1);
            rig.io.handle(change(NAME, FileAction::Modified));
            append_lines(&path, &[TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty(), "the turn ended once");
        }

        #[test]
        fn a_rollout_that_a_hook_already_tracks_is_never_silent() {
            let mut rig = silent_rig("xio-silent-tracked");
            let path = rollout_file(&rig.dir.path, NAME, SESSION_META);
            rig.hook(rig.payload("UserPromptSubmit", Some(&path)));
            assert!(matches!(rig.signals().last(), Some(Signal::CodexHook { .. })));
            append_lines(&path, &[TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(silent_of(rig.signals()).is_empty());
        }

        #[test]
        fn a_watcher_event_for_a_tracked_rollout_reads_its_new_records_once() {
            let mut rig = silent_rig("xio-tracked-read");
            let path = rollout_file(&rig.dir.path, NAME, SESSION_META);
            rig.hook(rig.payload("UserPromptSubmit", Some(&path)));
            rig.signals();
            let start = std::fs::metadata(&path).unwrap().len();
            append_lines(&path, &[TASK_STARTED, TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            let len = std::fs::metadata(&path).unwrap().len();
            match rig.signals().as_slice() {
                [Signal::CodexRollout { key, records, eof, folder_exists, .. }] => {
                    assert_eq!(key, &codex_key(SID));
                    assert_eq!(records.len(), 2);
                    assert_eq!(records[0].0, start);
                    assert_eq!(*eof, len);
                    assert!(!folder_exists, "the meta cwd does not exist, whatever the hook's cwd");
                }
                other => panic!("{other:?}"),
            }
            rig.io.handle(change(NAME, FileAction::Modified));
            assert_eq!(rig.signals(), [], "no new bytes, nothing sent");
            // The next hook finds nothing new either: no record is read twice.
            rig.hook(rig.payload("Stop", Some(&path)));
            assert!(matches!(rig.signals().as_slice(), [Signal::CodexHook { .. }]));
        }

        #[test]
        fn the_folder_of_a_tracked_rollout_comes_from_its_meta_cwd() {
            let mut rig = silent_rig("xio-tracked-folder");
            let work = rig.dir.path.join("work");
            let meta = serde_json::json!({
                "type": "session_meta",
                "payload": { "id": SID, "cwd": work.to_string_lossy(), "originator": "codex_vscode" }
            });
            let path = rollout_file(&rig.dir.path, NAME, &meta.to_string());
            rig.hook(rig.payload("UserPromptSubmit", Some(&path)));
            rig.signals();
            append_lines(&path, &[TASK_STARTED]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(matches!(rig.signals().as_slice(), [Signal::CodexRollout { folder_exists: true, .. }]));
            // The folder is checked again for every batch.
            std::fs::remove_dir_all(&work).unwrap();
            append_lines(&path, &[TASK_COMPLETE]);
            rig.io.handle(change(NAME, FileAction::Modified));
            assert!(matches!(rig.signals().as_slice(), [Signal::CodexRollout { folder_exists: false, .. }]));
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
                let path = rig.dir.path.join("sessions").join("2026").join("10").join("06").join(name);
                append_lines(&path, &[TASK_STARTED, USER_MSG, TASK_COMPLETE]);
                rig.io.handle(change(name, FileAction::Modified));
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
            assert!(silent_of(rig.signals()).is_empty(), "the first line is whole now, its turn has not ended");
            append_lines(&path, &[TASK_STARTED, USER_MSG, TASK_COMPLETE]);
            rig.io.handle(change(name, FileAction::Modified));
            assert_eq!(silent_of(rig.signals()).len(), 1, "a rollout that was new is read from its start");
        }

        #[test]
        fn a_session_removed_after_its_hook_never_comes_back_as_silent() {
            let mut rig = silent_rig("xio-silent-ghost");
            let name = "rollout-2026-10-06T10-00-00-aaaa.jsonl";
            let path = rollout_file(&rig.dir.path, name, SESSION_META);
            rig.hook(rig.payload("UserPromptSubmit", Some(&path)));
            rig.io.handle(CodexInput::Effect(Effect::UntrackRollout { key: codex_key(SID) }));
            rig.signals();
            append_lines(&path, &[TASK_STARTED, USER_MSG, TASK_COMPLETE]);
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
        fn a_failed_sessions_watch_is_started_again_by_retry() {
            let mut rig = silent_rig("xio-silent-restart");
            std::fs::create_dir_all(rig.dir.path.join("sessions")).unwrap();
            let (tx, _rx) = mpsc::channel();
            rig.io.watcher_tx = Some(tx);
            rig.io.handle(CodexInput::Retry);
            assert!(rig.io.watching);
            rig.io.handle(CodexInput::Sessions(WatchEvent::Error(std::io::Error::other("gone"))));
            assert!(!rig.io.watching, "the ended watch must not look alive");
            rig.io.handle(CodexInput::Retry);
            assert!(rig.io.watching);
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

        // ── the usage seed at start ──

        /// 2026-10-06T19:49:11.357Z
        const SEED_AT_MS: i64 = 1_791_316_151_357;
        const SEED_AT: &str = "2026-10-06T19:49:11.357Z";

        /// A paginated `token_count` line with the account's primary window at `percent`.
        fn limits_line(timestamp: Option<&str>, limit_id: &str, percent: u32) -> String {
            let mut line = serde_json::json!({
                "type": "event_msg",
                "payload": { "type": "token_count", "info": null, "rate_limits": {
                    "limit_id": limit_id,
                    "primary": { "used_percent": percent, "window_minutes": 300, "resets_at": 1_791_320_000 }
                } }
            });
            if let Some(timestamp) = timestamp {
                line["timestamp"] = serde_json::json!(timestamp);
            }
            line.to_string()
        }

        fn percent_of(usage: &CodexUsage) -> Vec<f64> {
            usage.windows.iter().map(|w| w.percent).collect()
        }

        /// `<home>\sessions\2026\10\06\<name>` with `text`, modified `age_s` seconds before a fixed
        /// base time (a larger age is older).
        fn seed_file(home: &Path, name: &str, text: &str, age_s: u64) -> PathBuf {
            let day = home.join("sessions").join("2026").join("10").join("06");
            std::fs::create_dir_all(&day).unwrap();
            let path = day.join(name);
            std::fs::write(&path, text).unwrap();
            let at = std::time::UNIX_EPOCH + Duration::from_secs(1_791_400_000 - age_s);
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(at).unwrap();
            path
        }

        fn seed_of(home: &Path) -> Option<CodexUsage> {
            seed_usage(&home.join("sessions"))
        }

        #[test]
        fn the_seed_comes_from_the_rollout_with_the_latest_modified_time() {
            let dir = TempDir::new("xio-seed-newest");
            let line = |percent| format!("{}\n", limits_line(Some(SEED_AT), "codex", percent));
            seed_file(&dir.path, "rollout-a.jsonl", &line(10), 300);
            seed_file(&dir.path, "rollout-b.jsonl", &line(20), 100);
            seed_file(&dir.path, "rollout-c.jsonl", &line(30), 200);
            // a newer file that is no plain rollout is never read
            seed_file(&dir.path, "rollout-d.jsonl.zst", &line(40), 1);
            seed_file(&dir.path, "notes.jsonl", &line(50), 2);
            assert_eq!(percent_of(&seed_of(&dir.path).unwrap()), [20.0]);
        }

        #[test]
        fn the_last_limits_line_wins_over_an_earlier_one() {
            let dir = TempDir::new("xio-seed-last");
            let text = format!(
                "{}\n{}\n{}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\"}}}}\n{{not json\n",
                limits_line(Some("2026-10-06T10:00:00.000Z"), "codex", 11),
                limits_line(Some(SEED_AT), "codex", 22),
                limits_line(Some("2026-10-06T21:00:00.000Z"), "codex_bengalfox", 99),
            );
            seed_file(&dir.path, "rollout-a.jsonl", &text, 0);
            let seed = seed_of(&dir.path).unwrap();
            assert_eq!(percent_of(&seed), [22.0], "another bucket and broken lines at the end are skipped");
            assert_eq!(seed.seen_at_ms, SEED_AT_MS, "the stamp is the record's own timestamp");
        }

        #[test]
        fn no_limits_give_no_seed() {
            let dir = TempDir::new("xio-seed-none");
            assert_eq!(seed_of(&dir.path), None, "no sessions folder");
            std::fs::create_dir_all(dir.path.join("sessions")).unwrap();
            assert_eq!(seed_of(&dir.path), None, "no rollout");
            seed_file(&dir.path, "rollout-a.jsonl", &format!("{TASK_STARTED}\n{}\n", limits_line(None, "premium", 5)), 0);
            assert_eq!(seed_of(&dir.path), None, "no limits of the account in the newest rollout");
            // the newest rollout alone counts: an older one with limits is not looked at
            seed_file(&dir.path, "rollout-b.jsonl", &format!("{}\n", limits_line(Some(SEED_AT), "codex", 7)), 900);
            assert_eq!(seed_of(&dir.path), None);
            let mut rig = silent_rig("xio-seed-none-io");
            rig.io.seed_usage();
            assert!(rig.signals().is_empty(), "no signal is sent");
        }

        #[test]
        fn a_partial_first_line_of_the_tail_is_skipped() {
            let dir = TempDir::new("xio-seed-partial");
            let limits = limits_line(Some(SEED_AT), "codex", 33);
            // the limits line starts before the 256 KiB window and ends inside it
            let filler = "x".repeat(SEED_TAIL as usize - limits.len() / 2);
            let cut = format!("{limits}\n{{\"type\":\"other\",\"pad\":\"{filler}\"}}\n");
            assert!(cut.len() as u64 > SEED_TAIL && (cut.len() - limits.len() - 1) as u64 > SEED_TAIL / 2);
            seed_file(&dir.path, "rollout-a.jsonl", &cut, 0);
            assert_eq!(seed_of(&dir.path), None, "a line cut by the window is never parsed");
            // the same line, complete inside the window, is found
            let kept = format!("{{\"type\":\"other\",\"pad\":\"{}\"}}\n{limits}\n", "x".repeat(SEED_TAIL as usize));
            seed_file(&dir.path, "rollout-a.jsonl", &kept, 0);
            assert_eq!(percent_of(&seed_of(&dir.path).unwrap()), [33.0]);
            // a window that starts exactly at a line boundary loses no line
            let mut value: Value = serde_json::from_str(&limits).unwrap();
            value["pad"] = serde_json::json!("");
            let room = SEED_TAIL as usize - 1 - value.to_string().len();
            value["pad"] = serde_json::json!("x".repeat(room));
            let exact_line = value.to_string();
            assert_eq!(exact_line.len() + 1, SEED_TAIL as usize);
            seed_file(&dir.path, "rollout-a.jsonl", &format!("{{\"a\":1}}\n{exact_line}\n"), 0);
            assert_eq!(percent_of(&seed_of(&dir.path).unwrap()), [33.0]);
        }

        #[test]
        fn the_timestamp_gives_the_seen_time_and_a_bad_one_the_modified_time() {
            let dir = TempDir::new("xio-seed-time");
            let line = |timestamp: Option<&str>| format!("{}\n", limits_line(timestamp, "codex", 5));
            seed_file(&dir.path, "rollout-a.jsonl", &line(Some(SEED_AT)), 0);
            assert_eq!(seed_of(&dir.path).unwrap().seen_at_ms, SEED_AT_MS);
            for bad in [None, Some("yesterday"), Some("2026-10-06T19:49:11")] {
                seed_file(&dir.path, "rollout-a.jsonl", &line(bad), 0);
                assert_eq!(seed_of(&dir.path).unwrap().seen_at_ms, 1_791_400_000_000, "{bad:?}");
            }
        }

        #[test]
        fn the_seed_is_sent_once_as_a_usage_signal_and_nothing_else() {
            let mut rig = silent_rig("xio-seed-signal");
            seed_file(&rig.dir.path, "rollout-a.jsonl", &format!("{}\n", limits_line(Some(SEED_AT), "codex", 44)), 0);
            rig.io.seed_usage();
            let signals = rig.signals();
            assert_eq!(signals.len(), 1, "{signals:?}");
            match &signals[0] {
                Signal::CodexUsageSeed(usage) => {
                    assert_eq!(usage.seen_at_ms, SEED_AT_MS);
                    assert_eq!(usage.windows, [CodexWindow { minutes: 300, percent: 44.0, resets_at_ms: Some(1_791_320_000_000) }]);
                }
                other => panic!("{other:?}"),
            }
        }
    }
}
