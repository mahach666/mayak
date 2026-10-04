//! mayak-hook core — everything the relay does that can be tested without an
//! agent spawning it. `main.rs` only wires argv, stdin and stdout to this.
//!
//! Hard rule: **never block the agent.**
//! * No pipe (Mayak closed) → nothing to say, exit 0 at once.
//! * Every pipe call runs on a worker thread under a deadline (`talk_to`), so a
//!   server that accepts and then stops reading cannot wedge a session.
//! * Only a Claude `PermissionRequest` waits for an answer. Codex never waits and
//!   never gets anything on stdout: the island only watches Codex.
//! * A foreign `--agent` value, or a Claude `PermissionRequest` that is not a
//!   real permission request, gets nothing sent and nothing printed.

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

// `win::pipe_path` builds the pipe name from `ident::PIPE_PREFIX`.
#[allow(dead_code)]
#[path = "../../ident.rs"]
mod ident;

pub mod win;
#[cfg(debug_assertions)]
pub mod dump;

/// Longest string forwarded for any single field.
pub const MAX_FIELD_LEN: usize = 2_000;
/// Budget for getting a pipe connection. Beyond this the agent wins, always.
pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
/// Whole Codex run, connect included. Codex waits on its hooks.
pub const CODEX_BUDGET: Duration = Duration::from_secs(1);
/// Whole run for a Claude event nobody waits on: connect and write, no more.
pub const FIRE_AND_FORGET_BUDGET: Duration = Duration::from_secs(2);
/// How long a Claude permission prompt may stay on the island.
pub const DECISION_BUDGET: Duration = Duration::from_secs(110);
/// Ancestor steps searched for `codex.exe`.
pub const MAX_ANCESTOR_STEPS: usize = 6;
/// `relay` field: lets the app tell this relay from the Coucou-era one.
pub const RELAY_VERSION: u64 = 2;
/// (payload key, env var) terminal context, inserted only when absent.
pub const TERM_CONTEXT: &[(&str, &str)] = &[
    ("term_program", "TERM_PROGRAM"),
    ("wt_session", "WT_SESSION"),
    ("term_session_id", "TERM_SESSION_ID"),
    ("vscode_pid", "VSCODE_PID"),
    ("session_pid", "CLAUDE_CODE_SSE_PORT"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    pub fn as_str(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// None: `--agent` named an agent Mayak does not serve (D86).
    pub agent: Option<Agent>,
    pub event: Option<String>,
    pub dump: bool,
}

/// argv without argv[0]: `[--agent <name>] [<EventName>]`, upstream's grammar.
/// `--agent` may stand anywhere and always consumes the next token: `codex` →
/// Codex; `claude`, an empty or a missing value → Claude; any other value → None,
/// and main sends and prints nothing (D86). The first other token that is not a
/// flag is the event; a lone `<EventName>` means Claude, as with the Coucou relay.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Args {
    let mut parsed = Args { agent: Some(Agent::Claude), event: None, dump: false };
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        if arg == "--agent" {
            parsed.agent = match it.next().as_deref() {
                None | Some("") | Some("claude") => Some(Agent::Claude),
                Some("codex") => Some(Agent::Codex),
                Some(_) => None,
            };
        } else if arg == "--dump" {
            // Spike S2 only: the flag does not exist in a release build.
            parsed.dump = cfg!(debug_assertions);
        } else if arg.starts_with("--") {
            // Unknown flag: ignored.
        } else if parsed.event.is_none() {
            parsed.event = Some(arg);
        }
    }
    parsed
}

/// Strips every leading UTF-8 BOM. Some shells add one, some add two.
pub fn strip_boms(raw: &[u8]) -> &[u8] {
    let mut rest = raw;
    while let Some(tail) = rest.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        rest = tail;
    }
    rest
}

/// Non-empty `payload.hook_event_name` wins, else the argv event, else "".
pub fn resolve_event(payload: &Map<String, Value>, arg_event: Option<&str>) -> String {
    payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or(arg_event)
        .unwrap_or_default()
        .to_string()
}

