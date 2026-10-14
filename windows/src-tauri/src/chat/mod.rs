//! The island's chat, run by the user's own agent CLIs (Claude Code, Codex, Gemini CLI, Cursor
//! Agent) instead of an API key: Mayak starts the CLI headless in the project folder and uses the
//! login of the CLI itself. Nothing here reads or stores a credential.
//!
//! * `agent.rs` — the `Adapter` trait and the types adapters and the manager speak in.
//! * `claude_code.rs`, `codex.rs` — checked against the real CLIs (their real output is in
//!   `fixtures/`). `gemini.rs`, `cursor.rs` — written from the official documentation only, marked
//!   unverified in their headers and in the `Verification` the island shows.
//! * `convo.rs` — the conversation the island shows; `runner.rs` — process, pipes, probes;
//!   `mcp.rs` — the hook point for the Mayak MCP server.
//! * this file — `ChatManager`: conversations, runs, answers, stopping.
//!
//! How a conversation runs: `send` starts the CLI (Claude Code: one process that stays between
//! turns and is ended after `IDLE` of rest; the others: one process per turn, resumed by the CLI's
//! own conversation id). A reader thread turns each stdout line into events (`Ev`), the
//! conversation applies them, and the island is told to read it again (coalesced). A permission
//! request from the CLI is shown in the island and answered by `answer_permission`, which only a
//! click on its button calls: nothing here answers, times out or defaults. Stopping asks the CLI to
//! interrupt (Claude Code) and ends its process group after a short grace; the group also ends with
//! Mayak.
//!
//! The sessions list needs nothing from here: Claude Code writes its own `~/.claude/sessions` file
//! while it runs and Codex fires Mayak's hooks, so the observers list both like any session.

mod agent;
mod claude_code;
mod codex;
mod convo;
mod cursor;
mod gemini;
mod mcp;
mod runner;
mod saved;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::ChildStdin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Once, PoisonError, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub use agent::{AgentKind, ChatError, Login, ModelChoice};
pub use convo::{Convo, State};
pub use mcp::{McpAttach, McpProvider};
pub use saved::FILE_NAME as SAVED_FILE;

use agent::{Adapter, Ev, Outcome, PermissionAsk, TurnSpec};
use runner::{Line, Tail};

/// A persistent process with no turn open ends this long after its last turn.
const IDLE: Duration = Duration::from_secs(10 * 60);
/// How long a CLI gets to end an interrupted turn before its process group is ended.
const GRACE: Duration = Duration::from_secs(3);
/// The island is told of changes no more often than this.
const COALESCE: Duration = Duration::from_millis(100);
/// How long the answer of the agent probes is reused.
const AGENTS_TTL: Duration = Duration::from_secs(30);
/// The longest message sent to a CLI, in characters.
const MAX_PROMPT: usize = 100_000;

/// What the app gives the chat; tests give a fake.
pub trait Host: Send + Sync + 'static {
    /// These conversations changed; the island reads them again. Called off the UI thread, at most
    /// every `COALESCE`.
    fn changed(&self, ids: &[u64]);
    /// A request waits for a click: the island opens on the chat.
    fn permission_asked(&self, conv: u64, request_id: &str, tool: &str);
    /// The CLI's absolute path.
    fn find_cli(&self, stem: &str) -> Option<PathBuf> {
        crate::platform::find_on_path(stem)
    }
    /// Where the conversations are kept across restarts (saved.rs); None keeps nothing.
    fn saved_path(&self) -> Option<PathBuf> {
        None
    }
}

