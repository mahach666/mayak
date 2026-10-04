// Named-pipe server and router for mayak-hook.
//
// `\\.\pipe\mayak-<sid>` — one instance per connection, one line of JSON per
// connection. `route()` decides what a line is for:
//   * Codex, any event → Observe: handed to the observer, never answered;
//   * Claude PermissionRequest → Approval: the legacy Allow/Deny card, the only
//     connection kept open — it waits for the island's decision and writes it
//     back on the same pipe;
//   * anything else → Ignore: for Claude the session observer is the source of
//     truth, not hooks.
//
// Claude Code is never blocked by us. Three things guarantee it:
//   * mayak-hook gives the connection 300 ms and exits cleanly if we are closed;
//   * we only wait for a human once the island has *confirmed* the card is on
//     screen, so a paused island or a webview that is not listening costs a few
//     hundred milliseconds, not two minutes;
//   * whatever happens we drop the connection after the decision timeout, and
//     the terminal takes over.
//
// What we write back is the bare word `allow` or `deny`. Turning that into the
// documented hookSpecificOutput JSON is mayak-hook's job, so the wire format
// Claude Code expects lives in exactly one place.
//
// The app is plugged in through `RouterSink` (lib.rs), so this file knows
// nothing about Tauri and its tests run a real pipe with a test sink.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::mpsc;

use crate::observe::keys::SessionKey;

/// A line longer than this is dropped, never parsed.
pub const MAX_PAYLOAD: usize = 1 << 20;
/// How long the island gets to say "the card is up". Without it, an island
/// that is paused, hidden behind a crashed webview or simply not listening
/// would leave Claude Code staring at a prompt nobody can see.
const ACK_TIMEOUT: Duration = Duration::from_millis(800);
/// Slightly under mayak-hook's own 110 s wait, so we always answer first.
const DECISION_TIMEOUT: Duration = Duration::from_secs(108);

/// What the island can say about a permission request.
pub enum Reply {
    /// The card is on screen and a human can act on it.
    Ack,
    /// A human clicked: `allow` or `deny`.
    Decision(String),
    /// Nobody can act on it — paused, or another request already holds the card.
    Decline,
}

/// Permission requests the island has been told about.
#[derive(Default)]
pub struct Pending(pub Mutex<HashMap<String, mpsc::Sender<Reply>>>);

static COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Observe,
    Approval,
    Ignore,
}

/// `agent` is payload["agent"] ("claude" when absent: a Coucou-era relay only
/// ever relayed Claude). Any other agent is ignored; mayak-hook sends nothing
/// for a foreign `--agent` anyway (D86), and upstream's `coucou_agent` tag is
/// not read.
pub fn route(agent: &str, event: &str) -> Route {
    match (agent, event) {
        ("codex", _) => Route::Observe,
        ("claude", "PermissionRequest") => Route::Approval,
        _ => Route::Ignore,
    }
}

/// What the island shows about the session behind an approval card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApprovalContext {
    pub session_key: Option<SessionKey>,
    pub project_name: Option<String>,
}

/// Sets `request_id`, `session_key` and `project_name` (string or null).
pub fn annotate_approval(payload: &mut Value, request_id: &str, ctx: &ApprovalContext) {
    if let Some(map) = payload.as_object_mut() {
        map.insert("request_id".into(), json!(request_id));
        map.insert("session_key".into(), json!(ctx.session_key));
        map.insert("project_name".into(), json!(ctx.project_name));
    }
}

#[derive(Debug, PartialEq)]
pub enum LineError {
    Oversize(usize),
    Invalid { len: usize, reason: String },
}

/// The line is everything before the first '\n' (or all of `buf`). The reason
/// of an Invalid line never quotes the line itself.
pub fn parse_line(buf: &[u8]) -> Result<Value, LineError> {
    let line = match buf.iter().position(|b| *b == b'\n') {
        Some(i) => &buf[..i],
        None => buf,
    };
    if line.len() > MAX_PAYLOAD {
        return Err(LineError::Oversize(line.len()));
    }
    match serde_json::from_slice::<Value>(line) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(LineError::Invalid { len: line.len(), reason: "not a JSON object".into() }),
        Err(err) => Err(LineError::Invalid {
            len: line.len(),
            reason: format!("{:?} error at line {} column {}", err.classify(), err.line(), err.column()),
        }),
    }
}

/// "hook <agent> <Event> s=<first 8 chars of session_id or '-'>". No payload text.
pub fn log_line_for(agent: &str, event: &str, session_id: Option<&str>) -> String {
    let short: String = match session_id.filter(|s| !s.is_empty()) {
        Some(s) => s.chars().take(8).collect(),
        None => "-".into(),
    };
    format!("hook {agent} {event} s={short}")
}

