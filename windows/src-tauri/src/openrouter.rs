// OpenRouter credits on the home page. Only when the user saved an OpenRouter API key in Settings
// (Credential Manager, `secrets`), and only when the home page asks (`usage_refresh` on opening it,
// `openrouter_refresh` on its minute tick): one `GET https://openrouter.ai/api/v1/key` with that key,
// when the credits held are older than FRESH_MS and no read failed within BACKOFF_MS
// (`codex_limits::Gate`). Never while Mayak is paused. The key and the answer are never logged; the
// log gets the HTTP status only. `/api/v1/credits` (the account's total) needs a management key,
// so it is not called: the key's own limit and usage is what the home page shows.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;

use crate::codex_limits::Gate;
use crate::log;
use crate::observe::model::{OpenRouterProblem, OpenRouterUsage, Signal};

/// The Credential Manager entry (secrets::KNOWN_KEYS).
pub const KEY: &str = "openrouter-api-key";
const URL: &str = "https://openrouter.ai/api/v1/key";
const TIMEOUT: Duration = Duration::from_secs(10);

// ── The answer (pure) ────────────────────────────────────────────────────────

/// A `GET /api/v1/key` answer: `{ "data": { limit, limit_remaining, limit_reset, usage,
/// usage_daily, usage_weekly, usage_monthly, … } }`. `limit` and `limit_remaining` are numbers or
/// null (no limit), `limit_reset` a string or null (never resets); the four usage numbers must be
/// there. None for anything else.
pub fn parse_key_info(body: &Value, fetched_at_ms: i64) -> Option<OpenRouterUsage> {
    let data = body.get("data")?.as_object()?;
    let number = |k: &str| data.get(k).and_then(Value::as_f64);
    let nullable = |k: &str| -> Option<Option<f64>> {
        match data.get(k) {
            None | Some(Value::Null) => Some(None),
            Some(v) => v.as_f64().map(Some),
        }
    };
    let limit_reset = match data.get("limit_reset") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str()?.to_string()),
    };
    Some(OpenRouterUsage {
        fetched_at_ms,
        limit: nullable("limit")?,
        limit_remaining: nullable("limit_remaining")?,
        limit_reset,
        usage: number("usage")?,
        usage_daily: number("usage_daily")?,
        usage_weekly: number("usage_weekly")?,
        usage_monthly: number("usage_monthly")?,
    })
}

// ── When to read (pure) ──────────────────────────────────────────────────────

/// True when a read starts now: a key is saved, Mayak is not paused, and the gate lets it
/// (`held_at`: the time of the credits the store holds). Without a key or while paused the gate is
/// not touched.
pub fn should_read(gate: &mut Gate, now: i64, held_at: Option<i64>, key_present: bool, paused: bool) -> bool {
    key_present && !paused && gate.try_start(now, held_at)
}

/// What one read gave.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Read(OpenRouterUsage),
    /// 401 or 403: the key is wrong or revoked. The credits held are dropped.
    Refused,
    Failed,
}

/// The outcome of an HTTP answer: its status and, for a 2xx, its body as JSON (None when it is not).
pub fn outcome(status: u16, body: Option<&Value>, now: i64) -> Outcome {
    match status {
        200..=299 => body.and_then(|b| parse_key_info(b, now)).map_or(Outcome::Failed, Outcome::Read),
        401 | 403 => Outcome::Refused,
        _ => Outcome::Failed,
    }
}

/// The Settings line after a read that ended in `out`, the line before being `prev`: a read with
/// credits clears it, a refused key says so, any other failure says OpenRouter could not be reached
/// unless the key is already known to be refused (a refused key stays the thing to fix).
pub fn problem_after(prev: Option<OpenRouterProblem>, out: &Outcome) -> Option<OpenRouterProblem> {
    match out {
        Outcome::Read(_) => None,
        Outcome::Refused => Some(OpenRouterProblem::Rejected),
        Outcome::Failed if prev == Some(OpenRouterProblem::Rejected) => prev,
        Outcome::Failed => Some(OpenRouterProblem::Unreachable),
    }
}

// ── The request (thin) ───────────────────────────────────────────────────────

