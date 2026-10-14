// Mayak's personal MCP server: the island's own state, offered to agents over MCP.
//
// An agent Mayak starts (the chat), or any Claude Code / Codex / Cursor / Gemini the user runs, can
// connect and read the sessions, the limits and the stats the island shows, and put a question on
// the island that only the user's click answers. Off by default; switched on in Settings.
//
// Transport: Streamable HTTP on 127.0.0.1 only, one endpoint (`POST /mcp`), stateless (no session
// id), every request carries `Authorization: Bearer <token>` (a random per-install token kept in
// the Credential Manager). A request is refused before its body is read unless the path, the
// `Host`, the `Origin` and the token are right (DNS rebinding and other users' processes are the
// two things this is for). Nothing here listens on another interface or calls out.
//
// The module is cut so that what matters is testable without the app:
//   * `http`  — the parts of HTTP/1.1 it needs, strict and bounded;
//   * `rpc`   — JSON-RPC / MCP messages (initialize, ping, tools/list, tools/call);
//   * `tools` — the tools, over a `Backend` trait the tests fake;
//   * `attach` — the arguments for a CLI Mayak starts and the line a user pastes for their own;
//   * `backend` — the `Backend` of the real app (observer, island, pending cards).

mod attach;
mod backend;
mod http;
mod rpc;
mod tools;

use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

pub use attach::{attach, install_command, Attach, Cli, Shell, CHAT_HEADER};
pub use backend::AppBackend;
use crate::message::Message;
use http::{Head, ReadError};
use tools::{Backend, Caller};

/// The name the server goes by in every CLI's config (`mcp__mayak__…` in Claude Code).
pub const SERVER_NAME: &str = "mayak";
/// The only path that answers.
pub const ENDPOINT: &str = "/mcp";
/// A port outside the range Windows hands out to outgoing connections (49152 and up), so that
/// the address a CLI stored today is still free tomorrow. A busy port is reported, never replaced.
pub const DEFAULT_PORT: u16 = 47831;
/// The Credential Manager entry of the bearer token (`secrets::KNOWN_KEYS`).
pub const TOKEN_KEY: &str = "mcp-token";

/// The head of a request must arrive within this, the body within the next.
const HEAD_TIMEOUT: Duration = Duration::from_secs(5);
const BODY_TIMEOUT: Duration = Duration::from_secs(10);
/// Connections served at once; a `ask_user` holds one for as long as it waits.
const MAX_CONNECTIONS: usize = 32;
/// Refusals are logged at most this often, so a caller that keeps knocking cannot fill the log.
const REFUSAL_LOG_EVERY: Duration = Duration::from_secs(10);

pub fn url_for(port: u16) -> String {
    format!("http://127.0.0.1:{port}{ENDPOINT}")
}

/// 64 hex characters from the system's random generator.
pub fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|err| format!("no random numbers: {err}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// A token as `new_token` makes it. One that is not (the entry was edited by hand) is replaced.
fn well_formed(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The saved token, or a new one saved first. `stored` is what the Credential Manager holds.
pub fn ensure_token(
    stored: Option<String>,
    store: impl FnOnce(&str) -> Result<(), String>,
    make: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    if let Some(token) = stored.filter(|t| well_formed(t)) {
        return Ok(token);
    }
    let token = make()?;
    store(&token)?;
    Ok(token)
}

/// Equal without stopping at the first difference.
fn same_secret(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

/// Why a request was refused before its body was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    NotFound,
    Forbidden(&'static str),
    Unauthorized,
    MethodNotAllowed,
    UnsupportedMedia,
}

impl Refusal {
    fn status(self) -> u16 {
        match self {
            Refusal::NotFound => 404,
            Refusal::Forbidden(_) => 403,
            Refusal::Unauthorized => 401,
            Refusal::MethodNotAllowed => 405,
            Refusal::UnsupportedMedia => 415,
        }
    }

    fn message(self) -> Option<&'static str> {
        match self {
            Refusal::NotFound | Refusal::MethodNotAllowed => None,
            Refusal::Forbidden(why) => Some(why),
            Refusal::Unauthorized => Some("A valid bearer token is required."),
            Refusal::UnsupportedMedia => Some("Send application/json."),
        }
    }

    /// The text of a log line: what was refused, never what was sent.
    fn log_text(self) -> &'static str {
        match self {
            Refusal::NotFound => "another path",
            Refusal::Forbidden(why) => why,
            Refusal::Unauthorized => "no valid token",
            Refusal::MethodNotAllowed => "a method this endpoint does not take",
            Refusal::UnsupportedMedia => "a body that is not JSON",
        }
    }

    fn to_response(self) -> Vec<u8> {
        let body = self.message().map(|m| {
            serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32600, "message": m } }).to_string()
        });
        let extra: &[(&str, &str)] = match self {
            Refusal::Unauthorized => &[("WWW-Authenticate", "Bearer realm=\"mayak\"")],
            Refusal::MethodNotAllowed => &[("Allow", "POST")],
            _ => &[],
        };
        http::response(self.status(), extra, body.as_deref().map(str::as_bytes))
    }
}

