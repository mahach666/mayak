// Moonshot (Kimi) balance on the home page. Only when the user saved a Moonshot API key in Settings
// (Credential Manager, `secrets`), and only when the home page asks (`usage_refresh` on opening it,
// `balances_refresh` on its minute tick), under the rules of `key_reads` (the same as OpenRouter's).
// Moonshot has two platforms whose keys do not work on each other's endpoint (a 401): the
// international one (`GET https://api.moonshot.ai/v1/users/me/balance`, in US dollars) and the
// Chinese one (`https://api.moonshot.cn/...`, in yuan). A key is tried on the international one
// first and, refused there, on the Chinese one; the platform that answered is remembered until the
// key changes. The key and the answer are never logged; the log gets the HTTP status only.

use std::sync::atomic::{AtomicU8, Ordering};

use serde_json::Value;

use crate::key_reads::{self, amount, Outcome, Reader};
use crate::observe::model::{MoonshotBalance, Signal};

/// The Credential Manager entry (secrets::KNOWN_KEYS).
pub const KEY: &str = "moonshot-api-key";

/// One of Moonshot's platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// platform.kimi.ai: api.moonshot.ai, US dollars.
    Global,
    /// platform.kimi.com: api.moonshot.cn, yuan.
    China,
}

impl Platform {
    fn url(self) -> &'static str {
        match self {
            Platform::Global => "https://api.moonshot.ai/v1/users/me/balance",
            Platform::China => "https://api.moonshot.cn/v1/users/me/balance",
        }
    }

    fn currency(self) -> &'static str {
        match self {
            Platform::Global => "USD",
            Platform::China => "CNY",
        }
    }
}

/// The platforms to try for a key, in order: the one known to take it, else both.
pub fn platforms(known: Option<Platform>) -> Vec<Platform> {
    match known {
        Some(p) => vec![p],
        None => vec![Platform::Global, Platform::China],
    }
}

/// A `GET /v1/users/me/balance` answer (Moonshot API reference, "Check Balance"): `{ "code": 0,
/// "data": { "available_balance", "voucher_balance", "cash_balance" }, "scode", "status" }`, the
/// three balances being numbers. None for another `code` or a missing balance.
pub fn parse_balance(body: &Value, platform: Platform, fetched_at_ms: i64) -> Option<MoonshotBalance> {
    if body.get("code")?.as_i64()? != 0 {
        return None;
    }
    let data = body.get("data")?.as_object()?;
    let number = |k: &str| data.get(k).filter(|v| v.is_number()).and_then(amount);
    Some(MoonshotBalance {
        fetched_at_ms,
        currency: platform.currency().to_string(),
        available: number("available_balance")?,
        voucher: number("voucher_balance")?,
        cash: number("cash_balance")?,
    })
}

/// 0: not known yet; 1: Global; 2: China.
static PLATFORM: AtomicU8 = AtomicU8::new(0);
static READER: Reader = Reader::new();

fn known_platform() -> Option<Platform> {
    match PLATFORM.load(Ordering::SeqCst) {
        1 => Some(Platform::Global),
        2 => Some(Platform::China),
        _ => None,
    }
}

/// Reads the balance on each platform of `platforms(known)` until one does not refuse the key.
/// Returns the outcome and the platform that gave a balance.
async fn fetch(key: &str, known: Option<Platform>) -> (Outcome<MoonshotBalance>, Option<Platform>) {
    let mut out = Outcome::Failed;
    for platform in platforms(known) {
        let Some((status, body)) = key_reads::get_json("moonshot", platform.url(), key).await else {
            return (Outcome::Failed, None);
        };
        out = key_reads::outcome(status, body.as_ref(), |b| parse_balance(b, platform, crate::observe::now_ms()));
        match out {
            Outcome::Read(_) => return (out, Some(platform)),
            Outcome::Refused => continue,
            Outcome::Failed => {
                if status / 100 == 2 {
                    crate::log::line("moonshot: the answer has no usable balance");
                }
                return (out, None);
            }
        }
    }
    (out, None)
}

