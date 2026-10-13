//! Gemini CLI, driven headless with `--output-format stream-json`, one process per turn.
//!
//! **UNVERIFIED**: Gemini CLI is not installed on the machine this was written on, so nothing
//! here was run. It follows the official sources: the event types are the TypeScript definitions
//! in `packages/core/src/output/types.ts` (init, message, tool_use, tool_result, error, result),
//! the flags are `docs/cli/cli-reference.md` (`--output-format`, `-m`, `--approval-mode`,
//! `--resume`) and `docs/cli/headless.md` (headless runs when stdin is not a terminal).
//! The tests use lines written from those definitions, not captured output. Things to check on a
//! machine that has the CLI before the picker offers it without the "unverified" tag:
//!   * the prompt on stdin without `-p` is read as the prompt;
//!   * `--resume <session id>` takes the id of the `init` event;
//!   * whether a final non-delta assistant message repeats the streamed deltas;
//!   * how a not-signed-in run fails (there is no status command; the CLI's message is shown).
//!
//! No command-line way to add an MCP server exists (they live in `settings.json`), and headless
//! mode has no approval channel (its ACP mode, `--experimental-acp`, has one; not built).

use serde_json::Value;

use super::agent::*;
use super::mcp::McpApply;

pub struct GeminiCli;

impl Adapter for GeminiCli {
    fn label(&self) -> &'static str {
        "Gemini CLI"
    }

    fn stem(&self) -> &'static str {
        "gemini"
    }

    fn verification(&self) -> Verification {
        Verification::from_docs()
    }

    fn models(&self) -> Vec<ModelChoice> {
        // The aliases of the CLI reference.
        [("", ""), ("auto", "Auto"), ("pro", "Pro"), ("flash", "Flash"), ("flash-lite", "Flash-Lite")]
            .iter()
            .map(|(id, label)| ModelChoice::new(id, label))
            .collect()
    }

    fn modes(&self) -> &'static [&'static str] {
        // Not auto_edit or yolo: they approve tool calls without asking.
        &["ask", "plan"]
    }

    fn build(&self, turn: &TurnSpec<'_>) -> Result<Invocation, ChatError> {
        check_model(turn.model)?;
        let mut args: Vec<String> = vec!["--output-format".into(), "stream-json".into()];
        if !turn.model.is_empty() {
            args.extend(["-m".to_string(), turn.model.to_string()]);
        }
        match turn.mode {
            "ask" => {}
            "plan" => args.extend(["--approval-mode".to_string(), "plan".to_string()]),
            other => return Err(bad_mode(other)),
        }
        if let Some(id) = turn.resume {
            check_resume(id)?;
            args.extend(["--resume".to_string(), id.to_string()]);
        }
        let mcp = if turn.mcp.is_none() {
            McpApply::None
        } else {
            McpApply::Unsupported("Gemini CLI takes MCP servers from its settings.json only")
        };
        Ok(Invocation { args, env: Vec::new(), stdin: turn.prompt.as_bytes().to_vec(), close_stdin: true, mcp })
    }

    fn parser(&self) -> Box<dyn Parser> {
        Box::new(GeminiParser)
    }
}

struct GeminiParser;

impl Parser for GeminiParser {
    fn line(&mut self, line: &str) -> Vec<Ev> {
        let Ok(d) = serde_json::from_str::<Value>(line) else { return Vec::new() };
        match d.get("type").and_then(Value::as_str) {
            Some("init") => match d.get("session_id").and_then(Value::as_str) {
                Some(id) => vec![Ev::Session {
                    id: id.to_string(),
                    model: d.get("model").and_then(Value::as_str).map(str::to_string),
                    api_key_source: None,
                }],
                None => Vec::new(),
            },
            Some("message") if d.get("role").and_then(Value::as_str) == Some("assistant") => {
                let Some(content) = d.get("content").and_then(Value::as_str).filter(|c| !c.is_empty()) else {
                    return Vec::new();
                };
                if d.get("delta").and_then(Value::as_bool) == Some(true) {
                    vec![Ev::TextDelta(content.to_string())]
                } else {
                    vec![Ev::Text(content.to_string())]
                }
            }
            Some("tool_use") => {
                let (Some(id), Some(name)) = (d.get("tool_id").and_then(Value::as_str), d.get("tool_name").and_then(Value::as_str))
                else {
                    return Vec::new();
                };
                let params = d.get("parameters").unwrap_or(&Value::Null);
                vec![Ev::ToolStart { id: id.to_string(), name: name.to_string(), summary: summarize(name, params) }]
            }
            Some("tool_result") => {
                let Some(id) = d.get("tool_id").and_then(Value::as_str) else { return Vec::new() };
                let ok = d.get("status").and_then(Value::as_str) == Some("success");
                let output = d
                    .get("output")
                    .and_then(Value::as_str)
                    .or_else(|| d.pointer("/error/message").and_then(Value::as_str))
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| clip(t.trim(), 600));
                vec![Ev::ToolEnd { id: id.to_string(), ok, output }]
            }
            Some("error") => match d.get("message").and_then(Value::as_str) {
                Some(message) => vec![Ev::Notice(message.to_string())],
                None => Vec::new(),
            },
            Some("result") => {
                if d.get("status").and_then(Value::as_str) == Some("success") {
                    vec![Ev::TurnEnd(Outcome::Done)]
                } else {
                    let text = d.pointer("/error/message").and_then(Value::as_str).unwrap_or("The turn failed.").to_string();
                    vec![Ev::TurnEnd(Outcome::Failed { text, login: false })]
                }
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn turn<'a>(mode: &'a str, model: &'a str, resume: Option<&'a str>) -> TurnSpec<'a> {
        TurnSpec { cwd: Path::new("."), prompt: "hello", model, mode, resume, mcp: None }
    }

