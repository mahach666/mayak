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

use model::{SessionsSnapshot, Store};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
