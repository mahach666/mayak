// The session model (spec §5.4, §6, §7, §10). Pure: no I/O, no clock, no threads.
// Every state decision of the observer is made by `Store::apply`; source threads send facts as
// `Signal`s and execute the `Effect`s it returns. Time always comes in as `now_ms`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

pub use super::antigravity::AntigravityEvent;
pub use super::claude::ClaudeSessionFile;
pub use super::codex::HookEvent;
#[allow(unused_imports)] // spec §5.4 names them model::claude_key / model::codex_key; callers use observe::keys
pub use super::keys::{claude_key, codex_key};
pub use super::keys::{HostKind, ProcId, SessionKey};
pub use super::paths::ProjectRef;

use super::claude_transcript::{self as transcript, Fact, FileEdit, TitleKind, TurnOutcome};
use super::codex::{parse_rollout_line, RolloutRecord, REQUEST_PERMISSIONS};
use super::silent::{is_alive, reason_of, SilentReason};
use crate::message::Message;
use super::{paths, text};

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

/// Where a session comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")] // "claude" | "codex" | "antigravity"
pub enum Source {
    Claude,
    Codex,
    /// Fed only by Mayak's Antigravity hooks; it has no health of its own.
    Antigravity,
}

/// Health of one source (spec §10 `Health`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")] // "ok" | "missing" | {"error":message} | {"degraded":{"unreadable":n}}
pub enum Health {
    Ok,
    Missing,
    /// Shown in Settings: the code is translated there, the OS error stays in the params.
    Error(Message),
    Degraded { unreadable: u32 },
}

impl Health {
    /// For the log: an error by its code only, never its params (paths, OS texts).
    pub fn log_label(&self) -> String {
        match self {
            Health::Error(message) => format!("Error({})", message.code),
            other => format!("{other:?}"),
        }
    }
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
    /// claude-io: what `.claude.json` says about the plan limits (usage-home spec §2.1); sent at
    /// its start, on Retry and on ReadUsage when it differs from the last value sent; None when the
    /// file or the key is absent.
    ClaudeUsage(Option<ClaudeUsage>),
    /// codex-io, once at its start: the limits of the newest rollout on disk, `seen_at_ms` being the
    /// time that record was written; also `codex_limits`, the limits read from the Codex app server,
    /// `seen_at_ms` being the time of that read. The store keeps it only when it is newer than the
    /// usage it has.
    CodexUsageSeed(CodexUsage),
    /// `openrouter`: what OpenRouter's `GET /api/v1/key` said about the user's key; None when the
    /// key was removed, replaced or refused.
    OpenRouterUsage(Option<OpenRouterUsage>),
    /// `openrouter`: what went wrong with the last read of the key (shown in Settings); None once a
    /// read succeeds or the key is saved or removed.
    OpenRouterProblem(Option<KeyProblem>),
    /// `deepseek`: what DeepSeek's `GET /user/balance` said about the user's key; None when the key
    /// was removed, replaced or refused.
    DeepSeekBalance(Option<DeepSeekBalance>),
    /// `deepseek`: what went wrong with the last read of the key (shown in Settings).
    DeepSeekProblem(Option<KeyProblem>),
    /// `moonshot`: what Moonshot's `GET /v1/users/me/balance` said about the user's key; None when
    /// the key was removed, replaced or refused.
    MoonshotBalance(Option<MoonshotBalance>),
    /// `moonshot`: what went wrong with the last read of the key (shown in Settings).
    MoonshotProblem(Option<KeyProblem>),
    /// codex-io sends it before the `CodexHook` of the same session. `folder_exists`: the `cwd` of
    /// the rollout's `session_meta` is a folder, checked by codex-io when it read this batch (the
    /// reason of a silent turn, silent-codex Task 7).
    CodexRollout { key: SessionKey, path: String, records: Vec<(u64, Value)>, eof: u64, folder_exists: bool },
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
    /// codex-io: a rollout of a session with no hook (spec silent-codex §2). `written_ms` is the
    /// store clock at the file event.
    CodexSilent { id: String, project: ProjectRef, host: HostKind, folder_exists: bool, written_ms: i64 },
    CodexHooksInstalled(bool),
    /// codex-io: one Antigravity hook (observe/antigravity.rs), its project, its host and the
    /// Antigravity process above the relay (when proved) resolved.
    AntigravityHook { key: SessionKey, event: AntigravityEvent, project: ProjectRef, proc: Option<ProcId>, host: HostKind },
    ProcExit { pid: u32, created_ft: u64 },
    Ack(SessionKey),
    /// The island is shown / hidden. `run_store` reads it to decide what to publish; the store itself
    /// ignores it (the Codex ticker does not depend on it).
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
    /// codex-io: 1 Hz rollout ticker on/off, wanted while a Codex session is Working or Waiting; emitted
    /// only when the wanted state changes.
    CodexTicker(bool),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied {
    pub alerts: Vec<Alert>,
    pub effects: Vec<Effect>,
    /// A silent Codex row was inserted, or a known Codex session got a silent mark, that was not
    /// alive before: the island should show itself once. It is no alert, no sound and no effect.
    pub wake: bool,
    /// A Claude session that was working or waiting is now idle or gone (its turn or the session
    /// ended): Claude Code may have cached new plan limits, so `run_store` has claude-io read
    /// `.claude.json` again.
    pub claude_turn_ended: bool,
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
    /// `call_id`s of the unanswered `request_permissions` calls that the current Waiting(Permission)
    /// was inferred from (spec §7): a set of at most MAX_PENDING_CALLS, oldest dropped; empty
    /// otherwise.
    pub pending_permission_calls: Vec<String>,
    /// A rollout `task_started` was read and no hook of this session arrived since. A hooked turn
    /// always delivers a hook (Stop at the latest) before its `task_complete` is written.
    pub turn_unhooked: bool,
    /// A user-role message record was read since the last `task_started`: the turn is a user turn,
    /// not a Codex-internal task. Only a user turn without a hook is silent.
    pub turn_user: bool,
    /// (store clock, folder_exists) of the last turn that ended with `turn_unhooked`; shown while
    /// `silent::is_alive`, cleared by the next hook.
    pub silent_mark: Option<(i64, bool)>,
    /// The last context fill read from the rollout or the transcript; every newer value replaces
    /// it, also a smaller one (compaction). It changes nothing else (context-fill spec §3).
    pub context: Option<ContextFill>,
    /// Claude: the model of the last main-chain answer (`message.model`), which gives the window of
    /// `context`. Cleared with it by a new transcript.
    pub model: Option<String>,
    /// Claude: the last context compaction read from the transcript; cleared with `context`.
    pub compacted: Option<Compacted>,
    /// Claude: the validated `hostSessionId` of the last session file, for "Open" on a Claude
    /// Desktop session (quick wins §3.1). Never logged, never shown.
    pub host_session_id: Option<String>,
    /// The agent's last reply in full (`text::reply`), for the row menu's «Copy last reply»
    /// (idea #8): Claude's last main-chain assistant text, the message of Codex's and Antigravity's
    /// last finished turn. Never logged, never sent to the page; put on the clipboard on a click only.
    pub last_reply: Option<Arc<str>>,
    /// Claude: the file changes of the transcript records with offset >= turn_start_offset (ideas
    /// §3 «Итог хода»), at most MAX_TURN_EDITS, oldest dropped; kept apart from `facts`.
    pub turn_edits: Vec<(u64, FileEdit)>,
    /// Edits were dropped from `turn_edits` this turn: its totals are a lower bound.
    pub turn_edits_dropped: bool,
    /// Store clock (relay clock for a hooked Codex turn) of a turn start that was seen live; None
    /// when the turn was already running, or had already ended, when the store first saw it.
    pub turn_began_ms: Option<i64>,
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

/// How full a session's context is (context-fill spec §3): the tokens of its last request and, for
/// Codex, the size of its window. Only numbers; nothing of the messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ContextFill {
    pub used: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<u64>,
}

/// The last context compaction of a Claude session (`system/compact_boundary`): when it ended (the
/// record's time, else the store clock when it was read) and the context before and after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Compacted {
    pub at: i64,
    pub pre: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post: Option<u64>,
}

