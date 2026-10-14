// The MCP server's view of the running app: the observer's snapshot, the Stats and Apps tabs (and
// which of those apps has a window open), the Focus timer and the Agent Orchestrator sessions as the
// island last had them, the island's question card, and an agent's line in its chat's column.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use super::tools::{AoRead, AskOutcome, AskRequest, Backend, StatusShown};
use crate::pipe::{self, Asked, Kind, Pending};

pub struct AppBackend {
    app: AppHandle,
    pending: Arc<Pending>,
    observe: crate::observe::Handle,
    /// Ids of the questions of an MCP agent that are on the island: at most one.
    asking: Mutex<HashSet<String>>,
}

impl AppBackend {
    pub fn new(app: AppHandle, pending: Arc<Pending>, observe: crate::observe::Handle) -> AppBackend {
        AppBackend { app, pending, observe, asking: Mutex::new(HashSet::new()) }
    }

    fn asking(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.asking.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Hands every question still on the island back to its agent (the server is stopping).
    pub fn release_asks(&self) {
        let ids: Vec<String> = self.asking().iter().cloned().collect();
        for id in ids {
            pipe::decline(&self.pending, &id);
        }
    }
}

/// The `hook` event of a question card, shaped as the island's page reads an `AskUserQuestion`:
/// the same card, the same answers back through `question_answer`. The card's name line is the
/// asker, never a session: no session is behind it.
pub fn card_payload(request_id: &str, ask: &AskRequest) -> Value {
    let options: Vec<Value> = ask
        .options
        .iter()
        .map(|o| json!({ "label": o.label, "description": o.description }))
        .collect();
    json!({
        "hook_event_name": "PreToolUse",
        "request_id": request_id,
        "session_id": "",
        "cwd": "",
        "tool_name": "AskUserQuestion",
        "tool_input": {
            "questions": [{
                "question": ask.question,
                "header": ask.header,
                "options": options,
                "multiSelect": ask.multi_select,
            }]
        },
        "session_key": null,
        "project_name": ask.asker,
        "kind": "question",
        "agent": "claude",
        "canAlways": false,
    })
}

/// Lower-case image names of the processes with a window open now; None where that cannot be read.
fn windowed_exe_names() -> Option<HashSet<String>> {
    #[cfg(windows)]
    {
        Some(crate::focus::win::windowed_exe_names())
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Whether the app whose window belongs to `exe` is open: None when the app names no exe or the
/// windows cannot be read here.
fn open_now(exe: Option<&str>, windowed: Option<&HashSet<String>>) -> Option<bool> {
    Some(windowed?.contains(&exe?.to_lowercase()))
}

/// The tiles (`apps::AppTile` as JSON) with `open` set on each, in the same order.
fn with_open(mut tiles: Value, open: &[Option<bool>]) -> Value {
    if let Some(list) = tiles.as_array_mut() {
        for (tile, open) in list.iter_mut().zip(open) {
            tile["open"] = json!(open);
        }
    }
    tiles
}

/// The answers a card sent back: `answer <compact JSON>`.
fn parse_answer(line: &str) -> Option<Value> {
    let json = line.strip_prefix("answer ")?;
    serde_json::from_str::<Value>(json).ok().filter(Value::is_object)
}

impl Backend for AppBackend {
    fn snapshot(&self) -> Value {
        serde_json::to_value(self.observe.snapshot()).unwrap_or_else(|err| {
            crate::log::line(format!("mcp: the snapshot could not be read: {err}"));
            Value::Null
        })
    }

    async fn stats(&self, days: u32) -> Result<Value, String> {
        let report = tauri::async_runtime::spawn_blocking(move || crate::stats_report(days))
            .await
            .map_err(|_| "the count stopped".to_string())??;
        serde_json::to_value(report).map_err(|err| err.to_string())
    }

    async fn apps(&self) -> Value {
        let settings = self.app.state::<crate::Shared>().settings.lock().unwrap_or_else(PoisonError::into_inner).clone();
        // One process snapshot and one window enumeration for all the tiles, off the async runtime.
        let read = tauri::async_runtime::spawn_blocking(move || {
            let tiles = crate::apps::list(&settings);
            let windowed = windowed_exe_names();
            let open: Vec<Option<bool>> = tiles
                .iter()
                .map(|tile| open_now(crate::apps::window_exe(&tile.id, &settings).as_deref(), windowed.as_ref()))
                .collect();
            (tiles, open)
        })
        .await;
        let (tiles, open) = read.unwrap_or_else(|_| {
            crate::log::line("mcp: the apps lookup stopped");
            (Vec::new(), Vec::new())
        });
        with_open(serde_json::to_value(tiles).unwrap_or(Value::Null), &open)
    }

    fn focus(&self) -> Value {
        serde_json::to_value(crate::pomodoro::get()).unwrap_or(Value::Null)
    }

    fn now_ms(&self) -> i64 {
        crate::observe::now_ms()
    }

    fn ao(&self) -> AoRead {
        AoRead { enabled: crate::integrations::enabled(&self.app, crate::ao::ID), last: crate::ao::last() }
    }

    fn set_status(&self, chat: u64, text: Option<String>) -> StatusShown {
        let Some(manager) = self.app.try_state::<crate::chat::ChatManager>() else { return StatusShown::NoChat };
        match manager.set_agent_note(chat, text) {
            crate::chat::Noted::Shown => StatusShown::Shown,
            crate::chat::Noted::NoTurn => StatusShown::NoTurn,
            crate::chat::Noted::NoChat => StatusShown::NoChat,
        }
    }

    async fn ask(&self, ask: AskRequest) -> AskOutcome {
        let request = {
            let mut asking = self.asking();
            if !asking.is_empty() {
                return AskOutcome::Busy;
            }
            let request = pipe::open(&self.pending, Kind::Question);
            asking.insert(request.id().to_string());
            request
        };
        let id = request.id().to_string();
        if let Err(err) = self.app.emit_to(crate::island::WINDOW_LABEL, "hook", card_payload(&id, &ask)) {
            crate::log::line(format!("mcp: the question could not be sent to the island: {err}"));
            pipe::decline(&self.pending, &id);
        }
        let asked = request.wait(&self.pending, ask.wait).await;
        self.asking().remove(&id);
        let (outcome, withdraw) = outcome_of(asked);
        if withdraw {
            // The island may still be showing the card, with nobody left to take a click: the
            // server stopped, the time ran out, or the card never made it. One the island let go
            // of itself is not on screen any more, and the page ignores the id then.
            if let Err(err) = self.app.emit_to(crate::island::WINDOW_LABEL, "hook-withdraw", withdraw_payload(&id)) {
                crate::log::line(format!("mcp: the question could not be withdrawn: {err}"));
            }
        }
        outcome
    }
}

/// What the tool answers for how the request ended, and whether the island is told to take the
/// card down: always, unless a click answered it.
fn outcome_of(asked: Asked) -> (AskOutcome, bool) {
    match asked {
        Asked::Answer(line) => match parse_answer(&line) {
            Some(answers) => (AskOutcome::Answered(answers), false),
            // A reply that is no answer ends the card without one.
            None => (AskOutcome::Released, true),
        },
        Asked::Released => (AskOutcome::Released, true),
        Asked::NotShown => (AskOutcome::NotShown, true),
        Asked::TimedOut => (AskOutcome::TimedOut, true),
    }
}

/// The `hook-withdraw` event: the island takes the card of this request down.
fn withdraw_payload(request_id: &str) -> Value {
    json!({ "request_id": request_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_server::tools::{AskOption, AskRequest};
    use std::time::Duration;

    fn ask() -> AskRequest {
        AskRequest {
            asker: "Codex".into(),
            header: "Deploy".into(),
            question: "Deploy now?".into(),
            options: vec![
                AskOption { label: "Yes".into(), description: String::new() },
                AskOption { label: "No".into(), description: "Wait for CI".into() },
            ],
            multi_select: false,
            wait: Duration::from_secs(45),
        }
    }

    /// The card the island's page reads: `scripts/checks/mcp-card.mjs` runs the same file through
    /// `parseQuestions` and the card's own filter, so the two sides cannot drift apart.
    #[test]
    fn the_card_is_what_the_islands_page_takes_for_a_question() {
        let payload = card_payload("1234-7", &ask());
        let expected: Value = serde_json::from_str(include_str!("../../../scripts/checks/fixtures/mcp-ask-card.json")).unwrap();
        assert_eq!(payload, expected);
    }

    #[test]
    fn the_island_is_told_to_take_the_card_down_unless_a_click_answered_it() {
        // A stopped server releases the request from the Rust side: the card is still on the page.
        for (asked, outcome) in [
            (Asked::Released, AskOutcome::Released),
            (Asked::NotShown, AskOutcome::NotShown),
            (Asked::TimedOut, AskOutcome::TimedOut),
            // A reply that is not an answer line.
            (Asked::Answer("release".into()), AskOutcome::Released),
        ] {
            assert_eq!(outcome_of(asked.clone()), (outcome, true), "{asked:?}");
        }
        // The click that answered it closed the card itself.
        let (outcome, withdraw) = outcome_of(Asked::Answer("answer {\"Q?\":\"Yes\"}".into()));
        assert_eq!(outcome, AskOutcome::Answered(json!({ "Q?": "Yes" })));
        assert!(!withdraw);
    }

    #[test]
    fn the_withdraw_event_names_the_request() {
        // The page reads `request_id`, as on the `hook` event (island/hooks.ts, `withdrawCard`).
        assert_eq!(withdraw_payload("4321-9"), json!({ "request_id": "4321-9" }));
    }

    #[test]
    fn an_app_is_open_when_a_process_of_its_exe_owns_a_window() {
        let windowed: HashSet<String> = ["cursor.exe".to_string()].into_iter().collect();
        assert_eq!(open_now(Some("Cursor.exe"), Some(&windowed)), Some(true));
        assert_eq!(open_now(Some("Code.exe"), Some(&windowed)), Some(false));
        // No exe to look for (Claude, Codex), or no way to read the windows: unknown, not closed.
        assert_eq!(open_now(None, Some(&windowed)), None);
        assert_eq!(open_now(Some("Cursor.exe"), None), None);
        let tiles = json!([{ "id": "cursor" }, { "id": "claude" }]);
        assert_eq!(with_open(tiles, &[Some(true), None]), json!([{ "id": "cursor", "open": true }, { "id": "claude", "open": null }]));
    }

    #[test]
    fn a_multi_select_card_says_so() {
        let mut a = ask();
        a.multi_select = true;
        assert_eq!(card_payload("1", &a)["tool_input"]["questions"][0]["multiSelect"], true);
    }

    #[test]
    fn a_card_answer_line_is_read_back_and_anything_else_is_not() {
        assert_eq!(parse_answer("answer {\"Q?\":\"Yes\"}"), Some(json!({ "Q?": "Yes" })));
        assert_eq!(parse_answer("answer {\"Q?\":[\"a\",\"b\"]}"), Some(json!({ "Q?": ["a", "b"] })));
        for line in ["allow", "release", "answer", "answer ", "answer [1]", "answer not json", "answer \"x\""] {
            assert_eq!(parse_answer(line), None, "{line:?}");
        }
    }
}
