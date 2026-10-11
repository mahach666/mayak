// Session observer (spec §4, §9). Three std threads: "claude-io" and "codex-io" turn file
// notifications and Codex (and Antigravity) hooks into Signals; "store" applies them to the pure model
// (model::Store), dispatches the returned effects back to the sources and publishes snapshots.
// Nothing here runs on the tokio runtime.

pub mod antigravity;
pub mod claude;
pub mod claude_transcript;
pub mod claude_usage;
pub mod codex;
pub mod keys;
pub mod model;
pub mod paths;
pub mod silent;
pub mod tail;
pub mod terminal;
pub mod text;
pub mod win;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::log;
use crate::pipe::ApprovalContext;
use claude::{ClaudeConfig, ClaudeInput};
use codex::{CodexConfig, CodexInput};
use model::{Alert, Effect, Health, SessionsSnapshot, Signal, Source, Store, Usage};
use win::proc;

/// Spec §9. `prev` is the last snapshot emitted. Visible: every rev change; hidden: only when
/// counts, attention, hold or aggregate change, or the batch had alerts; Visible(true) always.
pub(crate) fn should_emit(
    prev: Option<&SessionsSnapshot>,
    next: &SessionsSnapshot,
    visible: bool,
    became_visible: bool,
    had_alerts: bool,
) -> bool {
    let Some(prev) = prev else { return true };
    if became_visible {
        return true;
    }
    if visible {
        return next.rev != prev.rev;
    }
    had_alerts
        || next.counts != prev.counts
        || next.attention != prev.attention
        || next.hold != prev.hold
        || next.aggregate != prev.aggregate
}

/// How long after a plan-limit window's reset time its limits are read again: Claude Code may need a
/// moment to cache the new window.
pub(crate) const ROLLOVER_GRACE_MS: i64 = 5_000;

/// The next moment, strictly after `after`, a Claude or Codex limit window has reset
/// (`resets_at` + ROLLOVER_GRACE_MS); None when no window has a reset time to come.
pub(crate) fn next_rollover(usage: &Usage, after: i64) -> Option<i64> {
    let claude = usage.claude.iter().flat_map(|c| c.windows.iter().filter_map(|w| w.resets_at_ms));
    let codex = usage.codex.iter().flat_map(|c| c.windows.iter().filter_map(|w| w.resets_at_ms));
    claude.chain(codex).filter_map(|at| at.checked_add(ROLLOVER_GRACE_MS)).filter(|at| *at > after).min()
}

/// How long the store thread waits for a signal before a window's reset: None (no timer at all)
/// while the island is hidden or no reset is coming, so a hidden island never wakes the thread.
pub(crate) fn rollover_wait(visible: bool, next: Option<i64>, now: i64) -> Option<Duration> {
    let next = next.filter(|_| visible)?;
    Some(Duration::from_millis(u64::try_from(next.saturating_sub(now)).unwrap_or(0)))
}

/// Where the store thread publishes. The app emits to the island window; tests record.
pub trait Publisher: Send + 'static {
    /// "session-alert"
    fn alerts(&self, alerts: &[Alert]);
    /// "sessions"
    fn snapshot(&self, snapshot: &SessionsSnapshot);
    /// "session-silent": a new silent Codex row is in the snapshot just published; the island
    /// shows itself once. No payload.
    fn silent_wake(&self);
    /// "usage-rollover": a plan-limit window has just reset while the island is visible (Luch may
    /// smile, the home page redraws). No payload.
    fn rollover(&self);
}

/// Never blocks; the first failure (receiver gone) is logged, later ones are silent.
fn send_logged<T>(tx: &Sender<T>, value: T, to: &str, closed: &AtomicBool) {
    if tx.send(value).is_err() && !closed.swap(true, Ordering::Relaxed) {
        log::line(format!("observe: {to} is not running, messages are dropped"));
    }
}

