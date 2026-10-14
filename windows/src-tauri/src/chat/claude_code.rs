//! Claude Code, driven as `claude -p` with stream-json in and out. Checked against Claude Code
//! 2.1.296 on Windows (the real output is in `fixtures/claude_*.jsonl`):
//!
//!   * `--input-format stream-json` keeps the process alive between turns: a user turn is one
//!     line `{"type":"user","message":{"role":"user","content":"…"}}` on stdin.
//!   * `--output-format stream-json` needs `--verbose`. `--include-partial-messages` adds the
//!     `stream_event` deltas; the complete `assistant` text that follows them repeats what was
//!     streamed and is skipped.
//!   * `--permission-prompts host` (the default) alone does not give the host a prompt: the CLI
//!     denies it by itself (`system/permission_denied`). The host registers with the hidden
//!     `--permission-prompt-tool stdio` (what the Agent SDK passes): a `control_request` with
//!     `request.subtype == "can_use_tool"` then arrives on stdout and waits for a `control_response`.
//!   * `control_request {subtype: "interrupt"}` ends the turn with `result/error_during_execution`
//!     and `terminal_reason: "aborted_streaming"`.
//!   * `claude auth status` prints `{"loggedIn": bool, …}`; a run that is not signed in says so
//!     in an `assistant` event with `error: "authentication_failed"`.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use super::agent::*;
use super::mcp::McpApply;

pub struct ClaudeCode;

/// The aliases `claude --help` names for `--model`.
const MODELS: [(&str, &str); 5] = [("", ""), ("fable", "Fable"), ("opus", "Opus"), ("sonnet", "Sonnet"), ("haiku", "Haiku")];

impl Adapter for ClaudeCode {
    fn label(&self) -> &'static str {
        "Claude Code"
    }

    fn stem(&self) -> &'static str {
        "claude"
    }

    fn verification(&self) -> Verification {
        Verification::checked("2.1.296")
    }

    fn models(&self) -> Vec<ModelChoice> {
        MODELS.iter().map(|(id, label)| ModelChoice::new(id, label)).collect()
    }

    fn modes(&self) -> &'static [&'static str] {
        // Never bypassPermissions, dontAsk, auto or acceptEdits: each of them has the CLI approve
        // something itself, so no request would reach the island and no click would happen.
        &["ask", "plan"]
    }

    fn persistent(&self) -> bool {
        true
    }

    fn build(&self, turn: &TurnSpec<'_>) -> Result<Invocation, ChatError> {
        check_model(turn.model)?;
        let mut args: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-prompts",
            "host",
            "--permission-prompt-tool",
            "stdio",
        ]
        .iter()
        .map(|a| a.to_string())
        .collect();
        if !turn.model.is_empty() {
            args.extend(["--model".to_string(), turn.model.to_string()]);
        }
        match turn.mode {
            "ask" => {}
            "plan" => args.extend(["--permission-mode".to_string(), "plan".to_string()]),
            other => return Err(bad_mode(other)),
        }
        if let Some(id) = turn.resume {
            check_resume(id)?;
            args.extend(["--resume".to_string(), id.to_string()]);
        }
        // Last: `--mcp-config` is variadic and would take whatever came after it (the prompt is on stdin).
        let mut env = Vec::new();
        let mcp = match turn.mcp {
            Some(attach) => {
                args.extend(attach.args.iter().cloned());
                env.extend(attach.env.iter().cloned());
                McpApply::Applied
            }
            None => McpApply::None,
        };
        Ok(Invocation {
            args,
            env,
            stdin: self.user_line(turn.prompt).unwrap_or_default().into_bytes(),
            close_stdin: false,
            mcp,
        })
    }

    fn parser(&self) -> Box<dyn Parser> {
        Box::new(ClaudeParser::default())
    }

    fn user_line(&self, prompt: &str) -> Option<String> {
        Some(format!("{}\n", json!({ "type": "user", "message": { "role": "user", "content": prompt } })))
    }

    fn permission_reply(&self, ask: &PermissionAsk, allow: bool) -> Option<String> {
        let response = if allow {
            // The SDK protocol wants the input back with an allow; it is handed back unchanged.
            json!({ "behavior": "allow", "updatedInput": ask.input })
        } else {
            json!({ "behavior": "deny", "message": "The user denied this in Mayak." })
        };
        Some(format!(
            "{}\n",
            json!({
                "type": "control_response",
                "response": { "subtype": "success", "request_id": ask.request_id, "response": response }
            })
        ))
    }

    fn interrupt_line(&self) -> Option<String> {
        Some(format!("{}\n", json!({ "type": "control_request", "request_id": "mayak-interrupt", "request": { "subtype": "interrupt" } })))
    }

    fn login_probe(&self) -> Option<Probe> {
        Some(Probe::new(&["auth", "status"], 15))
    }

    fn parse_login(&self, _exit: Option<i32>, output: &str) -> Login {
        match serde_json::from_str::<Value>(output.trim()).ok().and_then(|v| v.get("loggedIn").and_then(Value::as_bool)) {
            Some(true) => Login::Ready,
            Some(false) => Login::NotLoggedIn,
            None => Login::Unknown,
        }
    }
}