/// Which Claude limit window (usage-home spec §3): `utilization.five_hour` is "5h", `seven_day` "7d",
/// `seven_day_<suffix>` a model's weekly window, sent as "7d:<suffix>" (suffix as in the key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeWindowId {
    FiveHour,
    SevenDay,
    SevenDayModel(String),
}

impl Serialize for ClaudeWindowId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::FiveHour => serializer.serialize_str("5h"),
            Self::SevenDay => serializer.serialize_str("7d"),
            Self::SevenDayModel(suffix) => serializer.serialize_str(&format!("7d:{suffix}")),
        }
    }
}

/// One Claude limit window: the percent used and, when its date parsed, the reset time (ms).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeWindow {
    pub id: ClaudeWindowId,
    pub percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<i64>,
}

/// What Claude Code cached about the plan limits (usage-home spec §2.1); `fetched_at_ms` is when
/// Claude Code got them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeUsage {
    pub fetched_at_ms: i64,
    pub windows: Vec<ClaudeWindow>,
}

/// One limit window of the Codex account (usage-home spec §2.2, §3): its length in minutes, the
/// percent used and, when Codex wrote it, the reset time (ms since the epoch).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWindow {
    pub minutes: u64,
    pub percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<i64>,
}

/// The last Codex limits read, from any session (usage-home spec §2.2); `seen_at_ms` is the store
/// clock when Mayak read them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUsage {
    pub seen_at_ms: i64,
    pub windows: Vec<CodexWindow>,
}

/// The credits of an OpenRouter key (`GET /api/v1/key`), in US dollars: `limit` and
/// `limit_remaining` are None for a key without a limit, `limit_reset` ("daily", "weekly",
/// "monthly") None for a limit that never resets. `fetched_at_ms` is when Mayak read them. The key
/// label and anything else of the answer are not kept.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRouterUsage {
    pub fetched_at_ms: i64,
    pub limit: Option<f64>,
    pub limit_remaining: Option<f64>,
    pub limit_reset: Option<String>,
    pub usage: f64,
    pub usage_daily: f64,
    pub usage_weekly: f64,
    pub usage_monthly: f64,
}

/// One currency of a DeepSeek balance (`balance_infos[]`): `currency` is three capital letters
/// ("CNY", "USD"), the amounts are DeepSeek's decimal strings read as numbers.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyBalance {
    pub currency: String,
    pub total: f64,
    pub granted: f64,
    pub topped_up: f64,
}

/// A DeepSeek key's balance (`GET /user/balance`): `is_available` is false when the balance is too
/// low for API calls; `fetched_at_ms` is when Mayak read it. Nothing else of the answer is kept.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepSeekBalance {
    pub fetched_at_ms: i64,
    pub is_available: bool,
    pub balances: Vec<CurrencyBalance>,
}

/// A Moonshot (Kimi) key's balance (`GET /v1/users/me/balance`): `available` = `voucher` + `cash`,
/// `cash` may be below 0 (owed). `currency` is "USD" for an api.moonshot.ai key and "CNY" for an
/// api.moonshot.cn one, as Moonshot documents for each platform.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoonshotBalance {
    pub fetched_at_ms: i64,
    pub currency: String,
    pub available: f64,
    pub voucher: f64,
    pub cash: f64,
}

/// Why the last read of a provider key (OpenRouter, DeepSeek, Moonshot) gave nothing: the provider
/// refused the key (401/403), or it could not be reached or answered with an error (network, 5xx,
/// an unusable answer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyProblem {
    Rejected,
    Unreachable,
}

