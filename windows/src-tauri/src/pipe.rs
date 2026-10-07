// Named-pipe server and router for mayak-hook.
//
// `\\.\pipe\mayak-<sid>` — one instance per connection, one line of JSON per
// connection. `route()` decides what a line is for:
//   * Claude PermissionRequest → Approval: the Allow / Always / Deny card;
//   * Claude PreToolUse for AskUserQuestion → Question: the question card;
//   * Codex PermissionRequest → Approval (Allow / Deny) while Mayak's Codex hook
//     waits for an answer (`codex_hooks::answers_enabled`), and the observer
//     gets the line too, so the session shows as waiting; otherwise Observe;
//   * any other Codex event → Observe: handed to the observer, never answered;
//   * anything else → Ignore: for Claude the session observer is the source of
//     truth, not hooks.
// Approval and Question are the only connections kept open: they wait for the
// island and write its reply back on the same pipe.
//
// The agent is never blocked by us. Three things guarantee it:
//   * mayak-hook gives the connection 300 ms and exits cleanly if we are closed;
//   * we only wait for a human once the island has *confirmed* the card is on
//     screen, so a paused island or a webview that is not listening costs a few
//     hundred milliseconds, not ten minutes;
//   * whatever happens we drop the connection after the decision timeout, and
//     the agent asks in its own window.
//
// What we write back is one line: `allow`, `always`, `deny`, `release` or
// `answer <compact JSON of the answers>`. Turning that into the documented
// hookSpecificOutput JSON is mayak-hook's job, so the wire format the agents
// expect lives in exactly one place.
//
// The app is plugged in through `RouterSink` (lib.rs), so this file knows
// nothing about Tauri and its tests run a real pipe with a test sink.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::mpsc;
use windows::core::HSTRING;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

use crate::observe::keys::SessionKey;

/// A line longer than this is dropped, never parsed.
pub const MAX_PAYLOAD: usize = 1 << 20;
/// How long the island gets to say "the card is up". Without it, an island
/// that is paused, hidden behind a crashed webview or simply not listening
/// would leave Claude Code staring at a prompt nobody can see.
const ACK_TIMEOUT: Duration = Duration::from_millis(800);
/// Under mayak-hook's own 595 s wait (the hooks get 600 s), so we always
/// answer first.
const DECISION_TIMEOUT: Duration = Duration::from_secs(590);

/// What the island can say about a request.
enum Reply {
    /// The card is on screen and a human can act on it.
    Ack,
    /// A human clicked: the reply line, without its '\n'.
    Decision(String),
    /// Released without a decision: paused, another card holds the view, the
    /// island folded, or "In the terminal". The relay is told `release`.
    Decline,
}

/// What a card asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Permission,
    Question,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Permission => "permission",
            Kind::Question => "question",
        }
    }
}

/// What the island is told about a request, besides the payload itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Card {
    pub kind: Kind,
    /// "claude" or "codex".
    pub agent: &'static str,
    /// A Claude permission with `permission_suggestions`: "Always" has a rule
    /// to hand back. Never for Codex, whose hook fails closed on it.
    pub can_always: bool,
}

struct Waiter {
    tx: mpsc::Sender<Reply>,
    kind: Kind,
    can_always: bool,
}

/// Requests the island has been told about, by request id.
#[derive(Default)]
pub struct Pending(Mutex<HashMap<String, Waiter>>);

/// Whether Codex permissions go to the island: Mayak's Codex `PermissionRequest`
/// hook waits 600 s (`codex_hooks::answers_enabled`). Read from hooks.json at
/// launch and after each Codex install or removal.
#[derive(Default)]
pub struct CodexAnswers(AtomicBool);

impl CodexAnswers {
    pub fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn set(&self, on: bool) {
        self.0.store(on, Ordering::Relaxed);
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Observe,
    Approval,
    Question,
    Ignore,
}

/// `agent` is payload["agent"] ("claude" when absent: a Coucou-era relay only
/// ever relayed Claude). Any other agent is ignored; mayak-hook sends nothing
/// for a foreign `--agent` anyway (D86), and upstream's `coucou_agent` tag is
/// not read. `codex_answers` is `CodexAnswers::get`: with Mayak's Codex hook
/// still on its short timeout, a card would die with the hook, so Codex stays
/// observed only.
pub fn route(agent: &str, event: &str, payload: &Value, codex_answers: bool) -> Route {
    match (agent, event) {
        ("codex", "PermissionRequest") if codex_answers => Route::Approval,
        ("codex", _) => Route::Observe,
        // A question is answered through PreToolUse only. Allow or Deny on its
        // permission request would break the answer; the relay already drops it
        // (D87), and an older relay must not get a card for it either.
        ("claude", "PermissionRequest") if is_question(payload) => Route::Ignore,
        ("claude", "PermissionRequest") => Route::Approval,
        ("claude", "PreToolUse") if is_question(payload) => Route::Question,
        _ => Route::Ignore,
    }
}

fn is_question(payload: &Value) -> bool {
    payload.get("tool_name").and_then(Value::as_str) == Some("AskUserQuestion")
}

/// A non-empty `permission_suggestions` list: the rule "Always" hands back.
pub fn has_suggestions(payload: &Value) -> bool {
    payload.get("permission_suggestions").and_then(Value::as_array).is_some_and(|rules| !rules.is_empty())
}

/// What the island shows about the session behind an approval card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApprovalContext {
    pub session_key: Option<SessionKey>,
    pub project_name: Option<String>,
}

