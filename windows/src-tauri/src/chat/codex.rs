//! Codex, driven as `codex exec --json`, one process per turn. Checked against codex-cli 0.162.1
//! on Windows (real output in `fixtures/codex_*.jsonl`):
//!
//!   * The prompt goes on stdin with a trailing `-` (also for `exec resume`); without a closed
//!     stdin Codex prints "Reading additional input from stdin..." and waits.
//!   * Events: `thread.started {thread_id}`, `turn.started`, `item.started` / `item.completed`
//!     with `item.type` `agent_message {text}`, `command_execution {command, aggregated_output,
//!     exit_code, status}` or `error {message}`, `turn.completed`, `turn.failed {error: {message}}`
//!     and a top-level `error {message}` (retries among them: "Reconnecting... 2/5 …").
//!     Other item types exist (file changes, MCP calls, searches…): they are shown as a tool line
//!     by their `type`; their fields have not been seen in a real run here.
//!   * `exec` has no approval channel: it never asks, and `exec resume` has no `-s`. So Mayak
//!     passes `-c approval_policy="never"` and the sandbox the user picked (`-s` for a new
//!     conversation, `-c sandbox_mode=` when resuming) and what the sandbox refuses simply fails.
//!     Real approvals exist only in `codex app-server` (its schema has CommandExecutionRequestApproval…).
//!   * `codex login status` exits 0 with "Logged in using …", or 1 with "Not logged in";
//!     `codex debug models` prints the catalog `{"models": [{"slug", "display_name", "visibility"}]}`.

use std::collections::HashSet;

use serde_json::Value;

use super::agent::*;
use super::mcp::McpApply;

pub struct Codex;

