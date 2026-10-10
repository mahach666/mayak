// What the balance providers read with a key the user saved in Settings (`deepseek`, `moonshot`)
// share: the outcome of one read, the Settings line it leaves, and the gate with the key's
// generation. The rules are those of `openrouter`: a read only when the home page asks and the
// balance held is older than FRESH_MS, none within BACKOFF_MS of a failed read, never while Mayak
// is paused, and a read that started with an old key is dropped. The key and the answer are never
// logged; the log gets the HTTP status only.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::Value;

use crate::codex_limits::Gate;
use crate::log;
use crate::observe::model::KeyProblem;

const TIMEOUT: Duration = Duration::from_secs(10);

/// What one read gave.
#[derive(Debug, PartialEq)]
pub enum Outcome<T> {
    Read(T),
    /// 401 or 403: the key is wrong or revoked. The balance held is dropped.
    Refused,
    Failed,
}

/// The outcome of an HTTP answer: its status and, for a 2xx, its body as JSON (None when it is
/// not), read by `parse`.
pub fn outcome<T>(status: u16, body: Option<&Value>, parse: impl FnOnce(&Value) -> Option<T>) -> Outcome<T> {
    match status {
        200..=299 => body.and_then(parse).map_or(Outcome::Failed, Outcome::Read),
        401 | 403 => Outcome::Refused,
        _ => Outcome::Failed,
    }
}

/// The Settings line after a read that ended in `out`, the line before being `prev`: a read clears
/// it, a refused key says so, any other failure says the provider could not be reached unless the
/// key is already known to be refused (a refused key stays the thing to fix).
pub fn problem_after<T>(prev: Option<KeyProblem>, out: &Outcome<T>) -> Option<KeyProblem> {
    match out {
        Outcome::Read(_) => None,
        Outcome::Refused => Some(KeyProblem::Rejected),
        Outcome::Failed if prev == Some(KeyProblem::Rejected) => prev,
        Outcome::Failed => Some(KeyProblem::Unreachable),
    }
}

/// True when a read starts now: a key is saved, Mayak is not paused, and the gate lets it
/// (`held_at`: the time of the balance the store holds). Without a key or while paused the gate is
/// not touched.
pub fn should_read(gate: &mut Gate, now: i64, held_at: Option<i64>, key_present: bool, paused: bool) -> bool {
    key_present && !paused && gate.try_start(now, held_at)
}

/// The gate of one provider and the generation of its key.
pub struct Reader {
    gate: Mutex<Gate>,
    /// Bumped when the key is saved or removed: a read that started with the old key is dropped.
    generation: AtomicU64,
}

impl Reader {
    pub const fn new() -> Self {
        Reader { gate: Mutex::new(Gate::new()), generation: AtomicU64::new(0) }
    }

    fn gate(&self) -> MutexGuard<'_, Gate> {
        self.gate.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The key saved under `key` when a read starts now, with the generation it started in.
    pub fn start(&self, key: &str, held_at: Option<i64>) -> Option<(String, u64)> {
        let paused = crate::integrations::PAUSED.load(Ordering::Relaxed);
        let value = crate::secrets::get(key)?;
        if !should_read(&mut self.gate(), crate::observe::now_ms(), held_at, true, paused) {
            return None;
        }
        Some((value, self.generation.load(Ordering::SeqCst)))
    }

    /// The read that started in `generation` ended (`ok`: it gave a balance). False when the key
    /// changed meanwhile: its answer belongs to the old key and is dropped.
    pub fn finish(&self, generation: u64, ok: bool) -> bool {
        let mut g = self.gate();
        g.finish(crate::observe::now_ms(), ok);
        if self.generation.load(Ordering::SeqCst) != generation {
            g.forget();
            return false;
        }
        true
    }

    /// The key was saved or removed: the next home page visit reads at once.
    pub fn key_changed(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.gate().forget();
    }
}