/// A `PermissionRequest` carrying `turn_id` comes from Codex whatever argv says:
/// Codex can import Claude's hooks, and an imported Allow/Deny entry must
/// neither wait nor answer Codex.
pub fn detect_agent(arg_agent: Agent, event: &str, payload: &Map<String, Value>) -> Agent {
    let has_turn = payload.get("turn_id").is_some_and(|v| !v.is_null());
    if event == "PermissionRequest" && has_turn {
        Agent::Codex
    } else {
        arg_agent
    }
}

pub fn waits_for_answer(agent: Agent, event: &str) -> bool {
    agent == Agent::Claude && event == "PermissionRequest"
}

/// The legacy Allow/Deny card is only for real Claude Code permission requests
/// (D87): they carry a non-empty string `session_id`, and `AskUserQuestion` is a
/// question for the user, which Allow or Deny from the island would break.
/// Anything else is left to the agent's own UI.
pub fn legacy_approval_allowed(payload: &Map<String, Value>) -> bool {
    let has_session = payload
        .get("session_id")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    let is_question = payload.get("tool_name").and_then(Value::as_str) == Some("AskUserQuestion");
    has_session && !is_question
}

pub fn budget(agent: Agent, waits: bool) -> Duration {
    match (agent, waits) {
        (Agent::Codex, _) => CODEX_BUDGET,
        (Agent::Claude, true) => DECISION_BUDGET,
        (Agent::Claude, false) => FIRE_AND_FORGET_BUDGET,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProc {
    pub pid: u32,
    pub created_ft: u64,
    pub image: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Enrichment {
    pub agent: Agent,
    pub event: String,
    /// ms since the Unix epoch, UTC.
    pub relay_ts: i64,
    pub cwd_fallback: Option<String>,
    /// Resolved TERM_CONTEXT values.
    pub term: Vec<(&'static str, String)>,
    /// From CLAUDE_PID (Claude only).
    pub claude_pid: Option<u32>,
    /// Ancestor codex.exe (Codex only).
    pub codex_proc: Option<AgentProc>,
}

/// Adds what the app needs and drops what it must not get.
pub fn enrich(payload: &mut Map<String, Value>, e: &Enrichment) {
    payload.insert("hook_event_name".into(), Value::String(e.event.clone()));
    payload.insert("agent".into(), Value::from(e.agent.as_str()));
    payload.insert("relay".into(), Value::from(RELAY_VERSION));
    payload.insert("relay_ts".into(), Value::from(e.relay_ts));
    // A whole file read or command output: never shown, can be enormous.
    payload.remove("tool_response");
    match e.agent {
        Agent::Claude => {
            // The observer reads Claude transcripts itself.
            payload.remove("transcript_path");
            payload.insert("agent_pid".into(), e.claude_pid.map_or(Value::Null, Value::from));
        }
        Agent::Codex => {
            // Codex keeps transcript_path: it is the rollout codex-io tails.
            let p = e.codex_proc.as_ref();
            payload.insert("agent_pid".into(), p.map_or(Value::Null, |p| Value::from(p.pid)));
            payload.insert(
                "agent_created".into(),
                p.map_or(Value::Null, |p| Value::from(p.created_ft)),
            );
            payload.insert(
                "agent_image".into(),
                p.and_then(|p| p.image.clone()).map_or(Value::Null, Value::String),
            );
        }
    }
    let cwd_missing = payload.get("cwd").and_then(Value::as_str).map_or(true, str::is_empty);
    if cwd_missing {
        if let Some(cwd) = &e.cwd_fallback {
            payload.insert("cwd".into(), Value::String(cwd.clone()));
        }
    }
    for (key, value) in &e.term {
        if !payload.contains_key(*key) {
            payload.insert((*key).to_string(), Value::String(value.clone()));
        }
    }
}

/// Caps every string. A single Write can carry a whole file.
pub fn truncate_strings(v: &mut Value) {
    match v {
        Value::String(s) => {
            if s.len() > MAX_FIELD_LEN {
                // Cut on a char boundary; a lone byte index can split UTF-8.
                let mut end = MAX_FIELD_LEN;
                while end > 0 && !s.is_char_boundary(end) {
                    end -= 1;
                }
                s.truncate(end);
                s.push('…');
            }
        }
        Value::Array(items) => items.iter_mut().for_each(truncate_strings),
        Value::Object(map) => map.values_mut().for_each(truncate_strings),
        _ => {}
    }
}

/// Compact JSON + '\n': the wire format of the pipe.
pub fn to_line(payload: &Map<String, Value>) -> String {
    let mut line = Value::Object(payload.clone()).to_string();
    line.push('\n');
    line
}

/// Connect, send, and — when `waits` — read one reply line. Runs on a worker
/// thread; returns None after `budget` at the latest. Never panics.
pub fn talk_to(pipe_name: &str, payload: &str, waits: bool, budget: Duration) -> Option<String> {
    if budget.is_zero() {
        return None;
    }
    let (tx, rx) = mpsc::channel::<Option<String>>();
    let name = pipe_name.to_string();
    let line = payload.to_string();
    // If the worker overruns, we stop listening; the process exit takes the
    // pipe handle with it.
    std::thread::spawn(move || {
        let _ = tx.send(talk(&name, &line, waits));
    });
    rx.recv_timeout(budget).ok().flatten()
}

fn talk(pipe_name: &str, payload: &str, waits: bool) -> Option<String> {
    let mut pipe = win::connect(pipe_name)?;
    pipe.write_all(payload.as_bytes()).ok()?;
    let _ = pipe.flush();
    if !waits {
        return None;
    }
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if chunk[..n].contains(&b'\n') {
                    break;
                }
            }
        }
    }
    let answer = String::from_utf8_lossy(&buf).trim().to_string();
    (!answer.is_empty()).then_some(answer)
}

/// The documented Claude Code PermissionRequest output. Anything unrecognised
/// prints nothing: silence is the safe answer.
pub fn decision_json(decision: &str) -> Option<String> {
    let behavior = match decision.trim() {
        // "always" still answers a plain allow; remembering it is the island's job.
        "allow" | "always" => r#"{"behavior":"allow"}"#.to_string(),
        "deny" => r#"{"behavior":"deny","message":"Denied from Mayak"}"#.to_string(),
        _ => return None,
    };
    Some(format!(
        r#"{{"hookSpecificOutput":{{"hookEventName":"PermissionRequest","decision":{behavior}}}}}"#
    ))
}

/// What to print on stdout. Only a Claude PermissionRequest with a recognised
/// reply prints anything; Codex never does.
pub fn stdout_for(agent: Agent, event: &str, reply: Option<&str>) -> Option<String> {
    if !waits_for_answer(agent, event) {
        return None;
    }
    reply.and_then(decision_json)
}

pub fn unix_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) mod testutil {
    //! Private copy of the app's `testutil::TempDir` and a std-thread pipe server.

    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::windows::io::FromRawHandle;
    #[cfg(debug_assertions)]
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;
    use std::thread::JoinHandle;
    use std::time::Duration;

    use windows::core::HSTRING;
    use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE};
    use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_NOWAIT, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
        PIPE_WAIT,
    };

    static NEXT: AtomicU32 = AtomicU32::new(0);

    fn next() -> u32 {
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    #[cfg(debug_assertions)]
    pub struct TempDir {
        pub path: PathBuf,
    }

    #[cfg(debug_assertions)]
    impl TempDir {
        pub fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("mayak-test-{tag}-{}-{}", std::process::id(), next()));
            std::fs::create_dir_all(&path).expect("create temp dir");
            TempDir { path }
        }
    }

    #[cfg(debug_assertions)]
    impl Drop for TempDir {
        fn drop(&mut self) {
            if let Err(err) = std::fs::remove_dir_all(&self.path) {
                eprintln!("mayak-hook tests: could not remove {}: {err}", self.path.display());
            }
        }
    }

    /// `\\.\pipe\mayak-test-<tag>-<pid>-<n>` — never the real `mayak-<SID>`.
    pub fn unique_pipe(tag: &str) -> String {
        format!(r"\\.\pipe\mayak-test-{tag}-{}-{}", std::process::id(), next())
    }

    /// One server instance (byte mode, one instance max).
    pub fn create_instance(name: &str, nowait: bool) -> HANDLE {
        let wait = if nowait { PIPE_NOWAIT } else { PIPE_WAIT };
        let h = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(name),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | wait,
                1,
                4096,
                4096,
                0,
                None,
            )
        };
        assert!(!h.is_invalid(), "CreateNamedPipeW failed for {name}");
        h
    }

    /// Server thread: create, signal ready, accept one client, read one line,
    /// write `reply` if any, keep the connection for `hold`, return the line.
    pub fn serve_once(name: &str, reply: Option<&'static str>, hold: Duration) -> JoinHandle<String> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let name = name.to_string();
        let server = std::thread::spawn(move || {
            let h = create_instance(&name, false);
            ready_tx.send(()).expect("ready");
            if let Err(err) = unsafe { ConnectNamedPipe(h, None) } {
                assert_eq!(err.code(), ERROR_PIPE_CONNECTED.to_hresult(), "accept failed");
            }
            let mut pipe = unsafe { File::from_raw_handle(h.0) };
            let mut line = Vec::new();
            let mut byte = [0u8; 1];
            while let Ok(1) = pipe.read(&mut byte) {
                line.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            if let Some(r) = reply {
                pipe.write_all(r.as_bytes()).expect("reply");
                pipe.flush().expect("flush");
            }
            std::thread::sleep(hold);
            String::from_utf8(line).expect("utf-8 line")
        });
        ready_rx.recv_timeout(Duration::from_secs(5)).expect("server ready");
        server
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::{create_instance, serve_once, unique_pipe};
    use super::*;
    use serde_json::json;
    use std::time::Instant;
    use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_LISTENING};
    use windows::Win32::System::Pipes::ConnectNamedPipe;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().expect("object")
    }

    fn args(list: &[&str]) -> Args {
        parse_args(list.iter().map(|s| s.to_string()))
    }

    fn enrichment(agent: Agent) -> Enrichment {
        Enrichment {
            agent,
            event: "PermissionRequest".into(),
            relay_ts: 1_700_000_000_123,
            cwd_fallback: Some(r"C:\fallback".into()),
            term: vec![("term_program", "vscode".into()), ("wt_session", String::new())],
            claude_pid: Some(4242),
            codex_proc: Some(AgentProc {
                pid: 77,
                created_ft: 133_000_000_000_000_000,
                image: Some(r"C:\x\codex.exe".into()),
            }),
        }
    }

    #[test]
    fn a_lone_event_argument_still_means_claude() {
        assert_eq!(
            args(&["PermissionRequest"]),
            Args { agent: Some(Agent::Claude), event: Some("PermissionRequest".into()), dump: false }
        );
        assert_eq!(args(&[]), Args { agent: Some(Agent::Claude), event: None, dump: false });
    }

    #[test]
    fn agent_flag_selects_the_agent_and_consumes_its_value() {
        assert_eq!(args(&["--agent", "codex"]).agent, Some(Agent::Codex));
        assert_eq!(args(&["--agent", "codex"]).event, None);
        assert_eq!(args(&["--agent", "claude", "Stop"]).event, Some("Stop".into()));
        // The flag may stand after the event (upstream grammar).
        assert_eq!(
            args(&["Stop", "--agent", "codex"]),
            Args { agent: Some(Agent::Codex), event: Some("Stop".into()), dump: false }
        );
        // Missing or empty value → Claude, as upstream treats an absent tag.
        assert_eq!(args(&["--agent"]).agent, Some(Agent::Claude));
        assert_eq!(args(&["--agent", ""]).agent, Some(Agent::Claude));
        // Any other value is a foreign agent (D86); it is consumed, never the event.
        assert_eq!(args(&["--agent", "cursor"]), Args { agent: None, event: None, dump: false });
        assert_eq!(args(&["--agent", "gemini", "PermissionRequest"]).agent, None);
        assert_eq!(args(&["--agent", "gemini", "PermissionRequest"]).event, Some("PermissionRequest".into()));
        // Unknown flags and extra tokens are ignored.
        assert_eq!(args(&["--verbose", "Stop", "extra"]).event, Some("Stop".into()));
    }

    #[test]
    fn legacy_approval_needs_a_session_and_is_never_a_question() {
        assert!(legacy_approval_allowed(&obj(json!({ "session_id": "s", "tool_name": "Bash" }))));
        // No session: not a Claude Code permission request (Cursor's agent sends
        // conversation_id instead), so no card.
        assert!(!legacy_approval_allowed(&obj(json!({ "tool_name": "Bash" }))));
        assert!(!legacy_approval_allowed(&obj(json!({ "conversation_id": "c", "tool_name": "Bash" }))));
        assert!(!legacy_approval_allowed(&obj(json!({ "session_id": "", "tool_name": "Bash" }))));
        assert!(!legacy_approval_allowed(&obj(json!({ "session_id": 7, "tool_name": "Bash" }))));
        // A question for the user: Allow/Deny from the island would break its answer.
        assert!(!legacy_approval_allowed(&obj(json!({ "session_id": "s", "tool_name": "AskUserQuestion" }))));
    }

    #[test]
    fn dump_flag_exists_only_in_debug_builds() {
        assert_eq!(args(&["--agent", "codex", "--dump"]).dump, cfg!(debug_assertions));
    }

    #[test]
    fn every_leading_bom_is_stripped() {
        let bom = [0xEF, 0xBB, 0xBF];
        let mut raw = Vec::new();
        raw.extend_from_slice(&bom);
        raw.extend_from_slice(&bom);
        raw.extend_from_slice(b"{}");
        assert_eq!(strip_boms(&raw), b"{}");
        assert_eq!(strip_boms(b"{}"), b"{}");
        assert_eq!(strip_boms(&bom), b"");
        // A BOM inside the text is content, not a prefix.
        let inner = [b'{', 0xEF, 0xBB, 0xBF, b'}'];
        assert_eq!(strip_boms(&inner), &inner);
    }

    #[test]
    fn payload_event_name_wins_over_argv() {
        let p = obj(json!({ "hook_event_name": "Stop" }));
        assert_eq!(resolve_event(&p, Some("PermissionRequest")), "Stop");
        let empty = obj(json!({ "hook_event_name": "" }));
        assert_eq!(resolve_event(&empty, Some("PermissionRequest")), "PermissionRequest");
        assert_eq!(resolve_event(&obj(json!({})), None), "");
    }

    #[test]
    fn turn_id_on_a_permission_request_means_codex() {
        let codexish = obj(json!({ "turn_id": "t1", "model": "gpt" }));
        assert_eq!(detect_agent(Agent::Claude, "PermissionRequest", &codexish), Agent::Codex);
        // ...and Codex never waits, so an imported Allow/Deny entry cannot answer it.
        assert!(!waits_for_answer(Agent::Codex, "PermissionRequest"));
        // Other events keep argv's agent; a null turn_id is no turn_id.
        assert_eq!(detect_agent(Agent::Claude, "Stop", &codexish), Agent::Claude);
        let null_turn = obj(json!({ "turn_id": null }));
        assert_eq!(detect_agent(Agent::Claude, "PermissionRequest", &null_turn), Agent::Claude);
        assert_eq!(detect_agent(Agent::Codex, "Stop", &obj(json!({}))), Agent::Codex);
    }

    #[test]
    fn only_claude_permission_requests_wait() {
        assert!(waits_for_answer(Agent::Claude, "PermissionRequest"));
        assert!(!waits_for_answer(Agent::Claude, "Stop"));
        assert_eq!(budget(Agent::Claude, true), DECISION_BUDGET);
        assert_eq!(budget(Agent::Claude, false), FIRE_AND_FORGET_BUDGET);
        assert_eq!(budget(Agent::Codex, false), CODEX_BUDGET);
        assert_eq!(budget(Agent::Codex, true), CODEX_BUDGET);
    }

    #[test]
    fn claude_enrichment_drops_transcript_path_and_adds_its_pid() {
        let mut p = obj(json!({
            "hook_event_name": "PermissionRequest", "cwd": "",
            "transcript_path": "C:\\t.jsonl", "tool_response": "big", "term_program": "kept"
        }));
        enrich(&mut p, &enrichment(Agent::Claude));
        assert_eq!(p["agent"], "claude");
        assert_eq!(p["relay"], 2);
        assert_eq!(p["relay_ts"], 1_700_000_000_123i64);
        assert_eq!(p["agent_pid"], 4242);
        assert!(!p.contains_key("transcript_path"));
        assert!(!p.contains_key("tool_response"));
        assert!(!p.contains_key("agent_created"));
        assert_eq!(p["cwd"], r"C:\fallback");
        assert_eq!(p["term_program"], "kept");
        assert_eq!(p["wt_session"], "");
    }

    #[test]
    fn codex_enrichment_keeps_transcript_path_and_adds_the_agent_process() {
        let mut p = obj(json!({
            "hook_event_name": "Stop", "cwd": r"C:\repo",
            "transcript_path": r"C:\r.jsonl", "tool_response": "big"
        }));
        let mut e = enrichment(Agent::Codex);
        e.event = "Stop".into();
        enrich(&mut p, &e);
        assert_eq!(p["agent"], "codex");
        assert_eq!(p["transcript_path"], r"C:\r.jsonl");
        assert!(!p.contains_key("tool_response"));
        assert_eq!(p["agent_pid"], 77);
        assert_eq!(p["agent_created"], 133_000_000_000_000_000u64);
        assert_eq!(p["agent_image"], r"C:\x\codex.exe");
        assert_eq!(p["cwd"], r"C:\repo");
    }

    #[test]
    fn codex_without_an_ancestor_still_sends_nulls() {
        let mut p = obj(json!({ "hook_event_name": "Stop", "cwd": r"C:\repo" }));
        let mut e = enrichment(Agent::Codex);
        e.codex_proc = None;
        enrich(&mut p, &e);
        assert_eq!(p["agent_pid"], Value::Null);
        assert_eq!(p["agent_created"], Value::Null);
        assert_eq!(p["agent_image"], Value::Null);
    }

    #[test]
    fn long_strings_are_cut_on_a_char_boundary() {
        let mut v = json!({ "tool_input": { "content": "é".repeat(4000) } });
        truncate_strings(&mut v);
        let s = v["tool_input"]["content"].as_str().unwrap();
        assert!(s.len() <= MAX_FIELD_LEN + 4);
        assert!(s.ends_with('…'));
    }

    #[test]
    fn a_line_is_compact_json_with_one_newline() {
        let p = obj(json!({ "a": "привет", "b": 1 }));
        let line = to_line(&p);
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(serde_json::from_str::<Value>(line.trim_end()).unwrap(), Value::Object(p));
    }

    #[test]
    fn decision_json_matches_the_documented_shape() {
        assert_eq!(
            decision_json("allow").unwrap(),
            r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#
        );
        assert_eq!(
            decision_json("deny").unwrap(),
            r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Denied from Mayak"}}}"#
        );
        assert!(decision_json("always").unwrap().contains(r#""behavior":"allow""#));
        assert!(decision_json("").is_none());
        assert!(decision_json("maybe").is_none());
        assert!(decision_json(r#"{"permissionDecision":"allow"}"#).is_none());
    }

    #[test]
    fn codex_never_prints_anything() {
        assert_eq!(stdout_for(Agent::Codex, "PermissionRequest", Some("allow")), None);
        assert_eq!(stdout_for(Agent::Claude, "Stop", Some("allow")), None);
        assert_eq!(stdout_for(Agent::Claude, "PermissionRequest", None), None);
        assert_eq!(
            stdout_for(Agent::Claude, "PermissionRequest", Some("deny")),
            decision_json("deny")
        );
    }

    #[test]
    fn claude_permission_request_gets_the_reply() {
        let name = unique_pipe("reply");
        let server = serve_once(&name, Some("allow\n"), Duration::ZERO);
        let reply = talk_to(&name, "{\"x\":1}\n", true, Duration::from_secs(5));
        assert_eq!(reply.as_deref(), Some("allow"));
        assert_eq!(server.join().unwrap(), "{\"x\":1}\n");
    }

    #[test]
    fn codex_sends_and_returns_without_waiting() {
        let name = unique_pipe("codex");
        // The server never answers and keeps the connection for 3 s.
        let server = serve_once(&name, None, Duration::from_secs(3));
        let started = Instant::now();
        let reply = talk_to(&name, "{\"agent\":\"codex\"}\n", false, CODEX_BUDGET);
        assert_eq!(reply, None);
        assert!(started.elapsed() < CODEX_BUDGET, "took {:?}", started.elapsed());
        assert_eq!(server.join().unwrap(), "{\"agent\":\"codex\"}\n");
    }

    #[test]
    fn a_silent_server_costs_the_budget_and_no_more() {
        let name = unique_pipe("silent");
        let server = serve_once(&name, None, Duration::from_secs(2));
        let started = Instant::now();
        let reply = talk_to(&name, "{}\n", true, Duration::from_millis(400));
        let took = started.elapsed();
        assert_eq!(reply, None);
        assert!(took >= Duration::from_millis(400) && took < Duration::from_secs(1), "took {took:?}");
        server.join().unwrap();
    }

    #[test]
    fn a_server_that_goes_away_frees_a_waiting_relay_at_once() {
        // Mayak quits or crashes while a Claude permission prompt waits on the island: the
        // closed pipe ends the wait at once, not the 110 s decision budget.
        let name = unique_pipe("gone");
        let server = serve_once(&name, None, Duration::from_millis(300));
        let started = Instant::now();
        let reply = talk_to(&name, "{\"hook_event_name\":\"PermissionRequest\"}\n", true, DECISION_BUDGET);
        let took = started.elapsed();
        assert_eq!(reply, None);
        assert!(took >= Duration::from_millis(300) && took < Duration::from_secs(2), "took {took:?}");
        assert_eq!(server.join().unwrap(), "{\"hook_event_name\":\"PermissionRequest\"}\n");
    }

    #[test]
    fn a_missing_pipe_is_given_up_at_once() {
        let started = Instant::now();
        assert_eq!(talk_to(&unique_pipe("missing"), "{}\n", true, Duration::from_secs(5)), None);
        assert!(started.elapsed() < CONNECT_TIMEOUT, "took {:?}", started.elapsed());
    }

    #[test]
    fn a_zero_budget_never_connects() {
        let name = unique_pipe("zero");
        let h = create_instance(&name, true);
        assert_eq!(talk_to(&name, "{}\n", false, Duration::ZERO), None);
        std::thread::sleep(Duration::from_millis(200));
        // PIPE_NOWAIT: still listening means no client ever opened the pipe.
        let err = unsafe { ConnectNamedPipe(h, None) }.unwrap_err();
        assert_eq!(err.code(), ERROR_PIPE_LISTENING.to_hresult());
        unsafe { CloseHandle(h).unwrap() };
    }

    #[test]
    fn a_nonzero_budget_does_connect_to_the_same_kind_of_instance() {
        // Control for a_zero_budget_never_connects: the probe sees a real client.
        let name = unique_pipe("control");
        let h = create_instance(&name, true);
        assert_eq!(talk_to(&name, "{}\n", false, Duration::from_secs(1)), None);
        let err = unsafe { ConnectNamedPipe(h, None) }.unwrap_err();
        assert_ne!(err.code(), ERROR_PIPE_LISTENING.to_hresult());
        unsafe { CloseHandle(h).unwrap() };
    }
}