/// The home page asks: reads the key's balance on a task of its own when the gate lets it, and
/// hands it to the store. Returns at once.
pub fn refresh(observe: &crate::observe::Handle) {
    let held = observe.snapshot().usage.moonshot.map(|b| b.fetched_at_ms);
    let Some((key, generation)) = READER.start(KEY, held) else {
        return;
    };
    let observe = observe.clone();
    tauri::async_runtime::spawn(async move {
        let (out, platform) = fetch(&key, known_platform()).await;
        drop(key);
        if !READER.finish(generation, matches!(out, Outcome::Read(_))) {
            return;
        }
        if let Some(p) = platform {
            PLATFORM.store(if p == Platform::Global { 1 } else { 2 }, Ordering::SeqCst);
        }
        let prev = observe.snapshot().usage.moonshot_problem;
        let problem = key_reads::problem_after(prev, &out);
        if problem != prev {
            observe.send(Signal::MoonshotProblem(problem));
        }
        match out {
            Outcome::Read(balance) => observe.send(Signal::MoonshotBalance(Some(balance))),
            Outcome::Refused => observe.send(Signal::MoonshotBalance(None)),
            Outcome::Failed => {}
        }
    });
}

/// The key was saved or removed in Settings: the balance of the old key and its platform go, and
/// the next home page visit reads at once.
pub fn key_changed(observe: &crate::observe::Handle) {
    READER.key_changed();
    PLATFORM.store(0, Ordering::SeqCst);
    observe.send(Signal::MoonshotBalance(None));
    observe.send(Signal::MoonshotProblem(None));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The example of Moonshot's API reference ("Check Balance"), verbatim.
    fn documented() -> Value {
        json!({
            "code": 0,
            "data": { "available_balance": 49.58894, "voucher_balance": 46.58893, "cash_balance": 3.00001 },
            "scode": "0x0",
            "status": true
        })
    }

    #[test]
    fn the_documented_answer_reads_in_the_platform_currency() {
        assert_eq!(
            parse_balance(&documented(), Platform::Global, 42),
            Some(MoonshotBalance { fetched_at_ms: 42, currency: "USD".into(), available: 49.58894, voucher: 46.58893, cash: 3.00001 })
        );
        assert_eq!(parse_balance(&documented(), Platform::China, 42).unwrap().currency, "CNY");
    }

    #[test]
    fn a_negative_cash_balance_is_kept() {
        let mut v = documented();
        v["data"]["cash_balance"] = json!(-1.5);
        v["data"]["available_balance"] = json!(0);
        let b = parse_balance(&v, Platform::Global, 1).unwrap();
        assert_eq!((b.available, b.cash), (0.0, -1.5));
    }

    #[test]
    fn a_malformed_answer_gives_nothing() {
        assert_eq!(parse_balance(&json!({}), Platform::Global, 1), None);
        let mut v = documented();
        v["code"] = json!(1);
        assert_eq!(parse_balance(&v, Platform::Global, 1), None, "a code that is not 0");
        for (field, bad) in [
            ("available_balance", json!(null)),
            ("available_balance", json!("49.5")),
            ("voucher_balance", json!([])),
            ("cash_balance", json!(true)),
        ] {
            let mut v = documented();
            v["data"][field] = bad.clone();
            assert_eq!(parse_balance(&v, Platform::Global, 1), None, "{field} = {bad}");
        }
        let mut v = documented();
        v["data"].as_object_mut().unwrap().remove("voucher_balance");
        assert_eq!(parse_balance(&v, Platform::Global, 1), None, "a missing balance");
        assert_eq!(parse_balance(&json!({ "code": 0, "data": null }), Platform::Global, 1), None);
    }

    #[test]
    fn an_unknown_key_tries_the_global_platform_then_the_chinese_one() {
        assert_eq!(platforms(None), [Platform::Global, Platform::China]);
        assert_eq!(platforms(Some(Platform::China)), [Platform::China]);
        assert_eq!(platforms(Some(Platform::Global)), [Platform::Global]);
        assert_eq!(Platform::Global.url(), "https://api.moonshot.ai/v1/users/me/balance");
        assert_eq!(Platform::China.url(), "https://api.moonshot.cn/v1/users/me/balance");
    }

    #[test]
    fn the_status_decides_the_outcome() {
        let body = documented();
        let parse = |b: &Value| parse_balance(b, Platform::Global, 5);
        assert!(matches!(key_reads::outcome(200, Some(&body), parse), Outcome::Read(b) if b.fetched_at_ms == 5));
        assert_eq!(key_reads::outcome(401, None, parse), Outcome::Refused, "a key of the other platform");
        assert_eq!(key_reads::outcome(500, None, parse), Outcome::Failed);
    }
}
