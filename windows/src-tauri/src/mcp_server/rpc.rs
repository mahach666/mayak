// JSON-RPC 2.0 and the MCP messages this server answers: initialize, ping, tools/list, tools/call.
//
// The server is stateless: no `Mcp-Session-Id` is minted, so every request stands alone and a
// restart of Mayak breaks nothing a client holds. The protocol versions it speaks are the ones whose
// handshake is `initialize` (2025-03-26 to 2025-11-25). Claude Code 2.1.296 first tries the newer
// stateless revision with `server/discover`; answering that with "method not found" makes it fall
// back to `initialize` (read off a real connection, see the tests of `mcp_server`).

use serde_json::{json, Map, Value};

use super::tools::{self, Backend, Caller, ToolOutput};

/// Newest first. What `initialize` answers with when the client asks for one of these, or else the first.
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

/// A result over this many characters is refused rather than put into the model's context.
const MAX_RESULT_CHARS: usize = 120_000;

const INSTRUCTIONS: &str = "Mayak shows the user's coding-agent sessions on an island at the top of their screen. \
The list_*/get_* tools only read what that island already shows. ask_user puts a question on the island \
and waits for the user's click; nothing is decided without it.";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// What goes back on the HTTP connection.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// A JSON body with this status.
    Json(u16, Value),
    /// `202 Accepted`, no body: a notification or a response was taken.
    Accepted,
}

fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// The id of a request: a string or a number; `null` is not one.
fn request_id(message: &Map<String, Value>) -> Option<Value> {
    message.get("id").filter(|id| id.is_string() || id.is_number()).cloned()
}

/// One POST body.
pub async fn handle<B: Backend>(backend: &B, caller: &Caller, body: &[u8]) -> Reply {
    let message: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return Reply::Json(400, error(Value::Null, PARSE_ERROR, "The body is not JSON.")),
    };
    let Value::Object(message) = message else {
        // Batches were allowed by 2025-03-26 and dropped again by 2025-06-18; no client of this
        // server sends one.
        return Reply::Json(400, error(Value::Null, INVALID_REQUEST, "Send one JSON-RPC message per request."));
    };
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Reply::Json(400, error(Value::Null, INVALID_REQUEST, "jsonrpc must be \"2.0\"."));
    }
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        // A response to a request of the server's: it sends none, and taking it is what the
        // transport asks for.
        return if message.contains_key("result") || message.contains_key("error") {
            Reply::Accepted
        } else {
            Reply::Json(400, error(Value::Null, INVALID_REQUEST, "A message needs a method."))
        };
    };
    // No `id` at all: a notification, which gets no answer whatever it says.
    if !message.contains_key("id") {
        return Reply::Accepted;
    }
    let Some(id) = request_id(&message) else {
        return Reply::Json(400, error(Value::Null, INVALID_REQUEST, "The id must be a string or a number."));
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    Reply::Json(200, dispatch(backend, caller, id, method, &params).await)
}

async fn dispatch<B: Backend>(backend: &B, caller: &Caller, id: Value, method: &str, params: &Value) -> Value {
    match method {
        "initialize" => initialize(id, params),
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({ "tools": tools::definitions() })),
        "tools/call" => call_tool(backend, caller, id, params).await,
        other => error(id, METHOD_NOT_FOUND, format!("Method not found: {other}")),
    }
}

fn initialize(id: Value, params: &Value) -> Value {
    let Some(asked) = params.get("protocolVersion").and_then(Value::as_str) else {
        return error(id, INVALID_PARAMS, "initialize needs a protocolVersion.");
    };
    let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).unwrap_or(&PROTOCOL_VERSIONS[0]);
    result(
        id,
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": super::SERVER_NAME, "title": "Mayak", "version": env!("CARGO_PKG_VERSION") },
            "instructions": INSTRUCTIONS,
        }),
    )
}