/// What the island's picker shows for one CLI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub kind: AgentKind,
    pub label: &'static str,
    /// The CLI is on PATH.
    pub found: bool,
    /// False: written from the documentation, not run (the picker says so).
    pub verified: bool,
    pub verified_with: Option<&'static str>,
    pub models: Vec<ModelChoice>,
    /// The ids of the modes the user may pick; the first is the default.
    pub modes: Vec<&'static str>,
    pub login: Login,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: u64,
    pub agent: AgentKind,
    pub cwd: String,
    pub title: String,
    pub state: State,
    pub rev: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRequest {
    pub agent: AgentKind,
    pub cwd: String,
    #[serde(default)]
    pub model: String,
    /// "" is the adapter's default mode.
    #[serde(default)]
    pub mode: String,
    /// The CLI's id of an existing conversation to continue.
    #[serde(default)]
    pub resume: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Runs ───────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct RunState {
    /// A turn is open: the user has sent and the CLI has not ended it.
    turn_open: bool,
    ended: bool,
    last_turn_end: Option<Instant>,
}

/// One CLI process of a conversation.
struct Run {
    id: u64,
    group: Arc<runner::ProcGroup>,
    stdin: Mutex<Option<ChildStdin>>,
    /// The user stopped the turn.
    cancel: AtomicBool,
    /// No new turn goes to this process (it is ending, or its model or mode changed).
    retired: AtomicBool,
    /// What the MCP server handed this process when it started. A process keeps it for its whole
    /// life, so a turn that would be given something else (the switch went on or off, a new token)
    /// starts a new process instead.
    mcp: Option<McpAttach>,
    state: Mutex<RunState>,
    cv: Condvar,
}

impl Run {
    fn usable(&self) -> bool {
        !self.retired.load(Ordering::SeqCst) && !lock(&self.state).ended
    }

    /// The user's next turn goes to this process: false when it is ending or gone.
    fn claim_turn(&self) -> bool {
        let mut s = lock(&self.state);
        if s.ended || self.retired.load(Ordering::SeqCst) {
            return false;
        }
        s.turn_open = true;
        true
    }

    fn write(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut guard = lock(&self.stdin);
        match guard.as_mut() {
            Some(stdin) => {
                stdin.write_all(bytes)?;
                stdin.flush()
            }
            None => Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stdin is closed")),
        }
    }

    /// Ends the process by closing its stdin: the CLI finishes and exits.
    fn retire(&self) {
        self.retired.store(true, Ordering::SeqCst);
        lock(&self.stdin).take();
        self.cv.notify_all();
    }

    fn set_turn(&self, open: bool) {
        let mut s = lock(&self.state);
        s.turn_open = open;
        if !open {
            s.last_turn_end = Some(Instant::now());
        }
        drop(s);
        self.cv.notify_all();
    }

    fn mark_ended(&self) {
        let mut s = lock(&self.state);
        s.ended = true;
        s.turn_open = false;
        drop(s);
        self.cv.notify_all();
    }
}

struct Entry {
    convo: Convo,
    run: Option<Arc<Run>>,
}

struct Pending {
    conv: u64,
    run: u64,
    ask: PermissionAsk,
}

#[derive(Default)]
struct Store {
    convos: BTreeMap<u64, Entry>,
    asks: HashMap<String, Pending>,
}

// ── Telling the island ─────────────────────────────────────────────────────────

struct NotifierShared {
    dirty: Mutex<BTreeSet<u64>>,
    cv: Condvar,
    closed: AtomicBool,
}

/// One thread that blocks until something changed, waits `COALESCE` for more, and tells the host
/// once. It starts with the first change, so a chat nobody opens has no thread; idle, it is parked
/// on a condvar: no timer runs.
struct Notifier {
    shared: Arc<NotifierShared>,
    host: Arc<dyn Host>,
    coalesce: Duration,
    started: Once,
}

impl Notifier {
    fn new(host: Arc<dyn Host>, coalesce: Duration) -> Self {
        let shared = Arc::new(NotifierShared { dirty: Mutex::new(BTreeSet::new()), cv: Condvar::new(), closed: AtomicBool::new(false) });
        Notifier { shared, host, coalesce, started: Once::new() }
    }

    fn start_thread(&self) {
        let (shared, host, coalesce) = (self.shared.clone(), self.host.clone(), self.coalesce);
        let spawned = std::thread::Builder::new().name("chat-notify".into()).spawn(move || loop {
            let mut dirty = lock(&shared.dirty);
            while dirty.is_empty() && !shared.closed.load(Ordering::SeqCst) {
                dirty = shared.cv.wait(dirty).unwrap_or_else(PoisonError::into_inner);
            }
            if shared.closed.load(Ordering::SeqCst) {
                return;
            }
            drop(dirty);
            std::thread::sleep(coalesce);
            let ids: Vec<u64> = std::mem::take(&mut *lock(&shared.dirty)).into_iter().collect();
            if !ids.is_empty() {
                host.changed(&ids);
            }
        });
        if let Err(err) = spawned {
            crate::log::line(format!("chat: notifier thread did not start: {:?}", err.kind()));
        }
    }

    fn touch(&self, conv: u64) {
        self.started.call_once(|| self.start_thread());
        lock(&self.shared.dirty).insert(conv);
        self.shared.cv.notify_one();
    }
}

impl Drop for Notifier {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        self.shared.cv.notify_all();
    }
}

// ── Keeping the conversations ────────────────────────────────────────────────────

/// How long the saver waits after a change before it writes, so a burst of changes is one write.
const SAVE_DELAY: Duration = Duration::from_millis(500);

/// One thread that writes the conversations' file after a change that matters (a turn begins or
/// ends, a conversation opens, closes or changes its model or mode): never for each streamed word.
/// It starts with the first such change; idle, it is parked on a condvar.
struct Saver {
    shared: Arc<NotifierShared>,
    manager: Weak<Inner>,
    started: Once,
}

impl Saver {
    fn new(manager: Weak<Inner>) -> Self {
        let shared = Arc::new(NotifierShared { dirty: Mutex::new(BTreeSet::new()), cv: Condvar::new(), closed: AtomicBool::new(false) });
        Saver { shared, manager, started: Once::new() }
    }

