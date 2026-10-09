// Fresh Codex limits from the Codex CLI's app server (JSON-RPC over stdio). The rollout files give
// the limits only while Codex is used; when the home page opens and the limits held are older
// than FRESH_MS, `codex app-server` is started on its own thread, asked `initialize` and then
// `account/rateLimits/read`, and shut down. No other method is ever sent: nothing that logs in or
// out, starts a thread or a turn, or spends a credit. The process uses its own login; Mayak reads
// no credential and writes no Codex file. Only the window numbers of the answer are kept.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::log;
use crate::observe::model::{CodexUsage, CodexWindow, Signal};

/// Limits younger than this (from a rollout or a read) are fresh enough: no read.
pub const FRESH_MS: i64 = 5 * 60_000;
/// After a failed read (no CLI, an error, a timeout) the next try waits this long.
pub const BACKOFF_MS: i64 = 15 * 60_000;
/// The whole exchange, start to answer.
const TIMEOUT: Duration = Duration::from_secs(10);
/// After stdin is closed the server exits on its own; past this it is killed.
const EXIT_GRACE: Duration = Duration::from_secs(2);
/// A longer output line is not an answer of ours: the read stops.
const MAX_LINE: u64 = 1 << 20;

const INITIALIZE_ID: i64 = 1;
const READ_ID: i64 = 2;
/// The bucket of the account's own limits, as in the rollout files (`observe::codex`).
const CODEX_LIMIT_ID: &str = "codex";

// ── Protocol (pure) ──────────────────────────────────────────────────────────

/// The handshake request: `initialize` with the client info.
pub fn initialize_request() -> Value {
    json!({
        "id": INITIALIZE_ID,
        "method": "initialize",
        "params": { "clientInfo": { "name": "mayak", "title": "Mayak", "version": env!("CARGO_PKG_VERSION") } }
    })
}

/// The notification that ends the handshake.
pub fn initialized_notification() -> Value {
    json!({ "method": "initialized" })
}

/// The one read: the account's limits, without the reset-credit detail lookup (a background poll).
pub fn read_request() -> Value {
    json!({ "id": READ_ID, "method": "account/rateLimits/read", "params": { "excludeResetCreditDetails": true } })
}

/// The answer to request `id` in a server message: Some(Ok(result)), Some(Err(why)) for a JSON-RPC
/// error, None for anything else (a notification, a server request, another id).
pub fn reply_to(msg: &Value, id: i64) -> Option<Result<&Value, String>> {
    if msg.get("method").is_some() || msg.get("id").and_then(Value::as_i64) != Some(id) {
        return None;
    }
    if let Some(result) = msg.get("result") {
        return Some(Ok(result));
    }
    let code = msg.get("error").and_then(|e| e.get("code")).and_then(Value::as_i64);
    Some(Err(match code {
        Some(code) => format!("error {code}"),
        None => "answer without a result".to_string(),
    }))
}

/// The limit windows of a `GetAccountRateLimitsResponse`: the `codex` bucket of
/// `rateLimitsByLimitId`, else `rateLimits` when its `limitId` is "codex" or null; `primary`, then
/// `secondary`, each kept by `window`. None when no window is kept.
pub fn limit_windows(result: &Value) -> Option<Vec<CodexWindow>> {
    let by_id = result.get("rateLimitsByLimitId").and_then(|m| m.get(CODEX_LIMIT_ID)).filter(|s| s.is_object());
    let snapshot = by_id.or_else(|| {
        result.get("rateLimits").filter(|s| match s.get("limitId") {
            None | Some(Value::Null) => true,
            Some(id) => id.as_str() == Some(CODEX_LIMIT_ID),
        })
    })?;
    let windows: Vec<CodexWindow> =
        ["primary", "secondary"].into_iter().filter_map(|k| snapshot.get(k)).filter_map(window).collect();
    (!windows.is_empty()).then_some(windows)
}

/// One window: a number `usedPercent` and an integer `windowDurationMins` > 0, else None;
/// `resetsAt` (seconds since the epoch) becomes milliseconds when it is an integer.
fn window(v: &Value) -> Option<CodexWindow> {
    let percent = v.get("usedPercent")?.as_f64()?;
    let minutes = v.get("windowDurationMins")?.as_u64().filter(|m| *m > 0)?;
    let resets_at_ms = v.get("resetsAt").and_then(Value::as_i64).and_then(|s| s.checked_mul(1_000));
    Some(CodexWindow { minutes, percent, resets_at_ms })
}

