// DeepSeek balance on the home page. Only when the user saved a DeepSeek API key in Settings
// (Credential Manager, `secrets`), and only when the home page asks (`usage_refresh` on opening it,
// `balances_refresh` on its minute tick): one `GET https://api.deepseek.com/user/balance` with that
// key, under the rules of `key_reads` (the same as OpenRouter's). The key and the answer are never
// logged; the log gets the HTTP status only.

use serde_json::Value;

use crate::key_reads::{self, amount, Outcome, Reader};
use crate::observe::model::{CurrencyBalance, DeepSeekBalance, Signal};

/// The Credential Manager entry (secrets::KNOWN_KEYS).
pub const KEY: &str = "deepseek-api-key";
const URL: &str = "https://api.deepseek.com/user/balance";

static READER: Reader = Reader::new();

/// A `GET /user/balance` answer (DeepSeek API reference, "Get User Balance"): `{ "is_available":
/// bool, "balance_infos": [{ "currency": "CNY" | "USD", "total_balance", "granted_balance",
/// "topped_up_balance" }] }`, the amounts being decimal strings. An entry whose currency is not
/// three capital letters or whose amounts do not read is left out; None when `is_available` is not
/// a boolean or `balance_infos` not a list.
pub fn parse_balance(body: &Value, fetched_at_ms: i64) -> Option<DeepSeekBalance> {
    let is_available = body.get("is_available")?.as_bool()?;
    let balances = body
        .get("balance_infos")?
        .as_array()?
        .iter()
        .filter_map(|info| {
            let currency = info.get("currency")?.as_str().filter(|c| c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase()))?;
            Some(CurrencyBalance {
                currency: currency.to_string(),
                total: amount(info.get("total_balance")?)?,
                granted: amount(info.get("granted_balance")?)?,
                topped_up: amount(info.get("topped_up_balance")?)?,
            })
        })
        .collect();
    Some(DeepSeekBalance { fetched_at_ms, is_available, balances })
}

async fn fetch(key: &str) -> Outcome<DeepSeekBalance> {
    let Some((status, body)) = key_reads::get_json("deepseek", URL, key).await else {
        return Outcome::Failed;
    };
    let out = key_reads::outcome(status, body.as_ref(), |b| parse_balance(b, crate::observe::now_ms()));
    if status / 100 == 2 && out == Outcome::Failed {
        crate::log::line("deepseek: the answer has no usable balance");
    }
    out
}

/// The home page asks: reads the key's balance on a task of its own when the gate lets it, and
/// hands it to the store. Returns at once.
pub fn refresh(observe: &crate::observe::Handle) {
    let held = observe.snapshot().usage.deepseek.map(|b| b.fetched_at_ms);
    let Some((key, generation)) = READER.start(KEY, held) else {
        return;
    };
    let observe = observe.clone();
    tauri::async_runtime::spawn(async move {
        let out = fetch(&key).await;
        drop(key);
        if !READER.finish(generation, matches!(out, Outcome::Read(_))) {
            return;
        }
        let prev = observe.snapshot().usage.deepseek_problem;
        let problem = key_reads::problem_after(prev, &out);
        if problem != prev {
            observe.send(Signal::DeepSeekProblem(problem));
        }
        match out {
            Outcome::Read(balance) => observe.send(Signal::DeepSeekBalance(Some(balance))),
            Outcome::Refused => observe.send(Signal::DeepSeekBalance(None)),
            Outcome::Failed => {}
        }
    });
}