impl Adapter for Codex {
    fn label(&self) -> &'static str {
        "Codex"
    }

    fn stem(&self) -> &'static str {
        "codex"
    }

    fn verification(&self) -> Verification {
        Verification::checked("0.162.1")
    }

    fn models(&self) -> Vec<ModelChoice> {
        vec![ModelChoice::new("", "")]
    }

    fn modes(&self) -> &'static [&'static str] {
        // Never danger-full-access: the sandbox stays.
        &["readOnly", "workspaceWrite"]
    }

    fn build(&self, turn: &TurnSpec<'_>) -> Result<Invocation, ChatError> {
        check_model(turn.model)?;
        let sandbox = match turn.mode {
            "readOnly" => "read-only",
            "workspaceWrite" => "workspace-write",
            other => return Err(bad_mode(other)),
        };
        let mut args: Vec<String> = vec!["exec".into()];
        if let Some(id) = turn.resume {
            check_resume(id)?;
            args.push("resume".into());
        }
        args.extend(["--json", "--skip-git-repo-check", "-c", r#"approval_policy="never""#].map(String::from));
        if turn.resume.is_some() {
            args.extend(["-c".to_string(), format!("sandbox_mode=\"{sandbox}\"")]);
        } else {
            args.extend(["-s".to_string(), sandbox.to_string()]);
        }
        if !turn.model.is_empty() {
            args.extend(["-m".to_string(), turn.model.to_string()]);
        }
        let mut env = Vec::new();
        let mcp = match turn.mcp {
            Some(attach) => {
                args.extend(attach.args.iter().cloned());
                env.extend(attach.env.iter().cloned());
                McpApply::Applied
            }
            None => McpApply::None,
        };
        if let Some(id) = turn.resume {
            args.push(id.to_string());
        }
        args.push("-".into());
        Ok(Invocation { args, env, stdin: turn.prompt.as_bytes().to_vec(), close_stdin: true, mcp })
    }

    fn parser(&self) -> Box<dyn Parser> {
        Box::new(CodexParser::default())
    }

    fn login_probe(&self) -> Option<Probe> {
        Some(Probe::new(&["login", "status"], 15))
    }

    fn parse_login(&self, exit: Option<i32>, output: &str) -> Login {
        let text = output.trim();
        if text.contains("Not logged in") {
            Login::NotLoggedIn
        } else if exit == Some(0) && text.starts_with("Logged in") {
            Login::Ready
        } else {
            Login::Unknown
        }
    }

    fn models_probe(&self) -> Option<Probe> {
        Some(Probe::new(&["debug", "models"], 20))
    }

    fn parse_models(&self, output: &str) -> Option<Vec<ModelChoice>> {
        // The CLI may print log lines before the JSON: start at the first brace.
        let json = &output[output.find('{')?..];
        let catalog: Value = serde_json::from_str(json).ok()?;
        let mut models = vec![ModelChoice::new("", "")];
        for m in catalog.get("models")?.as_array()? {
            if m.get("visibility").and_then(Value::as_str) != Some("list") {
                continue;
            }
            let Some(slug) = m.get("slug").and_then(Value::as_str).filter(|s| check_model(s).is_ok()) else { continue };
            let name = m.get("display_name").and_then(Value::as_str).unwrap_or(slug);
            models.push(ModelChoice::new(slug, name));
        }
        Some(models)
    }
}

#[derive(Default)]
struct CodexParser {
    /// Items whose start was reported, so a completion without a start still shows its line.
    started: HashSet<String>,
}

impl Parser for CodexParser {
    fn line(&mut self, line: &str) -> Vec<Ev> {
        let Ok(d) = serde_json::from_str::<Value>(line) else { return Vec::new() };
        match d.get("type").and_then(Value::as_str) {
            Some("thread.started") => match d.get("thread_id").and_then(Value::as_str) {
                Some(id) => vec![Ev::Session { id: id.to_string(), model: None, api_key_source: None }],
                None => Vec::new(),
            },
            Some("item.started") => self.item(&d, false),
            Some("item.completed") => self.item(&d, true),
            Some("turn.completed") => vec![Ev::TurnEnd(Outcome::Done)],
            Some("turn.failed") => {
                let raw = d.pointer("/error/message").and_then(Value::as_str).unwrap_or("The turn failed.");
                let text = error_text(raw);
                let login = text.contains("401 Unauthorized");
                vec![Ev::TurnEnd(Outcome::Failed { text, login })]
            }
            Some("error") => {
                let raw = d.get("message").and_then(Value::as_str).unwrap_or_default();
                if raw.is_empty() {
                    Vec::new()
                } else if raw.starts_with("Reconnecting...") {
                    vec![Ev::Status(raw.to_string())]
                } else {
                    // The failing turn's own `turn.failed` follows with the same text.
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }
}

impl CodexParser {
    fn item(&mut self, d: &Value, completed: bool) -> Vec<Ev> {
        let Some(item) = d.get("item") else { return Vec::new() };
        let (Some(id), Some(kind)) = (item.get("id").and_then(Value::as_str), item.get("type").and_then(Value::as_str)) else {
            return Vec::new();
        };
        match kind {
            "agent_message" => match item.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                Some(text) if completed => vec![Ev::Text(text.to_string())],
                _ => Vec::new(),
            },
            "error" => match item.get("message").and_then(Value::as_str) {
                Some(message) if completed => vec![Ev::Notice(message.to_string())],
                _ => Vec::new(),
            },
            // The model's own reasoning is not shown.
            "reasoning" => Vec::new(),
            _ => {
                let (name, summary) = if kind == "command_execution" {
                    ("Shell".to_string(), item.get("command").and_then(Value::as_str).map(|c| clip(c, 160)).unwrap_or_default())
                } else {
                    (kind.to_string(), summarize(kind, item))
                };
                let mut out = Vec::new();
                if self.started.insert(id.to_string()) {
                    out.push(Ev::ToolStart { id: id.to_string(), name, summary });
                }
                if completed {
                    let status = item.get("status").and_then(Value::as_str);
                    let exit = item.get("exit_code").and_then(Value::as_i64);
                    let ok = status.is_none_or(|s| s == "completed") && exit.is_none_or(|c| c == 0);
                    let output = item
                        .get("aggregated_output")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|t| !t.is_empty())
                        .map(|t| clip(t, 600));
                    out.push(Ev::ToolEnd { id: id.to_string(), ok, output });
                }
                out
            }
        }
    }
}

/// Codex wraps API errors as a JSON string (`{"type":"error","status":400,"error":{"message":…}}`):
/// the inner message is the readable one.
fn error_text(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn turn<'a>(mode: &'a str, model: &'a str, resume: Option<&'a str>) -> TurnSpec<'a> {
        TurnSpec { cwd: Path::new("."), prompt: "line one\nline \"two\"", model, mode, resume, mcp: None }
    }

    fn parse(fixture: &str) -> Vec<Ev> {
        let mut p = CodexParser::default();
        fixture.lines().flat_map(|l| p.line(l)).collect()
    }

    #[test]
    fn a_new_conversation_is_exec_with_the_sandbox_flag_and_the_prompt_on_stdin() {
        let inv = Codex.build(&turn("readOnly", "gpt-6.1-sol", None)).unwrap();
        assert_eq!(
            inv.args,
            ["exec", "--json", "--skip-git-repo-check", "-c", "approval_policy=\"never\"", "-s", "read-only", "-m", "gpt-6.1-sol", "-"]
        );
        assert_eq!(inv.stdin, b"line one\nline \"two\"");
        assert!(inv.close_stdin);
    }

    #[test]
    fn a_resumed_conversation_sets_the_sandbox_by_config_and_names_the_thread() {
        let id = "01a13656-8d5e-7880-b625-a5eb161ec4f7";
        let inv = Codex.build(&turn("workspaceWrite", "", Some(id))).unwrap();
        assert_eq!(
            inv.args,
            [
                "exec", "resume", "--json", "--skip-git-repo-check", "-c", "approval_policy=\"never\"", "-c",
                "sandbox_mode=\"workspace-write\"", id, "-"
            ]
        );
    }

    #[test]
    fn what_the_mcp_server_hands_over_goes_before_the_prompt_marker_with_its_token_in_the_environment() {
        let attach = super::super::mcp::McpAttach {
            args: vec!["-c".into(), "mcp_servers.mayak.url=\"http://127.0.0.1:1/mcp\"".into()],
            env: vec![("MAYAK_MCP_TOKEN".into(), "secret-token".into())],
        };
        let mut t = turn("readOnly", "", Some("01a13656-8d5e-7880-b625-a5eb161ec4f7"));
        t.mcp = Some(&attach);
        let inv = Codex.build(&t).unwrap();
        let n = inv.args.len();
        assert_eq!(inv.args[n - 4..], ["-c", "mcp_servers.mayak.url=\"http://127.0.0.1:1/mcp\"", "01a13656-8d5e-7880-b625-a5eb161ec4f7", "-"]);
        assert_eq!(inv.env, [("MAYAK_MCP_TOKEN".to_string(), "secret-token".to_string())]);
        assert!(inv.args.iter().all(|a| !a.contains("secret-token")));
        assert_eq!(inv.mcp, McpApply::Applied);
    }

    #[test]
    fn the_sandbox_cannot_be_left() {
        for mode in ["danger-full-access", "dangerFullAccess", "never", "", "ask"] {
            assert_eq!(Codex.build(&turn(mode, "", None)).unwrap_err().code, "badMode", "{mode}");
        }
        assert_eq!(Codex.modes(), ["readOnly", "workspaceWrite"]);
        assert_eq!(Codex.build(&turn("readOnly", "-m", None)).unwrap_err().code, "badModel");
    }

    #[test]
    fn a_text_turn_gives_the_thread_the_answer_and_the_end() {
        let evs = parse(include_str!("fixtures/codex_text.jsonl"));
        assert_eq!(
            evs,
            [
                Ev::Session { id: "01a13655-87f4-73d2-91e3-5ed13cda08cc".into(), model: None, api_key_source: None },
                Ev::Text("pong".into()),
                Ev::TurnEnd(Outcome::Done)
            ]
        );
    }

    #[test]
    fn a_command_is_a_tool_line_that_ends_ok() {
        let evs = parse(include_str!("fixtures/codex_command_ok.jsonl"));
        assert!(evs.contains(&Ev::ToolStart { id: "item_1".into(), name: "Shell".into(), summary: evs_summary(&evs) }));
        assert!(evs.contains(&Ev::ToolEnd { id: "item_1".into(), ok: true, output: Some("hi".into()) }));
        assert_eq!(evs.last(), Some(&Ev::TurnEnd(Outcome::Done)));
        assert!(evs.contains(&Ev::Text("The command output was `hi`.".into())));
    }

    fn evs_summary(evs: &[Ev]) -> String {
        evs.iter().find_map(|e| if let Ev::ToolStart { summary, .. } = e { Some(summary.clone()) } else { None }).unwrap()
    }

    #[test]
    fn a_command_the_sandbox_refused_ends_failed_with_the_reason() {
        let evs = parse(include_str!("fixtures/codex_command_failed.jsonl"));
        let ends: Vec<_> = evs.iter().filter(|e| matches!(e, Ev::ToolEnd { .. })).collect();
        assert_eq!(ends.len(), 2);
        assert!(ends.iter().all(|e| matches!(e, Ev::ToolEnd { ok: false, output: Some(o), .. } if o.contains("setup refresh had errors"))));
        assert_eq!(evs.last(), Some(&Ev::TurnEnd(Outcome::Done)), "the turn itself went on and ended");
    }

    #[test]
    fn a_failed_turn_gives_the_inner_api_message() {
        let evs = parse(include_str!("fixtures/codex_turn_failed.jsonl"));
        let Some(Ev::TurnEnd(Outcome::Failed { text, login })) = evs.last() else { panic!("{evs:?}") };
        assert_eq!(text, "The 'definitely-not-a-model' model is not supported when using Codex with a ChatGPT account.");
        assert!(!login);
        assert!(evs.iter().any(|e| matches!(e, Ev::Notice(n) if n.contains("Model metadata"))), "the warning item is a notice");
    }

    #[test]
    fn the_real_usage_limit_failure_is_recognised() {
        let evs = parse(include_str!("fixtures/codex_usage_limit.jsonl"));
        let Some(Ev::TurnEnd(Outcome::Failed { text, login })) = evs.last() else { panic!("{evs:?}") };
        assert!(!login);
        assert!(text.contains("try again at 1:23 AM"));
        assert!(super::super::agent::is_usage_limit(text));
        assert!(!super::super::agent::is_usage_limit("The model is not supported when using Codex with a ChatGPT account."));
    }

    #[test]
    fn a_signed_out_run_shows_the_reconnects_as_status_and_ends_with_the_login_flag() {
        let evs = parse(include_str!("fixtures/codex_loggedout.jsonl"));
        assert!(evs.iter().any(|e| matches!(e, Ev::Status(s) if s.starts_with("Reconnecting... 2/5"))));
        let Some(Ev::TurnEnd(Outcome::Failed { login, text })) = evs.last() else { panic!("{evs:?}") };
        assert!(*login && text.contains("401 Unauthorized"));
    }

    #[test]
    fn login_and_models_are_read_from_the_real_outputs() {
        assert_eq!(Codex.parse_login(Some(0), "Logged in using ChatGPT\n"), Login::Ready);
        assert_eq!(Codex.parse_login(Some(1), "Not logged in\n"), Login::NotLoggedIn);
        assert_eq!(Codex.parse_login(Some(0), "something else"), Login::Unknown);
        let catalog = r#"warning line
{"models":[
  {"slug":"gpt-6.1-sol","display_name":"GPT-6.1-Sol","visibility":"list"},
  {"slug":"gpt-daybreak-blue-latest","display_name":"Daybreak Blue","visibility":"hide"},
  {"slug":"--evil","display_name":"x","visibility":"list"},
  {"slug":"gpt-5.5","visibility":"list"}]}"#;
        assert_eq!(
            Codex.parse_models(catalog),
            Some(vec![ModelChoice::new("", ""), ModelChoice::new("gpt-6.1-sol", "GPT-6.1-Sol"), ModelChoice::new("gpt-5.5", "gpt-5.5")])
        );
        assert_eq!(Codex.parse_models("no json"), None);
    }

    #[test]
    fn an_unknown_item_type_is_a_tool_line_by_its_type() {
        let mut p = CodexParser::default();
        let started = r#"{"type":"item.started","item":{"id":"item_7","type":"web_search","query":"tauri"}}"#;
        let done = r#"{"type":"item.completed","item":{"id":"item_7","type":"web_search","query":"tauri"}}"#;
        let evs: Vec<Ev> = [started, done].iter().flat_map(|l| p.line(l)).collect();
        assert_eq!(
            evs,
            [
                Ev::ToolStart { id: "item_7".into(), name: "web_search".into(), summary: "tauri".into() },
                Ev::ToolEnd { id: "item_7".into(), ok: true, output: None }
            ]
        );
    }
}
