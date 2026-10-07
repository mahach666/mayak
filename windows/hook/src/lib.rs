//! mayak-hook core — everything the relay does that can be tested without an
//! agent spawning it. `main.rs` only wires argv, stdin and stdout to this.
//!
//! Hard rule: **never block the agent.**
//! * No pipe (Mayak closed) → nothing to say, exit 0 at once.
//! * Every pipe call runs on a worker thread under a deadline (`talk_to`), so a
//!   server that accepts and then stops reading cannot wedge a session.
//! * Only a permission request (Claude or Codex) and a Claude `AskUserQuestion`
//!   wait for an answer (`waits_for_answer`). A release, a timeout or anything
//!   unrecognised prints nothing, and the agent asks in its own UI.
//! * A foreign `--agent` value, a Claude `PermissionRequest` that is not a real
//!   permission request, or a Claude `PreToolUse` that is not a question gets
//!   nothing sent and nothing printed (`relayed`).

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::value::RawValue;
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
/// Whole run for a Codex event nobody answers, connect included. Codex waits on
/// its hooks.
pub const CODEX_BUDGET: Duration = Duration::from_secs(1);
/// Whole run for a Claude event nobody waits on: connect and write, no more.
pub const FIRE_AND_FORGET_BUDGET: Duration = Duration::from_secs(2);
/// Whole run for an event the island answers (Claude or Codex permission, Claude
/// question). Ends before the agent's 600 s hook timeout would kill us.
pub const ANSWER_BUDGET: Duration = Duration::from_secs(595);
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
/// Codex can import Claude's hooks, and an imported Allow/Deny entry must answer
/// Codex the Codex way (routed by Mayak's Codex setting, never "Always").
pub fn detect_agent(arg_agent: Agent, event: &str, payload: &Map<String, Value>) -> Agent {
    let has_turn = payload.get("turn_id").is_some_and(|v| !v.is_null());
    if event == "PermissionRequest" && has_turn {
        Agent::Codex
    } else {
        arg_agent
    }
}

/// The events the island may answer: a Claude or Codex `PermissionRequest`, and
/// a Claude `PreToolUse` for `AskUserQuestion`. Whether Codex is actually
/// answered is Mayak's call: when it only observes, it closes the pipe at once.
pub fn waits_for_answer(agent: Agent, event: &str, payload: &Map<String, Value>) -> bool {
    match (agent, event) {
        (_, "PermissionRequest") => true,
        (Agent::Claude, "PreToolUse") => is_question(payload),
        _ => false,
    }
}

/// Whether the event goes to Mayak at all. Claude Code only: a permission
/// request must be a real one (D87), and of all `PreToolUse` events only a
/// question from a Claude Code session is Mayak's business. Everything else
/// goes as before; Codex keeps feeding the observer.
pub fn relayed(agent: Agent, event: &str, payload: &Map<String, Value>) -> bool {
    match (agent, event) {
        (Agent::Claude, "PermissionRequest") => legacy_approval_allowed(payload),
        (Agent::Claude, "PreToolUse") => is_question(payload) && has_session(payload),
        _ => true,
    }
}

fn is_question(payload: &Map<String, Value>) -> bool {
    payload.get("tool_name").and_then(Value::as_str) == Some("AskUserQuestion")
}

fn has_session(payload: &Map<String, Value>) -> bool {
    payload.get("session_id").and_then(Value::as_str).is_some_and(|s| !s.is_empty())
}

/// The Allow/Deny card is only for real Claude Code permission requests (D87):
/// they carry a non-empty string `session_id`. `AskUserQuestion` arriving as a
/// `PermissionRequest` is left to Claude's own UI: the island answers questions
/// through `PreToolUse`, and Allow or Deny here would break the answer.
pub fn legacy_approval_allowed(payload: &Map<String, Value>) -> bool {
    has_session(payload) && !is_question(payload)
}