static GATE: Mutex<Gate> = Mutex::new(Gate::new());
/// Bumped when the key is saved or removed: a read that started with the old key is dropped.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn gate() -> std::sync::MutexGuard<'static, Gate> {
    GATE.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn fetch(key: &str) -> Outcome {
    let client = match reqwest::Client::builder().timeout(TIMEOUT).build() {
        Ok(c) => c,
        Err(_) => {
            log::line("openrouter: no HTTP client");
            return Outcome::Failed;
        }
    };
    let response = match client.get(URL).bearer_auth(key).send().await {
        Ok(r) => r,
        Err(e) => {
            let why = if e.is_timeout() { "timed out" } else if e.is_connect() { "could not connect" } else { "failed" };
            log::line(format!("openrouter: request {why}"));
            return Outcome::Failed;
        }
    };
    let status = response.status().as_u16();
    log::line(format!("openrouter: status {status}"));
    let body = if response.status().is_success() { response.json::<Value>().await.ok() } else { None };
    let out = outcome(status, body.as_ref(), crate::observe::now_ms());
    if status / 100 == 2 && out == Outcome::Failed {
        log::line("openrouter: the answer has no usable credits");
    }
    out
}

/// The home page asks: reads the key's credits on a task of its own when `should_read`, and hands
/// them to the store. Returns at once.
pub fn refresh(observe: &crate::observe::Handle) {
    let now = crate::observe::now_ms();
    let held = observe.snapshot().usage.openrouter.map(|u| u.fetched_at_ms);
    let paused = crate::integrations::PAUSED.load(Ordering::Relaxed);
    let Some(key) = crate::secrets::get(KEY) else {
        return;
    };
    if !should_read(&mut gate(), now, held, true, paused) {
        return;
    }
    let generation = GENERATION.load(Ordering::SeqCst);
    let observe = observe.clone();
    tauri::async_runtime::spawn(async move {
        let out = fetch(&key).await;
        drop(key);
        let ok = matches!(out, Outcome::Read(_));
        let mut g = gate();
        g.finish(crate::observe::now_ms(), ok);
        if GENERATION.load(Ordering::SeqCst) != generation {
            // The key changed while this read ran: its answer belongs to the old one.
            g.forget();
            return;
        }
        drop(g);
        let prev = observe.snapshot().usage.openrouter_problem;
        let problem = problem_after(prev, &out);
        if problem != prev {
            observe.send(Signal::OpenRouterProblem(problem));
        }
        match out {
            Outcome::Read(usage) => observe.send(Signal::OpenRouterUsage(Some(usage))),
            Outcome::Refused => observe.send(Signal::OpenRouterUsage(None)),
            Outcome::Failed => {}
        }
    });
}