/// The key was saved or removed in Settings: the balance of the old key goes, and the next home
/// page visit reads at once.
pub fn key_changed(observe: &crate::observe::Handle) {
    READER.key_changed();
    observe.send(Signal::DeepSeekBalance(None));
    observe.send(Signal::DeepSeekProblem(None));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The example of DeepSeek's API reference ("Get User Balance"), verbatim.
    fn documented() -> Value {
        json!({
            "is_available": true,
            "balance_infos": [
                { "currency": "CNY", "total_balance": "110.00", "granted_balance": "10.00", "topped_up_balance": "100.00" }
            ]
        })
    }

    fn cny(total: f64, granted: f64, topped_up: f64) -> CurrencyBalance {
        CurrencyBalance { currency: "CNY".into(), total, granted, topped_up }
    }

    #[test]
    fn the_documented_answer_reads() {
        assert_eq!(
            parse_balance(&documented(), 42),
            Some(DeepSeekBalance { fetched_at_ms: 42, is_available: true, balances: vec![cny(110.0, 10.0, 100.0)] })
        );
    }

    #[test]
    fn every_currency_is_kept_in_order() {
        let mut v = documented();
        v["balance_infos"].as_array_mut().unwrap().push(
            json!({ "currency": "USD", "total_balance": "5.25", "granted_balance": "0.00", "topped_up_balance": "5.25" }),
        );
        let b = parse_balance(&v, 1).unwrap();
        assert_eq!(
            b.balances,
            vec![cny(110.0, 10.0, 100.0), CurrencyBalance { currency: "USD".into(), total: 5.25, granted: 0.0, topped_up: 5.25 }]
        );
    }

    #[test]
    fn a_balance_too_low_for_calls_still_reads() {
        let v = json!({
            "is_available": false,
            "balance_infos": [{ "currency": "USD", "total_balance": "0.00", "granted_balance": "0.00", "topped_up_balance": "0.00" }]
        });
        let b = parse_balance(&v, 1).unwrap();
        assert!(!b.is_available);
        assert_eq!(b.balances[0].total, 0.0);
        let none = parse_balance(&json!({ "is_available": false, "balance_infos": [] }), 1).unwrap();
        assert_eq!((none.is_available, none.balances.len()), (false, 0), "no currency yet");
    }

    #[test]
    fn an_unusable_entry_is_left_out() {
        for (field, bad) in [
            ("currency", json!("cny")),
            ("currency", json!("YUAN")),
            ("currency", json!(null)),
            ("total_balance", json!("abc")),
            ("granted_balance", json!(null)),
            ("topped_up_balance", json!([])),
        ] {
            let mut v = documented();
            let good = v["balance_infos"][0].clone();
            v["balance_infos"][0][field] = bad.clone();
            v["balance_infos"].as_array_mut().unwrap().push(good);
            assert_eq!(parse_balance(&v, 1).unwrap().balances, vec![cny(110.0, 10.0, 100.0)], "{field} = {bad}");
        }
        let mut v = documented();
        v["balance_infos"][0].as_object_mut().unwrap().remove("total_balance");
        assert_eq!(parse_balance(&v, 1).unwrap().balances, vec![], "a missing amount");
    }

    #[test]
    fn a_malformed_answer_gives_nothing() {
        assert_eq!(parse_balance(&json!({}), 1), None);
        assert_eq!(parse_balance(&json!({ "is_available": "true", "balance_infos": [] }), 1), None);
        assert_eq!(parse_balance(&json!({ "is_available": true }), 1), None);
        assert_eq!(parse_balance(&json!({ "is_available": true, "balance_infos": {} }), 1), None);
        assert_eq!(parse_balance(&json!({ "error": { "message": "Authentication Fails" } }), 1), None);
    }

    #[test]
    fn the_status_decides_the_outcome() {
        let body = documented();
        let parse = |b: &Value| parse_balance(b, 5);
        assert!(matches!(key_reads::outcome(200, Some(&body), parse), Outcome::Read(b) if b.fetched_at_ms == 5));
        // DeepSeek's error codes: 401 Authentication Fails, 402 Insufficient Balance, 429, 5xx.
        assert_eq!(key_reads::outcome(401, None, parse), Outcome::Refused);
        for status in [402, 429, 500, 503] {
            assert_eq!(key_reads::outcome(status, None, parse), Outcome::Failed, "{status}");
        }
    }
}