/// The plan limits of the home page (usage-home spec §3); a side is null until its source gave a
/// value. A change bumps `rev` and nothing else.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Usage {
    pub claude: Option<ClaudeUsage>,
    pub codex: Option<CodexUsage>,
    /// Only while the user has an OpenRouter key and it was read.
    pub openrouter: Option<OpenRouterUsage>,
    /// The Settings page's line under the OpenRouter key field; null when there is nothing to say.
    #[serde(rename = "openrouterProblem")]
    pub openrouter_problem: Option<KeyProblem>,
    /// Only while the user has a DeepSeek key and it was read.
    pub deepseek: Option<DeepSeekBalance>,
    #[serde(rename = "deepseekProblem")]
    pub deepseek_problem: Option<KeyProblem>,
    /// Only while the user has a Moonshot (Kimi) key and it was read.
    pub moonshot: Option<MoonshotBalance>,
    #[serde(rename = "moonshotProblem")]
    pub moonshot_problem: Option<KeyProblem>,
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
    /// Set on a muted row of a Codex session whose hooks do not reach Mayak: why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silent: Option<SilentReason>,
    /// How full the context is; absent until the first value (context-fill spec §3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextFill>,
    /// The model id of a Claude session, once an answer named it; shown in the context tooltip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The last compaction of the session's context; absent until one was read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compacted: Option<Compacted>,
    /// What the last turn did (ideas §3 «Итог хода»): set once the turn has an outcome, when it
    /// changed files or its length is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnView>,
    /// Pid of the agent process, for the window switch (sub-project 2). Not serialized: the UI never needs it.
    #[serde(skip)]
    pub agent_pid: Option<u32>,
    /// Claude Desktop chat id, for the "Open" link (quick wins §3.1). Not serialized: never shown.
    #[serde(skip)]
    pub host_session_id: Option<String>,
    /// Claude `sessionId` / Codex `session_id`, for «Recent» and the resume command (resume.rs) and
    /// for the row menu's resume command (idea #8); None when empty. Not serialized: never shown.
    #[serde(skip)]
    pub logical_id: Option<String>,
    /// The session's folder, where it is resumed (resume.rs). Not serialized.
    #[serde(skip)]
    pub cwd: Option<String>,
    /// `Session::last_reply`, for the row menu's «Copy last reply» (idea #8). Not serialized: it
    /// reaches the clipboard from Rust, never the page.
    #[serde(skip)]
    pub last_reply: Option<Arc<str>>,
    /// The Agents chat conversation this session is (Mayak's chat runs its CLI): the row opens the
    /// chat. Set by the app when it publishes (lib.rs `tag_chat_sessions`), never by the store.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat: Option<u64>,
}

/// One changed file of a turn: its path (shown relative to the project, opened in the editor by
/// index through `session_file_open`), its line counts and the line of its first change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnFileView {
    pub path: String,
    pub added: u64,
    pub removed: u64,
    pub line: u32,
}

/// The summary of a turn (ideas §3): the changed files (at most MAX_TURN_FILES, `fileCount` is the
/// real number), the line totals over all of them, `partial` when edits were dropped (the totals
/// are then a lower bound), and the turn's length in ms when its start was seen live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnView {
    pub files: Vec<TurnFileView>,
    pub file_count: u32,
    pub added: u64,
    pub removed: u64,
    pub partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<i64>,
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
    pub usage: Usage,
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
                Some(parent) if collides => format!("{parent}{}{base}", paths::SEP),
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
    usage: Usage,
}

/// Sessions of one project, in display order.
struct Group<'a> {
    key: String,
    root: String,
    sessions: Vec<&'a Session>,
}

/// A Codex session seen only through its rollout (spec silent-codex §3).
struct SilentRow {
    project: ProjectRef,
    host: HostKind,
    folder_exists: bool,
    last_write_ms: i64,
}

/// At most this many silent rows are kept; the oldest write goes first.
const MAX_SILENT: usize = 64;