    fn start_thread(&self) {
        let (shared, manager) = (self.shared.clone(), self.manager.clone());
        let spawned = std::thread::Builder::new().name("chat-save".into()).spawn(move || loop {
            let mut dirty = lock(&shared.dirty);
            while dirty.is_empty() && !shared.closed.load(Ordering::SeqCst) {
                dirty = shared.cv.wait(dirty).unwrap_or_else(PoisonError::into_inner);
            }
            if shared.closed.load(Ordering::SeqCst) {
                return;
            }
            drop(dirty);
            std::thread::sleep(SAVE_DELAY);
            lock(&shared.dirty).clear();
            let Some(inner) = manager.upgrade() else { return };
            ChatManager(inner).save_now();
        });
        if let Err(err) = spawned {
            crate::log::line(format!("chat: saver thread did not start: {:?}", err.kind()));
        }
    }

    fn mark(&self) {
        self.started.call_once(|| self.start_thread());
        lock(&self.shared.dirty).insert(0);
        self.shared.cv.notify_one();
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        self.shared.cv.notify_all();
    }
}

// ── The manager ────────────────────────────────────────────────────────────────

struct Inner {
    host: Arc<dyn Host>,
    store: Mutex<Store>,
    next_conv: AtomicU64,
    next_run: AtomicU64,
    mcp: Mutex<Option<McpProvider>>,
    notifier: Notifier,
    saver: Saver,
    idle: Duration,
    grace: Duration,
    agents_cache: Mutex<Option<(Instant, Vec<AgentInfo>)>>,
}

#[derive(Clone)]
pub struct ChatManager(Arc<Inner>);

