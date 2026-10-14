// The tools Mayak offers over MCP, their schemas, and what each one does with the app.
//
// Read-only tools only hand back what the island itself already shows (the sessions snapshot the
// page gets, the limits in it, the Stats tab, the Apps tab, the Focus timer, the Agent Orchestrator
// sessions it last read). Two tools write, and only to the island: `ask_user` puts a question card
// on it and waits for the user's click, and `set_status` puts an agent's own line in its chat's
// column. Nothing is answered, approved or sent on anyone's behalf.

use std::future::Future;
use std::time::Duration;

use serde_json::{json, Map, Value};

/// How long `ask_user` waits when the caller names no time: under the 60 s after which Claude Code
/// ends a call of an HTTP server that sets no `timeout` of its own (read off a real run).
pub const ASK_DEFAULT_SECS: u64 = 45;
/// The longest it may wait. A client that ends the call sooner (see `attach`) gives up first.
pub const ASK_MAX_SECS: u64 = 240;
const ASK_MIN_SECS: u64 = 5;

const QUESTION_MAX_CHARS: usize = 500;
const HEADER_MAX_CHARS: usize = 40;
const LABEL_MAX_CHARS: usize = 100;
const DESCRIPTION_MAX_CHARS: usize = 300;
const KEY_MAX_CHARS: usize = 200;
/// Projects `get_stats` lists; the rest are only counted.
const STATS_PROJECTS: usize = 20;
/// The longest `set_status` line: the chat's column under Luch is narrow.
pub const STATUS_MAX_CHARS: usize = 60;

/// What the tools need of the app. `lib.rs` implements it with the observer, the island window and
/// the pending-request table; the tests with a fake.
pub trait Backend: Send + Sync + 'static {
    /// The sessions snapshot, serialized as the island's page gets it.
    fn snapshot(&self) -> Value;
    /// The Stats tab's report for 7 or 30 days.
    fn stats(&self, days: u32) -> impl Future<Output = Result<Value, String>> + Send;
    /// The Apps tab's tiles.
    fn apps(&self) -> impl Future<Output = Value> + Send;
    /// Puts the question on the island and waits for the click (or for `wait` to pass).
    fn ask(&self, ask: AskRequest) -> impl Future<Output = AskOutcome> + Send;
    /// The island's running pomodoro as the page last told it (`pomodoro::Focus`), or null.
    fn focus(&self) -> Value;
    /// Now, in ms since the Unix epoch.
    fn now_ms(&self) -> i64;
    /// Whether the Agent Orchestrator pill is on, and what its last poll read.
    fn ao(&self) -> AoRead;
    /// Shows `text` as the agent's own line in chat conversation `chat` (None clears it).
    fn set_status(&self, chat: u64, text: Option<String>) -> StatusShown;
}

/// The AO pill's switch and its last read: when (ms since the Unix epoch) and the pill's `data`.
#[derive(Debug, Clone, PartialEq)]
pub struct AoRead {
    pub enabled: bool,
    pub last: Option<(i64, Value)>,
}

/// What became of a `set_status` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusShown {
    Shown,
    /// The chat has no turn running.
    NoTurn,
    /// No open chat has that id.
    NoChat,
}

/// Who is calling, as far as the HTTP request tells: labels, never a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    /// The name on a question card.
    pub label: String,
    /// The chat conversation the agent runs in (`attach::CHAT_HEADER`), for `set_status`.
    pub chat: Option<u64>,
}