pub struct Store {
    sessions: BTreeMap<SessionKey, Session>,
    /// Silent Codex rows by session id.
    silent: BTreeMap<String, SilentRow>,
    rev: u64,
    prefs: AlertPrefs,
    claude_health: Health,
    codex_health: Health,
    codex_hooks: CodexHooksView,
    /// The plan limits of the home page (usage-home spec §3).
    usage: Usage,
    /// Last `CodexTicker` state emitted.
    ticker_on: bool,
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
            silent: BTreeMap::new(),
            rev: 0,
            prefs: AlertPrefs::default(),
            claude_health: Health::Ok,
            codex_health: Health::Ok,
            codex_hooks: CodexHooksView::NotInstalled,
            usage: Usage::default(),
            ticker_on: false,
            last_view: None,
        };
        store.last_view = Some(store.view(0));
        store
    }

    pub fn apply(&mut self, s: Signal, now_ms: i64) -> Applied {
        let mut fx: Vec<Effect> = Vec::new();
        let mut raised: Vec<Raised> = Vec::new();
        let mut wake = false;
        let busy_claude: Vec<SessionKey> = self
            .sessions
            .values()
            .filter(|s| s.agent == Source::Claude && matches!(s.phase, Phase::Working | Phase::Waiting { .. }))
            .map(|s| s.key.clone())
            .collect();
        match s {
            Signal::ClaudeTranscript { key, rebind_at, records, eof: _ } => {
                self.claude_transcript(&key, rebind_at, records, now_ms, &mut raised)
            }
            Signal::ClaudeFile { key, proc, host, project, file, transcript_eof } => {
                self.claude_file(key, proc, host, project, file, transcript_eof, now_ms, &mut fx, &mut raised)
            }
            Signal::ClaudeGone { key } => self.remove_session(&key, &mut fx),
            Signal::ClaudeUsage(usage) => self.usage.claude = usage,
            Signal::CodexUsageSeed(seed) => {
                if self.usage.codex.as_ref().map_or(true, |live| live.seen_at_ms < seed.seen_at_ms) {
                    self.usage.codex = Some(seed);
                }
            }
            Signal::OpenRouterUsage(usage) => self.usage.openrouter = usage,
            Signal::OpenRouterProblem(problem) => self.usage.openrouter_problem = problem,
            Signal::DeepSeekBalance(balance) => self.usage.deepseek = balance,
            Signal::DeepSeekProblem(problem) => self.usage.deepseek_problem = problem,
            Signal::MoonshotBalance(balance) => self.usage.moonshot = balance,
            Signal::MoonshotProblem(problem) => self.usage.moonshot_problem = problem,
            Signal::CodexRollout { key, path: _, records, eof: _, folder_exists } => {
                wake = self.codex_rollout(&key, records, folder_exists, now_ms, &mut raised)
            }
            Signal::CodexHook { key, event, proc, host, project, rollout_eof: _, title } => {
                self.codex_hook(key, event, proc, host, project, title, now_ms, &mut fx, &mut raised)
            }
            Signal::CodexSilent { id, project, host, folder_exists, written_ms } => {
                wake = self.codex_silent(id, project, host, folder_exists, written_ms, now_ms)
            }
            Signal::CodexHooksInstalled(installed) => self.codex_hooks_installed(installed),
            Signal::AntigravityHook { key, event, project, proc, host } => {
                self.antigravity_hook(key, event, project, proc, host, now_ms, &mut fx, &mut raised)
            }
            Signal::ProcExit { pid, created_ft } => self.proc_exit(pid, created_ft, &mut fx),
            Signal::Ack(key) => self.ack(&key, now_ms, &mut fx),
            // Visibility only steers what is published (`run_store`); the model has no use for it.
            Signal::Visible(_) => {}
            Signal::AlertPrefs { waiting, finished, error } => self.prefs = AlertPrefs { waiting, finished, error },
            Signal::SourceHealth { source: Source::Claude, health } => self.claude_health = health,
            Signal::SourceHealth { source: Source::Codex, health } => self.codex_health = health,
            // No source thread reports one: Antigravity's sessions come from its hooks only.
            Signal::SourceHealth { source: Source::Antigravity, .. } => {}
        }
        self.update_ticker(&mut fx);
        self.silent.retain(|_, r| is_alive(r.last_write_ms, now_ms));
        let alerts = self.complete_alerts(raised, now_ms);
        let view = self.view(now_ms);
        if self.last_view.as_ref() != Some(&view) {
            self.rev += 1;
            self.last_view = Some(view);
        }
        let claude_turn_ended =
            busy_claude.iter().any(|key| self.sessions.get(key).map_or(true, |s| s.phase == Phase::Idle));
        Applied { alerts, effects: fx, wake, claude_turn_ended }
    }

    pub fn snapshot(&self, now_ms: i64) -> SessionsSnapshot {
        let v = self.view(now_ms);
        SessionsSnapshot {
            rev: self.rev,
            now: now_ms,
            projects: v.projects,
            attention: v.attention,
            counts: v.counts,
            hold: v.hold,
            aggregate: v.aggregate,
            sources: v.sources,
            usage: v.usage,
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

    /// Ack sets `acked_at` and never changes the phase (spec §6, D49). An Idle Codex session without
    /// a confirmed process is removed instead, until its next hook (spec §7 Codex table only). An
    /// Antigravity session stays listed (`MAX_IDLE_ANTIGRAVITY`, `ProcExit`).
    fn ack(&mut self, key: &str, now_ms: i64, fx: &mut Vec<Effect>) {
        let Some(s) = self.sessions.get_mut(key) else { return };
        if s.agent == Source::Codex && s.phase == Phase::Idle && s.proc.is_none() {
            self.remove_session(key, fx);
            return;
        }
        s.acked_at = Some(episode_start(s).map_or(now_ms, |start| now_ms.max(start)));
    }

    fn remove_session(&mut self, key: &str, fx: &mut Vec<Effect>) {
        if let Some(s) = self.sessions.remove(key) {
            match s.agent {
                Source::Claude => fx.push(Effect::UntrackTranscript { key: s.key }),
                Source::Codex => fx.push(Effect::UntrackRollout { key: s.key }),
                // Nothing is tracked for it.
                Source::Antigravity => {}
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

    fn complete_alerts(&self, raised: Vec<Raised>, now_ms: i64) -> Vec<Alert> {
        if raised.is_empty() {
            return Vec::new();
        }
        let names = display_names(&self.project_pairs(now_ms));
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
    fn project_pairs(&self, now_ms: i64) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = Vec::new();
        for s in self.sessions.values() {
            if !pairs.iter().any(|(k, _)| *k == s.project.key) {
                pairs.push((s.project.key.clone(), s.project.root.clone()));
            }
        }
        for row in self.silent.values().filter(|r| is_alive(r.last_write_ms, now_ms)) {
            if !pairs.iter().any(|(k, _)| *k == row.project.key) {
                pairs.push((row.project.key.clone(), row.project.root.clone()));
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

    /// Alive silent rows go last in their project; a project with only silent rows goes after all
    /// the others. They feed nothing else (counts, attention, hold, aggregate, alerts).
    fn append_silent(&self, projects: &mut Vec<ProjectView>, names: &HashMap<String, String>, now_ms: i64) {
        let hooks_not_installed = self.codex_hooks == CodexHooksView::NotInstalled;
        for (id, row) in self.silent.iter().filter(|(_, r)| is_alive(r.last_write_ms, now_ms)) {
            let view = SessionView {
                key: format!("silent:{id}"),
                agent: Source::Codex,
                host: row.host,
                title: None,
                phase: PhaseView::Idle,
                since: row.last_write_ms,
                activity: None,
                detail: None,
                outcome: None,
                attention: None,
                acked: true,
                silent: Some(reason_of(hooks_not_installed, row.folder_exists)),
                context: None,
                model: None,
                compacted: None,
                turn: None,
                agent_pid: None,
                host_session_id: None,
                // The Codex session id: «Recent» leaves a session listed here out (resume.rs).
                logical_id: Some(id.clone()),
                cwd: None,
                last_reply: None,
                chat: None,
            };
            match projects.iter_mut().find(|p| p.key == row.project.key) {
                Some(p) => p.sessions.push(view),
                None => projects.push(ProjectView {
                    key: row.project.key.clone(),
                    name: names.get(&row.project.key).cloned().unwrap_or_default(),
                    root: row.project.root.clone(),
                    sessions: vec![view],
                }),
            }
        }
    }

    fn view(&self, now_ms: i64) -> View {
        let groups = self.groups();
        let names = display_names(&self.project_pairs(now_ms));
        let hooks_not_installed = self.codex_hooks == CodexHooksView::NotInstalled;
        let mut projects: Vec<ProjectView> = groups
            .iter()
            .map(|g| ProjectView {
                key: g.key.clone(),
                name: names.get(&g.key).cloned().unwrap_or_default(),
                root: g.root.clone(),
                sessions: g.sessions.iter().map(|s| session_view(s, hooks_not_installed, now_ms)).collect(),
            })
            .collect();
        self.append_silent(&mut projects, &names, now_ms);
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
            usage: self.usage.clone(),
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
            pending_permission_calls: Vec::new(),
            turn_unhooked: false,
            turn_user: false,
            silent_mark: None,
            context: None,
            model: None,
            compacted: None,
            host_session_id: None,
            last_reply: None,
            turn_edits: Vec::new(),
            turn_edits_dropped: false,
            turn_began_ms: None,
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

fn session_view(s: &Session, hooks_not_installed: bool, now_ms: i64) -> SessionView {
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
        silent: s
            .silent_mark
            .filter(|(at_ms, _)| is_alive(*at_ms, now_ms))
            .map(|(_, folder_exists)| reason_of(hooks_not_installed, folder_exists)),
        context: s.context,
        model: s.model.clone(),
        compacted: s.compacted,
        turn: turn_view(s),
        agent_pid: s.proc.as_ref().map(|p| p.pid),
        host_session_id: s.host_session_id.clone(),
        logical_id: Some(s.logical_id.clone()).filter(|id| !id.is_empty()),
        cwd: Some(s.cwd.clone()).filter(|cwd| !cwd.is_empty()),
        last_reply: s.last_reply.clone(),
        chat: None,
    }
}

/// Changed files sent per turn; the totals still count every file.
const MAX_TURN_FILES: usize = 50;

/// The turn summary of a session whose turn has an outcome (Finished, Error, Interrupted), or
/// None. Claude edits read after the session went Idle belong to the next turn, whose records may
/// come before its file change (D21): only those below `idle_eof` count then.
fn turn_view(s: &Session) -> Option<TurnView> {
    let at = match s.outcome {
        Outcome::Finished { at_ms, .. } | Outcome::Error { at_ms, .. } | Outcome::Interrupted { at_ms } => at_ms,
        Outcome::None | Outcome::Pending => return None,
    };
    let edits = s
        .turn_edits
        .iter()
        .filter(|(offset, _)| s.phase != Phase::Idle || *offset < s.idle_eof)
        .map(|(_, edit)| edit);
    let files = transcript::turn_files(edits);
    let ms = s.turn_began_ms.map(|began| at - began).filter(|ms| *ms >= 0);
    if files.is_empty() && ms.is_none() {
        return None;
    }
    Some(TurnView {
        file_count: u32::try_from(files.len()).unwrap_or(u32::MAX),
        added: files.iter().map(|f| f.added).sum(),
        removed: files.iter().map(|f| f.removed).sum(),
        partial: s.turn_edits_dropped,
        ms,
        files: files
            .into_iter()
            .take(MAX_TURN_FILES)
            .map(|f| TurnFileView { path: f.path, added: f.added, removed: f.removed, line: f.line })
            .collect(),
    })
}

// ── Claude state machine (spec §7 "Claude") ──

/// Facts kept per Claude session (D14).
const MAX_FACTS: usize = 1024;

/// File edits kept per Claude turn.
const MAX_TURN_EDITS: usize = 1024;

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
    s.turn_edits.retain(|(offset, _)| *offset >= start);
    s.turn_edits_dropped = false;
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
            // a turn already running past the start of what is read: its edits are a lower bound
            s.turn_edits_dropped = base > 0 && matches!(s.phase, Phase::Working | Phase::Waiting { .. });
            s.waiting_for = file.waiting_for;
            s.file_name = file.name;
            s.host_session_id = file.host_session_id;
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
        s.host_session_id = file.host_session_id.clone();
        if !session_id.is_empty() && session_id != s.logical_id {
            // /clear, resume, fork: same process, new transcript. Offsets come with the next
            // ClaudeTranscript{rebind_at} (spec §7 "Смена sessionId").
            fx.push(Effect::UntrackTranscript { key: key.clone() });
            fx.push(Effect::TrackTranscript { key: key.clone(), session_id: session_id.clone() });
            s.logical_id = session_id;
            s.transcript_bound = false;
            s.facts.clear();
            s.turn_edits.clear();
            s.turn_edits_dropped = false;
            // the old turn's length without its files would read as this transcript's turn
            s.turn_began_ms = None;
            s.pending_tools.clear();
            s.titles = TranscriptTitles::default();
            s.activity = None;
            s.detail = None;
            // a new transcript is a new context (/clear, resume): the old number is no longer true
            s.context = None;
            s.model = None;
            s.compacted = None;
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
                    s.turn_began_ms = Some(now_ms);
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
                        s.turn_began_ms = Some(now_ms);
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
                        s.turn_began_ms = None;
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
                s.turn_edits.clear();
                // bound past offset 0 in the middle of a turn: the edits made before are not
                // read, the totals are a lower bound (a turn start seen later resets it)
                s.turn_edits_dropped = o > 0 && s.phase != Phase::Idle;
                s.pending_tools.clear();
                s.titles = TranscriptTitles::default();
                s.activity = None;
                s.idle_eof = o;
                s.turn_start_offset = o;
                s.transcript_bound = true;
                // the records of the new binding tell it again
                s.compacted = None;
                s.last_reply = None;
            }
            // records of a transcript the model already let go (D52)
            None if !s.transcript_bound => return,
            None => {}
        }
        if let Some(model) = transcript::last_model(&records) {
            s.model = Some(model);
        }
        // Claude writes no window size (context-fill spec §2.2): it comes from the model's id.
        if let Some(used) = transcript::last_context_used(&records).or(s.context.map(|c| c.used)) {
            let (used, window) = transcript::claude_fill(used, s.model.as_deref());
            s.context = Some(ContextFill { used, window });
        }
        if let Some(c) = transcript::last_compaction(&records) {
            s.compacted = Some(Compacted { at: c.at_ms.unwrap_or(now_ms), pre: c.pre, post: c.post });
        }
        for (offset, record) in records {
            if let Some(reply) = transcript::reply_of(&record) {
                s.last_reply = Some(reply);
            }
            if offset >= s.turn_start_offset {
                if let Some(edit) = transcript::file_edit(&record) {
                    s.turn_edits.push((offset, edit));
                }
            }
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
        if s.turn_edits.len() > MAX_TURN_EDITS {
            let excess = s.turn_edits.len() - MAX_TURN_EDITS;
            s.turn_edits.drain(..excess);
            s.turn_edits_dropped = true;
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

// ── Codex state machine (spec §7 "Codex") ──

/// Closed turns remembered per Codex session.
const MAX_CLOSED_TURNS: usize = 8;

/// Turn start: `start_turn(T, ts)` of spec §7, plus `detail`/`activity` cleared (contract §2.15).
fn start_turn(s: &mut Session, turn: Option<String>, ts: i64) {
    s.current_turn = turn;
    s.turn_started_ts = ts;
    s.turn_began_ms = Some(ts);
    s.outcome = Outcome::None;
    s.wait_key = None;
    s.pending_permission_calls.clear();
    s.detail = None;
    s.activity = None;
}

/// Codex phase change: `since = max(prev, ts)`; `wait_key` and `pending_permission_calls` only live
/// in Waiting, so every turn end (Idle) and every resume (Working) drops them.
fn set_codex_phase(s: &mut Session, phase: Phase, ts: i64) {
    s.phase = phase;
    s.since_ms = s.since_ms.max(ts);
    if !matches!(s.phase, Phase::Waiting { .. }) {
        s.wait_key = None;
        s.pending_permission_calls.clear();
    }
}

/// Unanswered `request_permissions` calls remembered per session; the oldest is dropped.
const MAX_PENDING_CALLS: usize = 8;

/// A Waiting(Permission) that was inferred from `request_permissions` calls (spec §7) and that no hook
/// has taken over.
fn waiting_by_inference(s: &Session) -> bool {
    s.phase == (Phase::Waiting { kind: WaitKind::Permission })
        && s.wait_key.as_ref().is_some_and(|(_, tool, _)| tool == REQUEST_PERMISSIONS)
}

/// Waiting(Permission) of a Working session, inferred from an unanswered `request_permissions` call
/// (spec §7). Mirrors the PermissionRequest hook: a new episode (`since` = the store clock), `detail`,
/// `wait_key`. The alert is raised by `raise_inferred_wait` once the whole batch is applied.
fn wait_for_permission(s: &mut Session, call_id: String, now_ms: i64) {
    set_codex_phase(s, Phase::Waiting { kind: WaitKind::Permission }, now_ms);
    s.detail = Some(REQUEST_PERMISSIONS.into());
    s.wait_key = Some((s.current_turn.clone().unwrap_or_default(), REQUEST_PERMISSIONS.into(), fnv1a64(call_id.as_bytes())));
    s.pending_permission_calls = vec![call_id];
    s.acked_at = None;
}

/// Another unanswered call while the inferred wait lasts: the same episode, so no second alert. The
/// pending calls are a set; past MAX_PENDING_CALLS the oldest is forgotten.
fn remember_permission_call(s: &mut Session, call_id: String) {
    if s.pending_permission_calls.contains(&call_id) {
        return;
    }
    s.pending_permission_calls.push(call_id);
    if s.pending_permission_calls.len() > MAX_PENDING_CALLS {
        s.pending_permission_calls.remove(0);
    }
}

/// End of a batch that started an inferred wait: its Permission alert, only if the session still
/// waits for an unanswered call. A batch that went on to end the turn (or to start another one)
/// raises nothing for a wait that never outlived it.
fn raise_inferred_wait(s: &Session, raised: &mut Vec<Raised>) {
    if waiting_by_inference(s) {
        raised.push(Raised { key: s.key.clone(), kind: AttentionKind::Permission, body: s.detail.clone(), since: s.since_ms });
    }
}

/// The output of `call_id` arrived: the call is no longer pending. Once the last pending call is
/// answered the inferred wait ends. A wait that a hook has taken over (another `wait_key` tool) is
/// left as it is.
fn permission_answered(s: &mut Session, call_id: &str, now_ms: i64) {
    let Some(i) = s.pending_permission_calls.iter().position(|id| id == call_id) else { return };
    s.pending_permission_calls.remove(i);
    if s.pending_permission_calls.is_empty() && waiting_by_inference(s) {
        set_codex_phase(s, Phase::Working, now_ms);
        s.detail = None;
    }
}

fn close_turn(s: &mut Session, turn: Option<String>) {
    if let Some(t) = turn {
        if !s.closed_turns.contains(&t) {
            s.closed_turns.push_back(t);
            while s.closed_turns.len() > MAX_CLOSED_TURNS {
                s.closed_turns.pop_front();
            }
        }
    }
}

/// Idle + Finished(summary) at the store clock (D50), ack cleared (D49).
fn finish(s: &mut Session, message: Option<&str>, now_ms: i64) -> Raised {
    let summary = text::collapse(message.unwrap_or(""), text::SUMMARY_MAX);
    if let Some(reply) = message.and_then(text::reply) {
        s.last_reply = Some(reply);
    }
    s.outcome = Outcome::Finished { summary: summary.clone(), at_ms: now_ms };
    s.acked_at = None;
    Raised { key: s.key.clone(), kind: AttentionKind::Finished, body: Some(summary), since: now_ms }
}

/// Error(text) at the store clock (D50), ack cleared (D49).
fn fail(s: &mut Session, error: &str, now_ms: i64) -> Raised {
    let text = text::collapse(error, text::SUMMARY_MAX);
    s.outcome = Outcome::Error { code: None, status: None, text: text.clone(), at_ms: now_ms };
    s.acked_at = None;
    Raised { key: s.key.clone(), kind: AttentionKind::Error, body: Some(text::error_summary(None, &text)), since: now_ms }
}

impl Store {
    #[allow(clippy::too_many_arguments)]
    fn codex_hook(
        &mut self,
        key: SessionKey,
        ev: HookEvent,
        proc: Option<ProcId>,
        host: HostKind,
        project: ProjectRef,
        title: Option<String>,
        now_ms: i64,
        fx: &mut Vec<Effect>,
        raised: &mut Vec<Raised>,
    ) {
        self.silent.remove(&ev.session_id);
        self.note_receiving(host, ev.relay_ts);
        if ev.hook_event_name == "SessionEnd" {
            // removes a known session; an unknown key creates nothing (D22)
            self.remove_session(&key, fx);
            return;
        }
        // Creation: then the hook itself is applied as a live transition (spec §7).
        let s = self.sessions.entry(key.clone()).or_insert_with(|| {
            let mut s = Session::blank(
                key.clone(),
                Source::Codex,
                host,
                proc.clone(),
                project.clone(),
                ev.session_id.clone(),
                ev.cwd.clone(),
                ev.relay_ts,
            );
            s.current_turn = ev.turn_id.clone();
            s.turn_started_ts = ev.relay_ts;
            s
        });
        // Any hook proves that this turn's hooks reach Mayak.
        s.turn_unhooked = false;
        s.silent_mark = None;
        s.cwd = ev.cwd.clone();
        s.project = project;
        if host != HostKind::Unknown {
            s.host = host;
        }
        if let Some(p) = proc {
            s.proc = Some(p);
        }
        if let Some(t) = title {
            s.title = Some(t);
        }

        let ts = ev.relay_ts;
        // a missing turn_id means the current turn
        let turn = ev.turn_id.clone().or_else(|| s.current_turn.clone());
        match ev.hook_event_name.as_str() {
            "UserPromptSubmit" => {
                start_turn(s, turn, ts);
                set_codex_phase(s, Phase::Working, ts);
            }
            "PermissionRequest" => {
                if turn != s.current_turn {
                    start_turn(s, turn.clone(), ts);
                }
                let tool = ev.tool_name.clone().unwrap_or_default();
                let input = ev.tool_input.clone().unwrap_or(Value::Null);
                let wait_key = (turn.unwrap_or_default(), tool.clone(), fnv1a64(canonical_json(&input).as_bytes()));
                let retry = matches!(s.phase, Phase::Waiting { .. }) && s.wait_key.as_ref() == Some(&wait_key);
                if !retry {
                    let label = text::tool_label(&tool, ev.tool_input.as_ref());
                    s.detail = Some(match &ev.agent_id {
                        Some(id) => format!("{}: {}", ev.agent_type.as_deref().unwrap_or(id), label),
                        None => label,
                    });
                    set_codex_phase(s, Phase::Waiting { kind: WaitKind::Permission }, ts);
                    s.wait_key = Some(wait_key);
                    s.acked_at = None;
                    raised.push(Raised {
                        key: key.clone(),
                        kind: AttentionKind::Permission,
                        body: s.detail.clone(),
                        since: s.since_ms,
                    });
                }
            }
            "Stop" => {
                let closed = turn.as_ref().is_some_and(|t| s.closed_turns.contains(t));
                let stale = turn != s.current_turn && ts < s.turn_started_ts;
                if !closed && !stale {
                    s.current_turn = turn.clone();
                    set_codex_phase(s, Phase::Idle, ts);
                    raised.push(finish(s, ev.last_assistant_message.as_deref(), now_ms));
                    close_turn(s, turn);
                }
            }
            "Interrupt" => {
                if turn == s.current_turn {
                    set_codex_phase(s, Phase::Idle, ts);
                    s.outcome = Outcome::Interrupted { at_ms: now_ms };
                }
            }
            _ => {} // other events only refresh host/proc/cwd/project/title
        }
    }

    /// Rollout records of a known session; rollout-driven transitions use the store clock. True when
    /// a turn ended without any hook and the session was not silent-alive before: the island wakes.
    fn codex_rollout(
        &mut self,
        key: &str,
        records: Vec<(u64, Value)>,
        folder_exists: bool,
        now_ms: i64,
        raised: &mut Vec<Raised>,
    ) -> bool {
        let records: Vec<RolloutRecord> = records.iter().map(|(_, record)| parse_rollout_line(record)).collect();
        // The account's limits (usage-home spec §2.2): the batch's last value wins, read at the store
        // clock, whether or not the store still knows this session.
        let limits = records.iter().rev().find_map(|r| match r {
            RolloutRecord::Tokens { limits: Some(windows), .. } => Some(windows.clone()),
            _ => None,
        });
        if let Some(windows) = limits {
            self.usage.codex = Some(CodexUsage { seen_at_ms: now_ms, windows });
        }
        let Some(s) = self.sessions.get_mut(key) else { return false };
        // A call whose output is in the same batch was answered at once: it never waits.
        let answered: HashSet<String> = records
            .iter()
            .filter_map(|r| match r {
                RolloutRecord::CallDone { call_id } => Some(call_id.clone()),
                _ => None,
            })
            .collect();
        // An inferred wait started in this batch; its alert waits for the batch's end.
        let mut asked = false;
        // A user turn whose start was read here ended before any hook of the session arrived.
        let mut unhooked_end = false;
        for record in records {
            match &record {
                RolloutRecord::TaskStarted { .. } => {
                    s.turn_unhooked = true;
                    s.turn_user = false;
                }
                RolloutRecord::UserMessage => s.turn_user = true,
                RolloutRecord::TaskComplete { .. } | RolloutRecord::TurnAborted { .. } if s.turn_unhooked => {
                    unhooked_end |= s.turn_user;
                    s.turn_unhooked = false;
                    s.turn_user = false;
                }
                _ => {}
            }
            match record {
                RolloutRecord::TaskStarted { turn_id } => {
                    let turn = turn_id.or_else(|| s.current_turn.clone());
                    if turn != s.current_turn {
                        start_turn(s, turn, now_ms);
                        set_codex_phase(s, Phase::Working, now_ms);
                    }
                }
                RolloutRecord::TaskComplete { turn_id, last_agent_message, error } => {
                    let turn = turn_id.or_else(|| s.current_turn.clone());
                    if s.current_turn.is_some() && turn != s.current_turn {
                        continue;
                    }
                    s.current_turn = turn.clone();
                    let closed = turn.as_ref().is_some_and(|t| s.closed_turns.contains(t));
                    if !closed {
                        set_codex_phase(s, Phase::Idle, now_ms);
                        raised.push(match error.as_deref() {
                            Some(e) => fail(s, e, now_ms),
                            None => finish(s, last_agent_message.as_deref(), now_ms),
                        });
                        close_turn(s, turn);
                    } else if let (Outcome::Finished { .. }, Some(e)) = (&s.outcome, error.as_deref()) {
                        // Stop came first; the rollout says the turn failed (spec §7)
                        raised.push(fail(s, e, now_ms));
                    }
                }
                RolloutRecord::TurnAborted { turn_id } => {
                    let turn = turn_id.or_else(|| s.current_turn.clone());
                    if turn == s.current_turn {
                        set_codex_phase(s, Phase::Idle, now_ms);
                        s.outcome = Outcome::Interrupted { at_ms: now_ms };
                    }
                }
                RolloutRecord::Call { label, .. } => s.activity = Some(label),
                RolloutRecord::PermissionAsk { call_id, label } => {
                    s.activity = Some(label);
                    // Waiting is inferred from a working session only; another ask joins a wait that was
                    // inferred (spec §7).
                    if !answered.contains(&call_id) {
                        if s.phase == Phase::Working {
                            asked = true;
                            wait_for_permission(s, call_id, now_ms);
                        } else if waiting_by_inference(s) {
                            remember_permission_call(s, call_id);
                        }
                    }
                }
                RolloutRecord::CallDone { call_id } => permission_answered(s, &call_id, now_ms),
                RolloutRecord::Tokens { context: Some(fill), .. } => s.context = Some(fill),
                RolloutRecord::Tokens { context: None, .. }
                | RolloutRecord::Meta { .. }
                | RolloutRecord::UserMessage
                | RolloutRecord::Other
                | RolloutRecord::Unknown => {}
            }
        }
        if asked {
            raise_inferred_wait(s, raised);
        }
        if !unhooked_end {
            return false;
        }
        let was_alive = s.silent_mark.is_some_and(|(at_ms, _)| is_alive(at_ms, now_ms));
        s.silent_mark = Some((now_ms, folder_exists));
        !was_alive
    }

    /// Any Codex hook proves delivery: `receiving`, hosts appended once (D24).
    fn note_receiving(&mut self, host: HostKind, relay_ts: i64) {
        match &mut self.codex_hooks {
            CodexHooksView::Receiving { hosts, .. } => {
                if !hosts.contains(&host) {
                    hosts.push(host);
                }
            }
            other => *other = CodexHooksView::Receiving { hosts: vec![host], since: relay_ts },
        }
    }

    /// A turn that ended without a hook: a muted row, unless a hook already made this a real
    /// session. True when the row is new (not alive before): the island should wake once.
    fn codex_silent(
        &mut self,
        id: String,
        project: ProjectRef,
        host: HostKind,
        folder_exists: bool,
        written_ms: i64,
        now_ms: i64,
    ) -> bool {
        if self.sessions.contains_key(&codex_key(&id)) {
            return false;
        }
        let was_alive = self.silent.get(&id).is_some_and(|old| is_alive(old.last_write_ms, now_ms));
        let last_write_ms = self.silent.get(&id).map_or(written_ms, |old| old.last_write_ms.max(written_ms));
        self.silent.insert(id, SilentRow { project, host, folder_exists, last_write_ms });
        while self.silent.len() > MAX_SILENT {
            let Some(oldest) = self.silent.iter().min_by_key(|(_, r)| r.last_write_ms).map(|(id, _)| id.clone()) else { break };
            self.silent.remove(&oldest);
        }
        !was_alive
    }

    fn codex_hooks_installed(&mut self, installed: bool) {
        if !installed {
            self.codex_hooks = CodexHooksView::NotInstalled;
        } else if !matches!(self.codex_hooks, CodexHooksView::Receiving { .. }) {
            self.codex_hooks = CodexHooksView::InstalledUnseen;
        }
    }
}

// ── Antigravity (observe/antigravity.rs) ──

/// Idle Antigravity sessions kept; past it the one idle the longest goes. Antigravity never says a
/// conversation is over, and without a proved process nothing else would ever remove its rows.
const MAX_IDLE_ANTIGRAVITY: usize = 8;

impl Store {
    /// PreInvocation (the agent calls its model) and PostToolUse make the session Working, a new
    /// turn when it was not; PostToolUse names the tool as the activity. Stop ends the turn:
    /// Idle and Finished, or Error with the `error` Antigravity gave, once per turn. Antigravity
    /// asks for permissions in its own UI and no hook says so: the session never shows Waiting.
    /// Each event comes from its own relay process, so one older than the end of the turn
    /// (relay clock) does not reopen it. The session stays listed after its turn, like a Claude or
    /// a Codex one with a process: it goes when that Antigravity process exits (`ProcExit`), or
    /// when more than `MAX_IDLE_ANTIGRAVITY` are idle.
    #[allow(clippy::too_many_arguments)]
    fn antigravity_hook(
        &mut self,
        key: SessionKey,
        ev: AntigravityEvent,
        project: ProjectRef,
        proc: Option<ProcId>,
        host: HostKind,
        now_ms: i64,
        fx: &mut Vec<Effect>,
        raised: &mut Vec<Raised>,
    ) {
        let s = self.sessions.entry(key.clone()).or_insert_with(|| {
            Session::blank(
                key.clone(),
                Source::Antigravity,
                host,
                proc.clone(),
                project.clone(),
                ev.session_id.clone(),
                ev.cwd.clone(),
                ev.relay_ts,
            )
        });
        if host != HostKind::Unknown {
            s.host = host;
        }
        if let Some(p) = proc {
            s.proc = Some(p);
        }
        // A conversation without a workspace keeps the project it had.
        if !ev.cwd.is_empty() || s.cwd.is_empty() {
            s.cwd = ev.cwd.clone();
            s.project = project;
        }
        let ts = ev.relay_ts;
        match ev.hook_event_name.as_str() {
            "PreInvocation" | "PostToolUse" => {
                if s.phase == Phase::Idle && ts < s.since_ms {
                    return;
                }
                if s.phase != Phase::Working {
                    start_turn(s, None, ts);
                    set_codex_phase(s, Phase::Working, ts);
                }
                if let Some(tool) = ev.tool_name.as_deref().filter(|t| !t.is_empty()) {
                    s.activity = Some(text::tool_label(tool, ev.tool_input.as_ref()));
                }
            }
            "Stop" => {
                if s.phase != Phase::Idle {
                    set_codex_phase(s, Phase::Idle, ts);
                    raised.push(match ev.error.as_deref().filter(|e| !e.trim().is_empty()) {
                        Some(error) => fail(s, error, now_ms),
                        None => finish(s, ev.last_assistant_message.as_deref(), now_ms),
                    });
                }
            }
            _ => {}
        }
        self.trim_idle_antigravity(fx);
    }

    /// Past `MAX_IDLE_ANTIGRAVITY` idle Antigravity sessions, the ones idle the longest go.
    fn trim_idle_antigravity(&mut self, fx: &mut Vec<Effect>) {
        let mut idle: Vec<(i64, SessionKey)> = self
            .sessions
            .values()
            .filter(|s| s.agent == Source::Antigravity && s.phase == Phase::Idle)
            .map(|s| (last_activity(s), s.key.clone()))
            .collect();
        if idle.len() <= MAX_IDLE_ANTIGRAVITY {
            return;
        }
        idle.sort();
        let extra = idle.len() - MAX_IDLE_ANTIGRAVITY;
        for (_, key) in idle.into_iter().take(extra) {
            self.remove_session(&key, fx);
        }
    }
}

// ── Codex ticker (spec §4, §5.6) ──

impl Store {
    /// want = some Codex session is Working or Waiting, whether or not the island is visible (spec §4:
    /// the one timer that runs hidden, because the call and the answer of a `request_permissions`
    /// dialog exist only in the rollout); emits CodexTicker on change only.
    fn update_ticker(&mut self, fx: &mut Vec<Effect>) {
        let want = self
            .sessions
            .values()
            .any(|s| s.agent == Source::Codex && matches!(s.phase, Phase::Working | Phase::Waiting { .. }));
        if want != self.ticker_on {
            self.ticker_on = want;
            fx.push(Effect::CodexTicker(want));
        }
    }
}
