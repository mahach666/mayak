// The tools Mayak offers over MCP, their schemas, and what each one does with the app.
//
// Read-only tools only hand back what the island itself already shows (the sessions snapshot the
// page gets, the limits in it, the Stats tab, the Apps tab). The one tool with a side effect,
// `ask_user`, puts a question card on the island and waits for the user's click: nothing is
// answered, approved or sent on anyone's behalf.

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
}

/// Who is calling, as far as the HTTP request tells: a label for the card, never a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub label: String,
}

impl Caller {
    /// From the `User-Agent` header. Only the two clients whose headers were read on a real
    /// connection get their own name; any other product token is shown as sent, cleaned and cut.
    pub fn from_user_agent(user_agent: Option<&str>) -> Caller {
        let product = user_agent.and_then(|ua| ua.split(['/', ' ']).next()).unwrap_or_default();
        let label = match product {
            "claude-code" => "Claude Code".to_string(),
            "codex-mcp-client" => "Codex".to_string(),
            other => {
                let clean: String = other.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.').take(24).collect();
                if clean.is_empty() { "Agent".to_string() } else { clean }
            }
        };
        Caller { label }
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
            "The IDEs and agent apps on the island's Apps tab: id, name, whether it is the user's own app.",
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
    ]
}

/// The tool names a headless Mayak chat may call without a prompt: the read-only ones and
/// `ask_user`, whose gate is the user's click.
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
        "ask_user" => ask_user(backend, caller, args).await,
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
        projects.push(json!({ "name": project["name"], "root": project["root"], "sessions": sessions }));
    }
    text(&json!({ "now": snapshot["now"], "counts": snapshot["counts"], "projects": projects }))
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

    /// A backend with a fixed snapshot and a scripted answer to `ask`.
    pub struct Fake {
        pub snapshot: Value,
        pub answer: Mutex<AskOutcome>,
        pub asked: Mutex<Vec<AskRequest>>,
        pub stats_days: Mutex<Vec<u32>>,
    }

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
            json!([{ "id": "cursor", "name": "Cursor", "custom": false }])
        }

        async fn ask(&self, ask: AskRequest) -> AskOutcome {
            self.asked.lock().unwrap().push(ask);
            self.answer.lock().unwrap().clone()
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
        Caller { label: "Codex".into() }
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
        assert_eq!(listed, ["list_sessions", "get_session", "get_limits", "get_stats", "list_apps", "ask_user"]);
        for d in &defs {
            let read_only = d["annotations"]["readOnlyHint"].as_bool().unwrap();
            assert_eq!(read_only, d["name"] != "ask_user", "{}", d["name"]);
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
        assert_eq!(Caller::from_user_agent(Some("claude-code/2.1.296 (sdk-ts, agent-sdk/0.3.232)")).label, "Claude Code");
        assert_eq!(Caller::from_user_agent(Some("codex-mcp-client/0.162.1")).label, "Codex");
        assert_eq!(Caller::from_user_agent(Some("my-tool/1.0")).label, "my-tool");
        assert_eq!(Caller::from_user_agent(Some("<script>/1")).label, "script");
        assert_eq!(Caller::from_user_agent(Some("")).label, "Agent");
        assert_eq!(Caller::from_user_agent(None).label, "Agent");
        assert_eq!(Caller::from_user_agent(Some(&"x".repeat(100))).label.len(), 24);
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
