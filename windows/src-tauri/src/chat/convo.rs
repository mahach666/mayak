//! A conversation as the island shows it: the messages, and what is going on. Pure: it applies
//! adapter events (`Ev`) and the user's moves, and bumps `rev` on every change so the page can tell
//! what it has seen. No I/O, no threads, and no clock of its own: the manager hands the time in
//! (`clock`).

use serde::{Deserialize, Serialize};

use super::agent::*;
use super::saved::Saved;

/// Messages kept per conversation; older ones are dropped (the CLI keeps the full history).
const MAX_MESSAGES: usize = 400;
/// An assistant message stops growing past this many bytes of text.
const MAX_TEXT: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Idle,
    Working,
    /// A permission request waits for a click.
    Waiting,
    /// The last turn failed.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolStatus {
    Running,
    Ok,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionStatus {
    Pending,
    Allowed,
    Denied,
    /// The run ended before anybody clicked.
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Body {
    User { text: String },
    Assistant { text: String },
    Tool { tool_id: String, name: String, summary: String, status: ToolStatus, output: Option<String> },
    Permission {
        request_id: String,
        tool: String,
        summary: String,
        detail: Option<String>,
        tool_use_id: Option<String>,
        status: PermissionStatus,
    },
    /// Something the CLI or Mayak says about the run; `code` is a stable name the page translates
    /// ("cli" carries the CLI's own text, "apiKey" the name of the credential source).
    Notice { code: String, text: String },
    /// A failed turn: `code` "notLoggedIn", "rateLimit" (the plan's limit is used up), "failed" or
    /// "exited"; `text` is the CLI's own message.
    Error { code: String, text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Msg {
    pub id: u64,
    #[serde(flatten)]
    pub body: Body,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Convo {
    pub id: u64,
    pub agent: AgentKind,
    pub cwd: String,
    /// The model the user picked ("" is the CLI's default) and the mode.
    pub model: String,
    pub mode: String,
    pub title: String,
    /// The CLI's own id of the conversation, once it said it: what a later turn resumes.
    pub session: Option<String>,
    /// The model the CLI says it runs.
    pub used_model: Option<String>,
    pub state: State,
    /// A line shown while a turn goes on (a reconnect).
    pub status: Option<String>,
    /// What the agent itself says it is doing (the MCP server's `set_status`), for this turn only.
    pub agent_note: Option<String>,
    /// The last failure was "not signed in".
    pub login_hint: bool,
    /// When the turn under way, or the last one, started and ended (ms since the Unix epoch, the
    /// manager's clock): the page shows how long a turn runs and took.
    pub turn_started: Option<u64>,
    pub turn_ended: Option<u64>,
    /// The tokens the CLI's last request sent (the conversation's size as the model saw it), and the
    /// model's context window, when the CLI said them.
    pub context_tokens: Option<u64>,
    pub context_window: Option<u64>,
    /// The plan's usage windows, as the CLI last reported them; empty when it never did.
    pub limits: Vec<PlanWindow>,
    pub rev: u64,
    #[serde(rename = "messages")]
    pub msgs: Vec<Msg>,
    #[serde(skip)]
    next_msg: u64,
    #[serde(skip)]
    next_request: u64,
    /// Index of the assistant message that deltas are appended to.
    #[serde(skip)]
    open_text: Option<usize>,
    #[serde(skip)]
    key_noticed: bool,
}

/// What `apply` hands back to the manager.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// The id given to a new permission request (the page answers with it).
    pub permission: Option<String>,
}

impl Convo {
    pub fn new(id: u64, agent: AgentKind, cwd: String, model: String, mode: String, session: Option<String>) -> Self {
        Self {
            id,
            agent,
            cwd,
            model,
            mode,
            title: String::new(),
            session,
            used_model: None,
            state: State::Idle,
            status: None,
            agent_note: None,
            login_hint: false,
            turn_started: None,
            turn_ended: None,
            context_tokens: None,
            context_window: None,
            limits: Vec::new(),
            rev: 0,
            msgs: Vec::new(),
            next_msg: 1,
            next_request: 1,
            open_text: None,
            key_noticed: false,
        }
    }

    /// A conversation kept from an earlier run of the app (saved.rs), under a new id. No process is
    /// behind it: nothing was running, so a tool that was running failed and a request that waited
    /// was never answered. The next message resumes the CLI's session.
    pub fn restored(id: u64, saved: &Saved) -> Self {
        let mut c = Self::new(id, saved.agent, saved.cwd.clone(), saved.model.clone(), saved.mode.clone(), saved.session.clone());
        c.title.clone_from(&saved.title);
        c.used_model.clone_from(&saved.used_model);
        c.context_tokens = saved.context_tokens;
        c.context_window = saved.context_window;
        for (n, body) in saved.messages.iter().enumerate() {
            let body = match body.clone() {
                Body::Tool { tool_id, name, summary, status: ToolStatus::Running, output } => {
                    Body::Tool { tool_id, name, summary, status: ToolStatus::Failed, output }
                }
                // An old request keeps its answer but gets an id no new request can have.
                Body::Permission { tool, summary, detail, tool_use_id, status, .. } => Body::Permission {
                    request_id: format!("{id}.r{n}"),
                    tool,
                    summary,
                    detail,
                    tool_use_id,
                    status: if status == PermissionStatus::Pending { PermissionStatus::Cancelled } else { status },
                },
                other => other,
            };
            c.push(body);
        }
        c.touch();
        c
    }

    fn push(&mut self, body: Body) -> usize {
        let id = self.next_msg;
        self.next_msg += 1;
        self.msgs.push(Msg { id, body });
        if self.msgs.len() > MAX_MESSAGES {
            let cut = self.msgs.len() - MAX_MESSAGES;
            self.msgs.drain(..cut);
            self.open_text = self.open_text.and_then(|i| i.checked_sub(cut));
        }
        self.msgs.len() - 1
    }

    fn touch(&mut self) {
        self.rev += 1;
    }

    /// The user sends a message: the turn begins.
    pub fn user(&mut self, text: &str) {
        if self.title.is_empty() {
            self.title = clip(text.lines().next().unwrap_or_default().trim(), 60);
        }
        self.open_text = None;
        self.push(Body::User { text: text.to_string() });
        self.state = State::Working;
        self.status = None;
        self.agent_note = None;
        self.login_hint = false;
        self.turn_started = None;
        self.turn_ended = None;
        self.touch();
    }

    /// The manager's clock, after every change: a turn that is under way and has no start yet starts
    /// now, a turn that is over and has no end yet ended now.
    pub fn clock(&mut self, now_ms: u64) {
        let busy = matches!(self.state, State::Working | State::Waiting);
        if busy && self.turn_started.is_none() {
            self.turn_started = Some(now_ms);
            self.touch();
        } else if !busy && self.turn_started.is_some() && self.turn_ended.is_none() {
            self.turn_ended = Some(now_ms);
            self.touch();
        }
    }

    pub fn apply(&mut self, ev: &Ev) -> Applied {
        let mut applied = Applied::default();
        match ev {
            Ev::Session { id, model, api_key_source } => {
                self.session = Some(id.clone());
                if model.is_some() {
                    self.used_model.clone_from(model);
                }
                // Only Claude Code names where its credentials come from in a way that matters: a
                // key bills the key, not the subscription the user signed in with.
                if self.agent == AgentKind::ClaudeCode && !self.key_noticed {
                    if let Some(source) = api_key_source.as_deref().filter(|s| !s.is_empty() && *s != "none") {
                        self.key_noticed = true;
                        // The text is the source alone; the page words the notice.
                        self.push(Body::Notice { code: "apiKey".into(), text: source.to_string() });
                    }
                }
            }
            Ev::TextDelta(delta) => {
                self.status = None;
                self.append_text(delta);
            }
            Ev::Text(text) => {
                self.status = None;
                self.open_text = None;
                self.append_text(text);
                self.open_text = None;
            }
            Ev::ToolStart { id, name, summary } => {
                self.status = None;
                self.open_text = None;
                self.push(Body::Tool {
                    tool_id: id.clone(),
                    name: name.clone(),
                    summary: summary.clone(),
                    status: ToolStatus::Running,
                    output: None,
                });
            }
            Ev::ToolEnd { id, ok, output } => {
                if let Some(Body::Tool { status, output: out, .. }) =
                    self.msgs.iter_mut().rev().map(|m| &mut m.body).find(|b| matches!(b, Body::Tool { tool_id, .. } if tool_id == id))
                {
                    *status = if *ok { ToolStatus::Ok } else { ToolStatus::Failed };
                    // A tool that worked has its output in the log of the CLI; only a failure shows it.
                    *out = output.clone().filter(|_| !*ok);
                }
            }
            Ev::Permission(ask) => {
                let request_id = format!("{}.{}", self.id, self.next_request);
                self.next_request += 1;
                self.open_text = None;
                self.push(Body::Permission {
                    request_id: request_id.clone(),
                    tool: ask.tool.clone(),
                    summary: summarize(&ask.tool, &ask.input),
                    detail: ask.description.clone().or_else(|| ask.blocked_path.clone()),
                    tool_use_id: ask.tool_use_id.clone(),
                    status: PermissionStatus::Pending,
                });
                self.state = State::Waiting;
                applied.permission = Some(request_id);
            }
            Ev::Status(text) => self.status = Some(text.clone()),
            Ev::Notice(text) => {
                let same_as_last = matches!(self.msgs.last(), Some(Msg { body: Body::Notice { code, text: t }, .. }) if code == "cli" && t == text);
                if !same_as_last {
                    self.push(Body::Notice { code: "cli".into(), text: text.clone() });
                }
            }
            Ev::Usage { context_tokens, context_window } => {
                if context_tokens.is_some() {
                    self.context_tokens = *context_tokens;
                }
                if context_window.is_some() {
                    self.context_window = *context_window;
                }
            }
            Ev::Limits(windows) => self.limits.clone_from(windows),
            Ev::TurnEnd(outcome) => self.end_turn(outcome),
        }
        self.touch();
        applied
    }

    fn append_text(&mut self, delta: &str) {
        if let Some(i) = self.open_text {
            if let Some(Msg { body: Body::Assistant { text }, .. }) = self.msgs.get_mut(i) {
                if text.len() < MAX_TEXT {
                    text.push_str(delta);
                }
                return;
            }
        }
        let i = self.push(Body::Assistant { text: delta.to_string() });
        self.open_text = Some(i);
    }

    fn end_turn(&mut self, outcome: &Outcome) {
        self.status = None;
        self.agent_note = None;
        self.open_text = None;
        self.cancel_pending();
        let interrupted = matches!(outcome, Outcome::Aborted | Outcome::Failed { .. });
        if interrupted {
            self.fail_running_tools();
        }
        match outcome {
            Outcome::Done => self.state = State::Idle,
            Outcome::Aborted => {
                self.state = State::Idle;
                self.push(Body::Notice { code: "stopped".into(), text: "Stopped.".into() });
            }
            Outcome::Failed { text, login } => {
                self.state = State::Failed;
                self.login_hint = *login;
                let code = if *login {
                    "notLoggedIn"
                } else if super::agent::is_usage_limit(text) {
                    "rateLimit"
                } else {
                    "failed"
                };
                self.push(Body::Error { code: code.into(), text: text.clone() });
            }
        }
    }

    /// The process ended while a turn was open (it crashed, was killed, or never answered).
    pub fn process_ended(&mut self, text: String) {
        if !matches!(self.state, State::Working | State::Waiting) {
            return;
        }
        self.status = None;
        self.agent_note = None;
        self.open_text = None;
        self.cancel_pending();
        self.fail_running_tools();
        self.state = State::Failed;
        self.push(Body::Error { code: "exited".into(), text });
        self.touch();
    }

    /// The agent's own line about what it is doing (`None` clears it). Only while a turn runs: false
    /// and nothing changes otherwise, so a line never outlives the turn that set it.
    pub fn set_agent_note(&mut self, text: Option<String>) -> bool {
        if !matches!(self.state, State::Working | State::Waiting) {
            return false;
        }
        if self.agent_note != text {
            self.agent_note = text;
            self.touch();
        }
        true
    }

    /// The user stopped the turn and the process went (it does not say anything itself).
    pub fn stopped(&mut self) {
        if !matches!(self.state, State::Working | State::Waiting) {
            return;
        }
        self.end_turn(&Outcome::Aborted);
        self.touch();
    }

    fn fail_running_tools(&mut self) {
        for m in &mut self.msgs {
            if let Body::Tool { status, .. } = &mut m.body {
                if *status == ToolStatus::Running {
                    *status = ToolStatus::Failed;
                }
            }
        }
    }

    /// Every request nobody clicked is over.
    pub fn cancel_pending(&mut self) {
        for m in &mut self.msgs {
            if let Body::Permission { status, .. } = &mut m.body {
                if *status == PermissionStatus::Pending {
                    *status = PermissionStatus::Cancelled;
                }
            }
        }
    }

    /// The user clicked: the request is allowed or denied. False when it is not pending any more.
    pub fn resolve_permission(&mut self, request_id: &str, allowed: bool) -> bool {
        let mut done = false;
        for m in &mut self.msgs {
            if let Body::Permission { request_id: id, status, .. } = &mut m.body {
                if id == request_id && *status == PermissionStatus::Pending {
                    *status = if allowed { PermissionStatus::Allowed } else { PermissionStatus::Denied };
                    done = true;
                }
            }
        }
        if done {
            let pending = self.msgs.iter().any(|m| matches!(m.body, Body::Permission { status: PermissionStatus::Pending, .. }));
            if !pending && self.state == State::Waiting {
                self.state = State::Working;
            }
            self.touch();
        }
        done
    }

    #[cfg(test)]
    pub fn has_pending_permission(&self) -> bool {
        self.msgs.iter().any(|m| matches!(m.body, Body::Permission { status: PermissionStatus::Pending, .. }))
    }

    /// The model and mode for the next turn.
    pub fn set_choice(&mut self, model: String, mode: String) {
        if self.model != model || self.mode != mode {
            self.model = model;
            self.mode = mode;
            self.touch();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn convo() -> Convo {
        Convo::new(7, AgentKind::ClaudeCode, r"C:\work\app".into(), String::new(), "ask".into(), None)
    }

    fn texts(c: &Convo) -> Vec<String> {
        c.msgs
            .iter()
            .filter_map(|m| match &m.body {
                Body::Assistant { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn ask(tool: &str) -> PermissionAsk {
        PermissionAsk {
            request_id: "cli-1".into(),
            tool: tool.into(),
            input: json!({ "command": "ls" }),
            description: Some("List files".into()),
            tool_use_id: Some("toolu_1".into()),
            blocked_path: None,
        }
    }

    #[test]
    fn the_agents_own_line_lives_only_as_long_as_its_turn() {
        let mut c = convo();
        // No turn: nothing to say it about.
        assert!(!c.set_agent_note(Some("Reading".into())));
        assert_eq!(c.agent_note, None);
        c.user("go");
        let rev = c.rev;
        assert!(c.set_agent_note(Some("Running the tests".into())));
        assert_eq!(c.agent_note.as_deref(), Some("Running the tests"));
        assert!(c.rev > rev, "the page reads it again");
        // The same line again is no change.
        let rev = c.rev;
        assert!(c.set_agent_note(Some("Running the tests".into())));
        assert_eq!(c.rev, rev);
        // On the wire, for the page.
        assert_eq!(serde_json::to_value(&c).unwrap()["agentNote"], "Running the tests");
        assert!(c.set_agent_note(None));
        assert_eq!(c.agent_note, None);
        // The turn's end takes it away, whichever way the turn ends.
        c.set_agent_note(Some("x".into()));
        c.apply(&Ev::TurnEnd(Outcome::Done));
        assert_eq!(c.agent_note, None);
        c.user("again");
        c.set_agent_note(Some("y".into()));
        c.process_ended("gone".into());
        assert_eq!(c.agent_note, None);
        c.user("and again");
        c.set_agent_note(Some("z".into()));
        c.stopped();
        assert_eq!(c.agent_note, None);
    }

    #[test]
    fn deltas_grow_one_message_and_a_tool_closes_it() {
        let mut c = convo();
        c.user("hello\nsecond line");
        assert_eq!((c.state, c.title.as_str()), (State::Working, "hello"));
        c.apply(&Ev::TextDelta("Hel".into()));
        c.apply(&Ev::TextDelta("lo".into()));
        c.apply(&Ev::ToolStart { id: "t1".into(), name: "Bash".into(), summary: "ls".into() });
        c.apply(&Ev::TextDelta("after".into()));
        assert_eq!(texts(&c), ["Hello", "after"]);
        c.apply(&Ev::Text("block".into()));
        c.apply(&Ev::Text("block 2".into()));
        assert_eq!(texts(&c), ["Hello", "after", "block", "block 2"], "a whole block is its own message");
        c.apply(&Ev::TurnEnd(Outcome::Done));
        assert_eq!(c.state, State::Idle);
    }

    #[test]
    fn every_change_bumps_rev() {
        let mut c = convo();
        let mut last = c.rev;
        c.user("a");
        assert!(c.rev > last);
        last = c.rev;
        c.apply(&Ev::TextDelta("x".into()));
        assert!(c.rev > last);
        last = c.rev;
        c.apply(&Ev::Status("Reconnecting... 2/5".into()));
        assert!(c.rev > last);
        assert_eq!(c.status.as_deref(), Some("Reconnecting... 2/5"));
        c.apply(&Ev::TextDelta("y".into()));
        assert_eq!(c.status, None, "any progress clears the status line");
    }

    #[test]
    fn a_tool_ends_ok_or_failed_and_only_a_failure_keeps_its_output() {
        let mut c = convo();
        c.user("go");
        for id in ["a", "b"] {
            c.apply(&Ev::ToolStart { id: id.into(), name: "Bash".into(), summary: String::new() });
        }
        c.apply(&Ev::ToolEnd { id: "a".into(), ok: true, output: Some("fine".into()) });
        c.apply(&Ev::ToolEnd { id: "b".into(), ok: false, output: Some("denied".into()) });
        let tools: Vec<_> = c
            .msgs
            .iter()
            .filter_map(|m| if let Body::Tool { status, output, .. } = &m.body { Some((*status, output.clone())) } else { None })
            .collect();
        assert_eq!(tools, [(ToolStatus::Ok, None), (ToolStatus::Failed, Some("denied".into()))]);
        c.apply(&Ev::ToolEnd { id: "zzz".into(), ok: true, output: None });
    }

    #[test]
    fn a_permission_waits_for_a_click_then_the_turn_goes_on() {
        let mut c = convo();
        c.user("run it");
        let applied = c.apply(&Ev::Permission(ask("Bash")));
        let id = applied.permission.expect("a request id");
        assert_eq!(id, "7.1");
        assert_eq!(c.state, State::Waiting);
        assert!(c.has_pending_permission());
        assert!(c.resolve_permission(&id, true));
        assert_eq!(c.state, State::Working);
        assert!(!c.resolve_permission(&id, false), "a second click changes nothing");
        assert!(matches!(c.msgs.last().unwrap().body, Body::Permission { status: PermissionStatus::Allowed, .. }));
        assert!(!c.resolve_permission("7.99", true), "an unknown id changes nothing");
    }

    #[test]
    fn a_permission_summary_names_what_the_tool_touches() {
        let mut c = convo();
        c.apply(&Ev::Permission(ask("Bash")));
        let Body::Permission { summary, detail, tool_use_id, .. } = &c.msgs[0].body else { panic!() };
        assert_eq!((summary.as_str(), detail.as_deref(), tool_use_id.as_deref()), ("ls", Some("List files"), Some("toolu_1")));
    }

    #[test]
    fn a_turn_that_ends_cancels_what_nobody_answered() {
        let mut c = convo();
        c.user("x");
        c.apply(&Ev::ToolStart { id: "t".into(), name: "Bash".into(), summary: String::new() });
        c.apply(&Ev::Permission(ask("Bash")));
        c.apply(&Ev::TurnEnd(Outcome::Aborted));
        assert_eq!(c.state, State::Idle);
        let kinds: Vec<_> = c.msgs.iter().map(|m| &m.body).collect();
        assert!(matches!(kinds[1], Body::Tool { status: ToolStatus::Failed, .. }));
        assert!(matches!(kinds[2], Body::Permission { status: PermissionStatus::Cancelled, .. }));
        assert!(matches!(kinds[3], Body::Notice { code, .. } if code == "stopped"));
        assert!(!c.has_pending_permission());
    }

    #[test]
    fn a_failed_turn_shows_the_cli_message_and_remembers_the_login_flag() {
        let mut c = convo();
        c.user("x");
        c.apply(&Ev::TurnEnd(Outcome::Failed { text: "Not logged in · Please run /login".into(), login: true }));
        assert_eq!((c.state, c.login_hint), (State::Failed, true));
        assert!(matches!(&c.msgs.last().unwrap().body, Body::Error { code, text } if code == "notLoggedIn" && text.contains("/login")));
        c.user("again");
        assert_eq!((c.state, c.login_hint), (State::Working, false), "a new message starts over");
    }

    #[test]
    fn the_clock_marks_when_a_turn_started_and_ended() {
        let mut c = convo();
        c.clock(1_000);
        assert_eq!((c.turn_started, c.turn_ended), (None, None), "no turn yet");
        c.user("x");
        c.clock(2_000);
        c.clock(3_000);
        assert_eq!((c.turn_started, c.turn_ended), (Some(2_000), None), "the first time it sees the turn");
        c.apply(&Ev::Permission(ask("Bash")));
        c.clock(4_000);
        assert_eq!(c.turn_ended, None, "waiting for a click is still the turn");
        c.apply(&Ev::TurnEnd(Outcome::Done));
        c.clock(9_000);
        c.clock(12_000);
        assert_eq!((c.turn_started, c.turn_ended), (Some(2_000), Some(9_000)));
        c.user("again");
        assert_eq!((c.turn_started, c.turn_ended), (None, None), "a new turn starts over");
        c.clock(20_000);
        assert_eq!(c.turn_started, Some(20_000));
    }

    #[test]
    fn usage_and_plan_windows_keep_what_the_cli_said_last() {
        let mut c = convo();
        c.apply(&Ev::Usage { context_tokens: Some(26_155), context_window: None });
        c.apply(&Ev::Usage { context_tokens: None, context_window: Some(1_000_000) });
        assert_eq!((c.context_tokens, c.context_window), (Some(26_155), Some(1_000_000)), "either comes alone");
        c.apply(&Ev::Usage { context_tokens: Some(30_000), context_window: None });
        assert_eq!((c.context_tokens, c.context_window), (Some(30_000), Some(1_000_000)));
        let w = |kind: &str, used: f64| PlanWindow { kind: kind.into(), used, resets_at: Some(1) };
        c.apply(&Ev::Limits(vec![w("five_hour", 0.05), w("seven_day", 0.55)]));
        c.apply(&Ev::Limits(vec![w("five_hour", 0.06), w("seven_day", 0.55)]));
        assert_eq!(c.limits, vec![w("five_hour", 0.06), w("seven_day", 0.55)]);
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["contextTokens"], 30_000);
        assert_eq!(json["contextWindow"], 1_000_000);
        assert_eq!(json["limits"][0]["kind"], "five_hour");
        assert_eq!(json["limits"][0]["resetsAt"], 1);
        assert!(json["turnStarted"].is_null() && json["turnEnded"].is_null());
    }

    #[test]
    fn a_usage_limit_is_its_own_error_code() {
        let mut c = convo();
        c.user("x");
        c.apply(&Ev::TurnEnd(Outcome::Failed { text: "You’ve hit your usage limit. Try again at 1:23 AM.".into(), login: false }));
        assert_eq!(c.state, State::Failed);
        assert!(matches!(&c.msgs.last().unwrap().body, Body::Error { code, text } if code == "rateLimit" && text.contains("1:23 AM")));
        c.user("again");
        c.apply(&Ev::TurnEnd(Outcome::Failed { text: "The model is not supported".into(), login: false }));
        assert!(matches!(&c.msgs.last().unwrap().body, Body::Error { code, .. } if code == "failed"));
    }

    #[test]
    fn a_process_that_vanished_mid_turn_is_an_error_and_a_finished_one_is_not() {
        let mut c = convo();
        c.user("x");
        c.apply(&Ev::Permission(ask("Bash")));
        c.process_ended("claude exited with code 1".into());
        assert_eq!(c.state, State::Failed);
        assert!(matches!(&c.msgs.last().unwrap().body, Body::Error { code, .. } if code == "exited"));
        assert!(!c.has_pending_permission());
        let mut idle = convo();
        idle.process_ended("exit 0".into());
        assert_eq!((idle.state, idle.msgs.len(), idle.rev), (State::Idle, 0, 0), "nothing to report at rest");
    }

    #[test]
    fn stopping_a_turn_that_went_silent_ends_it_with_a_notice() {
        let mut c = convo();
        c.user("x");
        c.stopped();
        assert_eq!(c.state, State::Idle);
        assert!(matches!(&c.msgs.last().unwrap().body, Body::Notice { code, .. } if code == "stopped"));
        let rev = c.rev;
        c.stopped();
        assert_eq!(c.rev, rev, "stopping an idle conversation changes nothing");
    }

    #[test]
    fn the_session_id_and_the_model_come_from_the_session_event_and_a_key_is_noticed_once() {
        let mut c = convo();
        let ev = Ev::Session { id: "s-1".into(), model: Some("claude-haiku-5-5".into()), api_key_source: Some("ANTHROPIC_API_KEY".into()) };
        c.apply(&ev);
        c.apply(&ev);
        assert_eq!((c.session.as_deref(), c.used_model.as_deref()), (Some("s-1"), Some("claude-haiku-5-5")));
        let notices = c.msgs.iter().filter(|m| matches!(&m.body, Body::Notice { code, .. } if code == "apiKey")).count();
        assert_eq!(notices, 1);
        let mut none = convo();
        none.apply(&Ev::Session { id: "s".into(), model: None, api_key_source: Some("none".into()) });
        assert!(none.msgs.is_empty());
        let mut cursor = Convo::new(1, AgentKind::CursorAgent, "p".into(), String::new(), "proposeOnly".into(), None);
        cursor.apply(&Ev::Session { id: "s".into(), model: None, api_key_source: Some("login".into()) });
        assert!(cursor.msgs.is_empty(), "only Claude Code's key matters");
    }

    #[test]
    fn cli_notices_are_not_repeated_back_to_back() {
        let mut c = convo();
        for _ in 0..3 {
            c.apply(&Ev::Notice("Rate limited".into()));
        }
        c.apply(&Ev::Notice("Other".into()));
        assert_eq!(c.msgs.len(), 2);
    }

    #[test]
    fn old_messages_are_dropped_past_the_cap_and_the_open_text_follows() {
        let mut c = convo();
        for i in 0..MAX_MESSAGES + 10 {
            c.apply(&Ev::ToolStart { id: i.to_string(), name: "t".into(), summary: String::new() });
        }
        assert_eq!(c.msgs.len(), MAX_MESSAGES);
        c.apply(&Ev::TextDelta("a".into()));
        c.apply(&Ev::TextDelta("b".into()));
        assert_eq!(texts(&c), ["ab"]);
        let first = c.msgs.first().unwrap().id;
        assert!(first > 1, "the oldest ones went");
    }

    #[test]
    fn the_wire_form_is_flat_camel_case_with_the_kind_tag() {
        let mut c = convo();
        c.user("hi");
        c.apply(&Ev::ToolStart { id: "t1".into(), name: "Bash".into(), summary: "ls".into() });
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["agent"], "claudeCode");
        assert_eq!(v["state"], "working");
        assert_eq!(v["messages"][0], json!({ "id": 1, "kind": "user", "text": "hi" }));
        assert_eq!(
            v["messages"][1],
            json!({ "id": 2, "kind": "tool", "toolId": "t1", "name": "Bash", "summary": "ls", "status": "running", "output": null })
        );
        assert!(v.get("nextMsg").is_none() && v.get("openText").is_none());
    }
}