/// Where accepted lines go. lib.rs implements it on the app; tests on a fake.
pub trait RouterSink: Send + Sync + 'static {
    fn pending(&self) -> &Pending;
    /// Route::Observe. Must not block.
    fn observe(&self, payload: Value);
    /// Route::Approval: data for `annotate_approval`. May block (Win32 + file
    /// probes); `serve` always calls it through `spawn_blocking`.
    fn approval_context(&self, payload: &Value) -> ApprovalContext;
    /// Route::Approval: the annotated payload to show.
    fn show_approval(&self, payload: Value);
}

/// `\\.\pipe\mayak-<sid>` — must match mayak-hook's `win::pipe_path()` exactly.
pub fn pipe_name() -> String {
    let key = crate::platform::current_user_sid()
        .unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_else(|_| "user".into()));
    format!(r"\\.\pipe\{}{key}", crate::ident::PIPE_PREFIX)
}

/// The only place a pipe instance is created. `first` also means we refuse to
/// join a pipe somebody else already owns under our name.
pub fn create_instance(name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
    ServerOptions::new().first_pipe_instance(first).create(name)
}

/// Accept loop: one task per connection. Err only if the first instance cannot
/// be created. Runs on whatever runtime polls it (Tauri's in the app, the
/// test's own in tests): every instance is created here, and connection tasks
/// are spawned with `tokio::spawn`.
pub async fn serve<S: RouterSink>(name: String, sink: Arc<S>) -> std::io::Result<()> {
    let mut server = create_instance(&name, true)?;
    loop {
        if let Err(err) = server.connect().await {
            note(format!("relay pipe connect failed: {err}"));
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        }
        // Hand the connected instance to a task and listen on a fresh one.
        let next = match create_instance(&name, false) {
            Ok(s) => s,
            Err(err) => {
                note(format!("cannot reopen the relay pipe: {err}"));
                return Ok(());
            }
        };
        let connected = std::mem::replace(&mut server, next);
        tokio::spawn(handle(connected, sink.clone()));
    }
}

async fn handle<S: RouterSink>(mut pipe: NamedPipeServer, sink: Arc<S>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if chunk[..n].contains(&b'\n') || buf.len() > MAX_PAYLOAD {
                    break;
                }
            }
            Err(err) => {
                note(format!("relay read failed after {} bytes: {err}", buf.len()));
                return;
            }
        }
    }
    let payload = match parse_line(&buf) {
        Ok(v) => v,
        Err(LineError::Oversize(len)) => {
            note(format!("relay line dropped: oversize {len} bytes"));
            let _ = pipe.disconnect();
            return;
        }
        Err(LineError::Invalid { len, reason }) => {
            note(format!("relay line dropped: {len} bytes, {reason}"));
            let _ = pipe.disconnect();
            return;
        }
    };

    let agent = payload.get("agent").and_then(Value::as_str).unwrap_or("claude").to_string();
    let event = payload.get("hook_event_name").and_then(Value::as_str).unwrap_or_default().to_string();
    note(log_line_for(&agent, &event, payload.get("session_id").and_then(Value::as_str)));

    match route(&agent, &event) {
        Route::Observe => {
            sink.observe(payload);
            let _ = pipe.disconnect();
        }
        Route::Ignore => {
            let _ = pipe.disconnect();
        }
        Route::Approval => approval(pipe, payload, sink).await,
    }
}

async fn approval<S: RouterSink>(mut pipe: NamedPipeServer, mut payload: Value, sink: Arc<S>) {
    let id = format!("{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    let ctx = {
        let sink = sink.clone();
        let payload = payload.clone();
        match tokio::task::spawn_blocking(move || sink.approval_context(&payload)).await {
            Ok(ctx) => ctx,
            Err(err) => {
                note(format!("approval context failed id={id}: {err}"));
                ApprovalContext::default()
            }
        }
    };
    let (tx, mut rx) = mpsc::channel::<Reply>(4);
    sink.pending().0.lock().unwrap().insert(id.clone(), tx);
    annotate_approval(&mut payload, &id, &ctx);
    note(format!("approval id={id}"));
    sink.show_approval(payload);

    let decision = wait_for_decision(&id, &mut rx).await;
    sink.pending().0.lock().unwrap().remove(&id);

    // No decision: say nothing at all. mayak-hook then writes nothing to stdout
    // and Claude Code asks in the terminal, exactly as if Mayak were closed.
    if let Some(d) = decision {
        let _ = pipe.write_all(format!("{d}\n").as_bytes()).await;
        let _ = pipe.flush().await;
    }
    let _ = pipe.disconnect();
}