async fn call_tool<B: Backend>(backend: &B, caller: &Caller, id: Value, params: &Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return error(id, INVALID_PARAMS, "tools/call needs the name of a tool.");
    };
    if !tools::exists(name) {
        return error(id, INVALID_PARAMS, format!("Unknown tool: {name}"));
    }
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(a) => a.clone(),
    };
    let (text, is_error) = match tools::call(backend, caller, name, &arguments).await {
        ToolOutput::Text(t) if t.chars().count() > MAX_RESULT_CHARS => {
            ("The result is too large to hand over.".to_string(), true)
        }
        ToolOutput::Text(t) => (t, false),
        ToolOutput::Error(e) => (e, true),
    };
    result(id, json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_server::tools::fake::Fake;
    use crate::mcp_server::tools::AskOutcome;

    fn run<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    fn post(fake: &Fake, body: &str) -> Reply {
        run(handle(fake, &Caller { label: "Codex".into(), chat: None }, body.as_bytes()))
    }

    fn json_of(reply: Reply) -> (u16, Value) {
        match reply {
            Reply::Json(status, v) => (status, v),
            Reply::Accepted => panic!("expected a JSON reply, got 202"),
        }
    }

    #[test]
    fn initialize_answers_with_the_version_the_client_asked_for() {
        // What Claude Code and Codex send, read off a real connection.
        for asked in ["2025-11-25", "2025-06-18", "2025-03-26"] {
            let body = format!(
                r#"{{"jsonrpc":"2.0","id":0,"method":"initialize","params":{{"protocolVersion":"{asked}","capabilities":{{"elicitation":{{}}}},"clientInfo":{{"name":"codex-mcp-client","version":"0.162.1"}}}}}}"#
            );
            let (status, v) = json_of(post(&Fake::new(), &body));
            assert_eq!(status, 200);
            assert_eq!(v["jsonrpc"], "2.0");
            assert_eq!(v["id"], 0);
            assert_eq!(v["result"]["protocolVersion"], asked);
            assert_eq!(v["result"]["capabilities"]["tools"]["listChanged"], false);
            assert_eq!(v["result"]["serverInfo"]["name"], "mayak");
            assert!(v["result"]["instructions"].as_str().unwrap().contains("island"));
            // A stateless server declares no other capability.
            assert_eq!(v["result"]["capabilities"].as_object().unwrap().len(), 1);
        }
    }

    #[test]
    fn a_version_the_server_does_not_know_gets_its_newest() {
        for asked in ["2024-11-05", "2099-01-01", "nonsense"] {
            let body = format!(r#"{{"jsonrpc":"2.0","id":"a","method":"initialize","params":{{"protocolVersion":"{asked}"}}}}"#);
            let (_, v) = json_of(post(&Fake::new(), &body));
            assert_eq!(v["result"]["protocolVersion"], PROTOCOL_VERSIONS[0], "{asked}");
            assert_eq!(v["id"], "a");
        }
    }

    #[test]
    fn initialize_without_a_version_is_invalid_params() {
        let (status, v) = json_of(post(&Fake::new(), r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#));
        assert_eq!(status, 200);
        assert_eq!(v["error"]["code"], -32602);
        let (_, v) = json_of(post(&Fake::new(), r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#));
        assert_eq!(v["error"]["code"], -32602);
        let (_, v) = json_of(post(&Fake::new(), r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":3}}"#));
        assert_eq!(v["error"]["code"], -32602);
    }

    #[test]
    fn notifications_and_responses_are_accepted_without_an_answer() {
        let fake = Fake::new();
        for body in [
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#,
            r#"{"jsonrpc":"2.0","method":"something/unknown"}"#,
            r#"{"jsonrpc":"2.0","id":7,"result":{}}"#,
            r#"{"jsonrpc":"2.0","id":7,"error":{"code":-1,"message":"x"}}"#,
        ] {
            assert_eq!(post(&fake, body), Reply::Accepted, "{body}");
        }
    }

    #[test]
    fn ping_answers_with_an_empty_result() {
        let (_, v) = json_of(post(&Fake::new(), r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#));
        assert_eq!(v, json!({ "jsonrpc": "2.0", "id": 9, "result": {} }));
    }

    #[test]
    fn tools_list_gives_every_tool_with_its_schema() {
        // Codex puts a progress token in `_meta`; it changes nothing.
        let (status, v) = json_of(post(
            &Fake::new(),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{"progressToken":0}}}"#,
        ));
        assert_eq!(status, 200);
        let tools = v["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 9);
        assert_eq!(tools[0]["name"], "list_sessions");
        assert_eq!(tools[7]["name"], "ask_user");
        assert_eq!(tools[8]["name"], "set_status");
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object" && t["description"].is_string()));
        // Without params too, as Claude Code sends it.
        let (_, v) = json_of(post(&Fake::new(), r#"{"method":"tools/list","jsonrpc":"2.0","id":1}"#));
        assert_eq!(v["result"]["tools"].as_array().unwrap().len(), 9);
    }

    #[test]
    fn tools_call_runs_the_tool_and_wraps_its_text() {
        let fake = Fake::new();
        let (status, v) = json_of(post(
            &fake,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_sessions","arguments":{}}}"#,
        ));
        assert_eq!(status, 200);
        assert_eq!(v["result"]["isError"], false);
        assert_eq!(v["result"]["content"][0]["type"], "text");
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let inner: Value = serde_json::from_str(text).unwrap();
        assert_eq!(inner["projects"][0]["name"], "mayak");
        // `arguments` may be left out.
        let (_, v) = json_of(post(&fake, r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_limits"}}"#));
        assert_eq!(v["result"]["isError"], false);
    }

    #[test]
    fn a_tool_that_fails_says_so_in_its_result_not_as_a_protocol_error() {
        let (status, v) = json_of(post(
            &Fake::new(),
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_stats","arguments":{"days":3}}}"#,
        ));
        assert_eq!(status, 200);
        assert!(v.get("error").is_none());
        assert_eq!(v["result"]["isError"], true);
        assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains("7 or 30"));
    }

    #[test]
    fn an_unknown_tool_and_a_call_without_a_name_are_invalid_params() {
        let fake = Fake::new();
        let (status, v) = json_of(post(
            &fake,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rm_rf","arguments":{}}}"#,
        ));
        assert_eq!(status, 200);
        assert_eq!(v["error"]["code"], -32602);
        assert!(v["error"]["message"].as_str().unwrap().contains("rm_rf"));
        for body in [
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":5}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call"}"#,
        ] {
            let (_, v) = json_of(post(&fake, body));
            assert_eq!(v["error"]["code"], -32602, "{body}");
        }
    }

    #[test]
    fn ask_user_waits_for_the_backend_and_returns_the_click() {
        let fake = Fake::new();
        *fake.answer.lock().unwrap() = AskOutcome::Answered(json!({ "Deploy?": "Yes" }));
        let (_, v) = json_of(post(
            &fake,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"ask_user","arguments":{"question":"Deploy?","options":["Yes","No"]}}}"#,
        ));
        let inner: Value = serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(inner, json!({ "answered": true, "answer": "Yes" }));
        let asked = fake.asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].asker, "Codex");
    }

    #[test]
    fn methods_the_server_does_not_have_are_method_not_found_with_status_200() {
        // `server/discover` is the first thing Claude Code 2.1.296 sends; this answer, with status
        // 200, made it fall back to `initialize` on a real connection.
        for method in ["server/discover", "resources/list", "prompts/list", "completion/complete", "logging/setLevel"] {
            let body = format!(r#"{{"jsonrpc":"2.0","id":"server-discover-probe-1","method":"{method}","params":{{}}}}"#);
            let (status, v) = json_of(post(&Fake::new(), &body));
            assert_eq!(status, 200, "{method}");
            assert_eq!(v["error"]["code"], -32601, "{method}");
            assert_eq!(v["id"], "server-discover-probe-1");
        }
    }

    #[test]
    fn ids_come_back_as_they_were_sent() {
        for id in [json!(0), json!(41), json!("abc"), json!(-3)] {
            let body = json!({ "jsonrpc": "2.0", "id": id, "method": "ping" }).to_string();
            let (_, v) = json_of(post(&Fake::new(), &body));
            assert_eq!(v["id"], id);
        }
    }

    #[test]
    fn bad_messages_are_refused_with_the_right_code() {
        let fake = Fake::new();
        for (body, status, code) in [
            ("not json", 400, -32700),
            ("", 400, -32700),
            ("[]", 400, -32600),
            (r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#, 400, -32600),
            ("5", 400, -32600),
            (r#"{"id":1,"method":"ping"}"#, 400, -32600),
            (r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#, 400, -32600),
            (r#"{"jsonrpc":"2.0","id":1}"#, 400, -32600),
            (r#"{"jsonrpc":"2.0","id":1,"method":5}"#, 400, -32600),
            (r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#, 400, -32600),
            (r#"{"jsonrpc":"2.0","id":[1],"method":"ping"}"#, 400, -32600),
            (r#"{"jsonrpc":"2.0","id":true,"method":"ping"}"#, 400, -32600),
        ] {
            let (s, v) = json_of(post(&fake, body));
            assert_eq!((s, v["error"]["code"].clone()), (status, json!(code)), "{body}");
            assert_eq!(v["id"], Value::Null);
        }
    }

    #[test]
    fn a_result_too_large_for_the_model_is_refused() {
        struct Big;
        impl Backend for Big {
            fn snapshot(&self) -> Value {
                json!({ "usage": "x".repeat(MAX_RESULT_CHARS + 1) })
            }
            async fn stats(&self, _: u32) -> Result<Value, String> {
                Err("no".into())
            }
            async fn apps(&self) -> Value {
                Value::Null
            }
            async fn ask(&self, _: crate::mcp_server::tools::AskRequest) -> AskOutcome {
                AskOutcome::Busy
            }
            fn focus(&self) -> Value {
                Value::Null
            }
            fn now_ms(&self) -> i64 {
                0
            }
            fn ao(&self) -> crate::mcp_server::tools::AoRead {
                crate::mcp_server::tools::AoRead { enabled: false, last: None }
            }
            fn set_status(&self, _: u64, _: Option<String>) -> crate::mcp_server::tools::StatusShown {
                crate::mcp_server::tools::StatusShown::NoChat
            }
        }
        let reply = run(handle(
            &Big,
            &Caller { label: "x".into(), chat: None },
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_limits"}}"#,
        ));
        let (_, v) = json_of(reply);
        assert_eq!(v["result"]["isError"], true);
        assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains("too large"));
    }
}
