//! Cursor Agent (`cursor-agent`), driven with `-p --output-format stream-json
//! --stream-partial-output`, one process per turn.
//!
//! **UNVERIFIED**: Cursor Agent is not installed on the machine this was written on, so nothing
//! here was run. It follows cursor.com/docs/cli (parameters and output format). Those pages were
//! read through a summarising fetch tool, not verbatim, and show event fields through jq filters
//! rather than complete samples; the tests use lines written from that description, not captured
//! output. Things to check on a machine that has the CLI:
//!   * the exact binary name on Windows (`cursor-agent`, or `agent`) and whether it is a `.cmd`
//!     shim (then a prompt with a line break or a quote cannot be passed as an argument: it is refused);
//!   * `--resume <id>` with the `session_id` of the init event, and `--workspace`;
//!   * the field names of `tool_call` events other than read and write;
//!   * how a not-signed-in run fails (the CLI has `status` / `login`; its output is not parsed).
//!
//! Without `--force` the agent only proposes changes: that is the one mode offered here.
//! MCP servers are configured in `.cursor/mcp.json` only, and print mode has no approval channel.

use serde_json::Value;

use super::agent::*;
use super::mcp::McpApply;

pub struct CursorAgent;

/// Longest prompt passed as an argument: Windows ends a command line at 32,767 characters.
const MAX_PROMPT_ARG: usize = 8_000;

impl Adapter for CursorAgent {
    fn label(&self) -> &'static str {
        "Cursor Agent"
    }

    fn stem(&self) -> &'static str {
        "cursor-agent"
    }

    fn verification(&self) -> Verification {
        Verification::from_docs()
    }

    fn models(&self) -> Vec<ModelChoice> {
        // `cursor-agent models` lists them, in a format not seen here: not parsed.
        vec![ModelChoice::new("", "")]
    }

    fn modes(&self) -> &'static [&'static str] {
        &["proposeOnly"]
    }

    fn build(&self, turn: &TurnSpec<'_>) -> Result<Invocation, ChatError> {
        check_model(turn.model)?;
        if turn.mode != "proposeOnly" {
            return Err(bad_mode(turn.mode));
        }
        if turn.prompt.chars().count() > MAX_PROMPT_ARG {
            return Err(ChatError::new("promptTooLong", "That message is too long for Cursor Agent's command line."));
        }
        if turn.prompt.starts_with('-') {
            // The prompt is a positional argument: one that starts with a dash would be read as an option.
            return Err(ChatError::new("promptNotPassable", "Cursor Agent cannot take a message that starts with \"-\"."));
        }
        let mut args: Vec<String> =
            ["-p", "--output-format", "stream-json", "--stream-partial-output"].iter().map(|a| a.to_string()).collect();
        args.extend(["--workspace".to_string(), turn.cwd.to_string_lossy().into_owned()]);
        if !turn.model.is_empty() {
            args.extend(["--model".to_string(), turn.model.to_string()]);
        }
        if let Some(id) = turn.resume {
            check_resume(id)?;
            args.extend(["--resume".to_string(), id.to_string()]);
        }
        args.push(turn.prompt.to_string());
        let mcp = if turn.mcp.is_none() {
            McpApply::None
        } else {
            McpApply::Unsupported("Cursor Agent takes MCP servers from .cursor/mcp.json only")
        };
        Ok(Invocation { args, env: Vec::new(), stdin: Vec::new(), close_stdin: true, mcp })
    }

    fn parser(&self) -> Box<dyn Parser> {
        Box::new(CursorParser::default())
    }
}

#[derive(Default)]
struct CursorParser {
    /// Some text came as deltas this turn; else the final `result` text is the answer.
    streamed: bool,
}

impl Parser for CursorParser {
    fn line(&mut self, line: &str) -> Vec<Ev> {
        let Ok(d) = serde_json::from_str::<Value>(line) else { return Vec::new() };
        match d.get("type").and_then(Value::as_str) {
            Some("system") if d.get("subtype").and_then(Value::as_str) == Some("init") => {
                match d.get("session_id").and_then(Value::as_str) {
                    Some(id) => vec![Ev::Session {
                        id: id.to_string(),
                        model: d.get("model").and_then(Value::as_str).map(str::to_string),
                        api_key_source: d.get("apiKeySource").and_then(Value::as_str).map(str::to_string),
                    }],
                    None => Vec::new(),
                }
            }
            Some("assistant") => {
                // With --stream-partial-output only an event with `timestamp_ms` and no
                // `model_call_id` holds new text; the other two kinds repeat it.
                if d.get("timestamp_ms").is_none() || d.get("model_call_id").is_some() {
                    return Vec::new();
                }
                let text: String = d
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .map(|blocks| blocks.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect())
                    .unwrap_or_default();
                if text.is_empty() {
                    return Vec::new();
                }
                self.streamed = true;
                vec![Ev::TextDelta(text)]
            }
            Some("tool_call") => Self::tool_call(&d),
            Some("result") => {
                let is_error = d.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let text = d.get("result").and_then(Value::as_str).unwrap_or_default();
                if is_error {
                    let text = if text.is_empty() { "The turn failed." } else { text };
                    return vec![Ev::TurnEnd(Outcome::Failed { text: text.to_string(), login: false })];
                }
                let mut out = Vec::new();
                if !self.streamed && !text.is_empty() {
                    out.push(Ev::Text(text.to_string()));
                }
                out.push(Ev::TurnEnd(Outcome::Done));
                out
            }
            _ => Vec::new(),
        }
    }
}

