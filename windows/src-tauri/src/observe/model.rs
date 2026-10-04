// The session model (spec §5.4, §6, §7, §10). Pure: no I/O, no clock, no threads.
// Every state decision of the observer is made by `Store::apply`; source threads send facts as
// `Signal`s and execute the `Effect`s it returns. Time always comes in as `now_ms`.

use std::collections::{HashMap, VecDeque};

use serde::Serialize;
use serde_json::Value;

pub use super::claude::ClaudeSessionFile;
pub use super::codex::HookEvent;
#[allow(unused_imports)] // spec §5.4 names them model::claude_key / model::codex_key; callers use observe::keys
pub use super::keys::{claude_key, codex_key};
pub use super::keys::{HostKind, ProcId, SessionKey};
pub use super::paths::ProjectRef;

use super::claude_transcript::Fact;
use super::paths;

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
