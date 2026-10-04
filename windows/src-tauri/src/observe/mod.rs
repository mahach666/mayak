// Session observer (spec §4, §9). Three std threads: "claude-io" and "codex-io" turn file
// notifications and Codex hooks into Signals; "store" applies them to the pure model
// (model::Store), dispatches the returned effects back to the sources and publishes snapshots.
// Nothing here runs on the tokio runtime.

pub mod claude;
pub mod claude_transcript;
pub mod codex;
pub mod keys;
pub mod model;
pub mod paths;
pub mod tail;
pub mod text;
pub mod win;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};

use crate::log;
use claude::ClaudeInput;
use codex::CodexInput;
use model::{Alert, Effect, Health, SessionsSnapshot, Signal, Source, Store};

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

/// Where the store thread publishes. The app emits to the island window; tests record.
pub trait Publisher: Send + 'static {
    /// "session-alert"
    fn alerts(&self, alerts: &[Alert]);
    /// "sessions"
    fn snapshot(&self, snapshot: &SessionsSnapshot);
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
/// and the snapshot per `should_emit`. Returns when every sender of `rx` is gone.
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
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        let mut alerts: Vec<Alert> = Vec::new();
        let mut became_visible = false;
        for signal in batch {
            let shown = if let Signal::Visible(on) = &signal { Some(*on) } else { None };
            let applied = store.apply(signal, now());
            if let Some(on) = shown {
                visible = on;
                if on {
                    became_visible = true;
                    send_logged(&claude, ClaudeInput::Retry, "claude-io", &claude_closed);
                }
            }
            alerts.extend(applied.alerts);
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
        let snapshot = store.snapshot(now());
        *shared.lock().unwrap_or_else(PoisonError::into_inner) = snapshot.clone();
        if !alerts.is_empty() {
            publisher.alerts(&alerts);
        }
        if should_emit(published.as_ref(), &snapshot, visible, became_visible, !alerts.is_empty()) {
            publisher.snapshot(&snapshot);
            published = Some(snapshot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let (signals, rx) = mpsc::channel();
        let (publish_tx, published) = mpsc::channel();
        let (claude_tx, claude) = mpsc::channel();
        let (codex_tx, codex) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Store::new().snapshot(0)));
        let store_shared = shared.clone();
        let thread = std::thread::spawn(move || {
            run_store(rx, Box::new(FakePublisher(publish_tx)), claude_tx, codex_tx, store_shared, fixed_now)
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

    fn codex_prompt() -> Signal {
        let event = HookEvent {
            hook_event_name: "UserPromptSubmit".into(),
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
        let rig = store_rig();
        rig.send(Signal::Visible(true));
        rig.next_snapshot();
        rig.send(codex_prompt());
        assert!(matches!(rig.codex.recv_timeout(WAIT), Ok(CodexInput::Effect(Effect::CodexTicker(true)))));
        rig.send(Signal::Visible(false));
        assert!(matches!(rig.codex.recv_timeout(WAIT), Ok(CodexInput::Effect(Effect::CodexTicker(false)))));
        rig.finish();
    }
}