pub fn budget(agent: Agent, waits: bool) -> Duration {
    match (agent, waits) {
        (_, true) => ANSWER_BUDGET,
        (Agent::Codex, false) => CODEX_BUDGET,
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
        Value::String(s) => truncate_string(s),
        Value::Array(items) => items.iter_mut().for_each(truncate_strings),
        Value::Object(map) => map.values_mut().for_each(truncate_strings),
        _ => {}
    }
}

fn truncate_string(s: &mut String) {
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

/// One reply line from Mayak.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Allow,
    /// Allow, and let Claude Code keep the rule it suggested.
    Always,
    Deny,
    /// No decision: the agent asks in its own UI.
    Release,
    /// `answer <json>`: question text → label, or an array of labels.
    Answer(Value),
}

/// `allow`, `always`, `deny`, `release` or `answer <JSON object>`. Anything else,
/// an `answer` whose JSON does not parse or is not an object included, is a
/// release: silence is the safe answer.
pub fn parse_reply(line: &str) -> Reply {
    match line.trim() {
        "allow" => Reply::Allow,
        "always" => Reply::Always,
        "deny" => Reply::Deny,
        other => match other.strip_prefix("answer ").map(serde_json::from_str::<Value>) {
            Some(Ok(answers @ Value::Object(_))) => Reply::Answer(answers),
            _ => Reply::Release,
        },
    }
}

/// What the replies echo back, taken from stdin before `MAX_FIELD_LEN` cuts the
/// payload, and kept as the exact JSON text stdin carried.
#[derive(Debug, Default)]
pub struct RawInput {
    /// `tool_input.questions`.
    pub questions: Option<Box<RawValue>>,
    /// `permission_suggestions`.
    pub suggestions: Option<Box<RawValue>>,
}

impl RawInput {
    /// `body` is stdin without its BOMs. Not a JSON object → nothing kept.
    pub fn from_stdin(body: &[u8]) -> RawInput {
        type RawMap = BTreeMap<String, Box<RawValue>>;
        let Ok(mut top) = serde_json::from_slice::<RawMap>(body) else {
            return RawInput::default();
        };
        let questions = top
            .get("tool_input")
            .and_then(|input| serde_json::from_str::<RawMap>(input.get()).ok())
            .and_then(|mut input| input.remove("questions"));
        RawInput { questions, suggestions: top.remove("permission_suggestions") }
    }

    /// The suggestions text, only when it is a non-empty array.
    fn suggestions(&self) -> Option<&str> {
        let raw = self.suggestions.as_deref()?;
        match serde_json::from_str::<Value>(raw.get()) {
            Ok(Value::Array(items)) if !items.is_empty() => Some(raw.get()),
            _ => None,
        }
    }
}

const ALLOW_DECISION: &str = r#"{"behavior":"allow"}"#;
const DENY_DECISION: &str = r#"{"behavior":"deny","message":"Denied from Mayak"}"#;

/// What to print on stdout for `reply`: the documented `PermissionRequest`
/// decision (Claude and Codex) or the `PreToolUse` answer to `AskUserQuestion`
/// (Claude). None prints nothing: a release, "Always" for Codex (its hook fails
/// closed on `updatedPermissions`), an answer to anything but a question, or an
/// answer that does not fit the questions on stdin.
pub fn reply_json(event: &str, agent: Agent, reply: &Reply, raw: &RawInput) -> Option<String> {
    match (event, reply) {
        ("PermissionRequest", Reply::Allow) => Some(permission_json(ALLOW_DECISION)),
        ("PermissionRequest", Reply::Deny) => Some(permission_json(DENY_DECISION)),
        ("PermissionRequest", Reply::Always) => match (agent, raw.suggestions()) {
            (Agent::Codex, _) => None,
            (Agent::Claude, Some(rules)) => Some(permission_json(&format!(
                r#"{{"behavior":"allow","updatedPermissions":{rules}}}"#
            ))),
            // Nothing to remember: a plain allow.
            (Agent::Claude, None) => Some(permission_json(ALLOW_DECISION)),
        },
        ("PreToolUse", Reply::Answer(answers)) if agent == Agent::Claude => question_json(answers, raw),
        _ => None,
    }
}