/// `host` of a `Host` header or an `Origin` URL is this machine, on `port` when it names one.
fn loopback_host(host_port: &str, port: Option<u16>) -> bool {
    let host_port = host_port.to_ascii_lowercase();
    let (host, given) = match host_port.rsplit_once(':') {
        // "[::1]" has colons of its own: the port is what follows "]:".
        Some((host, p)) if !host.ends_with(':') && p.bytes().all(|b| b.is_ascii_digit()) && !p.is_empty() => (host, Some(p)),
        _ => (host_port.as_str(), None),
    };
    if !["127.0.0.1", "localhost", "[::1]"].contains(&host) {
        return false;
    }
    match (port, given) {
        (Some(want), Some(got)) => got.parse::<u16>() == Ok(want),
        (Some(_), None) => false,
        (None, _) => true,
    }
}

/// Everything that is decided from the head alone. `Ok` carries who is calling.
fn check_head(head: &Head, port: u16, token: &str) -> Result<Caller, Refusal> {
    let header = |name: &str| head.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
    if head.path != ENDPOINT {
        return Err(Refusal::NotFound);
    }
    // A browser page on another site that was pointed at 127.0.0.1 (DNS rebinding) still sends its
    // own name as Host, and a browser always sends Origin on a cross-site request.
    match header("host") {
        Some(host) if loopback_host(host, Some(port)) => {}
        _ => return Err(Refusal::Forbidden("The Host is not this server's address.")),
    }
    if let Some(origin) = header("origin") {
        let authority = origin.strip_prefix("http://").or_else(|| origin.strip_prefix("https://")).and_then(|rest| rest.split('/').next());
        if !authority.is_some_and(|a| loopback_host(a, None)) {
            return Err(Refusal::Forbidden("The Origin is not allowed."));
        }
    }
    let presented = header("authorization").and_then(|value| {
        let (scheme, rest) = value.split_once(' ')?;
        scheme.eq_ignore_ascii_case("bearer").then(|| rest.trim())
    });
    // An empty expected token (the server was started without one) refuses everyone.
    if token.is_empty() || !presented.is_some_and(|p| same_secret(p.as_bytes(), token.as_bytes())) {
        return Err(Refusal::Unauthorized);
    }
    if head.method != "POST" {
        return Err(Refusal::MethodNotAllowed);
    }
    let json = header("content-type")
        .is_some_and(|ct| ct.split(';').next().is_some_and(|t| t.trim().eq_ignore_ascii_case("application/json")));
    if !json {
        return Err(Refusal::UnsupportedMedia);
    }
    Ok(Caller::from_request(header("user-agent"), header(&CHAT_HEADER.to_ascii_lowercase())))
}

/// What one running server holds. The token is behind a lock so a new one takes effect on the very
/// next request.
pub struct Shared<B> {
    backend: B,
    token: Mutex<String>,
    port: u16,
    stop: AtomicBool,
    permits: Arc<Semaphore>,
    last_refusal_log: Mutex<Option<Instant>>,
}

impl<B: Backend> Shared<B> {
    pub fn new(backend: B, port: u16, token: String) -> Shared<B> {
        Shared {
            backend,
            token: Mutex::new(token),
            port,
            stop: AtomicBool::new(false),
            permits: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            last_refusal_log: Mutex::new(None),
        }
    }