/// Sets `request_id`, `session_key` and `project_name` (string or null), and
/// the card's `kind` ("permission" or "question"), `agent` ("claude" or
/// "codex") and `canAlways`.
pub fn annotate_approval(payload: &mut Value, request_id: &str, ctx: &ApprovalContext, card: &Card) {
    if let Some(map) = payload.as_object_mut() {
        map.insert("request_id".into(), json!(request_id));
        map.insert("session_key".into(), json!(ctx.session_key));
        map.insert("project_name".into(), json!(ctx.project_name));
        map.insert("kind".into(), json!(card.kind.as_str()));
        map.insert("agent".into(), json!(card.agent));
        map.insert("canAlways".into(), json!(card.can_always));
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
    /// `CodexAnswers::get` of the app.
    fn codex_answers(&self) -> bool;
    /// Route::Observe, and a Codex Route::Approval. Must not block.
    fn observe(&self, payload: Value);
    /// Route::Approval and Route::Question: data for `annotate_approval`. May
    /// block (Win32 + file probes); `serve` always calls it through
    /// `spawn_blocking`.
    fn approval_context(&self, payload: &Value) -> ApprovalContext;
    /// Route::Approval and Route::Question: the annotated payload to show.
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
///
/// Every instance carries a protected DACL with a single entry: full access for
/// the current user's SID. The default DACL also lets other accounts read and
/// connect; with this one only our own account (the relay) can. Spike S4
/// confirmed that the relay started by MSIX Codex Desktop still connects.
pub fn create_instance(name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
    let sid = crate::platform::current_user_sid()
        .ok_or_else(|| std::io::Error::other("cannot read the current user SID"))?;
    let descriptor = SecurityDescriptor::from_sddl(&current_user_sddl(&sid))?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0 .0,
        bInheritHandle: false.into(),
    };
    let mut options = ServerOptions::new();
    options.first_pipe_instance(first);
    // SAFETY: `attributes` and the descriptor it points to outlive the call.
    unsafe {
        options.create_with_security_attributes_raw(
            name,
            (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
        )
    }
}

/// `D:P(A;;GA;;;<sid>)` — protected (nothing inherited), one allow entry.
fn current_user_sddl(sid: &str) -> String {
    format!("D:P(A;;GA;;;{sid})")
}

/// A security descriptor allocated by Windows, freed with LocalFree on drop.
struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl SecurityDescriptor {
    fn from_sddl(sddl: &str) -> std::io::Result<Self> {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &HSTRING::from(sddl),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(std::io::Error::from)?;
        Ok(Self(descriptor))
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0 .0)));
        }
    }
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

    match route(&agent, &event, &payload, sink.codex_answers()) {
        Route::Observe => {
            sink.observe(payload);
            let _ = pipe.disconnect();
        }
        Route::Ignore => {
            let _ = pipe.disconnect();
        }
        Route::Approval if agent == "codex" => {
            // The observer still shows the session as waiting, exactly as in Observe.
            sink.observe(payload.clone());
            let card = Card { kind: Kind::Permission, agent: "codex", can_always: false };
            approval(pipe, payload, card, sink).await
        }
        Route::Approval => {
            let card = Card { kind: Kind::Permission, agent: "claude", can_always: has_suggestions(&payload) };
            approval(pipe, payload, card, sink).await
        }
        Route::Question => {
            let card = Card { kind: Kind::Question, agent: "claude", can_always: false };
            approval(pipe, payload, card, sink).await
        }
    }
}