impl Caller {
    /// From the `User-Agent` header and the chat header. Only the two clients whose headers were
    /// read on a real connection get their own name; any other product token is shown as sent,
    /// cleaned and cut. A chat header that is not a plain number names no chat.
    pub fn from_request(user_agent: Option<&str>, chat: Option<&str>) -> Caller {
        let product = user_agent.and_then(|ua| ua.split(['/', ' ']).next()).unwrap_or_default();
        let label = match product {
            "claude-code" => "Claude Code".to_string(),
            "codex-mcp-client" => "Codex".to_string(),
            other => {
                let clean: String = other.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.').take(24).collect();
                if clean.is_empty() { "Agent".to_string() } else { clean }
            }
        };
        let chat = chat
            .filter(|v| (1..=19).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|id| *id > 0);
        Caller { label, chat }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskRequest {
    pub asker: String,
    pub header: String,
    pub question: String,
    pub options: Vec<AskOption>,
    pub multi_select: bool,
    pub wait: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AskOutcome {
    /// What the card sent back: the question text → a label, a list of labels or `{"other": text}`.
    Answered(Value),
    /// The island let the card go: it was folded, the user chose "In terminal", the island is
    /// paused or already showed another card.
    Released,
    /// The island never took the card (closed, or its page is not listening).
    NotShown,
    TimedOut,
    /// Another question of an MCP agent is still on the island.
    Busy,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolOutput {
    /// A result for the model.
    Text(String),
    /// A failure of the call itself that the model may correct and retry.
    Error(String),
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
}

fn read_only(name: &str, title: &str, description: &str, input: Value) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": input,
        "annotations": { "readOnlyHint": true, "openWorldHint": false },
    })
}

/// `tools/list`: read-only tools first, in a fixed order.
pub fn definitions() -> Vec<Value> {
    vec![
        read_only(
            "list_sessions",
            "List agent sessions",
            "The coding-agent sessions Mayak sees (Claude Code, Codex, Antigravity), grouped by project, \
             with their phase (working, waiting for an answer, finished, error), what they are doing and \
             how full their context is. Only what the Mayak island itself shows.",
            schema(
                json!({ "needs_attention": { "type": "boolean", "description": "Only sessions that wait for an answer or ended in an error." } }),
                &[],
            ),
        ),
        read_only(
            "get_session",
            "Get one session",
            "One session from list_sessions by its key, with its last turn's outcome and changed files.",
            schema(json!({ "key": { "type": "string", "description": "The key list_sessions gave." } }), &["key"]),
        ),
        read_only(
            "get_limits",
            "Plan limits and balances",
            "The plan limit windows of Claude Code and Codex (percent used, when they reset) and the \
             OpenRouter, DeepSeek and Moonshot balances, when Mayak has them.",
            schema(json!({}), &[]),
        ),
        read_only(
            "get_stats",
            "Token statistics",
            "Tokens used per day, project and model over the last 7 or 30 days, counted from the agents' \
             own files. The first count after a long time can take a while.",
            schema(json!({ "days": { "type": "integer", "enum": [7, 30], "description": "The period. 7 by default." } }), &[]),
        ),
        read_only(
            "list_apps",
            "List the Apps tab",
            "The IDEs and agent apps on the island's Apps tab: id, name, whether it is the user's own app, \
             and `open`: whether it has a window open now (null where Mayak cannot tell: Claude and Codex, \
             which open by a link or as a package, and any app on macOS or Linux).",
            schema(json!({}), &[]),
        ),
        read_only(
            "get_focus",
            "The user's Focus timer",
            "Whether the island's Focus timer (a pomodoro) runs: the phase (work, a short or a long break), \
             the work cycle, when the phase ends, and whether agent alerts are held back meanwhile. Worth \
             reading before interrupting the user.",
            schema(json!({}), &[]),
        ),
        read_only(
            "list_ao_sessions",
            "List Agent Orchestrator sessions",
            "The Agent Orchestrator (AO) sessions Mayak last read: id, project, role, name, status, \
             activity, harness, pull requests and CI. Mayak reads AO only while its pill is on and the \
             island is on screen, so `readAt` says how old the list is; nothing is asked of AO for this call.",
            schema(json!({}), &[]),
        ),
        json!({
            "name": "ask_user",
            "title": "Ask the user on the Mayak island",
            "description": format!(
                "Shows a question with 2 to 4 options on the user's Mayak island and waits for their click. \
                 Nothing is decided without that click; the user may also type their own answer. It returns \
                 the chosen option(s), or says why there is no answer (the user closed the island or chose to \
                 answer in the terminal, the island is busy, or {ASK_DEFAULT_SECS} seconds passed). To only \
                 inform the user, offer options like \"OK\" and \"Not now\". Waits {ASK_DEFAULT_SECS} s unless \
                 timeout_seconds says otherwise (at most {ASK_MAX_SECS}); a client ends the call at its own tool \
                 timeout if that is shorter (Claude Code: 60 s unless its server entry sets a larger timeout)."
            ),
            "inputSchema": schema(
                json!({
                    "question": { "type": "string", "maxLength": QUESTION_MAX_CHARS, "description": "The question, in the user's language." },
                    "options": {
                        "type": "array", "minItems": 2, "maxItems": 4,
                        "description": "2 to 4 distinct options: a short label, or {label, description}.",
                        "items": { "anyOf": [
                            { "type": "string", "maxLength": LABEL_MAX_CHARS },
                            { "type": "object", "properties": {
                                "label": { "type": "string", "maxLength": LABEL_MAX_CHARS },
                                "description": { "type": "string", "maxLength": DESCRIPTION_MAX_CHARS } },
                              "required": ["label"], "additionalProperties": false },
                        ] },
                    },
                    "header": { "type": "string", "maxLength": HEADER_MAX_CHARS, "description": "A short tag above the question." },
                    "multi_select": { "type": "boolean", "description": "Let the user tick several options." },
                    "timeout_seconds": { "type": "integer", "minimum": ASK_MIN_SECS, "maximum": ASK_MAX_SECS },
                }),
                &["question", "options"],
            ),
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false },
        }),
        json!({
            "name": "set_status",
            "title": "Say what you are doing, in Mayak's chat",
            "description": format!(
                "For an agent running in Mayak's own chat: a short line ({STATUS_MAX_CHARS} characters at most) \
                 about what you are doing, shown under Luch in that chat's column until your turn ends. An empty \
                 text clears it. Called from anywhere else it shows nothing and says so."
            ),
            "inputSchema": schema(
                json!({ "text": { "type": "string", "maxLength": STATUS_MAX_CHARS, "description": "One short line; empty clears it." } }),
                &["text"],
            ),
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
        }),
    ]
}

/// The tool names a headless Mayak chat may call without a prompt: the read-only ones and the two
/// that only reach the island (`ask_user`, whose gate is the user's click, and `set_status`).
pub fn names() -> Vec<String> {
    definitions().iter().filter_map(|d| d["name"].as_str().map(String::from)).collect()
}

/// Is `name` a tool of this server.
pub fn exists(name: &str) -> bool {
    definitions().iter().any(|d| d["name"].as_str() == Some(name))
}

fn text(value: &Value) -> ToolOutput {
    ToolOutput::Text(value.to_string())
}

/// Runs the tool `name`. The caller has checked `exists(name)`.
pub async fn call<B: Backend>(backend: &B, caller: &Caller, name: &str, args: &Value) -> ToolOutput {
    let Some(args) = args.as_object() else {
        return ToolOutput::Error("The arguments must be a JSON object.".into());
    };
    match name {
        "list_sessions" => list_sessions(backend, args),
        "get_session" => get_session(backend, args),
        "get_limits" => text(&backend.snapshot().get("usage").cloned().unwrap_or(Value::Null)),
        "get_stats" => get_stats(backend, args).await,
        "list_apps" => text(&backend.apps().await),
        "get_focus" => get_focus(backend),
        "list_ao_sessions" => list_ao_sessions(backend),
        "ask_user" => ask_user(backend, caller, args).await,
        "set_status" => set_status(backend, caller, args),
        _ => ToolOutput::Error(format!("Unknown tool: {name}")),
    }
}

/// The fields of a session the tools pass on: what the island's own rows show. Never the working
/// folder, the last reply or a process id: the page does not get those either.
const SESSION_FIELDS: [&str; 12] = [
    "key", "agent", "host", "title", "phase", "since", "activity", "detail", "attention", "model", "context", "outcome",
];

fn pick(session: &Value, fields: &[&str]) -> Value {
    let mut out = Map::new();
    for field in fields {
        if let Some(v) = session.get(*field).filter(|v| !v.is_null()) {
            out.insert((*field).to_string(), v.clone());
        }
    }
    Value::Object(out)
}

fn list_sessions(backend: &impl Backend, args: &Map<String, Value>) -> ToolOutput {
    let only_attention = match args.get("needs_attention") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return ToolOutput::Error("needs_attention must be true or false.".into()),
    };
    let snapshot = backend.snapshot();
    let ao = ao_sessions(&backend.ao());
    let mut projects = Vec::new();
    for project in snapshot["projects"].as_array().into_iter().flatten() {
        let sessions: Vec<Value> = project["sessions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| !only_attention || !s["attention"].is_null())
            // Everything but the outcome, which only `get_session` carries.
            .map(|s| pick(s, &SESSION_FIELDS[..SESSION_FIELDS.len() - 1]))
            .collect();
        if sessions.is_empty() {
            continue;
        }
        let mut entry = json!({ "name": project["name"], "root": project["root"], "sessions": sessions });
        if let Some(tag) = ao_for_project(&ao, project) {
            entry["ao"] = json!({ "id": tag["id"], "name": tag["name"], "role": tag["role"], "status": tag["status"] });
        }
        projects.push(entry);
    }
    text(&json!({ "now": snapshot["now"], "counts": snapshot["counts"], "projects": projects }))
}