// ── When to read (pure) ──────────────────────────────────────────────────────

/// One read at a time; none while the held limits are fresh, none within FRESH_MS of a good read
/// or BACKOFF_MS of a failed one.
#[derive(Debug, Default)]
pub struct Gate {
    running: bool,
    /// When the last read ended, and whether it gave limits.
    last: Option<(i64, bool)>,
}

/// `at` lies within `span` before `now`. A time after `now` (the clock moved back) is not recent.
fn recent(now: i64, at: i64, span: i64) -> bool {
    (0..span).contains(&(now - at))
}

impl Gate {
    pub const fn new() -> Self {
        Gate { running: false, last: None }
    }

    /// True when a read starts now (`held_seen_at`: the time of the limits the store holds); it is
    /// then running until `finish`.
    pub fn try_start(&mut self, now: i64, held_seen_at: Option<i64>) -> bool {
        if self.running || held_seen_at.is_some_and(|seen| recent(now, seen, FRESH_MS)) {
            return false;
        }
        if let Some((at, ok)) = self.last {
            if recent(now, at, if ok { FRESH_MS } else { BACKOFF_MS }) {
                return false;
            }
        }
        self.running = true;
        true
    }

    pub fn finish(&mut self, now: i64, ok: bool) {
        self.running = false;
        self.last = Some((now, ok));
    }
}

// ── The process (thin) ───────────────────────────────────────────────────────

/// The Codex CLI: the one inside the Codex Desktop package, else `codex` on PATH.
pub fn find_codex(localappdata: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(exe) = appx_codex(localappdata) {
        return Some(exe);
    }
    #[cfg(not(windows))]
    let _ = localappdata;
    crate::platform::find_on_path("codex")
}

/// `<install folder>\app\resources\codex.exe` of an installed Codex Desktop package.
#[cfg(windows)]
fn appx_codex(localappdata: &Path) -> Option<PathBuf> {
    let family = crate::launcher::codex_family(&localappdata.join("Packages"))?;
    package_folders(&family)
        .into_iter()
        .map(|dir| dir.join("app").join("resources").join("codex.exe"))
        .find(|exe| exe.is_file())
}

/// The install folders of the packages of `family` that are installed for this user.
#[cfg(windows)]
fn package_folders(family: &str) -> Vec<PathBuf> {
    use ::windows::core::{HSTRING, PWSTR};
    use ::windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
    use ::windows::Win32::Storage::Packaging::Appx::{GetPackagePathByFullName, GetPackagesByPackageFamily};

    let family = HSTRING::from(family);
    let (mut count, mut len) = (0u32, 0u32);
    // SAFETY: the sizing call writes only the two counters.
    let sized = unsafe { GetPackagesByPackageFamily(&family, &mut count, None, &mut len, None) };
    if sized != ERROR_INSUFFICIENT_BUFFER || count == 0 || len == 0 {
        return Vec::new();
    }
    let mut names = vec![PWSTR::null(); count as usize];
    let mut buffer = vec![0u16; len as usize];
    // SAFETY: `names` holds `count` pointers and `buffer` `len` u16s, the sizes just returned.
    let got = unsafe {
        GetPackagesByPackageFamily(&family, &mut count, Some(names.as_mut_ptr()), &mut len, Some(PWSTR(buffer.as_mut_ptr())))
    };
    if got != ERROR_SUCCESS {
        return Vec::new();
    }
    let full_names: Vec<HSTRING> = names
        .iter()
        .take(count as usize)
        .filter(|name| !name.is_null())
        // SAFETY: each pointer is a NUL-terminated string inside `buffer`, which is still alive.
        .map(|name| unsafe { name.to_hstring() })
        .collect();
    full_names
        .iter()
        .filter_map(|full| {
            let mut len = 0u32;
            // SAFETY: the sizing call writes only `len`.
            if unsafe { GetPackagePathByFullName(full, &mut len, None) } != ERROR_INSUFFICIENT_BUFFER || len == 0 {
                return None;
            }
            let mut path = vec![0u16; len as usize];
            // SAFETY: `path` holds the `len` u16s just returned.
            if unsafe { GetPackagePathByFullName(full, &mut len, Some(PWSTR(path.as_mut_ptr()))) } != ERROR_SUCCESS {
                return None;
            }
            let end = path.iter().position(|c| *c == 0).unwrap_or(path.len());
            String::from_utf16(&path[..end]).ok().map(PathBuf::from)
        })
        .collect()
}