impl CursorParser {
    fn tool_call(d: &Value) -> Vec<Ev> {
        let (Some(id), Some(tool_call)) = (d.get("call_id").and_then(Value::as_str), d.get("tool_call").and_then(Value::as_object))
        else {
            return Vec::new();
        };
        let Some((key, body)) = tool_call.iter().next() else { return Vec::new() };
        match d.get("subtype").and_then(Value::as_str) {
            Some("started") => {
                // `readToolCall` → "read"; the generic form is `function {name, arguments}`.
                let (name, args) = if key == "function" {
                    let name = body.get("name").and_then(Value::as_str).unwrap_or("function").to_string();
                    let args = match body.get("arguments") {
                        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
                        Some(other) => other.clone(),
                        None => Value::Null,
                    };
                    (name, args)
                } else {
                    (key.strip_suffix("ToolCall").unwrap_or(key).to_string(), body.get("args").cloned().unwrap_or(Value::Null))
                };
                let summary = summarize(&name, &args);
                vec![Ev::ToolStart { id: id.to_string(), name, summary }]
            }
            Some("completed") => {
                let result = body.get("result");
                let failed = result.is_some_and(|r| r.get("error").is_some() || r.get("rejected").is_some());
                vec![Ev::ToolEnd { id: id.to_string(), ok: !failed, output: None }]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn turn<'a>(prompt: &'a str, resume: Option<&'a str>) -> TurnSpec<'a> {
        TurnSpec { cwd: Path::new("proj"), prompt, model: "", mode: "proposeOnly", resume, mcp: None }
    }

    // Lines written from the documented fields, not captured from a run.
    const STREAM: &str = r#"{"type":"system","subtype":"init","apiKeySource":"login","cwd":"/p","session_id":"s-1","model":"Sonnet 4","permissionMode":"default"}
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]},"session_id":"s-1"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hel"}]},"session_id":"s-1","timestamp_ms":1}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"lo"}]},"session_id":"s-1","timestamp_ms":2}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hello"}]},"session_id":"s-1","timestamp_ms":3,"model_call_id":"m-1"}
{"type":"tool_call","subtype":"started","call_id":"c1","tool_call":{"readToolCall":{"args":{"path":"a.txt"}}},"session_id":"s-1"}
{"type":"tool_call","subtype":"completed","call_id":"c1","tool_call":{"readToolCall":{"args":{"path":"a.txt"},"result":{"success":{"totalLines":3}}}},"session_id":"s-1"}
{"type":"tool_call","subtype":"started","call_id":"c2","tool_call":{"function":{"name":"shell","arguments":"{\"command\":\"ls\"}"}},"session_id":"s-1"}
{"type":"tool_call","subtype":"completed","call_id":"c2","tool_call":{"function":{"name":"shell","result":{"rejected":{}}}},"session_id":"s-1"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hello"}]},"session_id":"s-1"}
{"type":"result","subtype":"success","is_error":false,"duration_ms":10,"result":"Hello","session_id":"s-1"}"#;

    #[test]
    fn only_the_delta_events_carry_new_text() {
        let mut p = CursorParser::default();
        let evs: Vec<Ev> = STREAM.lines().flat_map(|l| p.line(l)).collect();
        assert_eq!(
            evs,
            [
                Ev::Session { id: "s-1".into(), model: Some("Sonnet 4".into()), api_key_source: Some("login".into()) },
                Ev::TextDelta("Hel".into()),
                Ev::TextDelta("lo".into()),
                Ev::ToolStart { id: "c1".into(), name: "read".into(), summary: "a.txt".into() },
                Ev::ToolEnd { id: "c1".into(), ok: true, output: None },
                Ev::ToolStart { id: "c2".into(), name: "shell".into(), summary: "ls".into() },
                Ev::ToolEnd { id: "c2".into(), ok: false, output: None },
                Ev::TurnEnd(Outcome::Done),
            ]
        );
    }

    #[test]
    fn a_turn_with_no_deltas_takes_its_answer_from_the_result() {
        let mut p = CursorParser::default();
        let line = r#"{"type":"result","subtype":"success","is_error":false,"result":"All done","session_id":"s"}"#;
        assert_eq!(p.line(line), [Ev::Text("All done".into()), Ev::TurnEnd(Outcome::Done)]);
        let mut p = CursorParser::default();
        let err = r#"{"type":"result","subtype":"error","is_error":true,"result":"Not authenticated","session_id":"s"}"#;
        assert_eq!(p.line(err), [Ev::TurnEnd(Outcome::Failed { text: "Not authenticated".into(), login: false })]);
    }

    #[test]
    fn the_command_line_follows_the_documented_flags_and_the_prompt_is_last() {
        let inv = CursorAgent.build(&turn("hello there", Some("s-1"))).unwrap();
        assert_eq!(
            inv.args,
            ["-p", "--output-format", "stream-json", "--stream-partial-output", "--workspace", "proj", "--resume", "s-1", "hello there"]
        );
        assert!(inv.stdin.is_empty() && inv.close_stdin);
    }

    #[test]
    fn force_is_never_offered_and_a_prompt_cannot_pose_as_an_option() {
        assert_eq!(CursorAgent.modes(), ["proposeOnly"]);
        let mut t = turn("x", None);
        t.mode = "force";
        assert_eq!(CursorAgent.build(&t).unwrap_err().code, "badMode");
        assert_eq!(CursorAgent.build(&turn("--force do it", None)).unwrap_err().code, "promptNotPassable");
        assert_eq!(CursorAgent.build(&turn(&"x".repeat(8_001), None)).unwrap_err().code, "promptTooLong");
        assert!(!CursorAgent.verification().verified);
    }
}