/// Two waits: a short one for "the card is up", then the long one for a human.
async fn wait_for_decision(id: &str, rx: &mut mpsc::Receiver<Reply>) -> Option<String> {
    match tokio::time::timeout(ACK_TIMEOUT, rx.recv()).await {
        Ok(Some(Reply::Ack)) => {}
        // A click that beats the ack is still a click.
        Ok(Some(Reply::Decision(d))) => {
            note(format!("approval id={id} answered {d}"));
            return Some(d);
        }
        Ok(Some(Reply::Decline)) => {
            note(format!("approval id={id} not shown — terminal takes over"));
            return None;
        }
        Ok(None) => return None,
        Err(_) => {
            note(format!("approval id={id} island never acknowledged — terminal takes over"));
            return None;
        }
    }

    match tokio::time::timeout(DECISION_TIMEOUT, rx.recv()).await {
        Ok(Some(Reply::Decision(d))) => {
            note(format!("approval id={id} answered {d}"));
            Some(d)
        }
        Ok(Some(Reply::Decline)) => {
            note(format!("approval id={id} released without a decision"));
            None
        }
        _ => {
            note(format!("approval id={id} timed out — terminal takes over"));
            None
        }
    }
}

fn send(pending: &Pending, request_id: &str, reply: Reply, keep: bool) {
    let sender = {
        let mut map = pending.0.lock().unwrap();
        if keep { map.get(request_id).cloned() } else { map.remove(request_id) }
    };
    match sender {
        Some(tx) => {
            let _ = tx.try_send(reply);
        }
        None => note(format!("reply for id={request_id} — no pending request")),
    }
}

/// The island has the card on screen; the long wait may begin.
pub fn acknowledge(pending: &Pending, request_id: &str) {
    send(pending, request_id, Reply::Ack, true);
}

/// Nobody can act on this one — paused, or another card already holds the view.
pub fn decline(pending: &Pending, request_id: &str) {
    note(format!("decline id={request_id}"));
    send(pending, request_id, Reply::Decline, false);
}

/// Called by the island's Allow / Deny buttons. Only ever a bare word: turning
/// it into Claude Code's JSON is mayak-hook's job.
pub fn answer(pending: &Pending, request_id: &str, decision: &str) {
    let word = match decision {
        "allow" | "always" => "allow",
        _ => "deny",
    };
    note(format!("decision id={request_id} {word}"));
    send(pending, request_id, Reply::Decision(word.to_string()), false);
}

/// Log sink of this module: mayak.log in the app, a capture buffer in tests
/// (tests must not write to the user's log).
#[cfg(not(test))]
fn note(message: String) {
    crate::log::line(message);
}