/// The running server: stdin closed first, then a short wait for it to exit, then a kill.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl Server {
    fn send(&mut self, msg: &Value) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("input closed")?;
        let mut line = msg.to_string();
        line.push('\n');
        stdin.write_all(line.as_bytes()).and_then(|()| stdin.flush()).map_err(|e| format!("write failed ({:?})", e.kind()))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let until = Instant::now() + EXIT_GRACE;
        while Instant::now() < until {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }
        if let Err(err) = self.child.kill() {
            log::line(format!("codex limits: could not stop the app server ({:?})", err.kind()));
        }
        let _ = self.child.wait();
    }
}

/// Starts `exe app-server`, does the handshake and the one read, and stops it. Blocking, at most
/// TIMEOUT plus EXIT_GRACE: call it off the UI and hook paths. Errors name no path and no body.
pub fn read_limits(exe: &Path) -> Result<Vec<CodexWindow>, String> {
    let mut cmd = Command::new(exe);
    cmd.arg("app-server").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    crate::platform::no_console(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("did not start ({:?})", e.kind()))?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let mut server = Server { child, stdin };
    let stdout = stdout.ok_or("no output pipe")?;

    let (tx, rx) = mpsc::channel::<Value>();
    std::thread::Builder::new()
        .name("codex-limits-out".into())
        .spawn(move || {
            let mut out = BufReader::new(stdout);
            let mut line = Vec::new();
            loop {
                line.clear();
                match Read::take(&mut out, MAX_LINE).read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => return,
                    Ok(_) if line.last() != Some(&b'\n') && line.len() as u64 >= MAX_LINE => return,
                    Ok(_) => {}
                }
                // A line that is not JSON (a log line) is skipped.
                if let Ok(msg) = serde_json::from_slice::<Value>(&line) {
                    if tx.send(msg).is_err() {
                        return;
                    }
                }
            }
        })
        .map_err(|e| format!("reader did not start ({:?})", e.kind()))?;

    let deadline = Instant::now() + TIMEOUT;
    server.send(&initialize_request())?;
    loop {
        let left = deadline.checked_duration_since(Instant::now()).ok_or("timed out")?;
        let msg = rx.recv_timeout(left).map_err(|e| match e {
            RecvTimeoutError::Timeout => "timed out",
            RecvTimeoutError::Disconnected => "output closed",
        })?;
        if let Some(reply) = reply_to(&msg, INITIALIZE_ID) {
            reply.map_err(|why| format!("initialize: {why}"))?;
            server.send(&initialized_notification())?;
            server.send(&read_request())?;
        } else if let Some(reply) = reply_to(&msg, READ_ID) {
            let result = reply.map_err(|why| format!("read: {why}"))?;
            return limit_windows(result).ok_or_else(|| "no codex limit window in the answer".to_string());
        }
    }
}

static GATE: Mutex<Gate> = Mutex::new(Gate::new());