    // Lines written from packages/core/src/output/types.ts, not captured from a run.
    const STREAM: &str = r#"{"type":"init","timestamp":"2026-10-13T10:00:00Z","session_id":"abc-123","model":"gemini-2.5-pro"}
{"type":"message","timestamp":"t","role":"user","content":"hello"}
{"type":"message","timestamp":"t","role":"assistant","content":"Hel","delta":true}
{"type":"message","timestamp":"t","role":"assistant","content":"lo!","delta":true}
{"type":"tool_use","timestamp":"t","tool_name":"read_file","tool_id":"t1","parameters":{"path":"a.txt"}}
{"type":"tool_result","timestamp":"t","tool_id":"t1","status":"success","output":"contents"}
{"type":"tool_use","timestamp":"t","tool_name":"run_shell_command","tool_id":"t2","parameters":{"command":"rm x"}}
{"type":"tool_result","timestamp":"t","tool_id":"t2","status":"error","error":{"type":"denied","message":"needs approval"}}
{"type":"error","timestamp":"t","severity":"warning","message":"Rate limited, retrying"}
{"type":"result","timestamp":"t","status":"success","stats":{"total_tokens":1,"input_tokens":1,"output_tokens":0,"cached":0,"input":1,"duration_ms":5,"tool_calls":2,"models":{}}}"#;

    #[test]
    fn the_documented_stream_maps_to_the_shared_events() {
        let mut p = GeminiParser;
        let evs: Vec<Ev> = STREAM.lines().flat_map(|l| p.line(l)).collect();
        assert_eq!(
            evs,
            [
                Ev::Session { id: "abc-123".into(), model: Some("gemini-2.5-pro".into()), api_key_source: None },
                Ev::TextDelta("Hel".into()),
                Ev::TextDelta("lo!".into()),
                Ev::ToolStart { id: "t1".into(), name: "read_file".into(), summary: "a.txt".into() },
                Ev::ToolEnd { id: "t1".into(), ok: true, output: Some("contents".into()) },
                Ev::ToolStart { id: "t2".into(), name: "run_shell_command".into(), summary: "rm x".into() },
                Ev::ToolEnd { id: "t2".into(), ok: false, output: Some("needs approval".into()) },
                Ev::Notice("Rate limited, retrying".into()),
                Ev::TurnEnd(Outcome::Done),
            ]
        );
    }

    #[test]
    fn a_failed_result_carries_the_cli_message() {
        let mut p = GeminiParser;
        let line = r#"{"type":"result","timestamp":"t","status":"error","error":{"type":"auth","message":"Please set an Auth method"}}"#;
        assert_eq!(p.line(line), [Ev::TurnEnd(Outcome::Failed { text: "Please set an Auth method".into(), login: false })]);
    }

    #[test]
    fn the_command_line_follows_the_cli_reference() {
        let inv = GeminiCli.build(&turn("plan", "flash", Some("abc-123"))).unwrap();
        assert_eq!(inv.args, ["--output-format", "stream-json", "-m", "flash", "--approval-mode", "plan", "--resume", "abc-123"]);
        assert_eq!(inv.stdin, b"hello");
        assert!(inv.close_stdin);
        let plain = GeminiCli.build(&turn("ask", "", None)).unwrap();
        assert_eq!(plain.args, ["--output-format", "stream-json"]);
        for mode in ["yolo", "auto_edit", "default", ""] {
            assert_eq!(GeminiCli.build(&turn(mode, "", None)).unwrap_err().code, "badMode", "{mode}");
        }
    }

    #[test]
    fn what_the_mcp_server_hands_over_is_reported_as_not_passed() {
        let attach = super::super::mcp::McpAttach { args: vec!["--x".into()], env: vec![("K".into(), "v".into())] };
        let mut t = turn("ask", "", None);
        t.mcp = Some(&attach);
        let inv = GeminiCli.build(&t).unwrap();
        assert!(matches!(inv.mcp, McpApply::Unsupported(_)));
        assert!(!inv.args.iter().any(|a| a == "--x") && inv.env.is_empty(), "nothing of it reaches the process");
        assert!(!GeminiCli.verification().verified);
    }
}