/// One `GET url` with the key as a bearer token: the HTTP status and, for a 2xx, the body as JSON.
/// None when no answer came. `name` prefixes the log lines, which hold the status only.
pub async fn get_json(name: &str, url: &str, key: &str) -> Option<(u16, Option<Value>)> {
    let client = match reqwest::Client::builder().timeout(TIMEOUT).build() {
        Ok(c) => c,
        Err(_) => {
            log::line(format!("{name}: no HTTP client"));
            return None;
        }
    };
    let response = match client.get(url).bearer_auth(key).send().await {
        Ok(r) => r,
        Err(e) => {
            let why = if e.is_timeout() { "timed out" } else if e.is_connect() { "could not connect" } else { "failed" };
            log::line(format!("{name}: request {why}"));
            return None;
        }
    };
    let status = response.status().as_u16();
    log::line(format!("{name}: status {status}"));
    let body = if response.status().is_success() { response.json::<Value>().await.ok() } else { None };
    Some((status, body))
}

/// A decimal amount as a provider sends it: a JSON number, or a string such as "110.00"
/// (DeepSeek). None for anything else, also for a number that is not finite.
pub fn amount(v: &Value) -> Option<f64> {
    let n = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_limits::{BACKOFF_MS, FRESH_MS};
    use serde_json::json;

    #[test]
    fn the_status_decides_the_outcome() {
        let parse = |v: &Value| v.get("n").and_then(Value::as_i64);
        let body = json!({ "n": 3 });
        assert_eq!(outcome(200, Some(&body), parse), Outcome::Read(3));
        assert_eq!(outcome(200, None, parse), Outcome::Failed, "a body that is not JSON");
        assert_eq!(outcome(200, Some(&json!({})), parse), Outcome::Failed, "an unusable answer");
        assert_eq!(outcome(401, Some(&body), parse), Outcome::Refused);
        assert_eq!(outcome(403, None, parse), Outcome::Refused);
        for status in [402, 404, 429, 500, 503] {
            assert_eq!(outcome(status, Some(&body), parse), Outcome::Failed, "{status}");
        }
    }

    #[test]
    fn the_settings_line_follows_the_reads() {
        use KeyProblem::{Rejected, Unreachable};
        let read: Outcome<()> = Outcome::Read(());
        for prev in [None, Some(Rejected), Some(Unreachable)] {
            assert_eq!(problem_after(prev, &read), None, "a read clears {prev:?}");
            assert_eq!(problem_after(prev, &Outcome::<()>::Refused), Some(Rejected), "after {prev:?}");
        }
        assert_eq!(problem_after(None, &Outcome::<()>::Failed), Some(Unreachable));
        assert_eq!(problem_after(Some(Rejected), &Outcome::<()>::Failed), Some(Rejected), "a refused key stays the thing to fix");
    }

    #[test]
    fn reads_follow_the_gate_and_never_run_without_a_key_or_while_paused() {
        let now = 10 * BACKOFF_MS;
        let mut g = Gate::new();
        assert!(!should_read(&mut g, now, None, false, false), "no key");
        assert!(!should_read(&mut g, now, None, true, true), "paused");
        assert!(!should_read(&mut g, now, Some(now - FRESH_MS + 1), true, false), "balance still fresh");
        assert!(should_read(&mut g, now, Some(now - FRESH_MS), true, false), "balance 5 min old");
        assert!(!should_read(&mut g, now, None, true, false), "one read at a time");
        g.finish(now, false);
        assert!(!should_read(&mut g, now + BACKOFF_MS - 1, None, true, false), "backing off 15 min");
        assert!(should_read(&mut g, now + BACKOFF_MS, None, true, false));
    }

    #[test]
    fn amounts_are_numbers_or_decimal_strings() {
        assert_eq!(amount(&json!("110.00")), Some(110.0));
        assert_eq!(amount(&json!(" 0.5 ")), Some(0.5));
        assert_eq!(amount(&json!(49.58894)), Some(49.58894));
        assert_eq!(amount(&json!(-3)), Some(-3.0));
        for bad in [json!(null), json!(""), json!("abc"), json!("NaN"), json!("inf"), json!(true), json!([]), json!({})] {
            assert_eq!(amount(&bad), None, "{bad}");
        }
    }
}