/// The key was saved or removed in Settings: the credits of the old key go, and the next home page
/// visit reads at once.
pub fn key_changed(observe: &crate::observe::Handle) {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    gate().forget();
    observe.send(Signal::OpenRouterUsage(None));
    observe.send(Signal::OpenRouterProblem(None));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_limits::{BACKOFF_MS, FRESH_MS};
    use serde_json::json;

    /// The shape of `GET /api/v1/key` in OpenRouter's API reference (limits page), values made up.
    fn unlimited() -> Value {
        json!({ "data": {
            "label": "sk-or-v1-abc...xyz",
            "limit": null,
            "limit_reset": null,
            "limit_remaining": null,
            "include_byok_in_limit": false,
            "usage": 12.5,
            "usage_daily": 1.2,
            "usage_weekly": 4.8,
            "usage_monthly": 12.5,
            "byok_usage": 0,
            "byok_usage_daily": 0,
            "byok_usage_weekly": 0,
            "byok_usage_monthly": 0,
            "is_free_tier": false,
            "free_model_daily_requests": { "used": 3, "limit": 1000, "remaining": 997 }
        } })
    }

    fn limited() -> Value {
        let mut v = unlimited();
        v["data"]["limit"] = json!(20);
        v["data"]["limit_remaining"] = json!(7.5);
        v["data"]["limit_reset"] = json!("monthly");
        v
    }

    #[test]
    fn a_key_without_a_limit_reads_as_unlimited() {
        let u = parse_key_info(&unlimited(), 42).unwrap();
        assert_eq!(
            u,
            OpenRouterUsage {
                fetched_at_ms: 42,
                limit: None,
                limit_remaining: None,
                limit_reset: None,
                usage: 12.5,
                usage_daily: 1.2,
                usage_weekly: 4.8,
                usage_monthly: 12.5,
            }
        );
    }

    #[test]
    fn a_key_with_a_limit_keeps_the_limit_the_rest_and_the_reset() {
        let u = parse_key_info(&limited(), 42).unwrap();
        assert_eq!((u.limit, u.limit_remaining, u.limit_reset.as_deref()), (Some(20.0), Some(7.5), Some("monthly")));
    }

    #[test]
    fn absent_limit_fields_read_as_null() {
        let mut v = unlimited();
        let data = v["data"].as_object_mut().unwrap();
        data.remove("limit");
        data.remove("limit_remaining");
        data.remove("limit_reset");
        let u = parse_key_info(&v, 1).unwrap();
        assert_eq!((u.limit, u.limit_remaining, u.limit_reset), (None, None, None));
    }

    #[test]
    fn a_malformed_answer_gives_nothing() {
        assert_eq!(parse_key_info(&json!({}), 1), None);
        assert_eq!(parse_key_info(&json!({ "data": null }), 1), None);
        assert_eq!(parse_key_info(&json!({ "data": [] }), 1), None);
        assert_eq!(parse_key_info(&json!({ "error": { "code": 401, "message": "No auth" } }), 1), None);
        for (field, bad) in [
            ("usage", Value::Null),
            ("usage", json!("12.5")),
            ("usage_daily", Value::Null),
            ("usage_weekly", json!(true)),
            ("usage_monthly", json!({})),
            ("limit", json!("20")),
            ("limit_remaining", json!([])),
            ("limit_reset", json!(1)),
        ] {
            let mut v = limited();
            v["data"][field] = bad.clone();
            assert_eq!(parse_key_info(&v, 1), None, "{field} = {bad}");
        }
        let mut v = unlimited();
        v["data"].as_object_mut().unwrap().remove("usage_monthly");
        assert_eq!(parse_key_info(&v, 1), None, "a missing usage number");
    }

    #[test]
    fn the_status_decides_the_outcome() {
        let body = limited();
        assert!(matches!(outcome(200, Some(&body), 5), Outcome::Read(u) if u.fetched_at_ms == 5));
        assert_eq!(outcome(200, None, 5), Outcome::Failed, "a body that is not JSON");
        assert_eq!(outcome(200, Some(&json!({ "data": {} })), 5), Outcome::Failed);
        assert_eq!(outcome(401, Some(&body), 5), Outcome::Refused);
        assert_eq!(outcome(403, None, 5), Outcome::Refused);
        for status in [402, 404, 429, 500, 503] {
            assert_eq!(outcome(status, Some(&body), 5), Outcome::Failed, "{status}");
        }
    }

    #[test]
    fn no_read_without_a_key_or_while_paused_and_the_gate_is_left_alone() {
        let mut g = Gate::new();
        assert!(!should_read(&mut g, 1_000_000, None, false, false), "no key");
        assert!(!should_read(&mut g, 1_000_000, None, true, true), "paused");
        assert!(should_read(&mut g, 1_000_000, None, true, false), "the gate was not started by the refusals");
    }

    #[test]
    fn reads_wait_for_fresh_credits_run_one_at_a_time_and_back_off_after_a_failure() {
        let now = 10 * BACKOFF_MS;
        let mut g = Gate::new();
        assert!(!should_read(&mut g, now, Some(now - FRESH_MS + 1), true, false), "credits still fresh");
        assert!(should_read(&mut g, now, Some(now - FRESH_MS), true, false), "credits 5 min old");
        assert!(!should_read(&mut g, now, None, true, false), "one read at a time");
        g.finish(now, true);
        assert!(!should_read(&mut g, now + FRESH_MS - 1, None, true, false), "within 5 min of a good read");
        assert!(should_read(&mut g, now + FRESH_MS, None, true, false));
        g.finish(now + FRESH_MS, false);
        let failed = now + FRESH_MS;
        assert!(!should_read(&mut g, failed + FRESH_MS, None, true, false), "the back-off is 15 min, not 5");
        assert!(!should_read(&mut g, failed + BACKOFF_MS - 1, None, true, false));
        assert!(should_read(&mut g, failed + BACKOFF_MS, None, true, false));
    }

    #[test]
    fn the_settings_line_follows_the_reads() {
        use OpenRouterProblem::{Rejected, Unreachable};
        let read = Outcome::Read(parse_key_info(&limited(), 1).unwrap());
        for prev in [None, Some(Rejected), Some(Unreachable)] {
            assert_eq!(problem_after(prev, &read), None, "a read with credits clears {prev:?}");
            assert_eq!(problem_after(prev, &Outcome::Refused), Some(Rejected), "after {prev:?}");
        }
        assert_eq!(problem_after(None, &Outcome::Failed), Some(Unreachable));
        assert_eq!(problem_after(Some(Unreachable), &Outcome::Failed), Some(Unreachable));
        assert_eq!(problem_after(Some(Rejected), &Outcome::Failed), Some(Rejected), "a refused key stays the thing to fix");
    }

    #[test]
    fn a_new_key_reads_at_once() {
        let now = 10 * BACKOFF_MS;
        let mut g = Gate::new();
        assert!(should_read(&mut g, now, None, true, false));
        g.finish(now, false);
        assert!(!should_read(&mut g, now + 1, None, true, false), "backing off");
        g.forget();
        assert!(should_read(&mut g, now + 1, None, true, false), "the back-off of the old key is forgotten");
    }
}