    pub fn set_token(&self, token: String) {
        *self.token.lock().unwrap_or_else(PoisonError::into_inner) = token;
    }

    fn token(&self) -> String {
        self.token.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Ends the accept loop: the flag, then one connection to wake it.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, self.port)), Duration::from_millis(500));
    }

    fn log_refusal(&self, refusal: Refusal) {
        // A GET for a stream (Claude Code sends one on every connection) and a probe of another path
        // are ordinary; only a caller that is not allowed is worth a line.
        if !matches!(refusal, Refusal::Forbidden(_) | Refusal::Unauthorized) {
            return;
        }
        let mut last = self.last_refusal_log.lock().unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|at| at.elapsed() < REFUSAL_LOG_EVERY) {
            return;
        }
        *last = Some(Instant::now());
        crate::log::line(format!("mcp: refused {}", refusal.log_text()));
    }
}

/// The loopback listener, not blocking. `port` 0 takes any free port (the tests). The socket is not
/// given SO_REUSEADDR, so a busy port is an error here and not a second owner of it.
pub fn bind(port: u16) -> std::io::Result<StdListener> {
    let listener = StdListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

/// A port given up this recently may still be held: the accept loop drops its listener on the
/// runtime a little after `stop` returns.
const JUST_STOPPED: Duration = Duration::from_secs(2);

/// `bind`; with `after_stop`, a busy port is tried again for half a second. A port that was never
/// ours is busy for good, and the answer comes at once.
fn bind_after_stop(port: u16, after_stop: bool) -> std::io::Result<StdListener> {
    let mut busy = None;
    for _ in 0..if after_stop { 10 } else { 1 } {
        match bind(port) {
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {
                busy = Some(err);
                std::thread::sleep(Duration::from_millis(50));
            }
            other => return other,
        }
    }
    Err(busy.unwrap_or_else(|| std::io::Error::from(std::io::ErrorKind::AddrInUse)))
}

/// Accepts until `Shared::stop`. Each connection is one request.
pub async fn serve<B: Backend>(listener: StdListener, shared: Arc<Shared<B>>) {
    let listener = match TcpListener::from_std(listener) {
        Ok(l) => l,
        Err(err) => {
            crate::log::line(format!("mcp: cannot listen: {}", err.kind()));
            return;
        }
    };
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(connection) => connection,
            Err(err) => {
                crate::log::line(format!("mcp: accept failed: {}", err.kind()));
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }
        if !peer.ip().is_loopback() {
            continue;
        }
        let Ok(permit) = shared.permits.clone().try_acquire_owned() else {
            continue;
        };
        let shared = shared.clone();
        tokio::spawn(async move {
            let _permit = permit;
            serve_connection(stream, &shared).await;
        });
    }
}

/// How much of a body that was refused unread is still read off the connection, and for how long.
const DRAIN_MAX: usize = 64 * 1024;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

/// Writes the response and closes. `unread` is how many bytes of the request's body have not been
/// read (a refusal comes before the body): they are read and dropped first, because Windows resets a
/// connection that is closed with received bytes unread, and the caller would lose the answer.
async fn send(stream: &mut TcpStream, bytes: &[u8], unread: usize) {
    // A caller that has gone away cannot be told anything.
    if stream.write_all(bytes).await.is_ok() {
        let _ = stream.flush().await;
    }
    let _ = stream.shutdown().await;
    let mut left = unread.min(DRAIN_MAX);
    let mut sink = [0u8; 4096];
    let _ = tokio::time::timeout(DRAIN_TIMEOUT, async {
        while left > 0 {
            match stream.read(&mut sink[..left.min(4096)]).await {
                Ok(0) | Err(_) => break,
                Ok(n) => left -= n,
            }
        }
    })
    .await;
}

fn read_failure(err: ReadError) -> Option<Vec<u8>> {
    let (status, message) = match err {
        ReadError::Closed | ReadError::Io => return None,
        ReadError::TooLarge => (413, "The request is too large."),
        ReadError::Malformed(_) => (400, "The request could not be read."),
        ReadError::Unsupported => (501, "Send the body with a Content-Length, not chunked."),
    };
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32600, "message": message } }).to_string();
    Some(http::response(status, &[], Some(body.as_bytes())))
}