impl ChatManager {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self::with_timing(host, IDLE, GRACE, COALESCE)
    }

    fn with_timing(host: Arc<dyn Host>, idle: Duration, grace: Duration, coalesce: Duration) -> Self {
        let manager = ChatManager(Arc::new_cyclic(|weak| Inner {
            notifier: Notifier::new(host.clone(), coalesce),
            saver: Saver::new(weak.clone()),
            host,
            store: Mutex::new(Store::default()),
            next_conv: AtomicU64::new(1),
            next_run: AtomicU64::new(1),
            mcp: Mutex::new(None),
            idle,
            grace,
            agents_cache: Mutex::new(None),
        }));
        manager.load();
        manager
    }

    /// The conversations kept from the last run of the app, oldest first so the newest gets the
    /// highest id (the list shows the newest first). A file that cannot be read is said in the log
    /// and replaced by the next save.
    fn load(&self) {
        let Some(path) = self.0.host.saved_path() else { return };
        let saved = match saved::read(&path) {
            Ok(saved) => saved,
            Err(err) => {
                crate::log::line(format!("chat: saved chats not read: {err}"));
                return;
            }
        };
        let mut store = lock(&self.0.store);
        for s in saved.iter().rev() {
            let id = self.0.next_conv.fetch_add(1, Ordering::SeqCst);
            store.convos.insert(id, Entry { convo: Convo::restored(id, s), run: None });
        }
        if !saved.is_empty() {
            crate::log::line(format!("chat: {} saved chats restored", saved.len()));
        }
    }

    /// Writes the conversations' file now (the saver thread, and the app quitting).
    fn save_now(&self) {
        let Some(path) = self.0.host.saved_path() else { return };
        // The newest first: what the file keeps when it has to drop some.
        let list: Vec<saved::Saved> = lock(&self.0.store).convos.values().rev().filter_map(|e| saved::of(&e.convo)).collect();
        // Nothing to keep: no file at all, rather than an empty one.
        let done = if list.is_empty() { saved::remove(&path) } else { saved::write(&path, list) };
        if let Err(err) = done {
            crate::log::line(format!("chat: saved chats not written: {:?}", err.kind()));
        }
    }

    /// Something worth keeping changed: the file is written shortly.
    fn persist(&self) {
        if self.0.host.saved_path().is_some() {
            self.0.saver.mark();
        }
    }

    /// Settings > Clear saved chats: every conversation that is not running a turn goes, and the file
    /// with them. A running one stays (it is kept again when its turn ends). The number removed.
    pub fn clear_saved(&self) -> usize {
        let (removed, runs): (Vec<u64>, Vec<Arc<Run>>) = {
            let mut store = lock(&self.0.store);
            let ids: Vec<u64> = store.convos.iter().filter(|(_, e)| !matches!(e.convo.state, State::Working | State::Waiting)).map(|(id, _)| *id).collect();
            let runs = ids.iter().filter_map(|id| store.convos.remove(id).and_then(|e| e.run)).collect();
            store.asks.retain(|_, p| !ids.contains(&p.conv));
            (ids, runs)
        };
        for run in runs {
            run.retire();
            run.group.kill();
        }
        if let Some(path) = self.0.host.saved_path() {
            if let Err(err) = saved::remove(&path) {
                crate::log::line(format!("chat: saved chats not cleared: {:?}", err.kind()));
            }
        }
        // What still runs is written again at once, so the file never lacks it (with nothing left,
        // the save leaves no file).
        self.persist();
        if !removed.is_empty() {
            self.0.host.changed(&removed);
        }
        removed.len()
    }

    /// The conversations kept on disk right now (Settings shows the number).
    pub fn saved_count(&self) -> usize {
        lock(&self.0.store).convos.values().filter(|e| !e.convo.msgs.is_empty()).count().min(saved::MAX_CONVOS)
    }

    /// Some conversation has a CLI session id (the session list has rows to mark).
    pub fn has_sessions(&self) -> bool {
        lock(&self.0.store).convos.values().any(|e| e.convo.session.is_some())
    }

    /// The conversation whose CLI session this is (Claude Code's session id, Codex's thread id), live or
    /// not: the session list opens it in the Agents chat.
    pub fn conv_for_session(&self, session_id: &str) -> Option<u64> {
        if session_id.is_empty() {
            return None;
        }
        lock(&self.0.store).convos.values().rev().find(|e| e.convo.session.as_deref() == Some(session_id)).map(|e| e.convo.id)
    }

    /// The hook for the Mayak MCP server: what the next turns hand over to the CLI they start.
    /// Asked on every turn that starts a process, so a server that comes up later is picked up.
    pub fn set_mcp_provider(&self, provider: Option<McpProvider>) {
        *lock(&self.0.mcp) = provider;
    }

    /// What the server hands a CLI of this kind right now.
    fn mcp_attach(&self, kind: AgentKind) -> Option<McpAttach> {
        let provider = lock(&self.0.mcp).clone();
        provider.and_then(|p| p(kind))
    }

    /// Whether a live Claude Code run of a conversation here has this session id: Mayak's
    /// permission hook lets the CLI go on at once for it, so the chat is the only place it asks.
    pub fn owns_session(&self, session_id: &str) -> bool {
        if session_id.is_empty() {
            return false;
        }
        let store = lock(&self.0.store);
        store.convos.values().any(|e| {
            e.convo.agent == AgentKind::ClaudeCode
                && e.convo.session.as_deref() == Some(session_id)
                && e.run.as_ref().is_some_and(|r| r.usable())
        })
    }

    // ── The picker ──

    /// The CLIs, found or not, with the login state and model list the CLIs can tell. Runs short
    /// commands: call it off the UI thread. The answer is reused for `AGENTS_TTL`.
    pub fn agents(&self, refresh: bool) -> Vec<AgentInfo> {
        if let Some((at, list)) = lock(&self.0.agents_cache).as_ref().filter(|_| !refresh) {
            if at.elapsed() < AGENTS_TTL {
                return list.clone();
            }
        }
        let list = self.probe_agents();
        *lock(&self.0.agents_cache) = Some((Instant::now(), list.clone()));
        list
    }

    fn probe_agents(&self) -> Vec<AgentInfo> {
        let host = self.0.host.clone();
        let cwd = std::env::temp_dir();
        let threads: Vec<_> = AgentKind::ALL
            .iter()
            .map(|kind| {
                let host = host.clone();
                let cwd = cwd.clone();
                let kind = *kind;
                std::thread::spawn(move || probe_agent(&*host, kind, &cwd))
            })
            .collect();
        threads.into_iter().zip(AgentKind::ALL).map(|(t, kind)| t.join().unwrap_or_else(|_| agent_info(kind, false, Login::Unknown, None))).collect()
    }

    // ── Conversations ──

    pub fn open(&self, req: OpenRequest) -> Result<u64, ChatError> {
        let adapter = req.agent.adapter();
        // No folder chosen: the user's home folder, where a terminal would open too.
        let folder = if req.cwd.trim().is_empty() { crate::platform::home_dir().to_string_lossy().into_owned() } else { req.cwd.clone() };
        let cwd = Path::new(&folder);
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(ChatError::new("badFolder", "That folder does not exist."));
        }
        agent::check_model(&req.model)?;
        let mode = if req.mode.is_empty() { adapter.modes()[0].to_string() } else { req.mode.clone() };
        if !adapter.modes().contains(&mode.as_str()) {
            return Err(agent::bad_mode(&mode));
        }
        if let Some(id) = &req.resume {
            agent::check_resume(id)?;
        }
        if self.0.host.find_cli(adapter.stem()).is_none() {
            return Err(cli_missing(adapter));
        }
        let id = self.0.next_conv.fetch_add(1, Ordering::SeqCst);
        let convo = Convo::new(id, req.agent, folder.clone(), req.model.clone(), mode, req.resume.clone());
        lock(&self.0.store).convos.insert(id, Entry { convo, run: None });
        self.0.notifier.touch(id);
        // Nothing to keep yet: a conversation is written once something was said in it.
        Ok(id)
    }

    pub fn list(&self) -> Vec<Summary> {
        let store = lock(&self.0.store);
        store
            .convos
            .values()
            .rev()
            .map(|e| Summary {
                id: e.convo.id,
                agent: e.convo.agent,
                cwd: e.convo.cwd.clone(),
                title: e.convo.title.clone(),
                state: e.convo.state,
                rev: e.convo.rev,
            })
            .collect()
    }

    /// The conversation as the island shows it; None when it is gone, or when `since` is its
    /// current revision (nothing new).
    pub fn get(&self, conv: u64, since: Option<u64>) -> Option<Convo> {
        let store = lock(&self.0.store);
        let convo = &store.convos.get(&conv)?.convo;
        (since != Some(convo.rev)).then(|| convo.clone())
    }

    /// Changes the model and mode of the next turns. Only while nothing is running.
    pub fn set_choice(&self, conv: u64, model: String, mode: String) -> Result<(), ChatError> {
        agent::check_model(&model)?;
        let mut store = lock(&self.0.store);
        let entry = store.convos.get_mut(&conv).ok_or_else(unknown)?;
        let adapter = entry.convo.agent.adapter();
        if !adapter.modes().contains(&mode.as_str()) {
            return Err(agent::bad_mode(&mode));
        }
        if matches!(entry.convo.state, State::Working | State::Waiting) {
            return Err(ChatError::new("busy", "Wait for the answer, or stop it first."));
        }
        entry.convo.set_choice(model, mode);
        // A persistent process was started with the old flags: the next turn starts a new one.
        if let Some(run) = &entry.run {
            run.retire();
        }
        drop(store);
        self.0.notifier.touch(conv);
        self.persist();
        Ok(())
    }

    pub fn send(&self, conv: u64, text: &str) -> Result<(), ChatError> {
        let text = text.trim();
        if text.is_empty() || text.chars().count() > MAX_PROMPT {
            return Err(ChatError::new("badPrompt", "That message is empty or too long."));
        }
        // What the turn is built from, read once.
        let (kind, cwd, model, mode, session, live) = {
            let store = lock(&self.0.store);
            let entry = store.convos.get(&conv).ok_or_else(unknown)?;
            if matches!(entry.convo.state, State::Working | State::Waiting) {
                return Err(ChatError::new("busy", "The agent is still working."));
            }
            (
                entry.convo.agent,
                PathBuf::from(&entry.convo.cwd),
                entry.convo.model.clone(),
                entry.convo.mode.clone(),
                entry.convo.session.clone(),
                entry.run.clone(),
            )
        };

        let adapter = kind.adapter();
        // A process that stays between turns takes the next message on its stdin. Claiming it and the
        // idle watcher's decision to end it are one step each under the run's lock, so a process
        // claimed here is not ended meanwhile.
        if let (Some(run), true) = (&live, adapter.persistent()) {
            if run.mcp != self.mcp_attach(kind) {
                // The MCP server was switched, or its token changed, since this process started.
                run.retire();
            } else if run.claim_turn() {
                let Some(line) = adapter.user_line(text) else {
                    run.set_turn(false);
                    return Err(ChatError::new("spawnFailed", "This CLI does not take a second message."));
                };
                if let Err(err) = self.begin_turn(conv, text) {
                    run.set_turn(false);
                    return Err(err);
                }
                match run.write(line.as_bytes()) {
                    Ok(()) => return Ok(()),
                    Err(err) => {
                        // The process went between the claim and the write: a new one takes the turn.
                        crate::log::line(format!("chat: run {} took no message ({:?}), starting a new process", run.id, err.kind()));
                        run.retire();
                        return self.spawn_turn(conv, kind, text, &cwd, &model, &mode, session.as_deref(), true);
                    }
                }
            }
        }
        self.spawn_turn(conv, kind, text, &cwd, &model, &mode, session.as_deref(), false)
    }

    /// Starts a process for the turn. `begun`: the user's message is already in the conversation, so
    /// a refusal ends the turn there instead of leaving the message unanswered; otherwise everything
    /// that can be refused is refused before the conversation changes.
    #[allow(clippy::too_many_arguments)]
    fn spawn_turn(
        &self,
        conv: u64,
        kind: AgentKind,
        text: &str,
        cwd: &Path,
        model: &str,
        mode: &str,
        session: Option<&str>,
        begun: bool,
    ) -> Result<(), ChatError> {
        let adapter = kind.adapter();
        let attach = self.mcp_attach(kind);
        let prepared = self
            .0
            .host
            .find_cli(adapter.stem())
            .ok_or_else(|| cli_missing(adapter))
            .and_then(|bin| adapter.build(&TurnSpec { cwd, prompt: text, model, mode, resume: session, mcp: attach.as_ref() }).map(|inv| (bin, inv)));
        let (bin, invocation) = match prepared {
            Ok(ready) => ready,
            Err(err) => {
                if begun {
                    self.fail_turn(conv, &err.message);
                }
                return Err(err);
            }
        };
        if let agent::McpApply::Unsupported(why) = &invocation.mcp {
            crate::log::line(format!("chat: mcp not passed to {}: {why}", adapter.stem()));
        }
        if !begun {
            self.begin_turn(conv, text)?;
        }
        let started = match runner::spawn(&bin, &invocation.args, &invocation.env, cwd) {
            Ok(s) => s,
            Err(err) => {
                self.fail_turn(conv, &err.message);
                return Err(err);
            }
        };
        self.start_run(conv, adapter, started, invocation, attach)
    }

    /// The user's message enters the conversation and the turn begins (checked again under the lock).
    fn begin_turn(&self, conv: u64, text: &str) -> Result<(), ChatError> {
        let mut store = lock(&self.0.store);
        let entry = store.convos.get_mut(&conv).ok_or_else(unknown)?;
        if matches!(entry.convo.state, State::Working | State::Waiting) {
            return Err(ChatError::new("busy", "The agent is still working."));
        }
        entry.convo.user(text);
        entry.convo.clock(now_ms());
        drop(store);
        self.0.notifier.touch(conv);
        self.persist();
        Ok(())
    }

    /// The turn could not start or send: the conversation shows why and is free again.
    fn fail_turn(&self, conv: u64, why: &str) {
        let mut store = lock(&self.0.store);
        if let Some(entry) = store.convos.get_mut(&conv) {
            entry.convo.apply(&Ev::TurnEnd(Outcome::Failed { text: why.to_string(), login: false }));
            entry.convo.clock(now_ms());
        }
        drop(store);
        self.0.notifier.touch(conv);
        self.persist();
    }

    fn start_run(
        &self,
        conv: u64,
        adapter: &'static dyn Adapter,
        started: runner::Started,
        inv: agent::Invocation,
        mcp: Option<McpAttach>,
    ) -> Result<(), ChatError> {
        let runner::Started { mut child, group } = started;
        let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let run = Arc::new(Run {
            id: self.0.next_run.fetch_add(1, Ordering::SeqCst),
            group,
            stdin: Mutex::new(stdin),
            cancel: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            mcp,
            state: Mutex::new(RunState { turn_open: true, ended: false, last_turn_end: None }),
            cv: Condvar::new(),
        });
        // Installed before the reader starts, so its first events find their run.
        {
            let mut store = lock(&self.0.store);
            match store.convos.get_mut(&conv) {
                Some(entry) => entry.run = Some(run.clone()),
                None => {
                    run.group.kill();
                    return Err(unknown());
                }
            }
        }
        crate::log::line(format!("chat: run {} started conv={conv} agent={}", run.id, adapter.stem()));

        let tail = Tail::default();
        if let Some(stderr) = stderr {
            let tail = tail.clone();
            std::thread::Builder::new().name("chat-stderr".into()).spawn(move || runner::drain_into(stderr, &tail)).ok();
        }
        if let Some(stdout) = stdout {
            let manager = self.clone();
            let reader_run = run.clone();
            let spawned = std::thread::Builder::new()
                .name("chat-read".into())
                .spawn(move || manager.read_loop(conv, adapter, reader_run, child, stdout, tail));
            if let Err(err) = spawned {
                run.group.kill();
                self.fail_turn(conv, &format!("Could not start reading the CLI ({:?}).", err.kind()));
                return Err(ChatError::new("spawnFailed", "Could not start reading the CLI."));
            }
        }

        if !inv.stdin.is_empty() {
            if let Err(err) = run.write(&inv.stdin) {
                // The process is gone or going; the reader reports its exit with what it said on stderr.
                crate::log::line(format!("chat: run {} took no message ({:?})", run.id, err.kind()));
                run.group.kill();
            }
        }
        if inv.close_stdin {
            lock(&run.stdin).take();
        }
        if adapter.persistent() {
            let manager = self.clone();
            let run = run.clone();
            std::thread::Builder::new().name("chat-idle".into()).spawn(move || manager.idle_watch(&run)).ok();
        }
        Ok(())
    }

    /// Reads one run's stdout to its end, then reports how the process ended.
    fn read_loop(&self, conv: u64, adapter: &'static dyn Adapter, run: Arc<Run>, mut child: std::process::Child, stdout: std::process::ChildStdout, tail: Tail) {
        let mut parser = adapter.parser();
        let mut reader = BufReader::new(stdout);
        loop {
            match runner::read_line(&mut reader, runner::MAX_LINE) {
                Ok(Line::Text(line)) => {
                    for ev in parser.line(&line) {
                        self.handle_event(conv, adapter, &run, ev);
                    }
                }
                Ok(Line::TooLong) => crate::log::line(format!("chat: a line over {} bytes was skipped conv={conv}", runner::MAX_LINE)),
                Ok(Line::Eof) | Err(_) => break,
            }
        }
        let code = child.wait().ok().and_then(|s| s.code());
        self.run_ended(conv, &run, code, &tail);
    }

    fn handle_event(&self, conv: u64, adapter: &'static dyn Adapter, run: &Arc<Run>, ev: Ev) {
        let mut ask_tool: Option<(String, String)> = None;
        let ended = matches!(ev, Ev::TurnEnd(_));
        {
            let mut store = lock(&self.0.store);
            let Some(entry) = store.convos.get_mut(&conv) else { return };
            // Whatever a run that is no longer the conversation's says is not shown.
            if entry.run.as_ref().map(|r| r.id) != Some(run.id) {
                return;
            }
            let applied = entry.convo.apply(&ev);
            entry.convo.clock(now_ms());
            match &ev {
                Ev::Permission(ask) => {
                    if let Some(request_id) = applied.permission {
                        ask_tool = Some((request_id.clone(), ask.tool.clone()));
                        store.asks.insert(request_id, Pending { conv, run: run.id, ask: ask.clone() });
                    }
                }
                Ev::TurnEnd(_) => {
                    store.asks.retain(|_, p| p.conv != conv);
                    // A stop request belongs to the turn it stopped. The run learns the turn is over
                    // under the same lock that shows the conversation idle, so the next message
                    // cannot be claimed before it.
                    run.cancel.store(false, Ordering::SeqCst);
                    run.set_turn(false);
                    if !adapter.persistent() {
                        // A one-shot CLI is done with its process: it ends by itself.
                        run.retire();
                    }
                }
                _ => {}
            }
        }
        self.0.notifier.touch(conv);
        if ended {
            self.persist();
        }
        if let Some((request_id, tool)) = ask_tool {
            self.0.host.permission_asked(conv, &request_id, &tool);
        }
    }

    fn run_ended(&self, conv: u64, run: &Arc<Run>, code: Option<i32>, tail: &Tail) {
        crate::log::line(format!("chat: run {} ended conv={conv} code={code:?}", run.id));
        run.mark_ended();
        let mut store = lock(&self.0.store);
        if let Some(entry) = store.convos.get_mut(&conv) {
            if entry.run.as_ref().map(|r| r.id) == Some(run.id) {
                entry.run = None;
                if run.cancel.load(Ordering::SeqCst) {
                    entry.convo.stopped();
                } else {
                    let why = tail.text(4);
                    let why = if why.is_empty() { format!("The CLI exited (code {}).", code.map_or_else(|| "unknown".to_string(), |c| c.to_string())) } else { why };
                    entry.convo.process_ended(why);
                }
                entry.convo.clock(now_ms());
            }
        }
        store.asks.retain(|_, p| !(p.conv == conv && p.run == run.id));
        drop(store);
        self.0.notifier.touch(conv);
        self.persist();
    }

    /// Ends a persistent process that has had no turn for `idle`: closing its stdin lets the CLI
    /// finish. Parked on a condvar the rest of the time.
    fn idle_watch(&self, run: &Run) {
        let mut state = lock(&run.state);
        loop {
            if state.ended || run.retired.load(Ordering::SeqCst) {
                return;
            }
            if state.turn_open {
                state = run.cv.wait(state).unwrap_or_else(PoisonError::into_inner);
                continue;
            }
            let rested = state.last_turn_end.map_or(Duration::ZERO, |t| t.elapsed());
            if rested >= self.0.idle {
                // Still under the run's lock: a message claims the process under it too (claim_turn).
                run.retire();
                return;
            }
            state = run.cv.wait_timeout(state, self.0.idle - rested).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    // ── Answers ──

    /// The user clicked Allow (true) or Deny (false) on a request. The only call that answers one.
    /// False when the request is not pending any more (answered, or its run ended): a late click
    /// changes nothing.
    pub fn answer_permission(&self, request_id: &str, allow: bool) -> bool {
        let (run, line, conv) = {
            let mut store = lock(&self.0.store);
            let Some(pending) = store.asks.remove(request_id) else { return false };
            let Some(entry) = store.convos.get_mut(&pending.conv) else { return false };
            let Some(run) = entry.run.clone().filter(|r| r.id == pending.run) else {
                entry.convo.cancel_pending();
                return false;
            };
            if !entry.convo.resolve_permission(request_id, allow) {
                return false;
            }
            let line = entry.convo.agent.adapter().permission_reply(&pending.ask, allow);
            (run, line, pending.conv)
        };
        crate::log::line(format!("chat: permission {request_id} {}", if allow { "allowed" } else { "denied" }));
        self.0.notifier.touch(conv);
        let Some(line) = line else { return true };
        if let Err(err) = run.write(line.as_bytes()) {
            crate::log::line(format!("chat: could not answer the CLI: {:?}", err.kind()));
            run.group.kill();
        }
        true
    }

    // ── Stopping ──

    /// Stops the running turn: an interrupt where the CLI has one, then the process group after
    /// `grace`; at once for a CLI that cannot be interrupted.
    pub fn cancel(&self, conv: u64) -> Result<(), ChatError> {
        let (run, interrupt) = {
            let store = lock(&self.0.store);
            let entry = store.convos.get(&conv).ok_or_else(unknown)?;
            if !matches!(entry.convo.state, State::Working | State::Waiting) {
                return Ok(());
            }
            let Some(run) = entry.run.clone() else { return Ok(()) };
            (run, entry.convo.agent.adapter().interrupt_line())
        };
        run.cancel.store(true, Ordering::SeqCst);
        let interrupted = interrupt.is_some_and(|line| run.write(line.as_bytes()).is_ok());
        if !interrupted {
            run.group.kill();
            return Ok(());
        }
        let grace = self.0.grace;
        std::thread::Builder::new()
            .name("chat-stop".into())
            .spawn(move || {
                let state = lock(&run.state);
                let (state, _) = run.cv.wait_timeout_while(state, grace, |s| s.turn_open && !s.ended).unwrap_or_else(PoisonError::into_inner);
                let still_open = state.turn_open && !state.ended;
                drop(state);
                if still_open {
                    crate::log::line(format!("chat: run {} did not stop in time, ended", run.id));
                    run.group.kill();
                }
            })
            .ok();
        Ok(())
    }

    /// Ends the conversation: its process goes, its messages are dropped. The CLI keeps its own history.
    pub fn close(&self, conv: u64) {
        let run = {
            let mut store = lock(&self.0.store);
            store.asks.retain(|_, p| p.conv != conv);
            store.convos.remove(&conv).and_then(|e| e.run)
        };
        if let Some(run) = run {
            run.cancel.store(true, Ordering::SeqCst);
            run.group.kill();
        }
        self.persist();
    }

    /// Ends every process (the app is quitting), after writing the conversations as they are. On
    /// Windows the job objects end the processes too if this is missed.
    pub fn shutdown(&self) {
        self.save_now();
        let runs: Vec<Arc<Run>> = lock(&self.0.store).convos.values().filter_map(|e| e.run.clone()).collect();
        for run in runs {
            run.group.kill();
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // The notifier thread ends with the manager (Notifier::drop); runs that are still alive end with it.
        let store = self.store.get_mut().unwrap_or_else(PoisonError::into_inner);
        for entry in store.convos.values() {
            if let Some(run) = &entry.run {
                run.group.kill();
            }
        }
    }
}

/// The wall clock in ms since the Unix epoch: what the page reads a turn's start and end against.
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn unknown() -> ChatError {
    ChatError::new("unknownConversation", "That conversation is gone.")
}

fn cli_missing(adapter: &dyn Adapter) -> ChatError {
    ChatError::new("cliMissing", format!("{} is not installed (no `{}` on PATH).", adapter.label(), adapter.stem()))
}

fn agent_info(kind: AgentKind, found: bool, login: Login, models: Option<Vec<ModelChoice>>) -> AgentInfo {
    let adapter = kind.adapter();
    let verification = adapter.verification();
    AgentInfo {
        kind,
        label: adapter.label(),
        found,
        verified: verification.verified,
        verified_with: verification.with,
        models: models.unwrap_or_else(|| adapter.models()),
        modes: adapter.modes().to_vec(),
        login,
    }
}

/// Finds one CLI and asks it what it can tell (login, models): short commands, run here.
fn probe_agent(host: &dyn Host, kind: AgentKind, cwd: &Path) -> AgentInfo {
    let adapter = kind.adapter();
    let Some(bin) = host.find_cli(adapter.stem()) else { return agent_info(kind, false, Login::Unknown, None) };
    let login = match adapter.login_probe() {
        Some(probe) => match runner::run_probe(&bin, &probe, cwd) {
            Ok(out) => adapter.parse_login(out.exit, &format!("{}\n{}", out.stdout, out.stderr)),
            Err(_) => Login::Unknown,
        },
        None => Login::Unknown,
    };
    let models = adapter
        .models_probe()
        .and_then(|probe| runner::run_probe(&bin, &probe, cwd).ok())
        .and_then(|out| adapter.parse_models(&out.stdout));
    agent_info(kind, true, login, models)
}

#[cfg(test)]
mod tests;