fn permission_json(decision: &str) -> String {
    format!(r#"{{"hookSpecificOutput":{{"hookEventName":"PermissionRequest","decision":{decision}}}}}"#)
}

/// `updatedInput` = the untruncated `questions` echoed as stdin spelled them +
/// `answers`. Every answer must be a label or a non-empty array of labels, and
/// must name one of the questions.
fn question_json(answers: &Value, raw: &RawInput) -> Option<String> {
    let questions_raw = raw.questions.as_deref()?;
    let Ok(Value::Array(questions)) = serde_json::from_str::<Value>(questions_raw.get()) else {
        return None;
    };
    let Value::Object(answers) = answers else { return None };
    if questions.is_empty() || answers.is_empty() {
        return None;
    }
    let texts: Vec<&str> =
        questions.iter().filter_map(|q| q.get("question").and_then(Value::as_str)).collect();
    let mut keyed = Map::new();
    for (key, value) in answers {
        let labels = match value {
            Value::String(_) => true,
            Value::Array(items) => !items.is_empty() && items.iter().all(Value::is_string),
            _ => false,
        };
        if !labels {
            return None;
        }
        keyed.insert(full_question(key, &texts)?.to_string(), value.clone());
    }
    Some(format!(
        r#"{{"hookSpecificOutput":{{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":{{"questions":{q},"answers":{a}}}}}}}"#,
        q = questions_raw.get(),
        a = Value::Object(keyed),
    ))
}

/// The question an answer key names. The island only saw the payload cut to
/// `MAX_FIELD_LEN`, so a key may be a cut text: Claude Code needs the full one.
fn full_question<'a>(key: &str, texts: &[&'a str]) -> Option<&'a str> {
    texts.iter().copied().find(|t| *t == key).or_else(|| {
        texts.iter().copied().find(|t| {
            let mut cut = t.to_string();
            truncate_string(&mut cut);
            cut == key
        })
    })
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
        // ...so an imported Claude entry answers it the Codex way: never "Always".
        assert_eq!(reply_json("PermissionRequest", Agent::Codex, &Reply::Always, &RawInput::default()), None);
        // Other events keep argv's agent; a null turn_id is no turn_id.
        assert_eq!(detect_agent(Agent::Claude, "Stop", &codexish), Agent::Claude);
        let null_turn = obj(json!({ "turn_id": null }));
        assert_eq!(detect_agent(Agent::Claude, "PermissionRequest", &null_turn), Agent::Claude);
        assert_eq!(detect_agent(Agent::Codex, "Stop", &obj(json!({}))), Agent::Codex);
    }

    #[test]
    fn permissions_and_claude_questions_wait() {
        let bash = obj(json!({ "session_id": "s", "tool_name": "Bash" }));
        let question = obj(json!({ "session_id": "s", "tool_name": "AskUserQuestion" }));
        assert!(waits_for_answer(Agent::Claude, "PermissionRequest", &bash));
        assert!(waits_for_answer(Agent::Claude, "PreToolUse", &question));
        assert!(waits_for_answer(Agent::Codex, "PermissionRequest", &bash));
        assert!(!waits_for_answer(Agent::Claude, "PreToolUse", &bash));
        assert!(!waits_for_answer(Agent::Claude, "PreToolUse", &obj(json!({ "session_id": "s" }))));
        assert!(!waits_for_answer(Agent::Codex, "PreToolUse", &question));
        assert!(!waits_for_answer(Agent::Claude, "Stop", &question));
        assert!(!waits_for_answer(Agent::Codex, "Stop", &bash));
        assert!(!waits_for_answer(Agent::Claude, "", &bash));
    }

    #[test]
    fn every_wait_gets_the_answer_budget() {
        assert_eq!(ANSWER_BUDGET, Duration::from_secs(595));
        assert_eq!(budget(Agent::Claude, true), ANSWER_BUDGET);
        assert_eq!(budget(Agent::Codex, true), ANSWER_BUDGET);
        assert_eq!(budget(Agent::Claude, false), FIRE_AND_FORGET_BUDGET);
        assert_eq!(budget(Agent::Codex, false), CODEX_BUDGET);
        assert_eq!(CONNECT_TIMEOUT, Duration::from_millis(300));
    }

    #[test]
    fn only_claude_questions_and_real_permission_requests_reach_the_pipe() {
        let question = obj(json!({ "session_id": "s", "tool_name": "AskUserQuestion" }));
        let bash = obj(json!({ "session_id": "s", "tool_name": "Bash" }));
        assert!(relayed(Agent::Claude, "PreToolUse", &question));
        assert!(relayed(Agent::Claude, "PermissionRequest", &bash));
        // D87: a question arriving as a PermissionRequest is Claude's to ask.
        assert!(!relayed(Agent::Claude, "PermissionRequest", &question));
        assert!(!relayed(Agent::Claude, "PermissionRequest", &obj(json!({ "tool_name": "Bash" }))));
        // Another tool's PreToolUse, or a question without a Claude Code session.
        assert!(!relayed(Agent::Claude, "PreToolUse", &bash));
        assert!(!relayed(Agent::Claude, "PreToolUse", &obj(json!({ "session_id": "s" }))));
        assert!(!relayed(Agent::Claude, "PreToolUse", &obj(json!({ "tool_name": "AskUserQuestion" }))));
        // Every other event goes as before; Codex keeps feeding the observer.
        assert!(relayed(Agent::Claude, "Stop", &obj(json!({}))));
        assert!(relayed(Agent::Codex, "PreToolUse", &bash));
        assert!(relayed(Agent::Codex, "PermissionRequest", &obj(json!({ "turn_id": "t" }))));
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

    const ALLOW: &str =
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#;
    const DENY: &str = r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Denied from Mayak"}}}"#;
    /// `permission_suggestions` as Claude Code sends it, spacing kept on purpose.
    const SUGGESTIONS: &str = r#"[{"type":"addRules", "rules":[{"toolName":"Bash","ruleContent":"npm test"}],"behavior":"allow","destination":"localSettings"}]"#;
    /// `tool_input.questions`: one single-select question, keys not in sorted order.
    const ONE_QUESTION: &str = r#"[{"question":"Which framework?","header":"Framework","options":[{"label":"React","description":"Component library"},{"label":"Vue","description":"Progressive framework"}],"multiSelect":false}]"#;
    const TWO_QUESTIONS: &str = r#"[{"question":"Which framework?","header":"Framework","options":[{"label":"React","description":"a"},{"label":"Vue","description":"b"}],"multiSelect":false},{"question":"Which tests?","header":"Tests","options":[{"label":"Unit","description":"c"},{"label":"E2E","description":"d"}],"multiSelect":true}]"#;

    fn raw(stdin: &str) -> RawInput {
        RawInput::from_stdin(stdin.as_bytes())
    }

    fn question_stdin(questions: &str) -> String {
        format!(
            r#"{{"session_id":"s","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_input":{{"questions":{questions}}}}}"#
        )
    }

    fn answer(answers: Value) -> Reply {
        Reply::Answer(answers)
    }

    fn answer_json(questions: &str, answers: &str) -> String {
        format!(
            r#"{{"hookSpecificOutput":{{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":{{"questions":{questions},"answers":{answers}}}}}}}"#
        )
    }

    #[test]
    fn permission_replies_match_the_documented_shape() {
        let none = RawInput::default();
        for agent in [Agent::Claude, Agent::Codex] {
            assert_eq!(reply_json("PermissionRequest", agent, &Reply::Allow, &none).as_deref(), Some(ALLOW));
            assert_eq!(reply_json("PermissionRequest", agent, &Reply::Deny, &none).as_deref(), Some(DENY));
        }
    }

    #[test]
    fn always_hands_back_the_suggestions_verbatim() {
        let stdin = format!(r#"{{"session_id":"s","tool_name":"Bash","permission_suggestions":{SUGGESTIONS}}}"#);
        assert_eq!(
            reply_json("PermissionRequest", Agent::Claude, &Reply::Always, &raw(&stdin)).unwrap(),
            format!(
                r#"{{"hookSpecificOutput":{{"hookEventName":"PermissionRequest","decision":{{"behavior":"allow","updatedPermissions":{SUGGESTIONS}}}}}}}"#
            )
        );
        // Codex fails closed on updatedPermissions: no "Always" for it at all.
        assert_eq!(reply_json("PermissionRequest", Agent::Codex, &Reply::Always, &raw(&stdin)), None);
    }

    #[test]
    fn always_without_suggestions_is_a_plain_allow() {
        for stdin in [
            r#"{"session_id":"s"}"#,
            r#"{"session_id":"s","permission_suggestions":[]}"#,
            r#"{"session_id":"s","permission_suggestions":null}"#,
            r#"{"session_id":"s","permission_suggestions":{"type":"addRules"}}"#,
        ] {
            assert_eq!(
                reply_json("PermissionRequest", Agent::Claude, &Reply::Always, &raw(stdin)).as_deref(),
                Some(ALLOW),
                "{stdin}"
            );
        }
        assert_eq!(
            reply_json("PermissionRequest", Agent::Claude, &Reply::Always, &RawInput::default()).as_deref(),
            Some(ALLOW)
        );
    }

    #[test]
    fn a_single_answer_echoes_the_questions() {
        let input = raw(&question_stdin(ONE_QUESTION));
        assert_eq!(
            reply_json("PreToolUse", Agent::Claude, &answer(json!({ "Which framework?": "React" })), &input).unwrap(),
            answer_json(ONE_QUESTION, r#"{"Which framework?":"React"}"#)
        );
    }

    #[test]
    fn a_multi_select_answer_is_an_array_of_labels() {
        let input = raw(&question_stdin(TWO_QUESTIONS));
        assert_eq!(
            reply_json("PreToolUse", Agent::Claude, &answer(json!({ "Which tests?": ["Unit", "E2E"] })), &input)
                .unwrap(),
            answer_json(TWO_QUESTIONS, r#"{"Which tests?":["Unit","E2E"]}"#)
        );
    }

    #[test]
    fn two_questions_get_one_answer_each() {
        let input = raw(&question_stdin(TWO_QUESTIONS));
        let reply = answer(json!({ "Which framework?": "Vue", "Which tests?": ["E2E"] }));
        assert_eq!(
            reply_json("PreToolUse", Agent::Claude, &reply, &input).unwrap(),
            answer_json(TWO_QUESTIONS, r#"{"Which framework?":"Vue","Which tests?":["E2E"]}"#)
        );
    }

    #[test]
    fn the_questions_are_echoed_byte_for_byte_whatever_the_spacing() {
        let spaced = r#"[ {"question" : "Café é?", "header":"H", "options":[{"label":"A","description":""},{"label":"B","description":""}], "multiSelect":false} ]"#;
        let input = raw(&question_stdin(spaced));
        assert_eq!(
            reply_json("PreToolUse", Agent::Claude, &answer(json!({ "Café é?": "A" })), &input).unwrap(),
            answer_json(spaced, r#"{"Café é?":"A"}"#)
        );
    }

    #[test]
    fn a_huge_question_is_echoed_untruncated_and_the_pipe_copy_stays_cut() {
        let text = "Q".repeat(5_000);
        let questions = format!(
            r#"[{{"question":"{text}","header":"Long","options":[{{"label":"Yes","description":"y"}},{{"label":"No","description":"n"}}],"multiSelect":false}}]"#
        );
        let stdin = question_stdin(&questions);
        // What main does: keep the raw copy, then cut the payload for the pipe.
        let input = raw(&stdin);
        let mut payload: Map<String, Value> = serde_json::from_str(&stdin).unwrap();
        payload.values_mut().for_each(truncate_strings);
        let sent = payload["tool_input"]["questions"][0]["question"].as_str().unwrap().to_string();
        assert!(sent.len() < text.len() && sent.ends_with('…'));
        assert!(!to_line(&payload).contains(&text));

        // The island keys its answer by the cut text it saw; Claude gets the full one.
        let printed = reply_json("PreToolUse", Agent::Claude, &answer(json!({ sent: "Yes" })), &input).unwrap();
        assert_eq!(printed, answer_json(&questions, &format!(r#"{{"{text}":"Yes"}}"#)));
        let back: Value = serde_json::from_str(&printed).unwrap();
        assert_eq!(back["hookSpecificOutput"]["updatedInput"]["questions"][0]["question"], text.as_str());
    }

    #[test]
    fn release_and_unusable_answers_print_nothing() {
        let question = raw(&question_stdin(ONE_QUESTION));
        let good = || answer(json!({ "Which framework?": "React" }));
        for agent in [Agent::Claude, Agent::Codex] {
            for event in ["PermissionRequest", "PreToolUse", "Stop"] {
                assert_eq!(reply_json(event, agent, &Reply::Release, &question), None, "{event}");
            }
        }
        // A garbage pipe line is a release.
        assert_eq!(reply_json("PermissionRequest", Agent::Claude, &parse_reply("maybe"), &question), None);
        // An answer belongs to a question; a decision to a permission request.
        assert_eq!(reply_json("PermissionRequest", Agent::Claude, &good(), &question), None);
        assert_eq!(reply_json("PermissionRequest", Agent::Codex, &good(), &question), None);
        assert_eq!(reply_json("Stop", Agent::Claude, &good(), &question), None);
        assert_eq!(reply_json("PreToolUse", Agent::Codex, &good(), &question), None);
        for decision in [Reply::Allow, Reply::Always, Reply::Deny] {
            assert_eq!(reply_json("PreToolUse", Agent::Claude, &decision, &question), None);
            assert_eq!(reply_json("Stop", Agent::Claude, &decision, &question), None);
        }
        // The answers object is required, filled, and only labels.
        for bad in [
            json!("React"),
            json!(["React"]),
            json!({}),
            json!({ "Which framework?": 1 }),
            json!({ "Which framework?": [] }),
            json!({ "Which framework?": ["React", 2] }),
            json!({ "Which framework?": null }),
            json!({ "Some other question?": "React" }),
        ] {
            assert_eq!(reply_json("PreToolUse", Agent::Claude, &answer(bad.clone()), &question), None, "{bad}");
        }
        // No questions on stdin: nothing to echo, nothing to print.
        for stdin in [
            r#"{"session_id":"s","tool_name":"AskUserQuestion"}"#,
            r#"{"session_id":"s","tool_input":{"questions":[]}}"#,
            r#"{"session_id":"s","tool_input":{"questions":"Which?"}}"#,
        ] {
            assert_eq!(reply_json("PreToolUse", Agent::Claude, &good(), &raw(stdin)), None, "{stdin}");
        }
        assert_eq!(reply_json("PreToolUse", Agent::Claude, &good(), &RawInput::default()), None);
        assert_eq!(reply_json("PreToolUse", Agent::Claude, &good(), &raw("not json")), None);
    }

    #[test]
    fn parse_reply_reads_every_form() {
        assert_eq!(parse_reply("allow"), Reply::Allow);
        assert_eq!(parse_reply("always"), Reply::Always);
        assert_eq!(parse_reply("deny"), Reply::Deny);
        assert_eq!(parse_reply("release"), Reply::Release);
        assert_eq!(parse_reply("  deny\r\n"), Reply::Deny);
        assert_eq!(
            parse_reply(r#"answer {"Which framework?":"React","Which tests?":["Unit"]}"#),
            Reply::Answer(json!({ "Which framework?": "React", "Which tests?": ["Unit"] }))
        );
        // Invalid or non-object JSON after "answer" is a release.
        assert_eq!(parse_reply(r#"answer {"Which framework?":"#), Reply::Release);
        assert_eq!(parse_reply(r#"answer ["React"]"#), Reply::Release);
        assert_eq!(parse_reply("answer"), Reply::Release);
        assert_eq!(parse_reply("answer "), Reply::Release);
        // Anything else is a release too.
        for line in ["", "maybe", "Allow", "allow now", r#"{"permissionDecision":"allow"}"#] {
            assert_eq!(parse_reply(line), Reply::Release, "{line}");
        }
    }

    #[test]
    fn a_question_gets_its_answer_through_the_pipe() {
        let name = unique_pipe("answer");
        let server = serve_once(&name, Some("answer {\"Which framework?\":\"Vue\"}\n"), Duration::ZERO);
        let line = talk_to(&name, "{\"tool_name\":\"AskUserQuestion\"}\n", true, Duration::from_secs(5));
        let reply = parse_reply(line.as_deref().unwrap());
        assert_eq!(
            reply_json("PreToolUse", Agent::Claude, &reply, &raw(&question_stdin(ONE_QUESTION))).unwrap(),
            answer_json(ONE_QUESTION, r#"{"Which framework?":"Vue"}"#)
        );
        assert_eq!(server.join().unwrap(), "{\"tool_name\":\"AskUserQuestion\"}\n");
    }

    #[test]
    fn a_codex_permission_waits_for_allow_or_deny() {
        for (line, printed) in [("allow\n", ALLOW), ("deny\n", DENY)] {
            let name = unique_pipe("codex-ask");
            let server = serve_once(&name, Some(line), Duration::ZERO);
            let reply = talk_to(&name, "{\"agent\":\"codex\"}\n", true, budget(Agent::Codex, true));
            let reply = reply.as_deref().map_or(Reply::Release, parse_reply);
            assert_eq!(
                reply_json("PermissionRequest", Agent::Codex, &reply, &RawInput::default()).as_deref(),
                Some(printed)
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn a_release_line_prints_nothing() {
        let name = unique_pipe("release");
        let server = serve_once(&name, Some("release\n"), Duration::ZERO);
        let reply = talk_to(&name, "{}\n", true, Duration::from_secs(5));
        assert_eq!(reply.as_deref(), Some("release"));
        let reply = parse_reply(reply.as_deref().unwrap());
        assert_eq!(reply_json("PermissionRequest", Agent::Claude, &reply, &RawInput::default()), None);
        server.join().unwrap();
    }

    #[test]
    fn no_reply_before_the_budget_prints_nothing() {
        let name = unique_pipe("timeout");
        let server = serve_once(&name, None, Duration::from_secs(1));
        let reply = talk_to(&name, "{}\n", true, Duration::from_millis(300));
        assert_eq!(reply, None);
        let reply = reply.as_deref().map_or(Reply::Release, parse_reply);
        assert_eq!(reply, Reply::Release);
        assert_eq!(reply_json("PreToolUse", Agent::Claude, &reply, &raw(&question_stdin(ONE_QUESTION))), None);
        server.join().unwrap();
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
        // closed pipe ends the wait at once, not the 595 s answer budget.
        let name = unique_pipe("gone");
        let server = serve_once(&name, None, Duration::from_millis(300));
        let started = Instant::now();
        let reply = talk_to(&name, "{\"hook_event_name\":\"PermissionRequest\"}\n", true, ANSWER_BUDGET);
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