async fn serve_connection<B: Backend>(mut stream: TcpStream, shared: &Shared<B>) {
    let (head, leftover) = match tokio::time::timeout(HEAD_TIMEOUT, http::read_head(&mut stream)).await {
        Ok(Ok(read)) => read,
        Ok(Err(err)) => {
            if let Some(reply) = read_failure(err) {
                send(&mut stream, &reply, 0).await;
            }
            return;
        }
        Err(_) => return,
    };
    let unread = head.content_length.saturating_sub(leftover.len());
    let caller = match check_head(&head, shared.port, &shared.token()) {
        Ok(caller) => caller,
        Err(refusal) => {
            shared.log_refusal(refusal);
            send(&mut stream, &refusal.to_response(), unread).await;
            return;
        }
    };
    if head.expects_continue && stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await.is_err() {
        return;
    }
    let body = match tokio::time::timeout(BODY_TIMEOUT, http::read_body(&mut stream, &head, leftover)).await {
        Ok(Ok(body)) => body,
        Ok(Err(err)) => {
            if let Some(reply) = read_failure(err) {
                send(&mut stream, &reply, 0).await;
            }
            return;
        }
        Err(_) => return,
    };
    let reply = match rpc::handle(&shared.backend, &caller, &body).await {
        rpc::Reply::Accepted => http::response(202, &[], None),
        rpc::Reply::Json(status, value) => http::response(status, &[], Some(value.to_string().as_bytes())),
    };
    send(&mut stream, &reply, 0).await;
}

/// What Settings shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    pub url: String,
    /// Why it does not run though it is switched on, as a text the page translates.
    pub error: Option<Message>,
}

struct Running {
    port: u16,
    shared: Arc<Shared<AppBackend>>,
}

/// The app's one MCP server: managed state, started and stopped from the Settings commands.
#[derive(Default)]
pub struct McpServer {
    running: Mutex<Option<Running>>,
    error: Mutex<Option<Message>>,
    /// When a running server was last stopped by this app (`JUST_STOPPED`).
    stopped_at: Mutex<Option<Instant>>,
}

impl McpServer {
    fn running(&self) -> std::sync::MutexGuard<'_, Option<Running>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts listening on `port` (and stops what ran before on another one). A port already in
    /// use is `Err` and leaves nothing running.
    pub fn start(&self, port: u16, token: String, backend: AppBackend) -> Result<(), Message> {
        let mut running = self.running();
        if running.as_ref().is_some_and(|r| r.port == port) {
            return Ok(());
        }
        if let Some(old) = running.take() {
            old.shared.stop();
            old.shared.backend.release_asks();
            *self.stopped_at.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        }
        let after_stop = self.stopped_at.lock().unwrap_or_else(PoisonError::into_inner).is_some_and(|at| at.elapsed() < JUST_STOPPED);
        let listener = match bind_after_stop(port, after_stop) {
            Ok(l) => l,
            Err(err) => {
                let message = if err.kind() == std::io::ErrorKind::AddrInUse {
                    Message::new(
                        "msg.mcp.portInUse",
                        format!("Port {port} is already in use. Set \"mcpPort\" in settings.json to a free port."),
                    )
                } else {
                    Message::new("msg.mcp.listenFailed", format!("Could not listen on port {port}."))
                };
                crate::log::line(format!("mcp: cannot listen on {port}: {:?}", err.kind()));
                drop(running);
                return Err(self.failed(message.with("port", port.to_string())));
            }
        };
        *self.error.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let shared = Arc::new(Shared::new(backend, port, token));
        tauri::async_runtime::spawn(serve(listener, shared.clone()));
        crate::log::line(format!("mcp: listening on 127.0.0.1:{port}"));
        *running = Some(Running { port, shared });
        Ok(())
    }

    /// Keeps `message` for Settings to show, and hands it back.
    pub fn failed(&self, message: Message) -> Message {
        *self.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(message.clone());
        message
    }