/// The AO sessions of the last read, when the pill is on; none otherwise (an old read of a pill
/// switched off since is not handed out).
fn ao_sessions(read: &AoRead) -> Vec<Value> {
    if !read.enabled {
        return Vec::new();
    }
    read.last.as_ref().and_then(|(_, data)| data["sessions"].as_array().cloned()).unwrap_or_default()
}

/// Whether the project key `project` is the AO session's folder `workspace` or lies inside it (both
/// as `observe::paths` keys): src/core/ao.ts `isInside`.
fn is_inside(project: &str, workspace: &str) -> bool {
    !project.is_empty()
        && !workspace.is_empty()
        && (project == workspace
            || project.strip_prefix(workspace).is_some_and(|rest| rest.starts_with('\\') || rest.starts_with('/')))
}

/// The AO session AO lists under the ids an agent's own environment named (a row's `ao`), as
/// src/core/ao.ts `byRef` finds it: the same session id, and the same project when both name one.
fn ao_by_ref<'a>(sessions: &'a [Value], ids: &Value) -> Option<&'a Value> {
    let id = ids.get("session")?.as_str()?;
    let project = ids.get("project").and_then(Value::as_str);
    sessions.iter().find(|s| {
        s["id"].as_str() == Some(id) && (project.is_none() || s["project"].is_null() || s["project"].as_str() == project)
    })
}

/// The AO session of a project of the snapshot, as the island's project header picks it
/// (src/core/ao.ts `aoForProject`): its folder first, else the first of its rows whose ids AO knows.
fn ao_for_project<'a>(sessions: &'a [Value], project: &Value) -> Option<&'a Value> {
    if let Some(found) = project["key"].as_str().and_then(|key| ao_tag(sessions, key)) {
        return Some(found);
    }
    project["sessions"].as_array().into_iter().flatten().find_map(|row| ao_by_ref(sessions, &row["ao"]))
}

/// The AO session a project runs in by its folder (src/core/ao.ts `tagFor`): the one whose folder
/// holds the project, the deepest folder winning, the first on a tie.
fn ao_tag<'a>(sessions: &'a [Value], project_key: &str) -> Option<&'a Value> {
    let mut found: Option<(&Value, usize)> = None;
    for s in sessions {
        let Some(workspace) = s["workspace"].as_str() else { continue };
        if is_inside(project_key, workspace) && found.is_none_or(|(_, len)| workspace.len() > len) {
            found = Some((s, workspace.len()));
        }
    }
    found.map(|(s, _)| s)
}

fn get_focus(backend: &impl Backend) -> ToolOutput {
    let focus = backend.focus();
    let Some(run) = focus.as_object() else {
        return text(&json!({ "running": false }));
    };
    let ends_at = run.get("endsAt").and_then(Value::as_i64).unwrap_or(0);
    let left = ends_at - backend.now_ms();
    text(&json!({
        "running": true,
        "phase": run.get("phase"),
        "cycle": run.get("cycle"),
        "cycles": run.get("cycles"),
        "endsAt": ends_at,
        // Whole minutes, rounded up, as the island counts them (pomodoro.ts `minutesLeft`).
        "minutesLeft": if left > 0 { (left + 59_999) / 60_000 } else { 0 },
        "alertsHeld": run.get("muted"),
        // A phase that ends while the island is hidden moves on only when it shows again.
        "phaseOver": left <= 0,
    }))
}

fn list_ao_sessions(backend: &impl Backend) -> ToolOutput {
    let read = backend.ao();
    if !read.enabled {
        return text(&json!({
            "enabled": false,
            "message": "The Agent Orchestrator pill is off in Mayak, so Mayak does not read AO.",
        }));
    }
    match read.last {
        None => text(&json!({
            "enabled": true,
            "readAt": null,
            "message": "Mayak reads AO while the island is on screen, and has not read it yet.",
        })),
        Some((at, data)) => text(&json!({
            "enabled": true,
            "readAt": at,
            "status": data["status"],
            "sessions": data.get("sessions").cloned().unwrap_or_else(|| json!([])),
        })),
    }
}

/// `text` on one line: control characters become spaces, runs of spaces one, no space at either end.
fn one_line(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn set_status(backend: &impl Backend, caller: &Caller, args: &Map<String, Value>) -> ToolOutput {
    let Some(raw) = args.get("text").and_then(Value::as_str) else {
        return ToolOutput::Error("text must be a text; an empty one clears the line.".into());
    };
    let line = one_line(raw);
    if line.chars().count() > STATUS_MAX_CHARS {
        return ToolOutput::Error(format!("text must be at most {STATUS_MAX_CHARS} characters."));
    }
    let Some(chat) = caller.chat else {
        return text(&json!({
            "shown": false,
            "reason": "not_in_chat",
            "message": "Only an agent running in Mayak's own chat has a place for a status line. Nothing was shown.",
        }));
    };
    text(&match backend.set_status(chat, (!line.is_empty()).then_some(line)) {
        StatusShown::Shown => json!({ "shown": true }),
        StatusShown::NoTurn => json!({
            "shown": false,
            "reason": "no_turn",
            "message": "That chat has no turn running, so there is nothing to show the line on.",
        }),
        StatusShown::NoChat => json!({ "shown": false, "reason": "no_chat", "message": "That chat is closed." }),
    })
}

fn get_session(backend: &impl Backend, args: &Map<String, Value>) -> ToolOutput {
    let Some(key) = args.get("key").and_then(Value::as_str).filter(|k| !k.is_empty() && k.chars().count() <= KEY_MAX_CHARS) else {
        return ToolOutput::Error("key must be a session key from list_sessions.".into());
    };
    let snapshot = backend.snapshot();
    for project in snapshot["projects"].as_array().into_iter().flatten() {
        for session in project["sessions"].as_array().into_iter().flatten() {
            if session["key"].as_str() != Some(key) {
                continue;
            }
            let mut out = pick(session, &SESSION_FIELDS);
            if let Some(turn) = session.get("turn").filter(|t| !t.is_null()) {
                out["turn"] = turn.clone();
            }
            out["project"] = json!({ "name": project["name"], "root": project["root"] });
            out["now"] = snapshot["now"].clone();
            return text(&out);
        }
    }
    ToolOutput::Error(format!("No session with the key {key}. The list changes: call list_sessions again."))
}

async fn get_stats(backend: &impl Backend, args: &Map<String, Value>) -> ToolOutput {
    let days = match args.get("days") {
        None | Some(Value::Null) => 7,
        Some(v) => match v.as_u64() {
            Some(d @ (7 | 30)) => d as u32,
            _ => return ToolOutput::Error("days must be 7 or 30.".into()),
        },
    };
    match backend.stats(days).await {
        Ok(mut report) => {
            // The projects come largest first; a month of work can list over a hundred.
            if let Some(projects) = report.get_mut("projects").and_then(Value::as_array_mut) {
                if projects.len() > STATS_PROJECTS {
                    let omitted = projects.len() - STATS_PROJECTS;
                    projects.truncate(STATS_PROJECTS);
                    report["projectsOmitted"] = json!(omitted);
                }
            }
            text(&report)
        }
        Err(err) => ToolOutput::Error(format!("The count failed: {err}")),
    }
}

/// A string argument that must be there, non-blank and at most `max` characters.
fn bounded(args: &Map<String, Value>, field: &str, max: usize) -> Result<String, String> {
    match args.get(field).and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() && s.chars().count() <= max => Ok(s.to_string()),
        _ => Err(format!("{field} must be a text of 1 to {max} characters.")),
    }
}

