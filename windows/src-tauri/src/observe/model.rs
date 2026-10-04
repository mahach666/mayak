// The session model (spec §5.4, §6, §7, §10). Pure: no I/O, no clock, no threads.
// Every state decision of the observer is made by `Store::apply`; source threads send facts as
// `Signal`s and execute the `Effect`s it returns. Time always comes in as `now_ms`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, VecDeque};

use serde::Serialize;
use serde_json::Value;

pub use super::claude::ClaudeSessionFile;
pub use super::codex::HookEvent;
#[allow(unused_imports)] // spec §5.4 names them model::claude_key / model::codex_key; callers use observe::keys
pub use super::keys::{claude_key, codex_key};
pub use super::keys::{HostKind, ProcId, SessionKey};
pub use super::paths::ProjectRef;

use super::claude_transcript::{self as transcript, Fact, TitleKind, TurnOutcome};
use super::{paths, text};

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

/// Where a session comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")] // "claude" | "codex"
pub enum Source {
    Claude,
    Codex,
}

/// Health of one source (spec §10 `Health`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")] // "ok" | "missing" | {"error":s} | {"degraded":{"unreadable":n}}
pub enum Health {
    Ok,
    Missing,
    Error(String),
    Degraded { unreadable: u32 },
}

/// Facts sent by the source threads (spec §5.4). One signal = one `Store::apply`.
#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    /// claude-io sends it before the `ClaudeFile` of the same session. `rebind_at = Some(o)` when
    /// the transcript is (re)bound: o = 0 for a new file, o = EOF for an existing file found by search.
    ClaudeTranscript { key: SessionKey, rebind_at: Option<u64>, records: Vec<(u64, Value)>, eof: u64 },
    ClaudeFile {
        key: SessionKey,
        proc: ProcId,
        host: HostKind,
        project: ProjectRef,
        file: ClaudeSessionFile,
        transcript_eof: Option<u64>,
    },
    /// Session file deleted, or procStart changed.
    ClaudeGone { key: SessionKey },
    /// codex-io sends it before the `CodexHook` of the same session.
    CodexRollout { key: SessionKey, path: String, records: Vec<(u64, Value)>, eof: u64 },
    CodexHook {
        key: SessionKey,
        event: HookEvent,
        proc: Option<ProcId>,
        host: HostKind,
        project: ProjectRef,
        rollout_eof: Option<u64>,
        /// session_index.jsonl title, filled on the first hook of a key in this run (D12).
        title: Option<String>,
    },
    CodexHooksInstalled(bool),
    ProcExit { pid: u32, created_ft: u64 },
    Ack(SessionKey),
    Visible(bool),
    AlertPrefs { waiting: bool, finished: bool, error: bool },
    SourceHealth { source: Source, health: Health },
}

/// Work for the source threads, returned by `Store::apply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// claude-io: find or wait for `<session_id>.jsonl`.
    TrackTranscript { key: SessionKey, session_id: String },
    UntrackTranscript { key: SessionKey },
    /// Emitted for every removed Codex session (D13).
    UntrackRollout { key: SessionKey },
    /// codex-io: 1 Hz rollout ticker on/off; emitted only when the wanted state changes.
    CodexTicker(bool),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied {
    pub alerts: Vec<Alert>,
    pub effects: Vec<Effect>,
}

// ── internal state ──

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitKind {
    Permission,
    Input,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Working,
    Waiting { kind: WaitKind },
    Idle,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    None,
    Pending,
    Finished { summary: String, at_ms: i64 },
    Error { code: Option<String>, status: Option<u16>, text: String, at_ms: i64 },
    Interrupted { at_ms: i64 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptTitles {
    pub custom: Option<String>,
    pub agent: Option<String>,
    pub ai: Option<String>,
}

/// Which alert kinds are returned by `apply` (Settings `alertWaiting/alertFinished/alertError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertPrefs {
    pub waiting: bool,
    pub finished: bool,
    pub error: bool,
}

impl Default for AlertPrefs {
    fn default() -> Self {
        AlertPrefs { waiting: true, finished: true, error: true }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub key: SessionKey,
    pub agent: Source,
    pub host: HostKind,
    pub proc: Option<ProcId>,
    /// Claude sessionId; Codex session_id.
    pub logical_id: String,
    pub cwd: String,
    pub project: ProjectRef,
    pub title: Option<String>,
    pub phase: Phase,
    pub since_ms: i64,
    pub activity: Option<String>,
    pub detail: Option<String>,
    pub outcome: Outcome,
    pub acked_at: Option<i64>,
    // Claude
    pub file_status_ts: i64,
    pub turn_start_offset: u64,
    pub idle_eof: u64,
    pub transcript_bound: bool,
    pub pending_tools: Vec<(String, String)>,
    /// Last `waitingFor` of the session file (waiting → waiting rule).
    pub waiting_for: Option<String>,
    /// Session file `name` (title fallback, D20).
    pub file_name: Option<String>,
    pub titles: TranscriptTitles,
    /// Facts with offset >= turn_start_offset, at most the last MAX_FACTS (D14).
    pub facts: Vec<(u64, Fact)>,
    // Codex
    pub current_turn: Option<String>,
    pub turn_started_ts: i64,
    /// At most MAX_CLOSED_TURNS, oldest dropped.
    pub closed_turns: VecDeque<String>,
    /// (turn, tool, fnv1a64(canonical_json(tool_input))).
    pub wait_key: Option<(String, String, u64)>,
}

/// A project as derived from the sessions (spec §6 `Project`). Test-only (D68): production
/// code reads projects from the snapshot.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub key: String,
    pub root: String,
    pub sessions: Vec<SessionKey>,
}

// ── wire types (spec §10) ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WaitView {
    Permission,
    Input,
    Other,
}