async fn approval<S: RouterSink>(mut pipe: NamedPipeServer, mut payload: Value, card: Card, sink: Arc<S>) {
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
    let waiter = Waiter { tx, kind: card.kind, can_always: card.can_always };
    sink.pending().0.lock().unwrap().insert(id.clone(), waiter);
    annotate_approval(&mut payload, &id, &ctx, &card);
    note(format!("{} id={id} agent={}", card.kind.as_str(), card.agent));
    sink.show_approval(payload);

    let line = wait_for_decision(&id, &mut rx).await;
    sink.pending().0.lock().unwrap().remove(&id);

    // Not acknowledged or timed out: say nothing at all. mayak-hook then writes
    // nothing to stdout and the agent asks in its own window, exactly as if
    // Mayak were closed.
    if let Some(line) = line {
        let _ = pipe.write_all(format!("{line}\n").as_bytes()).await;
        let _ = pipe.flush().await;
    }
    let _ = pipe.disconnect();
}

/// The first word of a reply line: what the log may say about it. An answer
/// line carries the labels the user picked, and those stay out of the log.
fn reply_word(line: &str) -> &str {
    line.split(' ').next().unwrap_or_default()
}

/// Two waits: a short one for "the card is up", then the long one for a human.
/// Returns the line to write: a decision, `release`, or None to write nothing.
async fn wait_for_decision(id: &str, rx: &mut mpsc::Receiver<Reply>) -> Option<String> {
    match tokio::time::timeout(ACK_TIMEOUT, rx.recv()).await {
        Ok(Some(Reply::Ack)) => {}
        // A click that beats the ack is still a click.
        Ok(Some(Reply::Decision(d))) => {
            note(format!("approval id={id} answered {}", reply_word(&d)));
            return Some(d);
        }
        Ok(Some(Reply::Decline)) => {
            note(format!("approval id={id} not shown — the agent asks itself"));
            return Some("release".into());
        }
        Ok(None) => return None,
        Err(_) => {
            note(format!("approval id={id} island never acknowledged — the agent asks itself"));
            return None;
        }
    }

    match tokio::time::timeout(DECISION_TIMEOUT, rx.recv()).await {
        Ok(Some(Reply::Decision(d))) => {
            note(format!("approval id={id} answered {}", reply_word(&d)));
            Some(d)
        }
        Ok(Some(Reply::Decline)) => {
            note(format!("approval id={id} released without a decision"));
            Some("release".into())
        }
        _ => {
            note(format!("approval id={id} timed out — the agent asks itself"));
            None
        }
    }
}

/// Removes and returns the waiter of `request_id` when `accepts` it. An
/// unknown id (already answered, released or timed out) and a reply of the
/// wrong kind are ignored: a late click must change nothing.
fn take(pending: &Pending, request_id: &str, accepts: impl FnOnce(&Waiter) -> bool) -> Option<Waiter> {
    // Decided under the lock, logged after it is released.
    let refused: Result<Waiter, Option<Kind>> = {
        let mut map = pending.0.lock().unwrap();
        match map.get(request_id) {
            None => Err(None),
            Some(waiter) if !accepts(waiter) => Err(Some(waiter.kind)),
            Some(_) => map.remove(request_id).ok_or(None),
        }
    };
    match refused {
        Ok(waiter) => Some(waiter),
        Err(None) => {
            note(format!("reply for id={request_id} — no pending request"));
            None
        }
        Err(Some(kind)) => {
            note(format!("reply for id={request_id} — not a {} reply", kind.as_str()));
            None
        }
    }
}

/// The island has the card on screen; the long wait may begin.
pub fn acknowledge(pending: &Pending, request_id: &str) {
    let tx = pending.0.lock().unwrap().get(request_id).map(|w| w.tx.clone());
    match tx {
        Some(tx) => {
            let _ = tx.try_send(Reply::Ack);
        }
        None => note(format!("reply for id={request_id} — no pending request")),
    }
}

/// Release without a decision: paused, another card already holds the view,
/// the island folded, or "In the terminal". The relay is told `release`.
pub fn decline(pending: &Pending, request_id: &str) {
    note(format!("decline id={request_id}"));
    if let Some(waiter) = take(pending, request_id, |_| true) {
        let _ = waiter.tx.try_send(Reply::Decline);
    }
}

/// The word the relay gets for a permission button. `always` only where the
/// card has a rule to hand back (Claude with `permission_suggestions`), a
/// plain `allow` otherwise; anything unknown stays `deny`, as before.
pub fn permission_word(decision: &str, can_always: bool) -> &'static str {
    match decision {
        "allow" => "allow",
        "always" if can_always => "always",
        "always" => "allow",
        _ => "deny",
    }
}

/// Called by the island's Allow / Always / Deny buttons. Only ever a bare word:
/// turning it into the agent's JSON is mayak-hook's job.
pub fn answer(pending: &Pending, request_id: &str, decision: &str) {
    let Some(waiter) = take(pending, request_id, |w| w.kind == Kind::Permission) else { return };
    let word = permission_word(decision, waiter.can_always);
    note(format!("decision id={request_id} {word}"));
    let _ = waiter.tx.try_send(Reply::Decision(word.to_string()));
}

