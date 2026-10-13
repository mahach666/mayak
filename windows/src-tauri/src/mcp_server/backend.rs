// The MCP server's view of the running app: the observer's snapshot, the Stats and Apps tabs, and
// the island's question card.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use super::tools::{AskOutcome, AskRequest, Backend};
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
        let tiles = tauri::async_runtime::spawn_blocking(move || crate::apps::list(&settings)).await.unwrap_or_else(|_| {
            crate::log::line("mcp: the apps lookup stopped");
            Vec::new()
        });
        serde_json::to_value(tiles).unwrap_or(Value::Null)
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
        match asked {
            Asked::Answer(line) => match parse_answer(&line) {
                Some(answers) => AskOutcome::Answered(answers),
                None => AskOutcome::Released,
            },
            Asked::Released => AskOutcome::Released,
            other => {
                // The island may still be showing it: take it down, or a click would go nowhere.
                if let Err(err) = self.app.emit_to(crate::island::WINDOW_LABEL, "hook-withdraw", json!({ "request_id": id })) {
                    crate::log::line(format!("mcp: the question could not be withdrawn: {err}"));
                }
                if other == Asked::NotShown { AskOutcome::NotShown } else { AskOutcome::TimedOut }
            }
        }
    }
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