#[cfg(test)]
fn note(message: String) {
    tests::NOTES.lock().unwrap().push(message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    use std::time::Instant;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

    pub(super) static NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static NEXT: AtomicU32 = AtomicU32::new(0);

    /// `\\.\pipe\mayak-test-<tag>-<pid>-<n>` — never the real `mayak-<SID>`.
    fn unique_pipe(tag: &str) -> String {
        format!(r"\\.\pipe\mayak-test-{tag}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    fn notes_with(prefix: &str) -> Vec<String> {
        NOTES.lock().unwrap().iter().filter(|n| n.starts_with(prefix)).cloned().collect()
    }

    struct TestSink {
        pending: Pending,
        ctx: ApprovalContext,
        observed: UnboundedSender<Value>,
        shown: UnboundedSender<Value>,
    }

    impl RouterSink for TestSink {
        fn pending(&self) -> &Pending {
            &self.pending
        }
        fn observe(&self, payload: Value) {
            self.observed.send(payload).unwrap();
        }
        fn approval_context(&self, _payload: &Value) -> ApprovalContext {
            self.ctx.clone()
        }
        fn show_approval(&self, payload: Value) {
            self.shown.send(payload).unwrap();
        }
    }

    fn sink(ctx: ApprovalContext) -> (Arc<TestSink>, UnboundedReceiver<Value>, UnboundedReceiver<Value>) {
        let (observed, observed_rx) = unbounded_channel();
        let (shown, shown_rx) = unbounded_channel();
        (Arc::new(TestSink { pending: Pending::default(), ctx, observed, shown }), observed_rx, shown_rx)
    }

    /// Retries while `serve` has not created the instance yet or it is busy.
    async fn client(name: &str) -> NamedPipeClient {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match ClientOptions::new().open(name) {
                Ok(c) => return c,
                Err(err) if Instant::now() < deadline && matches!(err.raw_os_error(), Some(2) | Some(231)) => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(err) => panic!("cannot open {name}: {err}"),
            }
        }
    }

    /// Reads until the server closes its end; returns what it wrote.
    async fn read_to_close(c: &mut NamedPipeClient) -> String {
        let mut out = Vec::new();
        let mut chunk = [0u8; 256];
        loop {
            match tokio::time::timeout(Duration::from_secs(5), c.read(&mut chunk)).await {
                Ok(Ok(0)) | Ok(Err(_)) => break,
                Ok(Ok(n)) => out.extend_from_slice(&chunk[..n]),
                Err(_) => panic!("server never closed the connection"),
            }
        }
        String::from_utf8(out).unwrap()
    }

    async fn recv(rx: &mut UnboundedReceiver<Value>) -> Value {
        tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("timed out").expect("closed")
    }

    #[test]
    fn route_table() {
        assert_eq!(route("codex", "PermissionRequest"), Route::Observe);
        assert_eq!(route("codex", "Stop"), Route::Observe);
        assert_eq!(route("codex", ""), Route::Observe);
        assert_eq!(route("claude", "PermissionRequest"), Route::Approval);
        assert_eq!(route("claude", "Stop"), Route::Ignore);
        assert_eq!(route("claude", "UserPromptSubmit"), Route::Ignore);
        assert_eq!(route("claude", ""), Route::Ignore);
        assert_eq!(route("cursor", "PermissionRequest"), Route::Ignore);
    }

    #[test]
    fn parse_line_takes_one_object_line() {
        assert_eq!(parse_line(b"{\"a\":1}\ntrailing").unwrap(), json!({ "a": 1 }));
        assert_eq!(parse_line(b"{\"a\":1}").unwrap(), json!({ "a": 1 }));
        assert_eq!(
            parse_line(b"[1,2]\n"),
            Err(LineError::Invalid { len: 5, reason: "not a JSON object".into() })
        );
        match parse_line(b"not json at all\n") {
            Err(LineError::Invalid { len: 15, reason }) => {
                assert!(reason.starts_with("Syntax error"), "{reason}");
                assert!(!reason.contains("not json"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(parse_line(b""), Err(LineError::Invalid { len: 0, .. })));
    }

    #[test]
    fn parse_line_refuses_more_than_one_mebibyte() {
        let mut big = vec![b' '; MAX_PAYLOAD + 1];
        big[0] = b'{';
        assert_eq!(parse_line(&big), Err(LineError::Oversize(MAX_PAYLOAD + 1)));
        // Exactly 1 MiB is still a line.
        let mut edge = vec![b' '; MAX_PAYLOAD];
        edge[0] = b'{';
        edge[MAX_PAYLOAD - 1] = b'}';
        assert_eq!(parse_line(&edge).unwrap(), json!({}));
    }

    #[test]
    fn annotations_are_strings_or_nulls() {
        let mut p = json!({ "hook_event_name": "PermissionRequest" });
        annotate_approval(&mut p, "9-1", &ApprovalContext::default());
        assert_eq!(p["request_id"], "9-1");
        assert_eq!(p["session_key"], Value::Null);
        assert_eq!(p["project_name"], Value::Null);
        let ctx = ApprovalContext { session_key: Some("claude:1:2".into()), project_name: Some("mayak".into()) };
        annotate_approval(&mut p, "9-2", &ctx);
        assert_eq!(p["session_key"], "claude:1:2");
        assert_eq!(p["project_name"], "mayak");
    }

    #[test]
    fn log_lines_carry_no_payload() {
        assert_eq!(log_line_for("codex", "Stop", Some("0199a1b2-c3d4")), "hook codex Stop s=0199a1b2");
        assert_eq!(log_line_for("claude", "PermissionRequest", Some("abc")), "hook claude PermissionRequest s=abc");
        assert_eq!(log_line_for("claude", "Stop", None), "hook claude Stop s=-");
        assert_eq!(log_line_for("claude", "Stop", Some("")), "hook claude Stop s=-");
    }

    #[test]
    fn serve_future_is_send_for_the_tauri_runtime() {
        fn assert_send<T: Send>(_: &T) {}
        let (s, _o, _a) = sink(ApprovalContext::default());
        let fut = serve(unique_pipe("send"), s);
        assert_send(&fut);
    }

    #[test]
    fn codex_lines_go_to_the_observer_and_are_never_answered() {
        runtime().block_on(async {
            let name = unique_pipe("observe");
            let (s, mut observed, mut shown) = sink(ApprovalContext::default());
            let server = tokio::spawn(serve(name.clone(), s.clone()));
            for session in ["s-one", "s-two"] {
                let mut c = client(&name).await;
                let line = format!("{{\"agent\":\"codex\",\"hook_event_name\":\"PermissionRequest\",\"session_id\":\"{session}\"}}\n");
                c.write_all(line.as_bytes()).await.unwrap();
                assert_eq!(read_to_close(&mut c).await, "");
                assert_eq!(recv(&mut observed).await["session_id"], session);
            }
            assert!(shown.try_recv().is_err());
            assert!(s.pending.0.lock().unwrap().is_empty());
            server.abort();
        });
    }

    #[test]
    fn claude_permission_request_waits_for_the_island_decision() {
        runtime().block_on(async {
            let name = unique_pipe("approval");
            let ctx = ApprovalContext { session_key: Some("claude:7:8".into()), project_name: Some("repo".into()) };
            let (s, mut observed, mut shown) = sink(ctx);
            let server = tokio::spawn(serve(name.clone(), s.clone()));
            let mut c = client(&name).await;
            c.write_all(b"{\"agent\":\"claude\",\"hook_event_name\":\"PermissionRequest\",\"cwd\":\"C:\\\\repo\"}\n")
                .await
                .unwrap();
            let card = recv(&mut shown).await;
            assert_eq!(card["session_key"], "claude:7:8");
            assert_eq!(card["project_name"], "repo");
            let id = card["request_id"].as_str().unwrap().to_string();
            acknowledge(&s.pending, &id);
            answer(&s.pending, &id, "always");
            assert_eq!(read_to_close(&mut c).await, "allow\n");
            assert!(observed.try_recv().is_err());
            assert!(s.pending.0.lock().unwrap().is_empty());
            server.abort();
        });
    }

    #[test]
    fn an_unacknowledged_card_lets_the_terminal_take_over() {
        runtime().block_on(async {
            let name = unique_pipe("noack");
            let (s, _observed, mut shown) = sink(ApprovalContext::default());
            let server = tokio::spawn(serve(name.clone(), s.clone()));
            let mut c = client(&name).await;
            c.write_all(b"{\"hook_event_name\":\"PermissionRequest\"}\n").await.unwrap();
            let card = recv(&mut shown).await;
            assert_eq!(card["session_key"], Value::Null);
            let started = Instant::now();
            assert_eq!(read_to_close(&mut c).await, "");
            assert!(started.elapsed() >= Duration::from_millis(700), "{:?}", started.elapsed());
            assert!(s.pending.0.lock().unwrap().is_empty());
            server.abort();
        });
    }

    #[test]
    fn other_claude_events_are_ignored() {
        runtime().block_on(async {
            let name = unique_pipe("ignore");
            let (s, mut observed, mut shown) = sink(ApprovalContext::default());
            let server = tokio::spawn(serve(name.clone(), s.clone()));
            let mut c = client(&name).await;
            c.write_all(b"{\"agent\":\"claude\",\"hook_event_name\":\"Stop\",\"session_id\":\"ignored-1\"}\n")
                .await
                .unwrap();
            assert_eq!(read_to_close(&mut c).await, "");
            assert!(observed.try_recv().is_err());
            assert!(shown.try_recv().is_err());
            assert_eq!(notes_with("hook claude Stop s=ignored-"), vec!["hook claude Stop s=ignored-".to_string()]);
            server.abort();
        });
    }

    #[test]
    fn invalid_and_oversize_lines_are_logged_and_dropped() {
        runtime().block_on(async {
            let name = unique_pipe("bad");
            let (s, mut observed, mut shown) = sink(ApprovalContext::default());
            let server = tokio::spawn(serve(name.clone(), s.clone()));

            let mut c = client(&name).await;
            c.write_all(b"definitely not json\n").await.unwrap();
            assert_eq!(read_to_close(&mut c).await, "");
            let invalid = notes_with("relay line dropped: 19 bytes, ");
            assert_eq!(invalid.len(), 1, "{invalid:?}");
            assert!(!invalid[0].contains("not json"), "{invalid:?}");

            let mut c = client(&name).await;
            let writer = tokio::spawn(async move {
                let big = vec![b'x'; MAX_PAYLOAD + 64 * 1024];
                let _ = c.write_all(&big).await;
                c
            });
            let mut c = writer.await.unwrap();
            let _ = read_to_close(&mut c).await;
            let oversize = notes_with("relay line dropped: oversize ");
            assert_eq!(oversize.len(), 1, "{oversize:?}");

            assert!(observed.try_recv().is_err());
            assert!(shown.try_recv().is_err());
            server.abort();
        });
    }
}