/// Answers that are not "question text → label, or a non-empty list of labels".
/// It never quotes what it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswersError;

impl std::fmt::Display for AnswersError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("The answers must map each question to a label or a list of labels.")
    }
}

/// `answer <compact JSON>` for a non-empty object whose every value is a label
/// or a non-empty list of labels. Compact JSON escapes line breaks inside the
/// texts, so the reply stays one line.
pub fn answers_line(answers: &Value) -> Result<String, AnswersError> {
    let Value::Object(map) = answers else { return Err(AnswersError) };
    let labels = |v: &Value| match v {
        Value::String(_) => true,
        Value::Array(items) => !items.is_empty() && items.iter().all(Value::is_string),
        _ => false,
    };
    if map.is_empty() || !map.values().all(labels) {
        return Err(AnswersError);
    }
    Ok(format!("answer {answers}"))
}

/// Called by the island's question card. Answers that are not labels are
/// refused and the card keeps waiting; an unknown id is ignored.
pub fn answer_question(pending: &Pending, request_id: &str, answers: &Value) -> Result<(), AnswersError> {
    let line = answers_line(answers)?;
    let Some(waiter) = take(pending, request_id, |w| w.kind == Kind::Question) else { return Ok(()) };
    note(format!("decision id={request_id} answer"));
    let _ = waiter.tx.try_send(Reply::Decision(line));
    Ok(())
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
        codex_answers: CodexAnswers,
        observed: UnboundedSender<Value>,
        shown: UnboundedSender<Value>,
    }

    impl RouterSink for TestSink {
        fn pending(&self) -> &Pending {
            &self.pending
        }
        fn codex_answers(&self) -> bool {
            self.codex_answers.get()
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

    type Rig = (Arc<TestSink>, UnboundedReceiver<Value>, UnboundedReceiver<Value>);

    fn sink(ctx: ApprovalContext) -> Rig {
        sink_with(ctx, false)
    }

    fn sink_with(ctx: ApprovalContext, codex_answers: bool) -> Rig {
        let (observed, observed_rx) = unbounded_channel();
        let (shown, shown_rx) = unbounded_channel();
        let flag = CodexAnswers::default();
        flag.set(codex_answers);
        let s = TestSink { pending: Pending::default(), ctx, codex_answers: flag, observed, shown };
        (Arc::new(s), observed_rx, shown_rx)
    }

    fn pending_is_empty(s: &TestSink) -> bool {
        s.pending.0.lock().unwrap().is_empty()
    }

    /// Sends one line and waits for the card it raises.
    async fn card_for(name: &str, shown: &mut UnboundedReceiver<Value>, line: &str) -> (NamedPipeClient, Value, String) {
        let mut c = client(name).await;
        c.write_all(line.as_bytes()).await.unwrap();
        let card = recv(shown).await;
        let id = card["request_id"].as_str().unwrap().to_string();
        (c, card, id)
    }

    const CLAUDE_PERMISSION: &str =
        "{\"agent\":\"claude\",\"hook_event_name\":\"PermissionRequest\",\"session_id\":\"s\",\"tool_name\":\"Bash\"}\n";
    const CLAUDE_PERMISSION_WITH_RULES: &str = "{\"agent\":\"claude\",\"hook_event_name\":\"PermissionRequest\",\
        \"session_id\":\"s\",\"tool_name\":\"Bash\",\"permission_suggestions\":[{\"type\":\"addRules\"}]}\n";
    const CODEX_PERMISSION: &str = "{\"agent\":\"codex\",\"hook_event_name\":\"PermissionRequest\",\
        \"session_id\":\"codex-s\",\"turn_id\":\"t\",\"permission_suggestions\":[{\"type\":\"addRules\"}]}\n";
    const CLAUDE_QUESTION: &str = "{\"agent\":\"claude\",\"hook_event_name\":\"PreToolUse\",\"session_id\":\"s\",\
        \"tool_name\":\"AskUserQuestion\",\"tool_input\":{\"questions\":[]}}\n";

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
        let question = json!({ "tool_name": "AskUserQuestion" });
        let bash = json!({ "tool_name": "Bash" });
        let none = json!({});
        for codex_answers in [false, true] {
            let r = |agent: &str, event: &str, payload: &Value| route(agent, event, payload, codex_answers);
            assert_eq!(r("codex", "Stop", &none), Route::Observe);
            assert_eq!(r("codex", "UserPromptSubmit", &none), Route::Observe);
            assert_eq!(r("codex", "PreToolUse", &question), Route::Observe);
            assert_eq!(r("codex", "", &none), Route::Observe);
            assert_eq!(r("claude", "PermissionRequest", &none), Route::Approval);
            assert_eq!(r("claude", "PermissionRequest", &bash), Route::Approval);
            assert_eq!(r("claude", "PermissionRequest", &question), Route::Ignore, "questions never become permissions");
            assert_eq!(r("claude", "PreToolUse", &question), Route::Question);
            assert_eq!(r("claude", "PreToolUse", &bash), Route::Ignore);
            assert_eq!(r("claude", "PreToolUse", &none), Route::Ignore);
            assert_eq!(r("claude", "Stop", &none), Route::Ignore);
            assert_eq!(r("claude", "UserPromptSubmit", &none), Route::Ignore);
            assert_eq!(r("claude", "", &none), Route::Ignore);
            assert_eq!(r("cursor", "PermissionRequest", &none), Route::Ignore);
            assert_eq!(r("cursor", "PreToolUse", &question), Route::Ignore);
        }
        // Codex permissions reach the island only while Mayak's hook waits 600 s for them.
        assert_eq!(route("codex", "PermissionRequest", &none, false), Route::Observe);
        assert_eq!(route("codex", "PermissionRequest", &none, true), Route::Approval);
    }

    #[test]
    fn the_decision_wait_ends_before_the_relay_gives_up() {
        // Relay budget 595 s, hook timeout 600 s.
        assert_eq!(DECISION_TIMEOUT, Duration::from_secs(590));
    }

    #[test]
    fn always_is_only_kept_where_there_is_a_rule_to_keep() {
        assert_eq!(permission_word("allow", true), "allow");
        assert_eq!(permission_word("allow", false), "allow");
        assert_eq!(permission_word("always", true), "always");
        assert_eq!(permission_word("always", false), "allow");
        assert_eq!(permission_word("deny", true), "deny");
        assert_eq!(permission_word("deny", false), "deny");
        // Anything else stays the safe side, as before.
        assert_eq!(permission_word("", true), "deny");
        assert_eq!(permission_word("Allow", true), "deny");
    }

    #[test]
    fn suggestions_mean_a_non_empty_list() {
        assert!(has_suggestions(&json!({ "permission_suggestions": [ { "type": "addRules" } ] })));
        assert!(!has_suggestions(&json!({ "permission_suggestions": [] })));
        assert!(!has_suggestions(&json!({ "permission_suggestions": { "type": "addRules" } })));
        assert!(!has_suggestions(&json!({ "permission_suggestions": null })));
        assert!(!has_suggestions(&json!({})));
    }

    #[test]
    fn answers_become_one_compact_line() {
        assert_eq!(answers_line(&json!({ "Which framework?": "React" })).unwrap(), r#"answer {"Which framework?":"React"}"#);
        assert_eq!(
            answers_line(&json!({ "Which framework?": "Vue", "Which tests?": ["Unit", "E2E"] })).unwrap(),
            r#"answer {"Which framework?":"Vue","Which tests?":["Unit","E2E"]}"#,
            "several questions, a multi-select as a list, keys in order"
        );
        // A line break inside a text stays escaped: the reply is still one line.
        let line = answers_line(&json!({ "Two\nlines?": "Да\r\n" })).unwrap();
        assert_eq!(line, r#"answer {"Two\nlines?":"Да\r\n"}"#);
        assert!(!line.contains('\n') && !line.contains('\r'));
    }

    #[test]
    fn answers_that_are_not_labels_are_rejected() {
        for bad in [
            json!(null),
            json!("React"),
            json!(["React"]),
            json!({}),
            json!({ "Q?": 1 }),
            json!({ "Q?": true }),
            json!({ "Q?": null }),
            json!({ "Q?": [] }),
            json!({ "Q?": ["a", 1] }),
            json!({ "Q?": [["a"]] }),
            json!({ "Q?": { "label": "a" } }),
            json!({ "Good?": "a", "Bad?": 2 }),
        ] {
            assert!(answers_line(&bad).is_err(), "{bad}");
        }
        // The error never quotes what it was given.
        let err = answers_line(&json!({ "secret-question?": 1 })).unwrap_err();
        assert!(!err.to_string().contains("secret"), "{err}");
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
        let claude = Card { kind: Kind::Permission, agent: "claude", can_always: true };
        let mut p = json!({ "hook_event_name": "PermissionRequest" });
        annotate_approval(&mut p, "9-1", &ApprovalContext::default(), &claude);
        assert_eq!(p["request_id"], "9-1");
        assert_eq!(p["session_key"], Value::Null);
        assert_eq!(p["project_name"], Value::Null);
        assert_eq!(p["kind"], "permission");
        assert_eq!(p["agent"], "claude");
        assert_eq!(p["canAlways"], true);
        let ctx = ApprovalContext { session_key: Some("claude:1:2".into()), project_name: Some("mayak".into()) };
        let question = Card { kind: Kind::Question, agent: "claude", can_always: false };
        annotate_approval(&mut p, "9-2", &ctx, &question);
        assert_eq!(p["session_key"], "claude:1:2");
        assert_eq!(p["project_name"], "mayak");
        assert_eq!(p["kind"], "question");
        assert_eq!(p["canAlways"], false);
        let codex = Card { kind: Kind::Permission, agent: "codex", can_always: false };
        annotate_approval(&mut p, "9-3", &ctx, &codex);
        assert_eq!(p["agent"], "codex");
        assert_eq!(p["kind"], "permission");
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
            assert_eq!(card["kind"], "permission");
            assert_eq!(card["agent"], "claude");
            assert_eq!(card["canAlways"], false, "no permission_suggestions");
            let id = card["request_id"].as_str().unwrap().to_string();
            acknowledge(&s.pending, &id);
            answer(&s.pending, &id, "always");
            assert_eq!(read_to_close(&mut c).await, "allow\n", "nothing to remember: a plain allow");
            assert!(observed.try_recv().is_err());
            assert!(pending_is_empty(&s));
            server.abort();
        });
    }

    #[test]
    fn every_decision_writes_its_reply_line() {
        runtime().block_on(async {
            let name = unique_pipe("lines");
            let (s, mut observed, mut shown) = sink_with(ApprovalContext::default(), true);
            let server = tokio::spawn(serve(name.clone(), s.clone()));
            let decide = |d: &'static str| move |p: &Pending, id: &str| answer(p, id, d);
            let release = |p: &Pending, id: &str| decline(p, id);
            type Act = Box<dyn Fn(&Pending, &str)>;
            let cases: Vec<(&str, &str, bool, Act, &str)> = vec![
                ("claude allow", CLAUDE_PERMISSION, true, Box::new(decide("allow")), "allow\n"),
                ("claude deny", CLAUDE_PERMISSION, true, Box::new(decide("deny")), "deny\n"),
                ("claude always, no rules", CLAUDE_PERMISSION, true, Box::new(decide("always")), "allow\n"),
                ("claude always", CLAUDE_PERMISSION_WITH_RULES, true, Box::new(decide("always")), "always\n"),
                ("claude allow, rules", CLAUDE_PERMISSION_WITH_RULES, true, Box::new(decide("allow")), "allow\n"),
                ("codex allow", CODEX_PERMISSION, true, Box::new(decide("allow")), "allow\n"),
                ("codex deny", CODEX_PERMISSION, true, Box::new(decide("deny")), "deny\n"),
                ("codex always", CODEX_PERMISSION, true, Box::new(decide("always")), "allow\n"),
                ("released", CLAUDE_PERMISSION, true, Box::new(release), "release\n"),
                ("released before the ack", CLAUDE_PERMISSION, false, Box::new(release), "release\n"),
                ("question released", CLAUDE_QUESTION, true, Box::new(release), "release\n"),
                ("decision before the ack", CODEX_PERMISSION, false, Box::new(decide("deny")), "deny\n"),
            ];
            for (what, line, ack, act, expected) in cases {
                let (mut c, _card, id) = card_for(&name, &mut shown, line).await;
                if ack {
                    acknowledge(&s.pending, &id);
                }
                act(&s.pending, &id);
                assert_eq!(read_to_close(&mut c).await, expected, "{what}");
                assert!(pending_is_empty(&s), "{what}");
            }
            // Only the four Codex requests fed the observer.
            for _ in 0..4 {
                assert_eq!(recv(&mut observed).await["agent"], "codex");
            }
            assert!(observed.try_recv().is_err());
            server.abort();
        });
    }

    #[test]
    fn a_question_card_gets_its_answer_line() {
        runtime().block_on(async {
            let name = unique_pipe("question");
            let (s, mut observed, mut shown) = sink(ApprovalContext::default());
            let server = tokio::spawn(serve(name.clone(), s.clone()));
            let (mut c, card, id) = card_for(&name, &mut shown, CLAUDE_QUESTION).await;
            assert_eq!(card["kind"], "question");
            assert_eq!(card["agent"], "claude");
            assert_eq!(card["canAlways"], false);
            assert_eq!(card["tool_input"], json!({ "questions": [] }), "the payload goes to the island as it came");
            acknowledge(&s.pending, &id);
            let answers = json!({ "Which framework?": "React", "secret-tests?": ["Unit", "E2E"] });
            assert_eq!(answer_question(&s.pending, &id, &answers), Ok(()));
            assert_eq!(
                read_to_close(&mut c).await,
                "answer {\"Which framework?\":\"React\",\"secret-tests?\":[\"Unit\",\"E2E\"]}\n"
            );
            assert!(pending_is_empty(&s));
            assert!(observed.try_recv().is_err());
            let notes = NOTES.lock().unwrap().clone();
            assert!(notes.iter().any(|n| n == &format!("decision id={id} answer")), "{notes:?}");
            assert!(!notes.iter().any(|n| n.contains("secret-tests")), "no answer text in the log");
            server.abort();
        });
    }

    #[test]
    fn a_reply_of_the_wrong_kind_or_shape_is_ignored() {
        runtime().block_on(async {
            let name = unique_pipe("kinds");
            let (s, _observed, mut shown) = sink(ApprovalContext::default());
            let server = tokio::spawn(serve(name.clone(), s.clone()));

            // A permission decision for a question changes nothing; nor do answers that are not labels.
            let (mut c, _card, id) = card_for(&name, &mut shown, CLAUDE_QUESTION).await;
            acknowledge(&s.pending, &id);
            answer(&s.pending, &id, "allow");
            assert!(answer_question(&s.pending, &id, &json!({ "Q?": 3 })).is_err());
            assert!(!pending_is_empty(&s), "the card still waits");
            assert_eq!(answer_question(&s.pending, &id, &json!({ "Q?": "A" })), Ok(()));
            assert_eq!(read_to_close(&mut c).await, "answer {\"Q?\":\"A\"}\n");

            // Answers for a permission change nothing either.
            let (mut c, _card, id) = card_for(&name, &mut shown, CLAUDE_PERMISSION).await;
            acknowledge(&s.pending, &id);
            assert_eq!(answer_question(&s.pending, &id, &json!({ "Q?": "A" })), Ok(()));
            assert!(!pending_is_empty(&s));
            answer(&s.pending, &id, "deny");
            assert_eq!(read_to_close(&mut c).await, "deny\n");
            assert!(pending_is_empty(&s));
            server.abort();
        });
    }

    #[test]
    fn a_click_after_the_request_ended_is_ignored() {
        runtime().block_on(async {
            let name = unique_pipe("expired");
            let (s, _observed, mut shown) = sink(ApprovalContext::default());
            let server = tokio::spawn(serve(name.clone(), s.clone()));
            // Never acknowledged: the request expires after ACK_TIMEOUT and the relay gets nothing.
            let (mut c, _card, id) = card_for(&name, &mut shown, CLAUDE_QUESTION).await;
            assert_eq!(read_to_close(&mut c).await, "");
            assert!(pending_is_empty(&s));
            // Clicks that come too late: nothing to answer, nothing written, no panic.
            acknowledge(&s.pending, &id);
            answer(&s.pending, &id, "allow");
            assert_eq!(answer_question(&s.pending, &id, &json!({ "Q?": "A" })), Ok(()));
            decline(&s.pending, &id);
            // A released one too.
            let (mut c, _card, id) = card_for(&name, &mut shown, CLAUDE_PERMISSION).await;
            decline(&s.pending, &id);
            assert_eq!(read_to_close(&mut c).await, "release\n");
            answer(&s.pending, &id, "allow");
            decline(&s.pending, &id);
            // And one that never existed.
            answer(&s.pending, "no-such-id", "deny");
            assert_eq!(answer_question(&s.pending, "no-such-id", &json!({ "Q?": "A" })), Ok(()));
            assert!(pending_is_empty(&s));
            assert!(notes_with(&format!("reply for id={id} ")).len() >= 2);
            server.abort();
        });
    }

    #[test]
    fn codex_permissions_reach_the_island_and_the_observer_while_answers_are_on() {
        runtime().block_on(async {
            let name = unique_pipe("codex-on");
            let ctx = ApprovalContext { session_key: Some("codex:codex-s".into()), project_name: None };
            let (s, mut observed, mut shown) = sink_with(ctx, true);
            let server = tokio::spawn(serve(name.clone(), s.clone()));
            let (mut c, card, id) = card_for(&name, &mut shown, CODEX_PERMISSION).await;
            assert_eq!(card["kind"], "permission");
            assert_eq!(card["agent"], "codex");
            assert_eq!(card["canAlways"], false, "Codex fails closed on updatedPermissions");
            assert_eq!(card["session_key"], "codex:codex-s");
            // The observer gets the line exactly as the relay sent it, as in Observe.
            let fed = recv(&mut observed).await;
            assert_eq!(fed, serde_json::from_str::<Value>(CODEX_PERMISSION.trim()).unwrap());
            acknowledge(&s.pending, &id);
            answer(&s.pending, &id, "allow");
            assert_eq!(read_to_close(&mut c).await, "allow\n");

            // Every other Codex event is still observed only.
            let mut c = client(&name).await;
            c.write_all(b"{\"agent\":\"codex\",\"hook_event_name\":\"Stop\",\"session_id\":\"codex-s\"}\n").await.unwrap();
            assert_eq!(read_to_close(&mut c).await, "");
            assert_eq!(recv(&mut observed).await["hook_event_name"], "Stop");
            assert!(shown.try_recv().is_err());
            assert!(pending_is_empty(&s));
            server.abort();
        });
    }

    #[test]
    fn other_claude_tools_before_use_are_ignored() {
        runtime().block_on(async {
            let name = unique_pipe("pretool");
            let (s, mut observed, mut shown) = sink(ApprovalContext::default());
            let server = tokio::spawn(serve(name.clone(), s.clone()));
            let mut c = client(&name).await;
            c.write_all(b"{\"agent\":\"claude\",\"hook_event_name\":\"PreToolUse\",\"tool_name\":\"Bash\"}\n")
                .await
                .unwrap();
            assert_eq!(read_to_close(&mut c).await, "");
            assert!(observed.try_recv().is_err());
            assert!(shown.try_recv().is_err());
            assert!(pending_is_empty(&s));
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

    /// The DACL of a pipe handle as SDDL, read back from Windows.
    fn dacl_sddl(server: &NamedPipeServer) -> String {
        use std::os::windows::io::AsRawHandle;
        use windows::core::PWSTR;
        use windows::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HANDLE, HLOCAL};
        use windows::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
            SE_KERNEL_OBJECT,
        };
        use windows::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
        unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            let status = GetSecurityInfo(
                HANDLE(server.as_raw_handle()),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                None,
                None,
                Some(&mut descriptor),
            );
            assert_eq!(status, ERROR_SUCCESS);
            let mut text = PWSTR::null();
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                None,
            )
            .unwrap();
            let sddl = text.to_string().unwrap();
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
            sddl
        }
    }

    /// A DACL trustee as the normalized `S-1-5-...` SID. Windows prints some
    /// SIDs as aliases (`LA` for the built-in Administrator, `BA`, ...), so the
    /// trustees are compared by SID, not by their SDDL text.
    fn trustee_sid(trustee: &str) -> String {
        use ::windows::core::PWSTR;
        use ::windows::Win32::Foundation::{LocalFree, HLOCAL};
        use ::windows::Win32::Security::Authorization::{
            ConvertSidToStringSidW, ConvertStringSidToSidW,
        };
        use ::windows::Win32::Security::PSID;
        unsafe {
            let mut sid = PSID::default();
            ConvertStringSidToSidW(&HSTRING::from(trustee), &mut sid).unwrap();
            let mut text = PWSTR::null();
            let converted = ConvertSidToStringSidW(sid, &mut text);
            let normalized = converted.map(|()| text.to_string().unwrap());
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            let _ = LocalFree(Some(HLOCAL(sid.0)));
            normalized.unwrap()
        }
    }

    #[test]
    fn the_sddl_names_only_the_given_sid() {
        assert_eq!(current_user_sddl("S-1-5-21-1-2-3-1001"), "D:P(A;;GA;;;S-1-5-21-1-2-3-1001)");
    }

    #[test]
    fn a_trustee_is_compared_by_sid_whatever_its_spelling() {
        assert_eq!(trustee_sid("BA"), "S-1-5-32-544");
        assert_eq!(trustee_sid("S-1-5-32-544"), "S-1-5-32-544");
    }

    #[test]
    fn every_instance_is_for_the_current_user_only() {
        runtime().block_on(async {
            let name = unique_pipe("dacl");
            let sid = crate::platform::current_user_sid().unwrap();
            let first = create_instance(&name, true).unwrap();
            let second = create_instance(&name, false).unwrap();
            for server in [&first, &second] {
                let sddl = dacl_sddl(server);
                assert!(sddl.starts_with("D:P("), "{sddl}");
                assert_eq!(sddl.matches("(A;").count(), 1, "{sddl}");
                assert!(sddl.ends_with(')'), "{sddl}");
                let trustee = sddl[..sddl.len() - 1].rsplit(";;;").next().unwrap();
                assert_eq!(trustee_sid(trustee), sid, "{sddl}");
            }
            // Our own account still gets in.
            let _client = ClientOptions::new().open(&name).unwrap();
            first.connect().await.unwrap();
        });
    }
}