#[derive(Default)]
struct ClaudeParser {
    /// The `message.id`s whose text was streamed as deltas.
    streamed: HashSet<String>,
    /// The id of the message being streamed (`message_start`).
    current: Option<String>,
    /// tool_use id → tool name, to name the result of a tool.
    tools: HashMap<String, String>,
    /// An `assistant` event said `authentication_failed`.
    login_failed: bool,
    /// The model the session runs (`system init`): whose context window `result` names.
    model: Option<String>,
}

impl Parser for ClaudeParser {
    fn line(&mut self, line: &str) -> Vec<Ev> {
        let Ok(d) = serde_json::from_str::<Value>(line) else { return Vec::new() };
        let kind = d.get("type").and_then(Value::as_str).unwrap_or_default();
        match kind {
            "system" => self.system(&d),
            "stream_event" => self.stream_event(&d),
            "assistant" => self.assistant(&d),
            "user" => Self::user(&d),
            "control_request" => Self::control_request(&d),
            "result" => self.result(&d),
            "rate_limit_event" => Self::rate_limit(&d),
            _ => Vec::new(),
        }
    }
}

impl ClaudeParser {
    fn system(&mut self, d: &Value) -> Vec<Ev> {
        if d.get("subtype").and_then(Value::as_str) != Some("init") {
            return Vec::new();
        }
        let Some(id) = d.get("session_id").and_then(Value::as_str) else { return Vec::new() };
        self.model = d.get("model").and_then(Value::as_str).map(str::to_string);
        vec![Ev::Session {
            id: id.to_string(),
            model: self.model.clone(),
            api_key_source: d.get("apiKeySource").and_then(Value::as_str).map(str::to_string),
        }]
    }