fn one_option(raw: &Value) -> Result<AskOption, String> {
    let (label, description) = match raw {
        Value::String(s) => (s.trim().to_string(), String::new()),
        Value::Object(o) => {
            let label = o.get("label").and_then(Value::as_str).map(|s| s.trim().to_string()).unwrap_or_default();
            let description = match o.get("description") {
                None | Some(Value::Null) => String::new(),
                Some(Value::String(s)) => s.trim().to_string(),
                Some(_) => return Err("an option's description must be a text.".into()),
            };
            (label, description)
        }
        _ => return Err("each option is a text or {label, description}.".into()),
    };
    if label.is_empty() || label.chars().count() > LABEL_MAX_CHARS {
        return Err(format!("an option's label must be 1 to {LABEL_MAX_CHARS} characters."));
    }
    if description.chars().count() > DESCRIPTION_MAX_CHARS {
        return Err(format!("an option's description must be at most {DESCRIPTION_MAX_CHARS} characters."));
    }
    Ok(AskOption { label, description })
}

/// The `ask_user` arguments as the island's question card needs them (2 to 4 options with
/// distinct labels). Anything else is an error the model can fix.
pub fn parse_ask(caller: &Caller, args: &Map<String, Value>) -> Result<AskRequest, String> {
    let question = bounded(args, "question", QUESTION_MAX_CHARS)?;
    let header = match args.get("header") {
        None | Some(Value::Null) => String::new(),
        Some(_) => bounded(args, "header", HEADER_MAX_CHARS)?,
    };
    let Some(raw) = args.get("options").and_then(Value::as_array).filter(|o| (2..=4).contains(&o.len())) else {
        return Err("options must be a list of 2 to 4 options.".into());
    };
    let mut options: Vec<AskOption> = Vec::new();
    for item in raw {
        let option = one_option(item)?;
        if options.iter().any(|o| o.label == option.label) {
            return Err("the options' labels must differ.".into());
        }
        options.push(option);
    }
    let multi_select = match args.get("multi_select") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("multi_select must be true or false.".into()),
    };
    let secs = match args.get("timeout_seconds") {
        None | Some(Value::Null) => ASK_DEFAULT_SECS,
        Some(v) => match v.as_u64() {
            Some(s) if (ASK_MIN_SECS..=ASK_MAX_SECS).contains(&s) => s,
            _ => return Err(format!("timeout_seconds must be {ASK_MIN_SECS} to {ASK_MAX_SECS}.")),
        },
    };
    Ok(AskRequest { asker: caller.label.clone(), header, question, options, multi_select, wait: Duration::from_secs(secs) })
}

async fn ask_user(backend: &impl Backend, caller: &Caller, args: &Map<String, Value>) -> ToolOutput {
    let ask = match parse_ask(caller, args) {
        Ok(ask) => ask,
        Err(reason) => return ToolOutput::Error(reason),
    };
    let question = ask.question.clone();
    let wait = ask.wait.as_secs();
    let outcome = backend.ask(ask).await;
    text(&ask_result(&question, wait, &outcome))
}