    pub fn stop(&self) {
        if let Some(old) = self.running().take() {
            old.shared.stop();
            // Questions still on the island go back to their agents.
            old.shared.backend.release_asks();
            *self.stopped_at.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
            crate::log::line("mcp: stopped");
        }
        *self.error.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// What a CLI that Mayak starts (the chat, for its conversation `chat`) is given to reach the
    /// running server, or None while the server is off.
    pub fn attach_for(&self, cli: Cli, chat: Option<u64>) -> Option<Attach> {
        let running = self.running();
        let r = running.as_ref()?;
        Some(attach(cli, &url_for(r.port), &r.shared.token(), chat))
    }

    /// A new token for the running server (the old one stops working at once).
    pub fn set_token(&self, token: String) {
        if let Some(r) = self.running().as_ref() {
            r.shared.set_token(token);
        }
    }

    pub fn status(&self, enabled: bool, port: u16) -> Status {
        let running = self.running().is_some();
        Status {
            enabled,
            running,
            port,
            url: url_for(port),
            error: self.error.lock().unwrap_or_else(PoisonError::into_inner).clone().filter(|_| enabled && !running),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tools::fake::Fake;
    use super::tools::AskOutcome;
    use super::*;
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const TOKEN: &str = "t0ken-t0ken-t0ken";

    fn run<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    /// A server on a free port with a fake backend; `f` gets its port.
    fn with_server<T>(fake: Fake, f: impl FnOnce(u16, Arc<Shared<Fake>>) -> std::pin::Pin<Box<dyn std::future::Future<Output = T>>>) -> T {
        run(async move {
            let listener = bind(0).unwrap();
            let port = listener.local_addr().unwrap().port();
            let shared = Arc::new(Shared::new(fake, port, TOKEN.to_string()));
            let task = tokio::spawn(serve(listener, shared.clone()));
            let out = f(port, shared.clone()).await;
            shared.stop();
            let _ = task.await;
            out
        })
    }

    /// One raw request; the whole response text.
    async fn raw(port: u16, request: &[u8]) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
        stream.write_all(request).await.unwrap();
        let mut out = Vec::new();
        stream.read_to_end(&mut out).await.unwrap();
        String::from_utf8(out).unwrap()
    }

    fn post(port: u16, token: Option<&str>, extra: &str, body: &str) -> Vec<u8> {
        let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
        format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{auth}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nUser-Agent: claude-code/2.1.296\r\n{extra}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    fn status_of(response: &str) -> u16 {
        response.split(' ').nth(1).unwrap().parse().unwrap()
    }

    fn body_of(response: &str) -> Value {
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"claude-code","version":"2.1.296"}}}"#;

    #[test]
    fn a_client_can_initialize_list_tools_and_call_one_over_http() {
        with_server(Fake::new(), |port, _| {
            Box::pin(async move {
                let r = raw(port, &post(port, Some(TOKEN), "", INITIALIZE)).await;
                assert_eq!(status_of(&r), 200);
                assert!(r.contains("Content-Type: application/json\r\n"));
                assert!(r.contains("Connection: close\r\n"));
                let v = body_of(&r);
                assert_eq!(v["result"]["protocolVersion"], "2025-11-25");
                assert_eq!(v["result"]["serverInfo"]["name"], "mayak");

                let r = raw(port, &post(port, Some(TOKEN), "MCP-Protocol-Version: 2025-11-25\r\n", r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)).await;
                assert_eq!(status_of(&r), 202);
                assert!(r.ends_with("\r\n\r\n"), "no body on 202");

                let r = raw(port, &post(port, Some(TOKEN), "", r#"{"method":"tools/list","jsonrpc":"2.0","id":1}"#)).await;
                assert_eq!(body_of(&r)["result"]["tools"].as_array().unwrap().len(), tools::names().len());

                let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_sessions","arguments":{}}}"#;
                let r = raw(port, &post(port, Some(TOKEN), "", call)).await;
                let v = body_of(&r);
                assert_eq!(v["result"]["isError"], false);
                assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains("Fix the pipe"));
            })
        });
    }

    #[test]
    fn a_request_without_the_right_token_is_refused_with_401_and_never_reaches_the_tools() {
        let fake = Fake::new();
        *fake.answer.lock().unwrap() = AskOutcome::Busy;
        with_server(fake, |port, shared| {
            Box::pin(async move {
                let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ask_user","arguments":{"question":"q","options":["a","b"]}}}"#;
                let wrong_length = "x".repeat(200);
                for token in [None, Some(""), Some("wrong"), Some(TOKEN.to_uppercase().as_str()), Some(&TOKEN[..TOKEN.len() - 1]), Some(wrong_length.as_str())] {
                    let r = raw(port, &post(port, token, "", call)).await;
                    assert_eq!(status_of(&r), 401, "{token:?}");
                    assert!(r.contains("WWW-Authenticate: Bearer"));
                    assert_eq!(body_of(&r)["error"]["code"], -32600);
                    assert!(!r.contains(TOKEN));
                }
                // Another scheme with the right secret is no token either.
                let basic = post(port, None, &format!("Authorization: Basic {TOKEN}\r\n"), call);
                assert_eq!(status_of(&raw(port, &basic).await), 401);
                assert!(shared.backend.asked.lock().unwrap().is_empty(), "nothing was asked on the island");
                // The scheme's case does not matter, the token's does.
                let lower = post(port, None, &format!("authorization: bearer {TOKEN}\r\n"), INITIALIZE);
                assert_eq!(status_of(&raw(port, &lower).await), 200);
            })
        });
    }

    #[test]
    fn a_new_token_works_at_once_and_the_old_one_stops() {
        with_server(Fake::new(), |port, shared| {
            Box::pin(async move {
                assert_eq!(status_of(&raw(port, &post(port, Some(TOKEN), "", INITIALIZE)).await), 200);
                shared.set_token("another-token".into());
                assert_eq!(status_of(&raw(port, &post(port, Some(TOKEN), "", INITIALIZE)).await), 401);
                assert_eq!(status_of(&raw(port, &post(port, Some("another-token"), "", INITIALIZE)).await), 200);
                shared.set_token(String::new());
                // A server with no token refuses everyone, an empty header included.
                assert_eq!(status_of(&raw(port, &post(port, Some(""), "", INITIALIZE)).await), 401);
            })
        });
    }

    #[test]
    fn a_wrong_host_or_origin_is_refused_before_the_token_is_looked_at() {
        with_server(Fake::new(), |port, _| {
            Box::pin(async move {
                let with_host = |host: &str| {
                    let body = INITIALIZE;
                    format!(
                        "POST /mcp HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .into_bytes()
                };
                // DNS rebinding: the page's own name arrives as Host.
                for host in ["evil.example:80", &format!("evil.example:{port}"), "127.0.0.1", &format!("127.0.0.1:{}", port.wrapping_add(1))] {
                    assert_eq!(status_of(&raw(port, &with_host(host)).await), 403, "{host}");
                }
                for host in [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("LOCALHOST:{port}")] {
                    assert_eq!(status_of(&raw(port, &with_host(&host)).await), 200, "{host}");
                }
                for origin in ["https://evil.example", "http://evil.example:80", "null", "http://127.0.0.1.evil.example", "file://"] {
                    let r = raw(port, &post(port, Some(TOKEN), &format!("Origin: {origin}\r\n"), INITIALIZE)).await;
                    assert_eq!(status_of(&r), 403, "{origin}");
                }
                for origin in ["http://localhost:6274", "http://127.0.0.1:1234", "http://[::1]:3000"] {
                    let r = raw(port, &post(port, Some(TOKEN), &format!("Origin: {origin}\r\n"), INITIALIZE)).await;
                    assert_eq!(status_of(&r), 200, "{origin}");
                }
            })
        });
    }

    #[test]
    fn only_post_to_the_one_path_with_a_json_body_is_served() {
        with_server(Fake::new(), |port, _| {
            Box::pin(async move {
                let get = |method: &str| {
                    format!("{method} /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {TOKEN}\r\nAccept: text/event-stream\r\n\r\n").into_bytes()
                };
                // Claude Code opens a GET for a server-to-client stream; "405" tells it there is none.
                for method in ["GET", "DELETE", "PUT", "OPTIONS"] {
                    let r = raw(port, &get(method)).await;
                    assert_eq!(status_of(&r), 405, "{method}");
                    assert!(r.contains("Allow: POST\r\n"));
                }
                let other_path = format!("POST /other HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\n\r\n");
                assert_eq!(status_of(&raw(port, other_path.as_bytes()).await), 404);
                let well_known = format!("GET /.well-known/oauth-protected-resource HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
                assert_eq!(status_of(&raw(port, well_known.as_bytes()).await), 404);
                let text = format!(
                    "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\n{{}}"
                );
                assert_eq!(status_of(&raw(port, text.as_bytes()).await), 415);
                let charset = post(port, Some(TOKEN), "", INITIALIZE);
                let charset = String::from_utf8(charset).unwrap().replace("application/json\r\n", "application/json; charset=utf-8\r\n");
                assert_eq!(status_of(&raw(port, charset.as_bytes()).await), 200);
            })
        });
    }

    #[test]
    fn a_json_error_body_comes_back_as_400_and_a_bad_request_is_refused() {
        with_server(Fake::new(), |port, _| {
            Box::pin(async move {
                let r = raw(port, &post(port, Some(TOKEN), "", "{not json")).await;
                assert_eq!(status_of(&r), 400);
                assert_eq!(body_of(&r)["error"]["code"], -32700);
                // The chunked body is refused, not misread.
                let chunked = format!(
                    "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{{}}\r\n0\r\n\r\n"
                );
                assert_eq!(status_of(&raw(port, chunked.as_bytes()).await), 501);
                assert_eq!(status_of(&raw(port, b"nonsense\r\n\r\n").await), 400);
                let too_big = format!(
                    "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    http::MAX_BODY + 1
                );
                assert_eq!(status_of(&raw(port, too_big.as_bytes()).await), 413);
            })
        });
    }

    #[test]
    fn a_body_is_not_read_from_a_caller_with_no_token() {
        with_server(Fake::new(), |port, _| {
            Box::pin(async move {
                // The head promises 1 MiB that never comes; the refusal does not wait for it.
                let head = format!(
                    "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    http::MAX_BODY
                );
                let started = Instant::now();
                let r = raw(port, head.as_bytes()).await;
                assert_eq!(status_of(&r), 401);
                assert!(started.elapsed() < Duration::from_secs(3));
            })
        });
    }

    #[test]
    fn expect_100_continue_is_answered_once_the_caller_is_known() {
        with_server(Fake::new(), |port, _| {
            Box::pin(async move {
                let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
                let head = format!(
                    "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nExpect: 100-continue\r\nContent-Length: {}\r\n\r\n",
                    INITIALIZE.len()
                );
                stream.write_all(head.as_bytes()).await.unwrap();
                let mut interim = [0u8; 25];
                stream.read_exact(&mut interim).await.unwrap();
                assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
                stream.write_all(INITIALIZE.as_bytes()).await.unwrap();
                let mut rest = Vec::new();
                stream.read_to_end(&mut rest).await.unwrap();
                assert_eq!(status_of(std::str::from_utf8(&rest).unwrap()), 200);
            })
        });
    }

    #[test]
    fn ask_user_over_http_returns_what_the_island_answered() {
        let fake = Fake::new();
        *fake.answer.lock().unwrap() = AskOutcome::Answered(json!({ "Deploy?": "No" }));
        with_server(fake, |port, shared| {
            Box::pin(async move {
                let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ask_user","arguments":{"question":"Deploy?","options":["Yes","No"]}}}"#;
                let r = raw(port, &post(port, Some(TOKEN), "", call)).await;
                let text = body_of(&r)["result"]["content"][0]["text"].as_str().unwrap().to_string();
                assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), json!({ "answered": true, "answer": "No" }));
                // The label comes from the User-Agent the real Claude Code sent.
                assert_eq!(shared.backend.asked.lock().unwrap()[0].asker, "Claude Code");
            })
        });
    }

    #[test]
    fn stopping_frees_the_port() {
        run(async {
            let listener = bind(0).unwrap();
            let port = listener.local_addr().unwrap().port();
            let shared = Arc::new(Shared::new(Fake::new(), port, TOKEN.into()));
            let task = tokio::spawn(serve(listener, shared.clone()));
            assert!(bind(port).is_err(), "the port is held while it serves");
            shared.stop();
            task.await.unwrap();
            assert!(bind(port).is_ok(), "and free again after");
        });
    }

    #[test]
    fn a_port_given_up_a_moment_ago_is_taken_once_it_is_free() {
        let first = bind(0).unwrap();
        let port = first.local_addr().unwrap().port();
        let holder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            drop(first);
        });
        assert!(bind_after_stop(port, true).is_ok());
        holder.join().unwrap();
        // One that stays busy is an error: after the half second when the app had just stopped
        // it, at once when it was never the app's.
        let held = bind(0).unwrap();
        let busy_port = held.local_addr().unwrap().port();
        let started = Instant::now();
        assert_eq!(bind_after_stop(busy_port, true).unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
        assert!(started.elapsed() >= Duration::from_millis(400));
        let started = Instant::now();
        assert_eq!(bind_after_stop(busy_port, false).unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn a_busy_port_is_an_error_not_a_second_owner() {
        let first = bind(0).unwrap();
        let port = first.local_addr().unwrap().port();
        assert_eq!(bind(port).unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
    }

    #[test]
    fn the_server_listens_on_the_loopback_interface_only() {
        let listener = bind(0).unwrap();
        assert_eq!(listener.local_addr().unwrap().ip(), std::net::IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn a_token_is_64_hex_characters_and_never_the_same_twice() {
        let a = new_token().unwrap();
        let b = new_token().unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn the_saved_token_is_kept_and_a_missing_or_damaged_one_is_replaced_and_saved() {
        let good = "ab".repeat(32);
        let saved = std::cell::RefCell::new(Vec::<String>::new());
        let store = |t: &str| {
            saved.borrow_mut().push(t.to_string());
            Ok(())
        };
        let make = || Ok("c0".repeat(32));
        assert_eq!(ensure_token(Some(good.clone()), store, make).unwrap(), good);
        assert!(saved.borrow().is_empty(), "nothing is written when the token is fine");
        for bad in [None, Some(String::new()), Some("short".to_string()), Some("AB".repeat(32)), Some("zz".repeat(32)), Some("ab".repeat(33))] {
            saved.borrow_mut().clear();
            assert_eq!(ensure_token(bad.clone(), store, make).unwrap(), "c0".repeat(32), "{bad:?}");
            assert_eq!(*saved.borrow(), ["c0".repeat(32)]);
        }
        // A token that cannot be saved is no token: the server must not start with one nobody can read back.
        assert!(ensure_token(None, |_| Err("locked".into()), make).is_err());
        assert!(ensure_token(None, store, || Err("no random".into())).is_err());
    }

    #[test]
    fn secrets_are_compared_whole() {
        assert!(same_secret(b"abc", b"abc"));
        assert!(!same_secret(b"abc", b"abd"));
        assert!(!same_secret(b"abc", b"abcd"));
        assert!(!same_secret(b"", b"a"));
        assert!(same_secret(b"", b""));
    }

    #[test]
    fn hosts_are_this_machine_on_this_port() {
        assert!(loopback_host("127.0.0.1:47831", Some(47831)));
        assert!(loopback_host("localhost:47831", Some(47831)));
        assert!(loopback_host("[::1]:47831", Some(47831)));
        assert!(!loopback_host("127.0.0.1", Some(47831)));
        assert!(!loopback_host("127.0.0.1:47832", Some(47831)));
        assert!(!loopback_host("127.0.0.1:80", Some(47831)));
        assert!(!loopback_host("0.0.0.0:47831", Some(47831)));
        assert!(!loopback_host("evil.example:47831", Some(47831)));
        assert!(!loopback_host("127.0.0.1.evil.example:47831", Some(47831)));
        assert!(!loopback_host("", Some(1)));
        // An origin names no port to match.
        assert!(loopback_host("localhost", None));
        assert!(loopback_host("localhost:3000", None));
        assert!(!loopback_host("example.com", None));
    }

    #[test]
    fn the_url_is_the_loopback_address_and_the_endpoint() {
        assert_eq!(url_for(47831), "http://127.0.0.1:47831/mcp");
    }
}
