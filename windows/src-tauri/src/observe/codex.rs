// Codex source (spec §5.6): hook events forwarded by the relay, rollout records (legacy
// mode), subagent meta, host classification and session_index titles. These functions are
// pure: payloads, lines, paths and %LOCALAPPDATA% come in as arguments.

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
    /// A legacy-shaped record that is not interesting.
    Other,
    /// Not legacy-shaped → source health Degraded.
    Unknown,
}

/// One rollout line, legacy mode only (spec §5.6). Top level `{timestamp, type, payload}`:
/// - `session_meta` with a string `payload.id` → Meta (cwd, parent_thread_id, agent_role);
///   parent_thread_id/agent_role from the top-level payload keys, else from
///   `payload.source.subagent.thread_spawn` (Codex `SubAgentSource::ThreadSpawn`);
/// - `event_msg` + `payload.type` task_started | task_complete | turn_aborted → the variants;
///   `task_complete.error`: string → itself; object → its "message" string, else compact JSON;
///   null or absent → None; any other JSON → compact JSON;
/// - `response_item` + `payload.type` function_call (label `tool_label(name, arguments parsed
///   as JSON)`), custom_tool_call (label `tool_label(name, None)`, i.e. the name), local_shell_call
///   (label `tool_label("shell", payload.action)`) → Call; `call_id` "" when absent;
/// - other `payload.type` values of event_msg/response_item, and top-level turn_context |
///   world_state | compacted → Other;
/// - anything else → Unknown: unknown or missing top-level type, non-object, event_msg or
///   response_item without a string `payload.type`, session_meta without `payload.id`,
///   function_call or custom_tool_call without `name`.
pub fn parse_rollout_line(v: &Value) -> RolloutRecord {
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
            Some(name) => RolloutRecord::Call { call_id: call_id(p), label: tool_label(&name, call_arguments(p).as_ref()) },
            None => RolloutRecord::Unknown,
        },
        ("response_item", Some(p), Some("custom_tool_call")) => match string(p, "name") {
            Some(name) => RolloutRecord::Call { call_id: call_id(p), label: tool_label(&name, None) },
            None => RolloutRecord::Unknown,
        },
        ("response_item", Some(p), Some("local_shell_call")) => {
            RolloutRecord::Call { call_id: call_id(p), label: tool_label("shell", p.get("action")) }
        }
        ("event_msg" | "response_item", Some(_), Some(_)) => RolloutRecord::Other,
        ("turn_context" | "world_state" | "compacted", _, _) => RolloutRecord::Other,
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
                Other, // function_call_output
                call("call_2", "exec"),
                Other, // custom_tool_call_output
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
}

// ── codex-io thread (T12, spec §5.6) ──────────────────────────────────────────
//
// A private child module keeps these imports apart from the parsing part above; the public
// items are re-exported as `observe::codex::{spawn, CodexConfig, CodexInput, TICK}`.

mod io_thread {
    use serde_json::Value;

    use crate::observe::model::Health;

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

    #[cfg(test)]
    mod tests {
        use super::*;

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
    }
}