/// label: Permission "permission", Input "input", Other → raw waitingFor; Unknown → raw status (D15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PhaseView {
    Working,
    Waiting { wait: WaitView, label: String },
    Idle,
    Unknown { label: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OutcomeKind {
    Finished,
    Error,
    Interrupted,
}

/// None/Pending → null; Error summary = text::error_summary(code, text); Interrupted summary "".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeView {
    pub kind: OutcomeKind,
    pub summary: String,
    pub at: i64,
}

/// Also the Alert kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AttentionKind {
    Permission,
    Input,
    Error,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BotState {
    Approval,
    Question,
    Error,
    Working,
    Finished,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub key: String,
    pub agent: Source,
    pub host: HostKind,
    pub title: Option<String>,
    pub phase: PhaseView,
    pub since: i64,
    pub activity: Option<String>,
    pub detail: Option<String>,
    pub outcome: Option<OutcomeView>,
    pub attention: Option<AttentionKind>,
    pub acked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub key: String,
    pub name: String,
    pub root: String,
    pub sessions: Vec<SessionView>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub waiting: u32,
    pub error: u32,
    pub finished: u32,
    pub working: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")] // "not_installed" | "installed_unseen" | {"receiving":{hosts,since}}
pub enum CodexHooksView {
    NotInstalled,
    InstalledUnseen,
    Receiving { hosts: Vec<HostKind>, since: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sources {
    pub claude: Health,
    pub codex: Health,
    pub codex_hooks: CodexHooksView,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsSnapshot {
    pub rev: u64,
    pub now: i64,
    pub projects: Vec<ProjectView>,
    pub attention: Vec<String>,
    pub counts: Counts,
    pub hold: bool,
    pub aggregate: BotState,
    pub sources: Sources,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub key: String,
    pub kind: AttentionKind,
    /// 0 permission, 1 input, 2 error, 3 finished; most important = smallest (D16).
    pub rank: u8,
    pub agent: Source,
    pub host: HostKind,
    /// Display name of the project (collision rule applied).
    pub project: String,
    pub title: Option<String>,
    /// The alert body: tool label / question (permission, input), summary (finished),
    /// error summary (error).
    pub detail: Option<String>,
    pub text: String,
    /// Episode start: Waiting `since_ms`, Finished/Error `at_ms`.
    pub since: i64,
}

// ── pure helpers ──

/// Object keys sorted recursively, compact. Input of the Codex `wait_key` hash (spec §7).
pub fn canonical_json(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                write_canonical(&map[k.as_str()], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// FNV-1a, 64 bit.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Display name per project key (spec §6): basename of the root; when the basenames of different
/// keys collide (case-insensitive) → "<parent basename>\<basename>" (a drive root keeps its basename).
pub fn display_names(projects: &[(String, String)]) -> HashMap<String, String> {
    let mut keys_by_base: HashMap<String, Vec<&str>> = HashMap::new();
    for (key, root) in projects {
        let keys = keys_by_base.entry(paths::basename(root).to_lowercase()).or_default();
        if !keys.contains(&key.as_str()) {
            keys.push(key);
        }
    }
    projects
        .iter()
        .map(|(key, root)| {
            let base = paths::basename(root);
            let collides = keys_by_base.get(&base.to_lowercase()).is_some_and(|keys| keys.len() > 1);
            let name = match paths::parent_basename(root) {
                Some(parent) if collides => format!("{parent}\\{base}"),
                _ => base,
            };
            (key.clone(), name)
        })
        .collect()
}

// ── store ──

/// An alert raised by a transition; completed into an `Alert` at the end of `apply`, when the
/// project display names of the new state are known.
#[derive(Debug, Clone, PartialEq)]
struct Raised {
    key: SessionKey,
    kind: AttentionKind,
    body: Option<String>,
    since: i64,
}

/// Every snapshot field except `rev` and `now`; a change after `apply` bumps `rev`.
#[derive(Debug, Clone, PartialEq)]
struct View {
    projects: Vec<ProjectView>,
    attention: Vec<String>,
    counts: Counts,
    hold: bool,
    aggregate: BotState,
    sources: Sources,
}

/// Sessions of one project, in display order.
struct Group<'a> {
    key: String,
    root: String,
    sessions: Vec<&'a Session>,
}

pub struct Store {
    sessions: BTreeMap<SessionKey, Session>,
    rev: u64,
    prefs: AlertPrefs,
    claude_health: Health,
    codex_health: Health,
    codex_hooks: CodexHooksView,
    /// View after the last `apply` (rev bookkeeping only).
    last_view: Option<View>,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    /// rev 0, prefs all true, both healths Ok, Codex hooks NotInstalled.
    pub fn new() -> Self {
        let mut store = Store {
            sessions: BTreeMap::new(),
            rev: 0,
            prefs: AlertPrefs::default(),
            claude_health: Health::Ok,
            codex_health: Health::Ok,
            codex_hooks: CodexHooksView::NotInstalled,
            last_view: None,
        };
        store.last_view = Some(store.view());
        store
    }

    pub fn apply(&mut self, s: Signal, now_ms: i64) -> Applied {
        let mut fx: Vec<Effect> = Vec::new();
        let mut raised: Vec<Raised> = Vec::new();
        match s {
            Signal::ClaudeTranscript { key, rebind_at, records, eof: _ } => {
                self.claude_transcript(&key, rebind_at, records, now_ms, &mut raised)
            }
            Signal::ClaudeFile { key, proc, host, project, file, transcript_eof } => {
                self.claude_file(key, proc, host, project, file, transcript_eof, now_ms, &mut fx, &mut raised)
            }
            Signal::ClaudeGone { key } => self.remove_session(&key, &mut fx),
            Signal::CodexRollout { .. } | Signal::CodexHook { .. } | Signal::CodexHooksInstalled(_) => {} // Codex transitions: T10
            Signal::ProcExit { pid, created_ft } => self.proc_exit(pid, created_ft, &mut fx),
            Signal::Ack(key) => self.ack(&key, now_ms),
            Signal::Visible(_) => {} // read only by the Codex ticker (T10)
            Signal::AlertPrefs { waiting, finished, error } => self.prefs = AlertPrefs { waiting, finished, error },
            Signal::SourceHealth { source: Source::Claude, health } => self.claude_health = health,
            Signal::SourceHealth { source: Source::Codex, health } => self.codex_health = health,
        }
        let alerts = self.complete_alerts(raised);
        let view = self.view();
        if self.last_view.as_ref() != Some(&view) {
            self.rev += 1;
            self.last_view = Some(view);
        }
        Applied { alerts, effects: fx }
    }

    pub fn snapshot(&self, now_ms: i64) -> SessionsSnapshot {
        let v = self.view();
        SessionsSnapshot {
            rev: self.rev,
            now: now_ms,
            projects: v.projects,
            attention: v.attention,
            counts: v.counts,
            hold: v.hold,
            aggregate: v.aggregate,
            sources: v.sources,
        }
    }

    /// Test-only accessors (D68): production code reads the store through `apply` and `snapshot`.
    #[cfg(test)]
    pub fn session(&self, key: &str) -> Option<&Session> {
        self.sessions.get(key)
    }

    /// Projects in snapshot order, sessions in snapshot order.
    #[cfg(test)]
    pub fn projects(&self) -> Vec<Project> {
        self.groups()
            .into_iter()
            .map(|g| Project { key: g.key, root: g.root, sessions: g.sessions.iter().map(|s| s.key.clone()).collect() })
            .collect()
    }

    #[cfg(test)]
    pub fn rev(&self) -> u64 {
        self.rev
    }

    /// Ack sets `acked_at` and never changes the phase (spec §6, D49).
    fn ack(&mut self, key: &str, now_ms: i64) {
        if let Some(s) = self.sessions.get_mut(key) {
            s.acked_at = Some(episode_start(s).map_or(now_ms, |start| now_ms.max(start)));
        }
    }

    fn remove_session(&mut self, key: &str, fx: &mut Vec<Effect>) {
        if let Some(s) = self.sessions.remove(key) {
            if s.agent == Source::Claude {
                fx.push(Effect::UntrackTranscript { key: s.key });
            }
        }
    }

    /// Removes every session of the exited process (spec §7).
    fn proc_exit(&mut self, pid: u32, created_ft: u64, fx: &mut Vec<Effect>) {
        let gone: Vec<SessionKey> = self
            .sessions
            .values()
            .filter(|s| s.proc.as_ref().is_some_and(|p| p.pid == pid && p.created_ft == created_ft))
            .map(|s| s.key.clone())
            .collect();
        for key in gone {
            self.remove_session(&key, fx);
        }
    }

    fn complete_alerts(&self, raised: Vec<Raised>) -> Vec<Alert> {
        if raised.is_empty() {
            return Vec::new();
        }
        let names = display_names(&self.project_pairs());
        raised
            .into_iter()
            .filter(|r| self.prefs.allows(r.kind))
            .filter_map(|r| {
                let s = self.sessions.get(&r.key)?;
                let project = names.get(&s.project.key).cloned().unwrap_or_default();
                Some(Alert {
                    text: text::alert_text(s.agent, &project, s.host, r.kind, r.body.as_deref()),
                    key: r.key,
                    kind: r.kind,
                    rank: attention_rank(r.kind),
                    agent: s.agent,
                    host: s.host,
                    project,
                    title: s.title.clone(),
                    detail: r.body,
                    since: r.since,
                })
            })
            .collect()
    }

    /// (project key, root) per project; the root of the first session in key order wins.
    fn project_pairs(&self) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = Vec::new();
        for s in self.sessions.values() {
            if !pairs.iter().any(|(k, _)| *k == s.project.key) {
                pairs.push((s.project.key.clone(), s.project.root.clone()));
            }
        }
        pairs
    }

    fn groups(&self) -> Vec<Group<'_>> {
        let mut by_key: BTreeMap<&str, Group<'_>> = BTreeMap::new();
        for s in self.sessions.values() {
            by_key
                .entry(s.project.key.as_str())
                .or_insert_with(|| Group { key: s.project.key.clone(), root: s.project.root.clone(), sessions: Vec::new() })
                .sessions
                .push(s);
        }
        let mut groups: Vec<Group<'_>> = by_key.into_values().collect();
        for g in &mut groups {
            g.sessions.sort_by(|a, b| cmp_sessions(a, b));
        }
        // best rank asc, then most recent activity desc, then key asc (spec §6)
        groups.sort_by(|a, b| {
            let best = |g: &Group<'_>| g.sessions.iter().map(|s| rank_of(s)).min().unwrap_or(5);
            let latest = |g: &Group<'_>| g.sessions.iter().map(|s| last_activity(s)).max().unwrap_or(i64::MIN);
            best(a).cmp(&best(b)).then_with(|| latest(b).cmp(&latest(a))).then_with(|| a.key.cmp(&b.key))
        });
        groups
    }

    fn view(&self) -> View {
        let groups = self.groups();
        let names = display_names(&self.project_pairs());
        let projects = groups
            .iter()
            .map(|g| ProjectView {
                key: g.key.clone(),
                name: names.get(&g.key).cloned().unwrap_or_default(),
                root: g.root.clone(),
                sessions: g.sessions.iter().map(|s| session_view(s)).collect(),
            })
            .collect();
        let mut flagged: Vec<&Session> = self.sessions.values().filter(|s| attention_of(s).is_some()).collect();
        flagged.sort_by(|a, b| cmp_sessions(a, b));
        let attention = flagged.iter().map(|s| s.key.clone()).collect();
        let mut counts = Counts::default();
        let (mut approval, mut question) = (false, false);
        for s in self.sessions.values() {
            match attention_of(s) {
                Some(AttentionKind::Permission) => {
                    counts.waiting += 1;
                    approval = true;
                }
                Some(AttentionKind::Input) => {
                    counts.waiting += 1;
                    question = true;
                }
                Some(AttentionKind::Error) => counts.error += 1,
                Some(AttentionKind::Finished) => counts.finished += 1,
                None => {}
            }
            if s.phase == Phase::Working {
                counts.working += 1;
            }
        }
        let hold = counts.waiting > 0 && self.prefs.waiting;
        let aggregate = if approval {
            BotState::Approval
        } else if question {
            BotState::Question
        } else if counts.error > 0 {
            BotState::Error
        } else if counts.working > 0 {
            BotState::Working
        } else if counts.finished > 0 {
            BotState::Finished
        } else {
            BotState::Idle
        };
        View {
            projects,
            attention,
            counts,
            hold,
            aggregate,
            sources: Sources {
                claude: self.claude_health.clone(),
                codex: self.codex_health.clone(),
                codex_hooks: self.codex_hooks.clone(),
            },
        }
    }
}

impl AlertPrefs {
    fn allows(&self, kind: AttentionKind) -> bool {
        match kind {
            AttentionKind::Permission | AttentionKind::Input => self.waiting,
            AttentionKind::Finished => self.finished,
            AttentionKind::Error => self.error,
        }
    }
}

impl Session {
    /// A session with no phase information yet (`Unknown("")`, outcome None, nothing acked).
    #[allow(clippy::too_many_arguments)]
    fn blank(
        key: SessionKey,
        agent: Source,
        host: HostKind,
        proc: Option<ProcId>,
        project: ProjectRef,
        logical_id: String,
        cwd: String,
        since_ms: i64,
    ) -> Session {
        Session {
            key,
            agent,
            host,
            proc,
            logical_id,
            cwd,
            project,
            title: None,
            phase: Phase::Unknown(String::new()),
            since_ms,
            activity: None,
            detail: None,
            outcome: Outcome::None,
            acked_at: None,
            file_status_ts: since_ms,
            turn_start_offset: 0,
            idle_eof: 0,
            transcript_bound: false,
            pending_tools: Vec::new(),
            waiting_for: None,
            file_name: None,
            titles: TranscriptTitles::default(),
            facts: Vec::new(),
            current_turn: None,
            turn_started_ts: 0,
            closed_turns: VecDeque::new(),
            wait_key: None,
        }
    }
}

// ── episodes, attention, ranks (spec §6) ──

/// Waiting → since_ms; Idle with Finished/Error → at_ms; otherwise no episode.
fn episode_start(s: &Session) -> Option<i64> {
    match (&s.phase, &s.outcome) {
        (Phase::Waiting { .. }, _) => Some(s.since_ms),
        (Phase::Idle, Outcome::Finished { at_ms, .. }) | (Phase::Idle, Outcome::Error { at_ms, .. }) => Some(*at_ms),
        _ => None,
    }
}

fn unacked(s: &Session) -> bool {
    episode_start(s).is_some_and(|start| s.acked_at.is_none_or(|acked| acked < start))
}

fn attention_of(s: &Session) -> Option<AttentionKind> {
    if !unacked(s) {
        return None;
    }
    match (&s.phase, &s.outcome) {
        (Phase::Waiting { kind: WaitKind::Permission }, _) => Some(AttentionKind::Permission),
        (Phase::Waiting { .. }, _) => Some(AttentionKind::Input),
        (Phase::Idle, Outcome::Error { .. }) => Some(AttentionKind::Error),
        (Phase::Idle, Outcome::Finished { .. }) => Some(AttentionKind::Finished),
        _ => None,
    }
}

fn attention_rank(kind: AttentionKind) -> u8 {
    match kind {
        AttentionKind::Permission => 0,
        AttentionKind::Input => 1,
        AttentionKind::Error => 2,
        AttentionKind::Finished => 3,
    }
}

/// 0..3 unacked episodes, 4 Working, 5 everything else.
fn rank_of(s: &Session) -> u8 {
    match attention_of(s) {
        Some(kind) => attention_rank(kind),
        None if s.phase == Phase::Working => 4,
        None => 5,
    }
}

fn outcome_at(s: &Session) -> Option<i64> {
    match &s.outcome {
        Outcome::Finished { at_ms, .. } | Outcome::Error { at_ms, .. } | Outcome::Interrupted { at_ms } => Some(*at_ms),
        Outcome::None | Outcome::Pending => None,
    }
}

fn last_activity(s: &Session) -> i64 {
    s.since_ms.max(outcome_at(s).unwrap_or(i64::MIN))
}

/// Rank asc; within rank 0,1,4 `since` asc; 2,3 `at_ms` desc; 5 `since` desc; ties by key asc.
fn cmp_sessions(a: &Session, b: &Session) -> Ordering {
    let (ra, rb) = (rank_of(a), rank_of(b));
    ra.cmp(&rb)
        .then_with(|| match ra {
            0 | 1 | 4 => a.since_ms.cmp(&b.since_ms),
            2 | 3 => outcome_at(b).cmp(&outcome_at(a)),
            _ => b.since_ms.cmp(&a.since_ms),
        })
        .then_with(|| a.key.cmp(&b.key))
}

fn phase_view(p: &Phase) -> PhaseView {
    match p {
        Phase::Working => PhaseView::Working,
        Phase::Waiting { kind: WaitKind::Permission } => {
            PhaseView::Waiting { wait: WaitView::Permission, label: "permission".into() }
        }
        Phase::Waiting { kind: WaitKind::Input } => PhaseView::Waiting { wait: WaitView::Input, label: "input".into() },
        Phase::Waiting { kind: WaitKind::Other(raw) } => PhaseView::Waiting { wait: WaitView::Other, label: raw.clone() },
        Phase::Idle => PhaseView::Idle,
        Phase::Unknown(raw) => PhaseView::Unknown { label: raw.clone() },
    }
}

fn outcome_view(o: &Outcome) -> Option<OutcomeView> {
    match o {
        Outcome::None | Outcome::Pending => None,
        Outcome::Finished { summary, at_ms } => {
            Some(OutcomeView { kind: OutcomeKind::Finished, summary: summary.clone(), at: *at_ms })
        }
        Outcome::Error { code, text, at_ms, .. } => Some(OutcomeView {
            kind: OutcomeKind::Error,
            summary: text::error_summary(code.as_deref(), text),
            at: *at_ms,
        }),
        Outcome::Interrupted { at_ms } => {
            Some(OutcomeView { kind: OutcomeKind::Interrupted, summary: String::new(), at: *at_ms })
        }
    }
}

fn session_view(s: &Session) -> SessionView {
    SessionView {
        key: s.key.clone(),
        agent: s.agent,
        host: s.host,
        title: s.title.clone(),
        phase: phase_view(&s.phase),
        since: s.since_ms,
        activity: s.activity.clone(),
        detail: s.detail.clone(),
        outcome: outcome_view(&s.outcome),
        attention: attention_of(s),
        acked: !unacked(s),
    }
}

// ── Claude state machine (spec §7 "Claude") ──

/// Facts kept per Claude session (D14).
const MAX_FACTS: usize = 1024;

/// What the session file's `status` asks for (the columns of the spec §7 Claude table).
enum FileStatus {
    Busy,
    Waiting(WaitKind),
    Idle,
    Unknown(String),
}

fn file_status(f: &ClaudeSessionFile) -> FileStatus {
    match f.status.as_deref() {
        Some("busy") => FileStatus::Busy,
        Some("waiting") => FileStatus::Waiting(wait_kind(f.waiting_for.as_deref())),
        Some("idle") => FileStatus::Idle,
        Some(other) => FileStatus::Unknown(other.to_string()),
        None => FileStatus::Unknown(String::new()),
    }
}

/// "permission prompt" → Permission; "input needed" → Input; else Other(waitingFor).
fn wait_kind(waiting_for: Option<&str>) -> WaitKind {
    match waiting_for {
        Some("permission prompt") => WaitKind::Permission,
        Some("input needed") => WaitKind::Input,
        other => WaitKind::Other(other.unwrap_or_default().to_string()),
    }
}

/// customTitle > agentName > aiTitle > session file `name` (D20).
fn claude_title(s: &Session) -> Option<String> {
    [&s.titles.custom, &s.titles.agent, &s.titles.ai, &s.file_name]
        .into_iter()
        .flatten()
        .find(|t| !t.trim().is_empty())
        .cloned()
}

/// Phase change with `since = max(prev, ts)`; staying in the same phase keeps `since`.
fn set_claude_phase(s: &mut Session, phase: Phase, ts: i64) {
    if s.phase != phase {
        s.phase = phase;
        s.since_ms = s.since_ms.max(ts);
    }
}

/// Turn start: `turn_start_offset = idle_eof`; facts below it are dropped and pending tools and
/// activity are rebuilt from the rest (records of the new turn may precede the file change, D21).
fn start_claude_turn(s: &mut Session) {
    s.turn_start_offset = s.idle_eof;
    let start = s.turn_start_offset;
    s.facts.retain(|(offset, _)| *offset >= start);
    s.pending_tools = transcript::pair_tools(s.facts.iter().map(|(_, f)| f));
    s.activity = transcript::last_tool_label(s.facts.iter().map(|(_, f)| f));
    s.detail = s.pending_tools.last().map(|(_, label)| label.clone());
}

/// Entering Waiting, or a new waiting episode: clears the ack (D49) and raises the alert.
fn enter_waiting(s: &mut Session, kind: WaitKind, since: i64, raised: &mut Vec<Raised>) {
    let alert = match kind {
        WaitKind::Permission => AttentionKind::Permission,
        WaitKind::Input | WaitKind::Other(_) => AttentionKind::Input,
    };
    s.phase = Phase::Waiting { kind };
    s.since_ms = since;
    s.acked_at = None;
    raised.push(Raised { key: s.key.clone(), kind: alert, body: s.detail.clone(), since });
}

/// Outcome evaluation while Pending (spec §7 "Вычисление итога"). Returns the deciding offset when
/// the outcome was resolved (D51). Finished/Error clear the ack (D49) and raise an alert;
/// Interrupted does not. `at_ms` is the store clock (D50).
fn evaluate_claude(s: &mut Session, now_ms: i64, raised: &mut Vec<Raised>) -> Option<u64> {
    if s.outcome != Outcome::Pending {
        return None;
    }
    let (result, deciding) = transcript::terminal_outcome(&s.facts, s.turn_start_offset);
    match result {
        TurnOutcome::Pending => return None,
        TurnOutcome::Interrupted => s.outcome = Outcome::Interrupted { at_ms: now_ms },
        TurnOutcome::Error(e) => {
            let body = text::error_summary(e.code.as_deref(), &e.text);
            s.outcome = Outcome::Error { code: e.code, status: e.status, text: e.text, at_ms: now_ms };
            s.acked_at = None;
            raised.push(Raised { key: s.key.clone(), kind: AttentionKind::Error, body: Some(body), since: now_ms });
        }
        TurnOutcome::Finished { summary } => {
            s.outcome = Outcome::Finished { summary: summary.clone(), at_ms: now_ms };
            s.acked_at = None;
            raised.push(Raised { key: s.key.clone(), kind: AttentionKind::Finished, body: Some(summary), since: now_ms });
        }
    }
    deciding
}

impl Store {
    #[allow(clippy::too_many_arguments)]
    fn claude_file(
        &mut self,
        key: SessionKey,
        proc: ProcId,
        host: HostKind,
        project: ProjectRef,
        file: ClaudeSessionFile,
        transcript_eof: Option<u64>,
        now_ms: i64,
        fx: &mut Vec<Effect>,
        raised: &mut Vec<Raised>,
    ) {
        let status = file_status(&file);
        let ts = file.status_updated_at.unwrap_or(now_ms);
        let session_id = file.session_id.clone().unwrap_or_default();

        let Some(s) = self.sessions.get_mut(&key) else {
            // First appearance: phase from status, offsets from transcript_eof (0 if unbound), no alert.
            let base = transcript_eof.unwrap_or(0);
            let cwd = file.cwd.clone().unwrap_or_default();
            let mut s = Session::blank(key.clone(), Source::Claude, host, Some(proc), project, session_id.clone(), cwd, ts);
            s.phase = match status {
                FileStatus::Busy => Phase::Working,
                FileStatus::Waiting(kind) => Phase::Waiting { kind },
                FileStatus::Idle => Phase::Idle,
                FileStatus::Unknown(label) => Phase::Unknown(label),
            };
            s.idle_eof = base;
            s.turn_start_offset = base;
            s.waiting_for = file.waiting_for;
            s.file_name = file.name;
            s.title = claude_title(&s);
            if !session_id.is_empty() {
                fx.push(Effect::TrackTranscript { key: key.clone(), session_id });
            }
            self.sessions.insert(key, s);
            return;
        };

        s.proc = Some(proc);
        s.host = host;
        s.project = project;
        if let Some(cwd) = file.cwd.clone() {
            s.cwd = cwd;
        }
        s.file_name = file.name.clone();
        if !session_id.is_empty() && session_id != s.logical_id {
            // /clear, resume, fork: same process, new transcript. Offsets come with the next
            // ClaudeTranscript{rebind_at} (spec §7 "Смена sessionId").
            fx.push(Effect::UntrackTranscript { key: key.clone() });
            fx.push(Effect::TrackTranscript { key: key.clone(), session_id: session_id.clone() });
            s.logical_id = session_id;
            s.transcript_bound = false;
            s.facts.clear();
            s.pending_tools.clear();
            s.titles = TranscriptTitles::default();
            s.activity = None;
            s.detail = None;
        }

        let new_ts = file.status_updated_at.is_some_and(|t| t != s.file_status_ts);
        let new_waiting_for = file.waiting_for != s.waiting_for;
        let from_idle_or_unknown = matches!(s.phase, Phase::Idle | Phase::Unknown(_));

        match status {
            FileStatus::Busy => match s.phase {
                Phase::Working => {} // activity comes from the transcript
                Phase::Waiting { .. } => set_claude_phase(s, Phase::Working, ts),
                Phase::Idle | Phase::Unknown(_) => {
                    start_claude_turn(s);
                    s.outcome = Outcome::None;
                    set_claude_phase(s, Phase::Working, ts);
                }
            },
            FileStatus::Waiting(kind) => {
                if matches!(s.phase, Phase::Waiting { .. }) {
                    if new_ts {
                        let since = s.since_ms.max(ts);
                        enter_waiting(s, kind, since, raised);
                    } else if new_waiting_for {
                        // same statusUpdatedAt, new waitingFor: still a distinct episode (D19)
                        let since = (s.since_ms + 1).max(ts);
                        enter_waiting(s, kind, since, raised);
                    }
                    // else: same episode; detail follows the transcript
                } else {
                    if from_idle_or_unknown {
                        start_claude_turn(s);
                        s.outcome = Outcome::None;
                    }
                    let since = s.since_ms.max(ts);
                    enter_waiting(s, kind, since, raised);
                }
            }
            FileStatus::Idle => match s.phase {
                Phase::Working | Phase::Waiting { .. } => {
                    set_claude_phase(s, Phase::Idle, ts);
                    s.outcome = Outcome::Pending;
                    evaluate_claude(s, now_ms, raised);
                    if let Some(eof) = transcript_eof {
                        s.idle_eof = eof;
                    }
                }
                Phase::Idle | Phase::Unknown(_) => {
                    if new_ts {
                        // collapsed turn: busy → idle happened between two reads
                        start_claude_turn(s);
                        s.phase = Phase::Idle;
                        s.since_ms = s.since_ms.max(ts);
                        s.outcome = Outcome::Pending;
                        evaluate_claude(s, now_ms, raised);
                        if let Some(eof) = transcript_eof {
                            s.idle_eof = eof;
                        }
                    } else if matches!(s.phase, Phase::Unknown(_)) {
                        // status is the phase source; without a new statusUpdatedAt there is no
                        // collapsed turn to evaluate (spec §7 «иначе —»)
                        set_claude_phase(s, Phase::Idle, ts);
                        if let Some(eof) = transcript_eof {
                            s.idle_eof = eof;
                        }
                    }
                }
            },
            FileStatus::Unknown(label) => {
                let unknown = Phase::Unknown(label);
                if s.phase != unknown {
                    if s.outcome == Outcome::Pending {
                        s.outcome = Outcome::None; // leaving Idle drops Pending
                    }
                    set_claude_phase(s, unknown, ts);
                }
            }
        }

        if let Some(t) = file.status_updated_at {
            s.file_status_ts = t;
        }
        s.waiting_for = file.waiting_for;
        s.title = claude_title(s);
    }

    fn claude_transcript(
        &mut self,
        key: &str,
        rebind_at: Option<u64>,
        records: Vec<(u64, Value)>,
        now_ms: i64,
        raised: &mut Vec<Raised>,
    ) {
        let Some(s) = self.sessions.get_mut(key) else { return };
        match rebind_at {
            Some(o) => {
                s.facts.clear();
                s.pending_tools.clear();
                s.titles = TranscriptTitles::default();
                s.activity = None;
                s.idle_eof = o;
                s.turn_start_offset = o;
                s.transcript_bound = true;
            }
            // records of a transcript the model already let go (D52)
            None if !s.transcript_bound => return,
            None => {}
        }
        for (offset, record) in records {
            match transcript::fact_of(&record) {
                Some(Fact::Title { kind, text }) => match kind {
                    TitleKind::Custom => s.titles.custom = Some(text),
                    TitleKind::Agent => s.titles.agent = Some(text),
                    TitleKind::Ai => s.titles.ai = Some(text),
                },
                Some(fact) if offset >= s.turn_start_offset => {
                    transcript::apply_tools(&mut s.pending_tools, &fact);
                    if let Fact::Assistant { tool_uses, .. } = &fact {
                        if let Some(last) = tool_uses.last() {
                            s.activity = Some(last.label.clone());
                        }
                    }
                    s.facts.push((offset, fact));
                }
                _ => {}
            }
        }
        if s.facts.len() > MAX_FACTS {
            let excess = s.facts.len() - MAX_FACTS;
            s.facts.drain(..excess);
        }
        s.detail = s.pending_tools.last().map(|(_, label)| label.clone());
        s.title = claude_title(s);
        if let Some(deciding) = evaluate_claude(s, now_ms, raised) {
            if s.phase == Phase::Idle {
                // the next turn must not re-read the record that decided this one (D51)
                s.idle_eof = s.idle_eof.max(deciding + 1);
            }
        }
    }
}
