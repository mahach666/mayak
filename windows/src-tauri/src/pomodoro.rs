// The running pomodoro as the island holds it (src/island/pomodoro.ts), so Rust can say what the
// user's focus is (the MCP server's `get_focus`). The page owns the run; it tells Rust when the run
// changes (started, moved on to the next phase, stopped, kept across a page reload) and at no other
// time: nothing ticks, so a hidden island costs nothing. Rust decides nothing from it.
//
// A phase that ends while the island is hidden moves on only when the island shows again
// (island.ts `checkPomodoro`), so the snapshot can name a phase whose end has passed: `get_focus`
// says so instead of guessing the next one.

use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

/// What the page sends: the run's phase, its work cycle, when the phase ends, and whether agent
/// alerts are held back during it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Focus {
    /// "work", "short" or "long".
    pub phase: String,
    /// The work cycle, 1…cycles.
    pub cycle: u32,
    /// Work intervals before a long break.
    pub cycles: u32,
    /// When the phase ends, ms since the Unix epoch.
    pub ends_at: i64,
    /// Agent alerts are held back now (a work interval with "mute" on).
    pub muted: bool,
}

/// The Focus tab offers 2 to 6 cycles (pomodoro.ts `POMODORO_CHOICES`).
const MAX_CYCLES: u32 = 6;

impl Focus {
    fn valid(&self) -> bool {
        matches!(self.phase.as_str(), "work" | "short" | "long")
            && (1..=MAX_CYCLES).contains(&self.cycles)
            && (1..=self.cycles).contains(&self.cycle)
            && self.ends_at > 0
    }
}

static CURRENT: Mutex<Option<Focus>> = Mutex::new(None);

/// The page's word: a run, or None for no run. A run that cannot be one is refused and changes nothing.
pub fn set(focus: Option<Focus>) -> Result<(), String> {
    if focus.as_ref().is_some_and(|f| !f.valid()) {
        return Err("not a pomodoro".to_string());
    }
    *CURRENT.lock().unwrap_or_else(PoisonError::into_inner) = focus;
    Ok(())
}

/// The run as the page last told it; None while none runs.
pub fn get() -> Option<Focus> {
    CURRENT.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The island's pomodoro started, moved on, stopped (None) or was kept across a reload.
#[tauri::command]
pub fn pomodoro_state(state: Option<Focus>) -> Result<(), String> {
    set(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(phase: &str, cycle: u32, cycles: u32) -> Focus {
        Focus { phase: phase.into(), cycle, cycles, ends_at: 1_000, muted: phase == "work" }
    }

    /// The page's JSON, as island.ts sends it.
    #[test]
    fn the_pages_snapshot_reads_as_it_is_sent() {
        let sent: Focus =
            serde_json::from_str(r#"{"phase":"work","cycle":2,"cycles":4,"endsAt":1791912666805,"muted":true}"#).unwrap();
        assert_eq!(sent, Focus { phase: "work".into(), cycle: 2, cycles: 4, ends_at: 1_791_912_666_805, muted: true });
    }

    // One test for the one shared slot, so no two tests race on it.
    #[test]
    fn a_run_is_kept_until_the_page_says_otherwise_and_nonsense_changes_nothing() {
        assert!(set(Some(run("short", 1, 4))).is_ok());
        assert_eq!(get(), Some(run("short", 1, 4)));
        for bad in [run("nap", 1, 4), run("work", 0, 4), run("work", 5, 4), run("long", 1, 7), run("work", 1, 0),
            Focus { ends_at: 0, ..run("work", 1, 4) }]
        {
            assert!(set(Some(bad.clone())).is_err(), "{bad:?}");
        }
        assert_eq!(get(), Some(run("short", 1, 4)), "a refused snapshot changes nothing");
        assert!(set(None).is_ok());
        assert_eq!(get(), None);
    }
}