/// The home page opened: when the Codex limits held are not fresh and no read ran lately, reads
/// them on a thread of its own and hands them to the store, which keeps whichever is newer.
/// Returns at once.
pub fn refresh(observe: &crate::observe::Handle, localappdata: PathBuf) {
    let now = crate::observe::now_ms();
    let held = observe.snapshot().usage.codex.map(|u| u.seen_at_ms);
    if !GATE.lock().unwrap_or_else(PoisonError::into_inner).try_start(now, held) {
        return;
    }
    let observe = observe.clone();
    let spawned = std::thread::Builder::new().name("codex-limits".into()).spawn(move || {
        let outcome = find_codex(&localappdata).ok_or_else(|| "no Codex CLI found".to_string()).and_then(|exe| read_limits(&exe));
        let ok = outcome.is_ok();
        match outcome {
            Ok(windows) => {
                log::line(format!("codex limits: read {} window(s) from the app server", windows.len()));
                observe.send(Signal::CodexUsageSeed(CodexUsage { seen_at_ms: crate::observe::now_ms(), windows }));
            }
            Err(why) => log::line(format!("codex limits: {why}")),
        }
        GATE.lock().unwrap_or_else(PoisonError::into_inner).finish(crate::observe::now_ms(), ok);
    });
    if let Err(err) = spawned {
        log::line(format!("codex limits: thread failed to start: {err}"));
        GATE.lock().unwrap_or_else(PoisonError::into_inner).finish(now, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The answer of `codex-cli 0.162`, account details replaced, as `GetAccountRateLimitsResponse`.
    fn response() -> Value {
        let snapshot = json!({
            "limitId": "codex", "limitName": null, "normalModelSlug": null,
            "primary": { "usedPercent": 85, "windowDurationMins": 300, "resetsAt": 1_791_594_305 },
            "secondary": { "usedPercent": 44, "windowDurationMins": 10_080, "resetsAt": 1_792_138_998 },
            "credits": { "hasCredits": false, "unlimited": false, "balance": "0" },
            "individualLimit": null, "spendControlReached": false, "planType": "plus", "rateLimitReachedType": null
        });
        json!({
            "ordinaryUsageAllowed": true,
            "rateLimits": snapshot,
            "rateLimitsByLimitId": { "codex": snapshot },
            "rateLimitResetCredits": { "availableCount": 1, "credits": null },
            "accountId": "acct", "rateLimitUpsell": null
        })
    }

    fn w(minutes: u64, percent: f64, resets_at_ms: Option<i64>) -> CodexWindow {
        CodexWindow { minutes, percent, resets_at_ms }
    }

    #[test]
    fn the_requests_are_the_handshake_and_the_one_read() {
        let init = initialize_request();
        assert_eq!((init["id"].as_i64(), init["method"].as_str()), (Some(INITIALIZE_ID), Some("initialize")));
        assert_eq!(init["params"]["clientInfo"]["name"], "mayak");
        assert!(init["params"]["clientInfo"]["version"].is_string());
        assert_eq!(initialized_notification(), json!({ "method": "initialized" }));
        let read = read_request();
        assert_eq!((read["id"].as_i64(), read["method"].as_str()), (Some(READ_ID), Some("account/rateLimits/read")));
        assert_eq!(read["params"], json!({ "excludeResetCreditDetails": true }));
    }

    #[test]
    fn a_reply_is_matched_by_id_and_never_taken_from_a_server_message() {
        let ok = json!({ "id": 2, "result": { "x": 1 } });
        assert_eq!(reply_to(&ok, 2), Some(Ok(&json!({ "x": 1 }))));
        assert_eq!(reply_to(&ok, 1), None, "another id");
        let err = json!({ "id": 2, "error": { "code": -32600, "message": "not logged in" } });
        assert_eq!(reply_to(&err, 2), Some(Err("error -32600".to_string())), "the message text is not kept");
        assert_eq!(reply_to(&json!({ "id": 2 }), 2), Some(Err("answer without a result".to_string())));
        let server_request = json!({ "id": 2, "method": "item/tool/call", "params": {} });
        assert_eq!(reply_to(&server_request, 2), None);
        let note = json!({ "method": "remoteControl/status/changed", "params": {} });
        assert_eq!(reply_to(&note, 2), None);
        assert_eq!(reply_to(&json!({ "id": "2", "result": {} }), 2), None, "a string id is not ours");
    }

    #[test]
    fn the_codex_bucket_gives_both_windows_in_ms() {
        let expected = vec![w(300, 85.0, Some(1_791_594_305_000)), w(10_080, 44.0, Some(1_792_138_998_000))];
        assert_eq!(limit_windows(&response()), Some(expected));
    }

    #[test]
    fn the_by_id_bucket_wins_over_the_single_bucket_view() {
        let mut r = response();
        r["rateLimits"] = json!({ "limitId": "codex_other", "primary": { "usedPercent": 1, "windowDurationMins": 60 } });
        r["rateLimitsByLimitId"]["codex"]["secondary"] = Value::Null;
        assert_eq!(limit_windows(&r), Some(vec![w(300, 85.0, Some(1_791_594_305_000))]));
    }

    #[test]
    fn without_a_by_id_codex_bucket_the_single_view_counts_only_for_codex_or_no_id() {
        let primary = json!({ "usedPercent": 12, "windowDurationMins": 300, "resetsAt": null });
        for by_id in [Value::Null, json!({}), json!({ "premium": { "primary": primary } })] {
            for (limit_id, kept) in [(json!("codex"), true), (Value::Null, true), (json!("codex_bengalfox"), false)] {
                let r = json!({ "rateLimits": { "limitId": limit_id, "primary": primary }, "rateLimitsByLimitId": by_id });
                let want = kept.then(|| vec![w(300, 12.0, None)]);
                assert_eq!(limit_windows(&r), want, "by_id {by_id}, limitId {limit_id}");
            }
        }
        let no_id = json!({ "rateLimits": { "primary": primary } });
        assert_eq!(limit_windows(&no_id), Some(vec![w(300, 12.0, None)]));
    }

    #[test]
    fn malformed_windows_are_skipped() {
        let good = json!({ "usedPercent": 40, "windowDurationMins": 10_080 });
        for bad in [
            Value::Null,
            json!({ "windowDurationMins": 300 }),
            json!({ "usedPercent": "5", "windowDurationMins": 300 }),
            json!({ "usedPercent": 5 }),
            json!({ "usedPercent": 5, "windowDurationMins": null }),
            json!({ "usedPercent": 5, "windowDurationMins": 0 }),
            json!({ "usedPercent": 5, "windowDurationMins": -300 }),
        ] {
            let r = json!({ "rateLimits": { "limitId": "codex", "primary": bad, "secondary": good } });
            assert_eq!(limit_windows(&r), Some(vec![w(10_080, 40.0, None)]), "primary {bad}");
        }
        let odd_reset = json!({ "usedPercent": 40, "windowDurationMins": 10_080, "resetsAt": "soon" });
        assert_eq!(limit_windows(&json!({ "rateLimits": { "primary": odd_reset } })), Some(vec![w(10_080, 40.0, None)]));
        assert_eq!(limit_windows(&json!({ "rateLimits": { "primary": null, "secondary": null } })), None);
        assert_eq!(limit_windows(&json!({ "rateLimits": null })), None);
        assert_eq!(limit_windows(&json!({})), None);
        assert_eq!(limit_windows(&Value::Null), None);
    }

    const NOW: i64 = 10_000_000;

    #[test]
    fn the_gate_reads_only_when_the_held_limits_are_stale() {
        let mut g = Gate::new();
        assert!(!g.try_start(NOW, Some(NOW - FRESH_MS + 1)), "fresh from a rollout");
        assert!(g.try_start(NOW, Some(NOW - FRESH_MS)), "five minutes old");
        let mut g = Gate::new();
        assert!(g.try_start(NOW, None), "nothing held");
        let mut g = Gate::new();
        assert!(g.try_start(NOW, Some(NOW + 60_000)), "held limits from the future do not block");
    }

    #[test]
    fn the_gate_runs_one_read_at_a_time() {
        let mut g = Gate::new();
        assert!(g.try_start(NOW, None));
        assert!(!g.try_start(NOW + BACKOFF_MS * 2, None), "still running");
        g.finish(NOW + 1_000, true);
        assert!(!g.try_start(NOW + 1_000 + FRESH_MS - 1, None), "a good read was just made");
        assert!(g.try_start(NOW + 1_000 + FRESH_MS, None));
    }

    #[test]
    fn the_gate_backs_off_after_a_failure() {
        let mut g = Gate::new();
        assert!(g.try_start(NOW, None));
        g.finish(NOW, false);
        assert!(!g.try_start(NOW + FRESH_MS, None));
        assert!(!g.try_start(NOW + BACKOFF_MS - 1, None));
        assert!(g.try_start(NOW + BACKOFF_MS, None));
        g.finish(NOW + BACKOFF_MS, false);
        assert!(g.try_start(NOW, None), "a failure stamped in the future (clock moved back) does not block");
    }

    #[test]
    fn a_missing_cli_fails_without_naming_a_path() {
        let err = read_limits(Path::new("Z:/no/such/dir/codex.exe")).unwrap_err();
        assert!(err.starts_with("did not start"), "{err}");
        assert!(!err.contains("no/such"), "{err}");
    }
}