/// Store loop body. Blocks on `rx`; each batch = first message + everything already queued.
/// Per batch: apply each signal with `now()`, dispatch its effects (TrackTranscript /
/// UntrackTranscript → claude-io, UntrackRollout / CodexTicker → codex-io), forward Retry to
/// claude-io on Visible(true); then write `shared`, emit alerts (if any) before the snapshot,
/// and the snapshot per `should_emit`. A batch with the end of a Claude turn has claude-io read the
/// plan limits again. While the island is visible and a limit window resets later, the wait for the
/// next batch ends at that reset (`rollover_wait`): claude-io reads the limits again and the island
/// hears of it; hidden, the thread only waits for signals (a reset missed then is read by the Retry
/// of the next Visible(true)). Returns when every sender of `rx` is gone.
pub(crate) fn run_store(
    rx: Receiver<Signal>,
    publisher: Box<dyn Publisher>,
    claude: Sender<ClaudeInput>,
    codex: Sender<CodexInput>,
    shared: Arc<Mutex<SessionsSnapshot>>,
    now: fn() -> i64,
) {
    let mut store = Store::new();
    let mut visible = false;
    let mut published: Option<SessionsSnapshot> = None;
    let claude_closed = AtomicBool::new(false);
    let codex_closed = AtomicBool::new(false);
    let mut usage = Usage::default();
    // The last reset already acted on: a clock that has not moved past it does not repeat it.
    let mut rolled_over_at = i64::MIN;
    loop {
        let now_ms = now();
        let next = next_rollover(&usage, now_ms.max(rolled_over_at));
        let first = match rollover_wait(visible, next, now_ms) {
            None => match rx.recv() {
                Ok(first) => first,
                Err(_) => return,
            },
            Some(wait) => match rx.recv_timeout(wait) {
                Ok(first) => first,
                Err(RecvTimeoutError::Timeout) => {
                    rolled_over_at = next.unwrap_or(now_ms);
                    send_logged(&claude, ClaudeInput::ReadUsage, "claude-io", &claude_closed);
                    publisher.rollover();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            },
        };
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        let mut alerts: Vec<Alert> = Vec::new();
        let mut became_visible = false;
        let mut wake = false;
        let mut turn_ended = false;
        for signal in batch {
            let shown = if let Signal::Visible(on) = &signal { Some(*on) } else { None };
            let applied = store.apply(signal, now());
            if let Some(on) = shown {
                visible = on;
                if on {
                    became_visible = true;
                    send_logged(&claude, ClaudeInput::Retry, "claude-io", &claude_closed);
                    send_logged(&codex, CodexInput::Retry, "codex-io", &codex_closed);
                }
            }
            alerts.extend(applied.alerts);
            wake |= applied.wake;
            turn_ended |= applied.claude_turn_ended;
            for effect in applied.effects {
                match effect {
                    Effect::TrackTranscript { .. } | Effect::UntrackTranscript { .. } => {
                        send_logged(&claude, ClaudeInput::Effect(effect), "claude-io", &claude_closed)
                    }
                    Effect::UntrackRollout { .. } | Effect::CodexTicker(_) => {
                        send_logged(&codex, CodexInput::Effect(effect), "codex-io", &codex_closed)
                    }
                }
            }
        }
        if turn_ended {
            send_logged(&claude, ClaudeInput::ReadUsage, "claude-io", &claude_closed);
        }
        let snapshot = store.snapshot(now());
        usage = snapshot.usage.clone();
        *shared.lock().unwrap_or_else(PoisonError::into_inner) = snapshot.clone();
        if !alerts.is_empty() {
            publisher.alerts(&alerts);
        }
        // A wake needs the row in the island's snapshot even while hidden, like an alert.
        if should_emit(published.as_ref(), &snapshot, visible, became_visible, !alerts.is_empty() || wake) {
            publisher.snapshot(&snapshot);
            published = Some(snapshot);
        }
        if wake {
            publisher.silent_wake();
        }
    }
}

pub struct ObserveConfig {
    /// $CLAUDE_CONFIG_DIR or %USERPROFILE%\.claude
    pub claude_dir: PathBuf,
    /// $CLAUDE_CONFIG_DIR\.claude.json or %USERPROFILE%\.claude.json (usage-home spec §2.1)
    pub claude_json: PathBuf,
    /// $CODEX_HOME or %USERPROFILE%\.codex
    pub codex_home: PathBuf,
    /// %APPDATA% (host classification)
    pub appdata: PathBuf,
    /// %LOCALAPPDATA% (host classification)
    pub localappdata: PathBuf,
}

#[derive(Clone)]
pub struct Handle {
    signals: Sender<Signal>,
    claude: Sender<ClaudeInput>,
    codex: Sender<CodexInput>,
    shared: Arc<Mutex<SessionsSnapshot>>,
    signals_closed: Arc<AtomicBool>,
    claude_closed: Arc<AtomicBool>,
    codex_closed: Arc<AtomicBool>,
}

impl Handle {
    /// Never blocks (unbounded channel); a failed send is logged once.
    pub fn send(&self, s: Signal) {
        send_logged(&self.signals, s, "store", &self.signals_closed);
    }

    /// Router intake (Route::Observe) → codex-io.
    pub fn codex_hook(&self, payload: Value) {
        send_logged(&self.codex, CodexInput::Hook(payload), "codex-io", &self.codex_closed);
    }

    /// Router intake of an Antigravity line (Route::Observe) → codex-io, which resolves its project.
    pub fn antigravity_hook(&self, payload: Value) {
        send_logged(&self.codex, CodexInput::Antigravity(payload), "codex-io", &self.codex_closed);
    }

    /// The home view opened (usage-home spec §2.1): claude-io reads `.claude.json` once more and
    /// sends the result to the store if it changed. Never blocks.
    pub fn claude_usage(&self) {
        send_logged(&self.claude, ClaudeInput::ReadUsage, "claude-io", &self.claude_closed);
    }

    /// Clone of the last snapshot the store thread wrote.
    pub fn snapshot(&self) -> SessionsSnapshot {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// Spawns threads "store", "claude-io", "codex-io" with one shared proc::Registry. A source that
/// fails to spawn reports SourceHealth Error and the others still run.
pub fn start(cfg: ObserveConfig, publisher: Box<dyn Publisher>) -> Handle {
    let (signals, rx) = mpsc::channel::<Signal>();
    let registry = proc::Registry::new();
    let shared = Arc::new(Mutex::new(Store::new().snapshot(now_ms())));

    let claude_cfg = ClaudeConfig { base: cfg.claude_dir, usage_file: cfg.claude_json, appdata: cfg.appdata, watch: true, terminal_of: terminal::of_process };
    let claude = match claude::spawn(claude_cfg, signals.clone(), registry.clone()) {
        Ok(tx) => tx,
        Err(err) => {
            source_failed(&signals, Source::Claude, &err);
            mpsc::channel().0
        }
    };
    let codex_cfg = CodexConfig { codex_home: cfg.codex_home, localappdata: cfg.localappdata, watch: true, terminal_of: terminal::of_process };
    let codex = match codex::spawn(codex_cfg, signals.clone(), registry) {
        Ok(tx) => tx,
        Err(err) => {
            source_failed(&signals, Source::Codex, &err);
            mpsc::channel().0
        }
    };

    let store_shared = shared.clone();
    let store_claude = claude.clone();
    let store_codex = codex.clone();
    let spawned = std::thread::Builder::new()
        .name("store".into())
        .spawn(move || run_store(rx, publisher, store_claude, store_codex, store_shared, now_ms));
    if let Err(err) = spawned {
        log::line(format!("observe: store thread failed to start: {err}"));
    }

    Handle {
        signals,
        claude,
        codex,
        shared,
        signals_closed: Arc::new(AtomicBool::new(false)),
        claude_closed: Arc::new(AtomicBool::new(false)),
        codex_closed: Arc::new(AtomicBool::new(false)),
    }
}

fn source_failed(signals: &Sender<Signal>, source: Source, err: &std::io::Error) {
    log::line(format!("observe: {source:?} source failed to start: {err}"));
    let health = Health::Error(
        crate::message::Message::new("msg.health.threadFailed", format!("thread failed to start: {err}"))
            .with("err", err.to_string()),
    );
    // `start` still owns the receiver at this point, so this send cannot fail.
    let _ = signals.send(Signal::SourceHealth { source, health });
}

/// Milliseconds since the Unix epoch (UTC); 0 if the clock is before 1970.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Router annotations for a Claude PermissionRequest (spec §5.3): payload["agent_pid"] →
/// proc::identify → claude_key(pid, created_ft); payload["cwd"] → project root basename (D39,
/// D40). Each part None on failure. Blocking (Win32 + .git probes): the router calls it on
/// spawn_blocking (D46).
pub fn approval_context(payload: &Value) -> ApprovalContext {
    let session_key = payload
        .get("agent_pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .and_then(|pid| proc::identify(pid).ok())
        .map(|(id, _handle)| keys::claude_key(id.pid, id.created_ft));
    let project_name = payload
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|cwd| !cwd.trim().is_empty())
        .map(|cwd| paths::basename(&paths::project_of(cwd, paths::git_probe).root));
    ApprovalContext { session_key, project_name }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use model::{ClaudeSessionFile, HookEvent, HostKind, ProcId, ProjectRef};
    use std::thread::JoinHandle;
    use std::time::Duration;

    #[test]
    fn should_emit_follows_the_publication_rules() {
        let base = Store::new().snapshot(0);
        let mut rev = base.clone();
        rev.rev += 1;
        let mut counts = rev.clone();
        counts.counts.waiting = 1;
        let mut hold = rev.clone();
        hold.hold = true;
        let mut attention = rev.clone();
        attention.attention = vec!["claude:1:2".into()];
        let mut aggregate = rev.clone();
        aggregate.aggregate = model::BotState::Working;

        assert!(should_emit(None, &base, false, false, false), "first snapshot");
        assert!(should_emit(Some(&base), &base, true, true, false), "Visible(true) always");
        assert!(should_emit(Some(&base), &rev, true, false, false), "visible: rev changed");
        assert!(!should_emit(Some(&base), &base, true, false, true), "visible: same rev");
        assert!(!should_emit(Some(&base), &rev, false, false, false), "hidden: only rev changed");
        assert!(should_emit(Some(&base), &rev, false, false, true), "hidden: alerts");
        for changed in [&counts, &hold, &attention, &aggregate] {
            assert!(should_emit(Some(&base), changed, false, false, false), "hidden: {changed:?}");
        }
    }

    const WAIT: Duration = Duration::from_secs(5);

    #[derive(Debug, PartialEq)]
    enum Published {
        Alerts(Vec<String>),
        Snapshot(SessionsSnapshot),
        SilentWake,
        Rollover,
    }

    struct FakePublisher(Sender<Published>);

    impl Publisher for FakePublisher {
        fn alerts(&self, alerts: &[Alert]) {
            let keys = alerts.iter().map(|a| a.key.clone()).collect();
            self.0.send(Published::Alerts(keys)).expect("test receiver alive");
        }
        fn snapshot(&self, snapshot: &SessionsSnapshot) {
            self.0.send(Published::Snapshot(snapshot.clone())).expect("test receiver alive");
        }
        fn silent_wake(&self) {
            self.0.send(Published::SilentWake).expect("test receiver alive");
        }
        fn rollover(&self) {
            self.0.send(Published::Rollover).expect("test receiver alive");
        }
    }

    fn fixed_now() -> i64 {
        1_000
    }

    struct StoreRig {
        signals: Sender<Signal>,
        published: Receiver<Published>,
        claude: Receiver<ClaudeInput>,
        codex: Receiver<CodexInput>,
        shared: Arc<Mutex<SessionsSnapshot>>,
        thread: JoinHandle<()>,
    }

    fn store_rig() -> StoreRig {
        store_rig_at(fixed_now)
    }

    fn store_rig_at(now: fn() -> i64) -> StoreRig {
        let (signals, rx) = mpsc::channel();
        let (publish_tx, published) = mpsc::channel();
        let (claude_tx, claude) = mpsc::channel();
        let (codex_tx, codex) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Store::new().snapshot(0)));
        let store_shared = shared.clone();
        let thread = std::thread::spawn(move || {
            run_store(rx, Box::new(FakePublisher(publish_tx)), claude_tx, codex_tx, store_shared, now)
        });
        StoreRig { signals, published, claude, codex, shared, thread }
    }

    impl StoreRig {
        fn send(&self, s: Signal) {
            self.signals.send(s).unwrap();
        }
        fn next(&self) -> Published {
            self.published.recv_timeout(WAIT).expect("a publication")
        }
        fn next_snapshot(&self) -> SessionsSnapshot {
            match self.next() {
                Published::Snapshot(s) => s,
                other => panic!("expected a snapshot, got {other:?}"),
            }
        }
        /// Ends the store loop and returns whatever was published after the last `next`.
        fn finish(self) -> (Vec<Published>, SessionsSnapshot) {
            let StoreRig { signals, published, shared, thread, .. } = self;
            drop(signals);
            thread.join().expect("store loop ends when the senders are gone");
            let rest = published.try_iter().collect();
            let last = shared.lock().unwrap().clone();
            (rest, last)
        }
    }

    fn claude_file(status: &str, at: i64) -> Signal {
        let file = ClaudeSessionFile {
            pid: Some(42),
            session_id: Some("s-1".into()),
            cwd: Some(r"C:\work\app".into()),
            proc_start: Some("7".into()),
            status: Some(status.into()),
            waiting_for: (status == "waiting").then(|| "permission prompt".to_string()),
            status_updated_at: Some(at),
            ..ClaudeSessionFile::default()
        };
        Signal::ClaudeFile {
            key: keys::claude_key(42, 7),
            proc: ProcId { pid: 42, created_ft: 7, image: r"C:\bin\claude.exe".into() },
            host: HostKind::Unknown,
            project: ProjectRef { key: r"c:\work\app".into(), root: r"C:\work\app".into() },
            file,
            transcript_eof: None,
        }
    }

    fn codex_event(name: &str) -> Signal {
        let event = HookEvent {
            hook_event_name: name.into(),
            session_id: "t-1".into(),
            turn_id: Some("u1".into()),
            cwd: r"C:\work\app".into(),
            transcript_path: None,
            tool_name: None,
            tool_input: None,
            last_assistant_message: None,
            agent_id: None,
            agent_type: None,
            relay_ts: 500,
            agent_pid: None,
            agent_created: None,
            agent_image: None,
        };
        Signal::CodexHook {
            key: keys::codex_key("t-1"),
            event,
            proc: None,
            host: HostKind::Unknown,
            project: ProjectRef { key: r"c:\work\app".into(), root: r"C:\work\app".into() },
            rollout_eof: None,
            title: None,
        }
    }

    #[test]
    fn hidden_changes_wait_for_counts_or_visibility() {
        let rig = store_rig();
        rig.send(Signal::SourceHealth { source: Source::Claude, health: Health::Missing });
        assert_eq!(rig.next_snapshot().sources.claude, Health::Missing, "first batch is published");
        // Hidden, counts unchanged: not published on its own…
        rig.send(Signal::SourceHealth { source: Source::Codex, health: Health::Degraded { unreadable: 1 } });
        // …but Visible(true) always publishes the full snapshot.
        rig.send(Signal::Visible(true));
        let shown = rig.next_snapshot();
        assert_eq!(shown.sources.codex, Health::Degraded { unreadable: 1 });
        assert!(matches!(rig.claude.recv_timeout(WAIT), Ok(ClaudeInput::Retry)));
        // Visible: every rev change.
        rig.send(Signal::SourceHealth { source: Source::Claude, health: Health::Ok });
        let after = rig.next_snapshot();
        assert_eq!(after.sources.claude, Health::Ok);
        assert!(after.rev > shown.rev);
        let (rest, last) = rig.finish();
        assert_eq!(rest, []);
        assert_eq!(last, after, "shared holds the last snapshot");
    }

    #[test]
    fn effects_go_to_their_source_and_alerts_precede_their_snapshot() {
        let rig = store_rig();
        let key = keys::claude_key(42, 7);
        rig.send(claude_file("busy", 100));
        rig.next_snapshot();
        match rig.claude.recv_timeout(WAIT) {
            Ok(ClaudeInput::Effect(Effect::TrackTranscript { key: k, session_id })) => {
                assert_eq!(k, key);
                assert_eq!(session_id, "s-1");
            }
            other => panic!("{other:?}"),
        }
        rig.send(claude_file("waiting", 200));
        assert_eq!(rig.next(), Published::Alerts(vec![key.clone()]));
        let waiting = rig.next_snapshot();
        assert_eq!(waiting.attention, [key.clone()]);
        assert!(waiting.hold);
        rig.send(Signal::ProcExit { pid: 42, created_ft: 7 });
        assert_eq!(rig.next_snapshot().counts.waiting, 0, "hidden, but counts changed");
        assert!(matches!(
            rig.claude.recv_timeout(WAIT),
            Ok(ClaudeInput::Effect(Effect::UntrackTranscript { key: k })) if k == key
        ));
        let (rest, _) = rig.finish();
        assert_eq!(rest, []);
    }

    #[test]
    fn a_new_silent_row_wakes_the_island_once_after_its_snapshot() {
        let rig = store_rig(); // hidden
        let silent = |written_ms| Signal::CodexSilent {
            id: "s-1".into(),
            project: ProjectRef { key: r"c:\work\app".into(), root: r"C:\work\app".into() },
            host: HostKind::CodexDesktop,
            folder_exists: true,
            written_ms,
        };
        rig.send(silent(900));
        let shown = rig.next_snapshot();
        assert_eq!(shown.projects.iter().flat_map(|p| p.sessions.iter()).filter(|s| s.silent.is_some()).count(), 1);
        assert_eq!(rig.next(), Published::SilentWake, "after the snapshot that has the row");
        // A refresh of the live row wakes nothing.
        rig.send(silent(950));
        let (rest, _) = rig.finish();
        assert!(!rest.contains(&Published::SilentWake), "{rest:?}");
    }

    #[test]
    fn alert_prefs_reach_the_model_through_the_store() {
        let rig = store_rig();
        rig.send(Signal::AlertPrefs { waiting: false, finished: true, error: true });
        rig.next_snapshot();
        rig.send(claude_file("busy", 100));
        rig.next_snapshot();
        rig.send(claude_file("waiting", 200));
        // No alert (D18), but the row and the counters still change: published while hidden.
        let waiting = rig.next_snapshot();
        assert_eq!(waiting.counts.waiting, 1);
        assert!(!waiting.hold, "hold needs alertWaiting");
        let (rest, _) = rig.finish();
        assert_eq!(rest, []);
    }

    #[test]
    fn codex_ticker_effects_go_to_codex_io() {
        let rig = store_rig(); // hidden: the store thread has not seen Visible
        rig.send(codex_event("UserPromptSubmit"));
        assert!(matches!(rig.codex.recv_timeout(WAIT), Ok(CodexInput::Effect(Effect::CodexTicker(true)))));
        // The island showing and hiding does not touch the ticker; the end of the turn does.
        rig.send(Signal::Visible(true));
        rig.send(Signal::Visible(false));
        assert!(matches!(rig.codex.recv_timeout(WAIT), Ok(CodexInput::Retry)), "showing the island retries the sessions watch");
        rig.send(codex_event("Stop"));
        assert!(matches!(rig.codex.recv_timeout(WAIT), Ok(CodexInput::Effect(Effect::CodexTicker(false)))));
        rig.finish();
    }

    // ── Plan limits read again on events ──

    fn claude_limits(resets_at_ms: Option<i64>) -> Signal {
        Signal::ClaudeUsage(Some(model::ClaudeUsage {
            fetched_at_ms: 0,
            windows: vec![model::ClaudeWindow { id: model::ClaudeWindowId::FiveHour, percent: 90.0, resets_at_ms }],
        }))
    }

    fn codex_limits(resets_at_ms: Option<i64>) -> Signal {
        Signal::CodexUsageSeed(model::CodexUsage {
            seen_at_ms: 0,
            windows: vec![model::CodexWindow { minutes: 300, percent: 10.0, resets_at_ms }],
        })
    }

    fn usage_of(signals: Vec<Signal>) -> Usage {
        let mut store = Store::new();
        for s in signals {
            store.apply(s, 0);
        }
        store.snapshot(0).usage
    }

    /// The claude-io inputs the store sent within `wait`, ReadUsage and Retry only.
    fn reads(rig: &StoreRig, wait: Duration) -> Vec<&'static str> {
        let until = std::time::Instant::now() + wait;
        let mut got = Vec::new();
        while let Some(left) = until.checked_duration_since(std::time::Instant::now()) {
            match rig.claude.recv_timeout(left) {
                Ok(ClaudeInput::ReadUsage) => got.push("read"),
                Ok(ClaudeInput::Retry) => got.push("retry"),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        got
    }

    #[test]
    fn the_next_rollover_is_the_earliest_reset_to_come_of_claude_and_codex() {
        let g = ROLLOVER_GRACE_MS;
        assert_eq!(next_rollover(&Usage::default(), 0), None, "no limits");
        assert_eq!(next_rollover(&usage_of(vec![claude_limits(None)]), 0), None, "no reset time");
        let both = usage_of(vec![claude_limits(Some(50_000)), codex_limits(Some(20_000))]);
        assert_eq!(next_rollover(&both, 0), Some(20_000 + g), "Codex first");
        assert_eq!(next_rollover(&both, 20_000 + g - 1), Some(20_000 + g));
        assert_eq!(next_rollover(&both, 20_000 + g), Some(50_000 + g), "a reset acted on is not repeated");
        assert_eq!(next_rollover(&both, 50_000 + g), None, "every reset is behind");
        assert_eq!(next_rollover(&usage_of(vec![claude_limits(Some(i64::MAX))]), 0), None, "no overflow");
    }

    #[test]
    fn the_store_waits_for_a_reset_only_while_the_island_is_visible() {
        assert_eq!(rollover_wait(false, Some(10_000), 1_000), None, "hidden: no timer");
        assert_eq!(rollover_wait(true, None, 1_000), None, "no reset to come: no timer");
        assert_eq!(rollover_wait(true, Some(10_000), 1_000), Some(Duration::from_millis(9_000)));
        assert_eq!(rollover_wait(true, Some(500), 1_000), Some(Duration::ZERO), "already due");
    }

    #[test]
    fn the_end_of_a_claude_turn_reads_the_limits_again() {
        let rig = store_rig(); // hidden: the end of a turn reads all the same (a file, no network)
        rig.send(claude_file("busy", 100));
        rig.next_snapshot();
        assert_eq!(reads(&rig, Duration::from_millis(100)), [] as [&str; 0], "a turn that starts");
        rig.send(claude_file("idle", 200));
        rig.next_snapshot();
        assert_eq!(reads(&rig, Duration::from_millis(500)), ["read"], "the turn ended");
        rig.finish();
    }

    static VISIBLE_CLOCK: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1_000);
    fn visible_clock() -> i64 {
        VISIBLE_CLOCK.load(Ordering::SeqCst)
    }

    #[test]
    fn a_reset_while_visible_reads_the_limits_once_and_tells_the_island() {
        let rig = store_rig_at(visible_clock);
        rig.send(Signal::Visible(true));
        rig.next_snapshot();
        assert_eq!(reads(&rig, Duration::from_millis(100)), ["retry"]);
        // Due 50 ms from now on the store's clock.
        rig.send(claude_limits(Some(1_000 - ROLLOVER_GRACE_MS + 50)));
        rig.next_snapshot();
        assert_eq!(rig.next(), Published::Rollover);
        assert_eq!(reads(&rig, Duration::from_millis(300)), ["read"], "once, though the clock did not move");
        let (rest, _) = rig.finish();
        assert!(!rest.contains(&Published::Rollover), "{rest:?}");
    }

    static HIDDEN_CLOCK: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1_000);
    fn hidden_clock() -> i64 {
        HIDDEN_CLOCK.load(Ordering::SeqCst)
    }

    #[test]
    fn a_reset_while_hidden_wakes_nothing_and_is_read_on_the_next_show() {
        let rig = store_rig_at(hidden_clock);
        rig.send(claude_limits(Some(1_000 - ROLLOVER_GRACE_MS + 50)));
        rig.next_snapshot();
        assert_eq!(reads(&rig, Duration::from_millis(300)), [] as [&str; 0], "hidden: no timer");
        // The reset passed while hidden; showing the island reads the limits (Retry), no smile.
        HIDDEN_CLOCK.store(10_000, Ordering::SeqCst);
        rig.send(Signal::Visible(true));
        rig.next_snapshot();
        assert_eq!(reads(&rig, Duration::from_millis(300)), ["retry"]);
        let (rest, _) = rig.finish();
        assert!(!rest.contains(&Published::Rollover), "{rest:?}");
    }

    #[test]
    fn start_reports_a_missing_claude_dir_and_serves_the_snapshot() {
        let dir = TempDir::new("observe-start");
        let (publish_tx, published) = mpsc::channel();
        let handle = start(
            ObserveConfig {
                claude_dir: dir.path.join("no-claude"),
                claude_json: dir.path.join("no-claude.json"),
                codex_home: dir.path.join("no-codex"),
                appdata: dir.path.clone(),
                localappdata: dir.path.clone(),
            },
            Box::new(FakePublisher(publish_tx)),
        );
        handle.send(Signal::Visible(true));
        loop {
            match published.recv_timeout(WAIT).expect("a publication") {
                Published::Snapshot(s) if s.sources.claude == Health::Missing => break,
                _ => continue,
            }
        }
        assert_eq!(handle.snapshot().sources.claude, Health::Missing);
    }

    #[test]
    fn handle_sends_never_panic_without_receivers() {
        let handle = Handle {
            signals: mpsc::channel().0,
            claude: mpsc::channel().0,
            codex: mpsc::channel().0,
            shared: Arc::new(Mutex::new(Store::new().snapshot(0))),
            signals_closed: Arc::new(AtomicBool::new(false)),
            claude_closed: Arc::new(AtomicBool::new(false)),
            codex_closed: Arc::new(AtomicBool::new(false)),
        };
        handle.send(Signal::Visible(true));
        handle.send(Signal::Visible(false));
        handle.codex_hook(serde_json::json!({}));
        handle.claude_usage();
        assert!(handle.signals_closed.load(Ordering::Relaxed));
        assert!(handle.claude_closed.load(Ordering::Relaxed));
        assert!(handle.codex_closed.load(Ordering::Relaxed));
        assert_eq!(handle.snapshot().rev, 0);
    }

    #[test]
    fn approval_context_names_the_process_and_the_project() {
        let dir = TempDir::new("approval-ctx");
        std::fs::create_dir_all(dir.path.join("repo").join(".git")).unwrap();
        std::fs::create_dir_all(dir.path.join("repo").join("src")).unwrap();
        let me = proc::identify(std::process::id()).expect("identify self").0;
        let cwd = dir.path.join("repo").join("src").to_string_lossy().into_owned();
        let ctx = approval_context(&serde_json::json!({ "agent_pid": std::process::id(), "cwd": cwd }));
        assert_eq!(ctx.session_key, Some(keys::claude_key(me.pid, me.created_ft)));
        assert_eq!(ctx.project_name.as_deref(), Some("repo"));
        let none = approval_context(&serde_json::json!({ "agent_pid": null, "cwd": "  " }));
        assert_eq!(none, ApprovalContext::default());
    }
}