/// What the model is told about an outcome.
pub fn ask_result(question: &str, wait_secs: u64, outcome: &AskOutcome) -> Value {
    let not_answered = |reason: &str, message: String| json!({ "answered": false, "reason": reason, "message": message });
    match outcome {
        AskOutcome::Answered(answers) => match answers.get(question) {
            Some(Value::String(label)) => json!({ "answered": true, "answer": label }),
            Some(Value::Array(labels)) => json!({ "answered": true, "answers": labels }),
            Some(Value::Object(other)) => json!({ "answered": true, "other": other.get("other") }),
            _ => not_answered("no_answer", "The card came back without an answer to this question.".into()),
        },
        AskOutcome::Released => not_answered(
            "dismissed",
            "The island did not keep the question: the user closed it or chose to answer in the terminal, or the island is paused or showing another card. Ask in your own window, or go on without.".into(),
        ),
        AskOutcome::NotShown => not_answered(
            "not_shown",
            "The island is not showing questions (it is closed, or its window is not listening). Ask in your own window.".into(),
        ),
        AskOutcome::TimedOut => not_answered(
            "timeout",
            format!("The user did not answer within {wait_secs} seconds; the question was withdrawn."),
        ),
        AskOutcome::Busy => not_answered(
            "busy",
            "Another question from an agent is still waiting on the island. Try again in a moment.".into(),
        ),
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Mutex;

    /// A backend with a fixed snapshot and scripted answers.
    pub struct Fake {
        pub snapshot: Value,
        pub answer: Mutex<AskOutcome>,
        pub asked: Mutex<Vec<AskRequest>>,
        pub stats_days: Mutex<Vec<u32>>,
        pub focus: Mutex<Value>,
        pub ao: Mutex<AoRead>,
        pub status_answer: Mutex<StatusShown>,
        pub statuses: Mutex<Vec<(u64, Option<String>)>>,
    }

    /// The fake's clock.
    pub const NOW: i64 = 10_000_000;

    impl Fake {
        pub fn new() -> Fake {
            Fake {
                // The wire shape of `observe::model::SessionsSnapshot`
                // (`snapshot_serializes_to_the_spec_wire_shape`), with a few more fields.
                snapshot: json!({
                    "rev": 3,
                    "now": 1_000,
                    "counts": { "waiting": 1, "error": 0, "finished": 0, "working": 1 },
                    "projects": [
                        { "key": "c:\\work\\mayak", "name": "mayak", "root": "C:\\work\\mayak", "sessions": [
                            { "key": "claude:1:2", "agent": "claude", "host": "windowsTerminal", "title": "Fix the pipe",
                              "phase": { "kind": "working" }, "since": 10, "activity": "Editing", "detail": null,
                              "outcome": null, "attention": null, "acked": false, "model": "claude-x",
                              "context": { "used": 1000, "window": 200000 },
                              "turn": { "files": [], "fileCount": 0, "added": 0, "removed": 0, "partial": false } },
                            { "key": "codex:abc", "agent": "codex", "host": "vsCode", "title": null,
                              "phase": { "kind": "waiting", "wait": "permission", "label": "permission" }, "since": 20,
                              "activity": null, "detail": "Bash · rm -rf build", "attention": "permission",
                              "acked": false, "outcome": { "kind": "finished", "summary": "Done", "at": 5 } },
                        ] },
                        { "key": "c:\\work\\empty", "name": "empty", "root": "C:\\work\\empty", "sessions": [] },
                    ],
                    "attention": ["codex:abc"],
                    "hold": true,
                    "aggregate": "approval",
                    "usage": { "claude": { "fetchedAtMs": 1, "windows": [ { "id": "fiveHour", "percent": 12.5 } ] },
                        "codex": null, "openrouter": null, "openrouterProblem": null, "deepseek": null,
                        "deepseekProblem": null, "moonshot": null, "moonshotProblem": null },
                    "sources": { "claude": "ok", "codex": "ok", "codexHooks": "not_installed" },
                }),
                answer: Mutex::new(AskOutcome::TimedOut),
                asked: Mutex::new(Vec::new()),
                stats_days: Mutex::new(Vec::new()),
                focus: Mutex::new(Value::Null),
                ao: Mutex::new(AoRead { enabled: false, last: None }),
                status_answer: Mutex::new(StatusShown::Shown),
                statuses: Mutex::new(Vec::new()),
            }
        }
    }

    impl Backend for Fake {
        fn snapshot(&self) -> Value {
            self.snapshot.clone()
        }

        async fn stats(&self, days: u32) -> Result<Value, String> {
            self.stats_days.lock().unwrap().push(days);
            let projects: Vec<Value> = (0..25).map(|i| json!({ "project": format!("C:\\p{i}"), "name": format!("p{i}"), "total": 100 - i })).collect();
            Ok(json!({ "days": [], "projects": projects, "total": { "tokens": 7 } }))
        }

        async fn apps(&self) -> Value {
            json!([{ "id": "cursor", "name": "Cursor", "custom": false, "open": true }, { "id": "claude", "name": "Claude", "custom": false, "open": null }])
        }

        async fn ask(&self, ask: AskRequest) -> AskOutcome {
            self.asked.lock().unwrap().push(ask);
            self.answer.lock().unwrap().clone()
        }

        fn focus(&self) -> Value {
            self.focus.lock().unwrap().clone()
        }

        fn now_ms(&self) -> i64 {
            NOW
        }

        fn ao(&self) -> AoRead {
            self.ao.lock().unwrap().clone()
        }

        fn set_status(&self, chat: u64, text: Option<String>) -> StatusShown {
            self.statuses.lock().unwrap().push((chat, text));
            *self.status_answer.lock().unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::Fake;
    use super::*;

    fn run<T>(f: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    fn caller() -> Caller {
        Caller { label: "Codex".into(), chat: None }
    }

    fn output(out: ToolOutput) -> Value {
        match out {
            ToolOutput::Text(t) => serde_json::from_str(&t).unwrap(),
            ToolOutput::Error(e) => panic!("a tool error: {e}"),
        }
    }

    #[test]
    fn the_definitions_are_read_only_first_and_the_only_write_is_ask_user() {
        let defs = definitions();
        let listed: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert_eq!(
            listed,
            ["list_sessions", "get_session", "get_limits", "get_stats", "list_apps", "get_focus", "list_ao_sessions", "ask_user", "set_status"]
        );
        for d in &defs {
            let read_only = d["annotations"]["readOnlyHint"].as_bool().unwrap();
            assert_eq!(read_only, !["ask_user", "set_status"].contains(&d["name"].as_str().unwrap()), "{}", d["name"]);
            assert_eq!(d["annotations"]["openWorldHint"], false);
            assert_eq!(d["inputSchema"]["type"], "object");
            assert_eq!(d["inputSchema"]["additionalProperties"], false);
            let name = d["name"].as_str().unwrap();
            assert!(name.len() <= 128 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'));
        }
        assert_eq!(names(), listed);
        assert!(exists("ask_user") && !exists("delete_everything"));
    }

    #[test]
    fn the_caller_is_labelled_from_the_user_agent() {
        let label = |ua: Option<&str>| Caller::from_request(ua, None).label;
        assert_eq!(label(Some("claude-code/2.1.296 (sdk-ts, agent-sdk/0.3.232)")), "Claude Code");
        assert_eq!(label(Some("codex-mcp-client/0.162.1")), "Codex");
        assert_eq!(label(Some("my-tool/1.0")), "my-tool");
        assert_eq!(label(Some("<script>/1")), "script");
        assert_eq!(label(Some("")), "Agent");
        assert_eq!(label(None), "Agent");
        assert_eq!(label(Some(&"x".repeat(100))).len(), 24);
    }

    #[test]
    fn the_chat_header_names_a_chat_only_when_it_is_a_plain_number() {
        let chat = |v: Option<&str>| Caller::from_request(None, v).chat;
        assert_eq!(chat(Some("7")), Some(7));
        assert_eq!(chat(Some("18446744073709551615")), None, "20 digits");
        assert_eq!(chat(Some("1844674407370955161")), Some(1_844_674_407_370_955_161));
        for bad in ["", "0", "-1", "+1", "1.5", " 7", "7 ", "0x10", "seven", "\u{0667}"] {
            assert_eq!(chat(Some(bad)), None, "{bad:?}");
        }
        assert_eq!(chat(None), None);
    }

    #[test]
    fn list_sessions_groups_by_project_and_leaves_out_what_the_page_does_not_get() {
        let fake = Fake::new();
        let out = output(run(call(&fake, &caller(), "list_sessions", &json!({}))));
        assert_eq!(out["counts"]["waiting"], 1);
        let projects = out["projects"].as_array().unwrap();
        // The project without sessions is not listed.
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["name"], "mayak");
        let sessions = projects[0]["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0]["key"], "claude:1:2");
        // Null fields are dropped, and so is everything outside the allow-list ("acked", "turn", "outcome").
        assert!(sessions[1].get("title").is_none());
        assert!(sessions[0].get("detail").is_none());
        assert!(sessions[1].get("outcome").is_none());
        assert_eq!(sessions[1]["phase"]["wait"], "permission");
        // The context fill is in the list; the outcome is for `get_session`.
        assert_eq!(sessions[0]["context"]["used"], 1000);
        assert!(sessions[0].get("acked").is_none());
        assert!(sessions[0].get("turn").is_none());
    }

    #[test]
    fn list_sessions_can_keep_only_the_ones_that_need_attention() {
        let fake = Fake::new();
        let out = output(run(call(&fake, &caller(), "list_sessions", &json!({ "needs_attention": true }))));
        let sessions = out["projects"][0]["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["key"], "codex:abc");
        let bad = run(call(&fake, &caller(), "list_sessions", &json!({ "needs_attention": "yes" })));
        assert!(matches!(bad, ToolOutput::Error(_)));
    }

    #[test]
    fn get_session_finds_one_by_key_and_says_so_when_it_is_gone() {
        let fake = Fake::new();
        let out = output(run(call(&fake, &caller(), "get_session", &json!({ "key": "claude:1:2" }))));
        assert_eq!(out["title"], "Fix the pipe");
        assert_eq!(out["project"]["name"], "mayak");
        assert_eq!(out["turn"]["fileCount"], 0);
        assert_eq!(out["context"]["window"], 200000);
        // The outcome of a finished session comes with the one session, not with the list.
        let finished = output(run(call(&fake, &caller(), "get_session", &json!({ "key": "codex:abc" }))));
        assert_eq!(finished["outcome"]["summary"], "Done");
        let missing = run(call(&fake, &caller(), "get_session", &json!({ "key": "claude:9:9" })));
        assert!(matches!(&missing, ToolOutput::Error(e) if e.contains("claude:9:9")));
        for args in [json!({}), json!({ "key": "" }), json!({ "key": 5 }), json!({ "key": "k".repeat(201) })] {
            assert!(matches!(run(call(&fake, &caller(), "get_session", &args)), ToolOutput::Error(_)), "{args}");
        }
    }

    #[test]
    fn the_tools_read_a_real_snapshot_too() {
        // A snapshot of the real type, with no session in it.
        struct Real(Value);
        impl Backend for Real {
            fn snapshot(&self) -> Value {
                self.0.clone()
            }
            async fn stats(&self, _: u32) -> Result<Value, String> {
                Err("not here".into())
            }
            async fn apps(&self) -> Value {
                Value::Null
            }
            async fn ask(&self, _: AskRequest) -> AskOutcome {
                AskOutcome::Busy
            }
            fn focus(&self) -> Value {
                // No pomodoro, as the real `pomodoro::Focus` serializes it.
                serde_json::to_value(None::<crate::pomodoro::Focus>).unwrap()
            }
            fn now_ms(&self) -> i64 {
                5
            }
            fn ao(&self) -> AoRead {
                // A real "not running" read of the AO pill.
                AoRead { enabled: true, last: Some((5, crate::ao::model::payload(crate::ao::model::Status::NotRunning, true, &[]))) }
            }
            fn set_status(&self, _: u64, _: Option<String>) -> StatusShown {
                StatusShown::NoChat
            }
        }
        let real = Real(serde_json::to_value(crate::observe::model::Store::new().snapshot(5)).unwrap());
        let out = output(run(call(&real, &caller(), "list_sessions", &json!({}))));
        assert_eq!(out["now"], 5);
        assert_eq!(out["projects"], json!([]));
        assert_eq!(out["counts"], json!({ "waiting": 0, "error": 0, "finished": 0, "working": 0 }));
        let out = output(run(call(&real, &caller(), "get_limits", &json!({}))));
        assert_eq!(out["claude"], Value::Null);
        assert!(out.as_object().unwrap().contains_key("openrouter"));
        assert!(matches!(run(call(&real, &caller(), "get_session", &json!({ "key": "claude:1:1" }))), ToolOutput::Error(_)));
        assert_eq!(output(run(call(&real, &caller(), "get_focus", &json!({})))), json!({ "running": false }));
        let out = output(run(call(&real, &caller(), "list_ao_sessions", &json!({}))));
        assert_eq!((out["status"].clone(), out["sessions"].clone(), out["readAt"].clone()), (json!("notRunning"), json!([]), json!(5)));
    }

    #[test]
    fn list_apps_says_which_app_has_a_window_and_where_it_cannot_tell() {
        let out = output(run(call(&Fake::new(), &caller(), "list_apps", &json!({}))));
        assert_eq!(out[0]["open"], true);
        assert_eq!(out[1]["open"], Value::Null);
    }

    #[test]
    fn get_focus_says_whether_a_pomodoro_runs_and_what_is_left_of_its_phase() {
        let fake = Fake::new();
        assert_eq!(output(run(call(&fake, &caller(), "get_focus", &json!({})))), json!({ "running": false }));
        // As the page sends it (pomodoro.rs `Focus`), ending 61 s from now.
        *fake.focus.lock().unwrap() = json!({ "phase": "work", "cycle": 2, "cycles": 4, "endsAt": fake::NOW + 61_000, "muted": true });
        let out = output(run(call(&fake, &caller(), "get_focus", &json!({}))));
        assert_eq!(
            out,
            json!({ "running": true, "phase": "work", "cycle": 2, "cycles": 4, "endsAt": fake::NOW + 61_000,
                    "minutesLeft": 2, "alertsHeld": true, "phaseOver": false })
        );
        // A phase whose end passed while the island was hidden: said, not guessed past.
        *fake.focus.lock().unwrap() = json!({ "phase": "short", "cycle": 2, "cycles": 4, "endsAt": fake::NOW - 5, "muted": false });
        let out = output(run(call(&fake, &caller(), "get_focus", &json!({}))));
        assert_eq!((out["minutesLeft"].clone(), out["phaseOver"].clone(), out["phase"].clone()), (json!(0), json!(true), json!("short")));
    }

    fn ao_data() -> Value {
        json!({ "status": "ok", "cli": true, "sessions": [
            { "id": "mayak-1", "project": "mayak", "role": "orchestrator", "name": "Orchestrator", "status": "idle",
              "prs": 0, "prsOpen": 0, "workspace": "c:\\work" },
            { "id": "mayak-3", "project": "mayak", "role": "worker", "name": "MCP", "status": "working",
              "prs": 1, "prsOpen": 1, "ci": "passing", "workspace": "c:\\work\\mayak" },
            { "id": "other", "project": "x", "role": "worker", "name": null, "status": "working", "prs": 0, "prsOpen": 0, "workspace": null },
        ] })
    }

    #[test]
    fn list_ao_sessions_hands_over_the_last_read_and_only_while_the_pill_is_on() {
        let fake = Fake::new();
        let out = output(run(call(&fake, &caller(), "list_ao_sessions", &json!({}))));
        assert_eq!(out["enabled"], false);
        assert!(out.get("sessions").is_none());
        // On, not read yet (the island has not been on screen).
        *fake.ao.lock().unwrap() = AoRead { enabled: true, last: None };
        let out = output(run(call(&fake, &caller(), "list_ao_sessions", &json!({}))));
        assert_eq!((out["enabled"].clone(), out["readAt"].clone()), (json!(true), Value::Null));
        *fake.ao.lock().unwrap() = AoRead { enabled: true, last: Some((123, ao_data())) };
        let out = output(run(call(&fake, &caller(), "list_ao_sessions", &json!({}))));
        assert_eq!(out["readAt"], 123);
        assert_eq!(out["status"], "ok");
        assert_eq!(out["sessions"], ao_data()["sessions"]);
        // Switched off after a read: the old read is not handed out.
        *fake.ao.lock().unwrap() = AoRead { enabled: false, last: Some((123, ao_data())) };
        let out = output(run(call(&fake, &caller(), "list_ao_sessions", &json!({}))));
        assert!(out.get("sessions").is_none());
    }

    #[test]
    fn a_project_inside_an_ao_session_is_tagged_with_the_deepest_one() {
        let fake = Fake::new();
        let tags = |fake: &Fake| {
            let out = output(run(call(fake, &caller(), "list_sessions", &json!({}))));
            out["projects"].as_array().unwrap().iter().map(|p| p.get("ao").cloned()).collect::<Vec<_>>()
        };
        assert_eq!(tags(&fake), [None], "no tag while the pill is off");
        *fake.ao.lock().unwrap() = AoRead { enabled: true, last: Some((1, ao_data())) };
        // c:\work\mayak is in both c:\work and c:\work\mayak: the deeper folder wins.
        assert_eq!(
            tags(&fake),
            [Some(json!({ "id": "mayak-3", "name": "MCP", "role": "worker", "status": "working" }))]
        );
        assert!(is_inside("c:\\work\\mayak\\sub", "c:\\work\\mayak"));
        assert!(is_inside("/home/me/app/sub", "/home/me/app"));
        assert!(!is_inside("c:\\work\\mayak-2", "c:\\work\\mayak"), "a sibling with a longer name is not inside");
        assert!(!is_inside("", "c:\\work"));
        assert!(!is_inside("c:\\work", ""));
    }

    #[test]
    fn without_a_folder_match_a_project_is_tagged_by_the_ids_its_agent_named() {
        let mut fake = Fake::new();
        // AO knows no folder; the Codex row's hooks named AO session "other" (observe::model AoRef).
        let mut data = ao_data();
        for s in data["sessions"].as_array_mut().unwrap() {
            s["workspace"] = Value::Null;
        }
        *fake.ao.lock().unwrap() = AoRead { enabled: true, last: Some((1, data)) };
        let tag = |fake: &Fake| output(run(call(fake, &caller(), "list_sessions", &json!({}))))["projects"][0].get("ao").cloned();
        assert_eq!(tag(&fake), None, "no row names an AO session yet");
        fake.snapshot["projects"][0]["sessions"][1]["ao"] = json!({ "session": "other", "project": null });
        assert_eq!(tag(&fake), Some(json!({ "id": "other", "name": null, "role": "worker", "status": "working" })));
        // A project id that AO lists otherwise is another session.
        fake.snapshot["projects"][0]["sessions"][1]["ao"] = json!({ "session": "other", "project": "elsewhere" });
        assert_eq!(tag(&fake), None);
        // Ids AO does not list name nothing.
        fake.snapshot["projects"][0]["sessions"][1]["ao"] = json!({ "session": "gone", "project": null });
        assert_eq!(tag(&fake), None);
    }

    fn chat_caller(chat: u64) -> Caller {
        Caller { label: "Claude Code".into(), chat: Some(chat) }
    }

    #[test]
    fn set_status_shows_a_line_only_in_the_chat_the_agent_runs_in() {
        let fake = Fake::new();
        let out = output(run(call(&fake, &caller(), "set_status", &json!({ "text": "Running the tests" }))));
        assert_eq!((out["shown"].clone(), out["reason"].clone()), (json!(false), json!("not_in_chat")));
        assert!(fake.statuses.lock().unwrap().is_empty(), "nothing reached a chat");

        let out = output(run(call(&fake, &chat_caller(7), "set_status", &json!({ "text": "  Running\nthe   tests\t" }))));
        assert_eq!(out, json!({ "shown": true }));
        // An empty text clears the line.
        output(run(call(&fake, &chat_caller(7), "set_status", &json!({ "text": "   " }))));
        assert_eq!(*fake.statuses.lock().unwrap(), [(7, Some("Running the tests".to_string())), (7, None)]);

        for (answer, reason) in [(StatusShown::NoTurn, "no_turn"), (StatusShown::NoChat, "no_chat")] {
            *fake.status_answer.lock().unwrap() = answer;
            let out = output(run(call(&fake, &chat_caller(7), "set_status", &json!({ "text": "x" }))));
            assert_eq!((out["shown"].clone(), out["reason"].clone()), (json!(false), json!(reason)));
        }
    }

    #[test]
    fn set_status_refuses_what_is_no_line() {
        let fake = Fake::new();
        for args in [json!({}), json!({ "text": 5 }), json!({ "text": "x".repeat(STATUS_MAX_CHARS + 1) })] {
            assert!(matches!(run(call(&fake, &chat_caller(7), "set_status", &args)), ToolOutput::Error(_)), "{args}");
        }
        // Exactly the most is fine, counted in characters.
        let most = "é".repeat(STATUS_MAX_CHARS);
        assert_eq!(output(run(call(&fake, &chat_caller(7), "set_status", &json!({ "text": most })))), json!({ "shown": true }));
        assert_eq!(fake.statuses.lock().unwrap().len(), 1);
    }

    #[test]
    fn get_limits_is_the_usage_of_the_snapshot() {
        let fake = Fake::new();
        let out = output(run(call(&fake, &caller(), "get_limits", &json!({}))));
        assert_eq!(out["claude"]["windows"][0]["percent"], 12.5);
    }

    #[test]
    fn get_stats_takes_7_or_30_days_and_nothing_else() {
        let fake = Fake::new();
        let out = output(run(call(&fake, &caller(), "get_stats", &json!({}))));
        assert_eq!(out["total"]["tokens"], 7);
        // The largest 20 of the 25 projects, and the count of the rest.
        assert_eq!(out["projects"].as_array().unwrap().len(), 20);
        assert_eq!(out["projects"][0]["name"], "p0");
        assert_eq!(out["projectsOmitted"], 5);
        run(call(&fake, &caller(), "get_stats", &json!({ "days": 30 })));
        assert_eq!(*fake.stats_days.lock().unwrap(), [7, 30]);
        for days in [json!(1), json!("7"), json!(7.5), json!(-7)] {
            let out = run(call(&fake, &caller(), "get_stats", &json!({ "days": days })));
            assert!(matches!(out, ToolOutput::Error(_)), "{days}");
        }
        assert_eq!(fake.stats_days.lock().unwrap().len(), 2);
    }

    #[test]
    fn arguments_that_are_not_an_object_are_refused() {
        let fake = Fake::new();
        for args in [json!(null), json!([]), json!("x"), json!(1)] {
            assert!(matches!(run(call(&fake, &caller(), "list_apps", &args)), ToolOutput::Error(_)), "{args}");
        }
    }

    fn ask_args() -> Map<String, Value> {
        json!({ "question": "Deploy now?", "options": ["Yes", { "label": "No", "description": "Wait for CI" }] })
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn ask_arguments_become_a_card_the_island_can_show() {
        let ask = parse_ask(&caller(), &ask_args()).unwrap();
        assert_eq!(ask.asker, "Codex");
        assert_eq!(ask.question, "Deploy now?");
        assert_eq!(ask.header, "");
        assert!(!ask.multi_select);
        assert_eq!(ask.wait, Duration::from_secs(ASK_DEFAULT_SECS));
        assert_eq!(ask.options[0], AskOption { label: "Yes".into(), description: String::new() });
        assert_eq!(ask.options[1].description, "Wait for CI");

        let mut args = ask_args();
        args.insert("header".into(), json!("Deploy"));
        args.insert("multi_select".into(), json!(true));
        args.insert("timeout_seconds".into(), json!(120));
        let ask = parse_ask(&caller(), &args).unwrap();
        assert_eq!((ask.header.as_str(), ask.multi_select, ask.wait), ("Deploy", true, Duration::from_secs(120)));
    }

    #[test]
    fn ask_arguments_the_card_cannot_show_are_refused_with_a_reason() {
        let bad = |mutate: &dyn Fn(&mut Map<String, Value>)| {
            let mut args = ask_args();
            mutate(&mut args);
            parse_ask(&caller(), &args).unwrap_err()
        };
        assert!(bad(&|a| { a.remove("question"); }).contains("question"));
        assert!(bad(&|a| { a.insert("question".into(), json!("  ")); }).contains("question"));
        assert!(bad(&|a| { a.insert("question".into(), json!("q".repeat(501))); }).contains("question"));
        assert!(bad(&|a| { a.insert("options".into(), json!(["only one"])); }).contains("2 to 4"));
        assert!(bad(&|a| { a.insert("options".into(), json!(["a", "b", "c", "d", "e"])); }).contains("2 to 4"));
        assert!(bad(&|a| { a.insert("options".into(), json!(["same", "same"])); }).contains("differ"));
        assert!(bad(&|a| { a.insert("options".into(), json!(["a", 5])); }).contains("option"));
        assert!(bad(&|a| { a.insert("options".into(), json!(["a", { "description": "no label" }])); }).contains("label"));
        assert!(bad(&|a| { a.insert("options".into(), json!(["a", { "label": "b", "description": 3 }])); }).contains("description"));
        assert!(bad(&|a| { a.insert("header".into(), json!("h".repeat(41))); }).contains("header"));
        assert!(bad(&|a| { a.insert("multi_select".into(), json!("yes")); }).contains("multi_select"));
        assert!(bad(&|a| { a.insert("timeout_seconds".into(), json!(4)); }).contains("timeout_seconds"));
        assert!(bad(&|a| { a.insert("timeout_seconds".into(), json!(241)); }).contains("timeout_seconds"));
        assert!(bad(&|a| { a.insert("timeout_seconds".into(), json!("30")); }).contains("timeout_seconds"));
    }

    #[test]
    fn a_refused_ask_never_reaches_the_island() {
        let fake = Fake::new();
        let out = run(call(&fake, &caller(), "ask_user", &json!({ "question": "q", "options": ["only"] })));
        assert!(matches!(out, ToolOutput::Error(_)));
        assert!(fake.asked.lock().unwrap().is_empty());
    }

    #[test]
    fn what_the_user_clicked_comes_back_as_the_answer() {
        let fake = Fake::new();
        let args = json!({ "question": "Deploy now?", "options": ["Yes", "No"] });

        *fake.answer.lock().unwrap() = AskOutcome::Answered(json!({ "Deploy now?": "Yes" }));
        assert_eq!(output(run(call(&fake, &caller(), "ask_user", &args))), json!({ "answered": true, "answer": "Yes" }));

        *fake.answer.lock().unwrap() = AskOutcome::Answered(json!({ "Deploy now?": ["Yes", "No"] }));
        assert_eq!(output(run(call(&fake, &caller(), "ask_user", &args))), json!({ "answered": true, "answers": ["Yes", "No"] }));

        *fake.answer.lock().unwrap() = AskOutcome::Answered(json!({ "Deploy now?": { "other": "tomorrow" } }));
        assert_eq!(output(run(call(&fake, &caller(), "ask_user", &args))), json!({ "answered": true, "other": "tomorrow" }));

        // A card that came back with another question's text is no answer to this one.
        *fake.answer.lock().unwrap() = AskOutcome::Answered(json!({ "Something else?": "Yes" }));
        let out = output(run(call(&fake, &caller(), "ask_user", &args)));
        assert_eq!((out["answered"].clone(), out["reason"].clone()), (json!(false), json!("no_answer")));
        assert_eq!(fake.asked.lock().unwrap().len(), 4);
    }

    #[test]
    fn every_way_of_not_getting_an_answer_says_why() {
        let args = json!({ "question": "Deploy now?", "options": ["Yes", "No"], "timeout_seconds": 30 });
        for (outcome, reason) in [
            (AskOutcome::Released, "dismissed"),
            (AskOutcome::NotShown, "not_shown"),
            (AskOutcome::TimedOut, "timeout"),
            (AskOutcome::Busy, "busy"),
        ] {
            let fake = Fake::new();
            *fake.answer.lock().unwrap() = outcome;
            let out = output(run(call(&fake, &caller(), "ask_user", &args)));
            assert_eq!(out["answered"], false);
            assert_eq!(out["reason"], reason);
            assert!(out["message"].as_str().unwrap().len() > 20);
            if reason == "timeout" {
                assert!(out["message"].as_str().unwrap().contains("30 seconds"));
            }
        }
    }
}