    fn stream_event(&mut self, d: &Value) -> Vec<Ev> {
        // A subagent's own stream carries a parent tool call: only the main conversation is shown.
        if !d.get("parent_tool_use_id").is_none_or(Value::is_null) {
            return Vec::new();
        }
        let Some(event) = d.get("event") else { return Vec::new() };
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                self.current = event.pointer("/message/id").and_then(Value::as_str).map(str::to_string);
                Vec::new()
            }
            Some("content_block_delta") if event.pointer("/delta/type").and_then(Value::as_str) == Some("text_delta") => {
                let Some(text) = event.pointer("/delta/text").and_then(Value::as_str).filter(|t| !t.is_empty()) else {
                    return Vec::new();
                };
                if let Some(id) = &self.current {
                    self.streamed.insert(id.clone());
                }
                vec![Ev::TextDelta(text.to_string())]
            }
            _ => Vec::new(),
        }
    }

    fn assistant(&mut self, d: &Value) -> Vec<Ev> {
        if !d.get("parent_tool_use_id").is_none_or(Value::is_null) {
            return Vec::new();
        }
        if d.get("error").and_then(Value::as_str) == Some("authentication_failed") {
            self.login_failed = true;
        }
        let message = d.get("message").unwrap_or(&Value::Null);
        let id = message.get("id").and_then(Value::as_str).unwrap_or_default();
        let mut out = Vec::new();
        // What this request sent: the conversation's size as the model saw it.
        if let Some(usage) = message.get("usage") {
            let field = |k: &str| usage.get(k).and_then(Value::as_u64);
            if let Some(input) = field("input_tokens") {
                let context = input + field("cache_creation_input_tokens").unwrap_or(0) + field("cache_read_input_tokens").unwrap_or(0);
                out.push(Ev::Usage { context_tokens: Some(context), context_window: None });
            }
        }
        let Some(blocks) = message.get("content").and_then(Value::as_array) else { return out };
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") if !self.streamed.contains(id) => {
                    if let Some(text) = block.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                        out.push(Ev::Text(text.to_string()));
                    }
                }
                Some("tool_use") => {
                    let (Some(tool_id), Some(name)) =
                        (block.get("id").and_then(Value::as_str), block.get("name").and_then(Value::as_str))
                    else {
                        continue;
                    };
                    let input = block.get("input").unwrap_or(&Value::Null);
                    self.tools.insert(tool_id.to_string(), name.to_string());
                    out.push(Ev::ToolStart { id: tool_id.to_string(), name: name.to_string(), summary: summarize(name, input) });
                }
                _ => {}
            }
        }
        out
    }

    fn user(d: &Value) -> Vec<Ev> {
        let Some(blocks) = d.pointer("/message/content").and_then(Value::as_array) else { return Vec::new() };
        blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            .filter_map(|b| {
                Some(Ev::ToolEnd {
                    id: b.get("tool_use_id").and_then(Value::as_str)?.to_string(),
                    ok: !b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                    output: b.get("content").and_then(text_of).map(|t| clip(&t, 600)),
                })
            })
            .collect()
    }

    fn control_request(d: &Value) -> Vec<Ev> {
        let Some(request) = d.get("request") else { return Vec::new() };
        if request.get("subtype").and_then(Value::as_str) != Some("can_use_tool") {
            return Vec::new();
        }
        let (Some(request_id), Some(tool)) =
            (d.get("request_id").and_then(Value::as_str), request.get("tool_name").and_then(Value::as_str))
        else {
            return Vec::new();
        };
        vec![Ev::Permission(PermissionAsk {
            request_id: request_id.to_string(),
            tool: tool.to_string(),
            input: request.get("input").cloned().unwrap_or(Value::Null),
            description: request.get("description").and_then(Value::as_str).map(str::to_string),
            tool_use_id: request.get("tool_use_id").and_then(Value::as_str).map(str::to_string),
            blocked_path: request.get("blocked_path").and_then(Value::as_str).map(str::to_string),
        })]
    }

    fn result(&mut self, d: &Value) -> Vec<Ev> {
        let is_error = d.get("is_error").and_then(Value::as_bool).unwrap_or(false);
        let outcome = if !is_error {
            Outcome::Done
        } else if d.get("terminal_reason").and_then(Value::as_str).is_some_and(|r| r.starts_with("aborted")) {
            Outcome::Aborted
        } else {
            let text = d
                .get("result")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .or_else(|| d.get("subtype").and_then(Value::as_str))
                .unwrap_or("The turn failed.")
                .to_string();
            Outcome::Failed { text, login: self.login_failed }
        };
        let mut out = Vec::new();
        // The model's context window: `modelUsage` lists every model the turn used; the session's
        // own is the one that counts, or the only one listed.
        if let Some(models) = d.get("modelUsage").and_then(Value::as_object) {
            let entry = self.model.as_deref().and_then(|m| models.get(m)).or_else(|| (models.len() == 1).then(|| models.values().next()).flatten());
            if let Some(window) = entry.and_then(|e| e.get("contextWindow")).and_then(Value::as_u64) {
                out.push(Ev::Usage { context_tokens: None, context_window: Some(window) });
            }
        }
        out.push(Ev::TurnEnd(outcome));
        out
    }

    /// The plan's windows (`rate_limit_info.unifiedWindows`): how much of each is used and when it resets.
    fn rate_limit(d: &Value) -> Vec<Ev> {
        let Some(windows) = d.pointer("/rate_limit_info/unifiedWindows").and_then(Value::as_object) else { return Vec::new() };
        let list: Vec<PlanWindow> = windows
            .iter()
            .filter_map(|(kind, w)| {
                Some(PlanWindow {
                    kind: kind.clone(),
                    used: w.get("utilization").and_then(Value::as_f64)?.clamp(0.0, 1.0),
                    resets_at: w.get("resetsAt").and_then(Value::as_u64),
                })
            })
            .collect();
        if list.is_empty() {
            Vec::new()
        } else {
            vec![Ev::Limits(list)]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn turn<'a>(prompt: &'a str, mode: &'a str, model: &'a str, resume: Option<&'a str>) -> TurnSpec<'a> {
        TurnSpec { cwd: Path::new("."), prompt, model, mode, resume, mcp: None }
    }

    fn parse(fixture: &str) -> Vec<Ev> {
        let mut p = ClaudeParser::default();
        fixture.lines().flat_map(|l| p.line(l)).collect()
    }

    #[test]
    fn the_size_of_the_conversation_its_window_and_the_plan_windows_are_read_from_the_real_stream() {
        // A real run (2026-10-14, Claude Code 2.1.x, haiku), its init line cut to the fields read.
        let evs = parse(include_str!("fixtures/claude_usage.jsonl"));
        let w = |kind: &str, used: f64, at: u64| PlanWindow { kind: kind.into(), used, resets_at: Some(at) };
        assert_eq!(evs[1], Ev::Limits(vec![w("five_hour", 0.05, 1_791_983_400), w("seven_day", 0.55, 1_792_285_200)]));
        assert_eq!(evs[2], Ev::Usage { context_tokens: Some(2 + 9_731 + 16_422), context_window: None }, "input and cache: what the request sent");
        assert_eq!(evs[3], Ev::Text("pong".into()));
        assert_eq!(evs[4], Ev::Usage { context_tokens: None, context_window: Some(1_000_000) }, "the session model's window");
        assert_eq!(evs[5], Ev::TurnEnd(Outcome::Done));
        // A subagent's request is not the conversation's size; a window with no utilization is skipped.
        let mut p = ClaudeParser::default();
        let sub = r#"{"type":"assistant","message":{"id":"m","content":[],"usage":{"input_tokens":5,"cache_read_input_tokens":9}},"parent_tool_use_id":"toolu_1"}"#;
        assert!(p.line(sub).is_empty());
        let odd = r#"{"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{"five_hour":{"resetsAt":1}}}}"#;
        assert!(p.line(odd).is_empty());
        // Several models in one turn: the session's model names the window; with no init, only a lone one does.
        let two = r#"{"type":"result","is_error":false,"modelUsage":{"claude-haiku-5-5":{"contextWindow":200000},"claude-opus-5-5":{"contextWindow":1000000}}}"#;
        let mut p = ClaudeParser { model: Some("claude-opus-5-5".into()), ..ClaudeParser::default() };
        assert_eq!(p.line(two)[0], Ev::Usage { context_tokens: None, context_window: Some(1_000_000) });
        let mut p = ClaudeParser::default();
        assert_eq!(p.line(two), vec![Ev::TurnEnd(Outcome::Done)], "two models and no init: no guess");
    }

    #[test]
    fn a_new_turn_is_the_verified_command_line_with_the_prompt_on_stdin() {
        let inv = ClaudeCode.build(&turn("say \"hi\"\nplease", "ask", "", None)).unwrap();
        assert_eq!(
            inv.args,
            [
                "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
                "--include-partial-messages", "--permission-prompts", "host", "--permission-prompt-tool", "stdio"
            ]
        );
        assert!(!inv.close_stdin, "the process stays for the answers and the next turns");
        let line: Value = serde_json::from_slice(&inv.stdin).unwrap();
        assert_eq!(line["message"]["content"], "say \"hi\"\nplease");
        assert!(inv.stdin.ends_with(b"\n") && !inv.stdin[..inv.stdin.len() - 1].contains(&b'\n'), "one line");
    }

    #[test]
    fn model_mode_and_resume_become_flags() {
        let t = turn("p", "plan", "haiku", Some("8635e3a1-0139-4dde-a006-297b29acb8af"));
        let inv = ClaudeCode.build(&t).unwrap();
        let tail: Vec<&str> = inv.args.iter().skip(11).map(String::as_str).collect();
        assert_eq!(tail, ["--model", "haiku", "--permission-mode", "plan", "--resume", "8635e3a1-0139-4dde-a006-297b29acb8af"]);
        assert_eq!(inv.mcp, McpApply::None);
    }

    #[test]
    fn what_the_mcp_server_hands_over_goes_last_in_the_arguments_and_the_token_in_the_environment() {
        let attach = super::super::mcp::McpAttach {
            args: vec!["--mcp-config".into(), "{\"mcpServers\":{}}".into()],
            env: vec![("MAYAK_MCP_TOKEN".into(), "secret-token".into())],
        };
        let mut t = turn("p", "plan", "haiku", Some("8635e3a1-0139-4dde-a006-297b29acb8af"));
        t.mcp = Some(&attach);
        let inv = ClaudeCode.build(&t).unwrap();
        let n = inv.args.len();
        assert_eq!(inv.args[n - 2..], ["--mcp-config", "{\"mcpServers\":{}}"], "variadic: nothing may come after it");
        assert!(inv.args.iter().any(|a| a == "--resume"), "the rest is still there");
        assert_eq!(inv.env, [("MAYAK_MCP_TOKEN".to_string(), "secret-token".to_string())]);
        assert!(inv.args.iter().all(|a| !a.contains("secret-token")), "the token is not on the command line");
        assert_eq!(inv.mcp, McpApply::Applied);
        assert!(!String::from_utf8_lossy(&inv.stdin).contains("--mcp-config"), "the prompt goes on stdin");
    }

    #[test]
    fn nothing_that_runs_without_asking_can_be_built() {
        for mode in ["bypassPermissions", "dontAsk", "auto", "acceptEdits", "manual", "", "ASK"] {
            assert_eq!(ClaudeCode.build(&turn("p", mode, "", None)).unwrap_err().code, "badMode", "{mode}");
        }
        assert_eq!(ClaudeCode.build(&turn("p", "ask", "--help", None)).unwrap_err().code, "badModel");
        assert_eq!(ClaudeCode.build(&turn("p", "ask", "", Some("a b"))).unwrap_err().code, "badResume");
        assert_eq!(ClaudeCode.modes(), ["ask", "plan"]);
    }

    #[test]
    fn a_plain_turn_streams_text_and_ends_done() {
        let evs = parse(include_str!("fixtures/claude_text.jsonl"));
        assert!(matches!(&evs[0], Ev::Session { id, model: Some(m), api_key_source: Some(k) }
            if !id.is_empty() && m == "claude-haiku-5-5" && k == "none"), "{:?}", evs[0]);
        let streamed: String = evs.iter().filter_map(|e| if let Ev::TextDelta(t) = e { Some(t.as_str()) } else { None }).collect();
        assert_eq!(streamed, "pong");
        assert!(!evs.iter().any(|e| matches!(e, Ev::Text(_))), "the full text repeats the deltas and is skipped");
        assert_eq!(evs.last(), Some(&Ev::TurnEnd(Outcome::Done)));
    }

    #[test]
    fn an_allowed_tool_is_start_permission_end_then_text() {
        let evs = parse(include_str!("fixtures/claude_tool_allow.jsonl"));
        let start = evs.iter().position(|e| matches!(e, Ev::ToolStart { name, .. } if name == "Bash")).expect("tool start");
        let ask = evs.iter().position(|e| matches!(e, Ev::Permission(_))).expect("permission");
        let end = evs.iter().position(|e| matches!(e, Ev::ToolEnd { .. })).expect("tool end");
        assert!(start < ask && ask < end, "{start} {ask} {end}");
        let Ev::Permission(ask) = &evs[ask] else { unreachable!() };
        assert_eq!(ask.tool, "Bash");
        assert_eq!(ask.input["command"], "cmd /c echo hello > probe_out.txt");
        assert!(ask.tool_use_id.is_some() && ask.blocked_path.is_some());
        let Ev::ToolStart { id, summary, .. } = &evs[start] else { unreachable!() };
        assert_eq!(summary, "cmd /c echo hello > probe_out.txt");
        assert_eq!(ask.tool_use_id.as_deref(), Some(id.as_str()), "the request names the tool line it is about");
        assert!(matches!(&evs[end], Ev::ToolEnd { ok: true, .. }));
        assert_eq!(evs.last(), Some(&Ev::TurnEnd(Outcome::Done)));
    }

    #[test]
    fn a_denied_tool_ends_in_error_and_the_turn_still_completes() {
        let evs = parse(include_str!("fixtures/claude_tool_deny.jsonl"));
        let end = evs.iter().find_map(|e| if let Ev::ToolEnd { ok, output, .. } = e { Some((*ok, output.clone())) } else { None });
        assert_eq!(end, Some((false, Some("The user denied this in Mayak.".to_string()))));
        assert_eq!(evs.last(), Some(&Ev::TurnEnd(Outcome::Done)));
    }

    #[test]
    fn an_interrupted_turn_is_aborted() {
        let evs = parse(include_str!("fixtures/claude_interrupt.jsonl"));
        assert_eq!(evs.last(), Some(&Ev::TurnEnd(Outcome::Aborted)));
        assert!(evs.iter().any(|e| matches!(e, Ev::TextDelta(_))));
    }

    #[test]
    fn a_run_that_is_not_signed_in_fails_with_the_login_flag_and_the_cli_text() {
        let evs = parse(include_str!("fixtures/claude_loggedout.jsonl"));
        assert!(evs.contains(&Ev::Text("Not logged in · Please run /login".into())), "{evs:?}");
        assert_eq!(
            evs.last(),
            Some(&Ev::TurnEnd(Outcome::Failed { text: "Not logged in · Please run /login".into(), login: true }))
        );
    }

    #[test]
    fn answers_are_the_sdk_control_responses() {
        let ask = PermissionAsk {
            request_id: "r-1".into(),
            tool: "Bash".into(),
            input: json!({ "command": "ls" }),
            description: None,
            tool_use_id: None,
            blocked_path: None,
        };
        let allow: Value = serde_json::from_str(ClaudeCode.permission_reply(&ask, true).unwrap().trim()).unwrap();
        assert_eq!(
            allow,
            json!({ "type": "control_response", "response": { "subtype": "success", "request_id": "r-1",
                "response": { "behavior": "allow", "updatedInput": { "command": "ls" } } } })
        );
        let deny: Value = serde_json::from_str(ClaudeCode.permission_reply(&ask, false).unwrap().trim()).unwrap();
        assert_eq!(deny["response"]["response"]["behavior"], "deny");
        assert!(deny["response"]["response"]["message"].as_str().unwrap().contains("denied"));
        let stop: Value = serde_json::from_str(ClaudeCode.interrupt_line().unwrap().trim()).unwrap();
        assert_eq!(stop["request"]["subtype"], "interrupt");
    }

    #[test]
    fn garbage_and_unknown_lines_give_nothing() {
        let mut p = ClaudeParser::default();
        for line in ["", "not json", "{}", r#"{"type":"rate_limit_event"}"#, r#"{"type":"system","subtype":"hook_started"}"#, "[1,2]"] {
            assert!(p.line(line).is_empty(), "{line:?}");
        }
    }

    #[test]
    fn a_subagent_stream_is_not_part_of_the_conversation() {
        let mut p = ClaudeParser::default();
        let line = r#"{"type":"stream_event","parent_tool_use_id":"toolu_1","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"inner"}}}"#;
        assert!(p.line(line).is_empty());
    }

    #[test]
    fn the_login_state_is_read_from_auth_status() {
        assert_eq!(ClaudeCode.parse_login(Some(0), r#"{"loggedIn": true, "authMethod": "claude.ai"}"#), Login::Ready);
        assert_eq!(ClaudeCode.parse_login(Some(0), r#"{"loggedIn": false, "authMethod": "none"}"#), Login::NotLoggedIn);
        assert_eq!(ClaudeCode.parse_login(Some(1), "boom"), Login::Unknown);
    }
}
