// Claude plan limits (usage-home spec §2.1). Claude Code caches its answer about the plan limits in
// `.claude.json`, key `cachedUsageUtilization`. Only `fetchedAtMs` and the `utilization` and
// `resets_at` of `utilization.five_hour` / `utilization.seven_day` are read: every other key of the
// file is skipped by the deserializer, never held, logged or stored. Pure, except `read_claude_usage`.

use std::io;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::model::{ClaudeUsage, ClaudeWindow, ClaudeWindowId};

/// The only keys of `.claude.json` the deserializer keeps; serde skips every other key.
#[derive(Deserialize)]
struct ClaudeJson {
    #[serde(rename = "cachedUsageUtilization", default)]
    cached: Option<Cached>,
}

#[derive(Deserialize)]
struct Cached {
    #[serde(rename = "fetchedAtMs", default)]
    fetched_at_ms: Option<Value>,
    #[serde(default)]
    utilization: Option<Windows>,
}

#[derive(Deserialize)]
struct Windows {
    #[serde(default)]
    five_hour: Option<Value>,
    #[serde(default)]
    seven_day: Option<Value>,
}

/// What `.claude.json` says about the plan limits (usage-home spec §2.1). Ok(None): no
/// `cachedUsageUtilization` (or one, or a `utilization`, of another shape), no integer
/// `fetchedAtMs`, or no window with a number `utilization`. Err: the bytes are no complete JSON
/// document (Claude Code may be writing the file); the error is the serde_json category only
/// ("Syntax", "Eof", "Io"), never any text of the file.
pub fn parse_claude_usage(bytes: &[u8]) -> Result<Option<ClaudeUsage>, String> {
    let file: ClaudeJson = match serde_json::from_slice(bytes) {
        Ok(file) => file,
        Err(err) if err.classify() == serde_json::error::Category::Data => return Ok(None),
        Err(err) => return Err(format!("{:?}", err.classify())),
    };
    let Some(cached) = file.cached else { return Ok(None) };
    let Some(fetched_at_ms) = cached.fetched_at_ms.as_ref().and_then(Value::as_i64) else { return Ok(None) };
    let Some(utilization) = cached.utilization else { return Ok(None) };
    let windows: Vec<ClaudeWindow> = [(ClaudeWindowId::FiveHour, utilization.five_hour), (ClaudeWindowId::SevenDay, utilization.seven_day)]
        .into_iter()
        .filter_map(|(id, v)| claude_window(id, &v?))
        .collect();
    Ok((!windows.is_empty()).then_some(ClaudeUsage { fetched_at_ms, windows }))
}

/// A window with a number `utilization`, else None; `resets_at` kept when it parses.
fn claude_window(id: ClaudeWindowId, v: &Value) -> Option<ClaudeWindow> {
    let percent = v.get("utilization")?.as_f64()?;
    let resets_at_ms = v.get("resets_at").and_then(Value::as_str).and_then(parse_rfc3339_ms);
    Some(ClaudeWindow { id, percent, resets_at_ms })
}

/// `.claude.json` read once (usage-home spec §2.1). Ok(None) when the file does not exist; Err with
/// the I/O kind or the JSON category when it cannot be read or parsed right now.
pub fn read_claude_usage(path: &Path) -> Result<Option<ClaudeUsage>, String> {
    match std::fs::read(path) {
        Ok(bytes) => parse_claude_usage(&bytes),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("{:?}", err.kind())),
    }
}

/// Milliseconds since the Unix epoch of an RFC 3339 date-time: `YYYY-MM-DDTHH:MM:SS`, an optional
/// `.` with 1 to 9 digits (rounded down to the millisecond), then `Z` or `±HH:MM`. None for
/// anything else, also for a date that does not exist (2026-02-29) or a second 60.
pub fn parse_rfc3339_ms(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        b.get(from..from + len)?
            .iter()
            .try_fold(0_i64, |acc, d| d.is_ascii_digit().then(|| acc * 10 + i64::from(d - b'0')))
    };
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hour, minute, second) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut i = 19;
    let mut millis = 0_i64;
    if b[i] == b'.' {
        let digits = b[i + 1..].iter().take_while(|d| d.is_ascii_digit()).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let kept = digits.min(3);
        millis = b[i + 1..i + 1 + kept].iter().fold(0, |acc, d| acc * 10 + i64::from(d - b'0'));
        millis *= 10_i64.pow(3 - kept as u32);
        i += 1 + digits;
    }
    let offset_minutes = match &b[i..] {
        [b'Z'] => 0,
        [sign @ (b'+' | b'-'), _, _, b':', _, _] => {
            let (hours, minutes) = (num(i + 1, 2)?, num(i + 4, 2)?);
            if hours > 23 || minutes > 59 {
                return None;
            }
            let total = hours * 60 + minutes;
            if *sign == b'-' { -total } else { total }
        }
        _ => return None,
    };
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second - offset_minutes * 60;
    Some(seconds * 1_000 + millis)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to a proleptic Gregorian date (H. Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12; // March = 0
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use serde_json::json;

    const RESETS: &str = "2026-10-06T14:20:00.123456+00:00";
    /// `RESETS` as ms since the epoch (Node: Date.parse).
    const RESETS_MS: i64 = 1_791_296_400_123;

    /// A `.claude.json` with keys Claude Code writes around the cached limits (values generic).
    fn file(cached: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "numStartups": 12,
            "oauthAccount": { "emailAddress": "dev@example.com", "accountUuid": "00000000-0000-4000-8000-000000000000" },
            "projects": { "C:/work/app": { "allowedTools": [] } },
            "cachedUsageUtilization": cached
        }))
        .unwrap()
    }

    fn window(utilization: Value, resets_at: Value) -> Value {
        json!({ "utilization": utilization, "resets_at": resets_at, "limit_dollars": null, "used_dollars": null })
    }

    #[test]
    fn the_full_object_gives_both_windows_and_nothing_else() {
        let cached = json!({
            "fetchedAtMs": 1_791_290_000_000i64,
            "accountUuid": "00000000-0000-4000-8000-000000000000",
            "utilization": {
                "five_hour": window(json!(37), json!(RESETS)),
                "seven_day": window(json!(81.5), json!("2026-10-09T17:20:00+03:00")),
                "seven_day_opus": null,
                "iguana_necktie": window(json!(99), json!(RESETS)),
                "extra_usage": { "is_enabled": false, "utilization": null },
                "spend": { "percent": 4 }
            }
        });
        assert_eq!(
            parse_claude_usage(&file(cached)),
            Ok(Some(ClaudeUsage {
                fetched_at_ms: 1_791_290_000_000,
                windows: vec![
                    ClaudeWindow { id: ClaudeWindowId::FiveHour, percent: 37.0, resets_at_ms: Some(RESETS_MS) },
                    ClaudeWindow { id: ClaudeWindowId::SevenDay, percent: 81.5, resets_at_ms: Some(1_791_555_600_000) },
                ],
            }))
        );
    }

    #[test]
    fn no_cached_object_gives_none() {
        let without = serde_json::to_vec(&json!({ "numStartups": 12, "oauthAccount": { "emailAddress": "dev@example.com" } })).unwrap();
        assert_eq!(parse_claude_usage(&without), Ok(None));
        assert_eq!(parse_claude_usage(&file(Value::Null)), Ok(None), "null");
        assert_eq!(parse_claude_usage(&file(json!("cached"))), Ok(None), "not an object");
        assert_eq!(parse_claude_usage(&file(json!({ "fetchedAtMs": 5, "utilization": 3 }))), Ok(None), "utilization not an object");
        assert_eq!(parse_claude_usage(&file(json!({ "fetchedAtMs": 5 }))), Ok(None), "no utilization");
        assert_eq!(parse_claude_usage(b"[1, 2]"), Ok(None), "not an object at the top");
    }

    #[test]
    fn a_window_whose_utilization_is_no_number_is_skipped() {
        let cached =
            |five: Value, seven: Value| file(json!({ "fetchedAtMs": 1_000, "utilization": { "five_hour": five, "seven_day": seven } }));
        let only_week = parse_claude_usage(&cached(window(json!("37"), json!(RESETS)), window(json!(12), Value::Null)));
        assert_eq!(
            only_week.unwrap().unwrap().windows,
            [ClaudeWindow { id: ClaudeWindowId::SevenDay, percent: 12.0, resets_at_ms: None }]
        );
        for bad in [json!(null), json!(true), json!([37]), json!({ "v": 37 })] {
            assert_eq!(parse_claude_usage(&cached(window(bad.clone(), json!(RESETS)), Value::Null)), Ok(None), "utilization {bad}");
        }
        assert_eq!(parse_claude_usage(&cached(json!(37), json!("x"))), Ok(None), "windows that are no objects");
    }

    #[test]
    fn an_unparsable_reset_date_gives_a_window_without_it() {
        for resets_at in [json!("soon"), json!("2026-10-06"), json!("2026-10-06T14:20:00"), json!(1_791_296_400), Value::Null] {
            let bytes = file(json!({ "fetchedAtMs": 1_000, "utilization": { "five_hour": window(json!(5), resets_at.clone()) } }));
            let usage = parse_claude_usage(&bytes).unwrap().unwrap();
            assert_eq!(
                usage.windows,
                [ClaudeWindow { id: ClaudeWindowId::FiveHour, percent: 5.0, resets_at_ms: None }],
                "resets_at {resets_at}"
            );
        }
    }

    #[test]
    fn fetched_at_must_be_an_integer() {
        for fetched in [Value::Null, json!("1791290000000"), json!(1.5)] {
            let bytes = file(json!({ "fetchedAtMs": fetched, "utilization": { "five_hour": window(json!(5), json!(RESETS)) } }));
            assert_eq!(parse_claude_usage(&bytes), Ok(None), "fetchedAtMs {fetched}");
        }
        let missing = file(json!({ "utilization": { "five_hour": window(json!(5), json!(RESETS)) } }));
        assert_eq!(parse_claude_usage(&missing), Ok(None));
    }

    #[test]
    fn a_torn_file_is_an_error_that_names_no_text_of_it() {
        // Claude Code rewrites the file while Mayak may read it: the caller keeps the last value.
        let whole = file(json!({ "fetchedAtMs": 1_000, "accountUuid": "secret-account-uuid", "utilization": {} }));
        assert_eq!(parse_claude_usage(&whole[..whole.len() - 30]), Err("Eof".to_string()));
        assert_eq!(parse_claude_usage(b""), Err("Eof".to_string()));
        let broken = br#"{"oauthAccount": {"emailAddress": "dev@example.com"} "cachedUsageUtilization": {}}"#;
        assert_eq!(parse_claude_usage(broken), Err("Syntax".to_string()));
    }

    #[test]
    fn a_missing_file_gives_none_and_an_existing_one_is_read() {
        let dir = TempDir::new("claude-usage");
        let path = dir.path.join(".claude.json");
        assert_eq!(read_claude_usage(&path), Ok(None));
        let bytes = file(json!({ "fetchedAtMs": 1_000, "utilization": { "seven_day": window(json!(3), json!(RESETS)) } }));
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(
            read_claude_usage(&path).unwrap().unwrap().windows,
            [ClaudeWindow { id: ClaudeWindowId::SevenDay, percent: 3.0, resets_at_ms: Some(RESETS_MS) }]
        );
        assert!(read_claude_usage(&dir.path).is_err(), "a folder is no file");
    }

    #[test]
    fn rfc3339_dates_become_epoch_milliseconds() {
        let at = 1_791_296_400_000; // 2026-10-06T14:20:00Z (Node: Date.parse)
        assert_eq!(parse_rfc3339_ms("2026-10-06T14:20:00Z"), Some(at));
        assert_eq!(parse_rfc3339_ms("2026-10-06T14:20:00.123456+00:00"), Some(at + 123), "fraction rounded down to ms");
        assert_eq!(parse_rfc3339_ms("2026-10-06T14:20:00.9Z"), Some(at + 900));
        assert_eq!(parse_rfc3339_ms("2026-10-06T17:20:00+03:00"), Some(at));
        assert_eq!(parse_rfc3339_ms("2026-10-06T09:20:00-05:00"), Some(at));
        assert_eq!(parse_rfc3339_ms("2024-02-29T00:00:00Z"), Some(1_709_164_800_000), "leap day");
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_ms("1969-12-31T23:59:59Z"), Some(-1_000));
    }

    #[test]
    fn anything_else_is_no_date() {
        for text in [
            "", "soon", "2026-10-06", "2026-10-06T14:20:00", "2026-10-06 14:20:00Z", "2026-10-06t14:20:00z",
            "2026-13-01T00:00:00Z", "2026-02-29T00:00:00Z", "2026-10-32T00:00:00Z", "2026-10-06T24:00:00Z",
            "2026-10-06T14:60:00Z", "2026-10-06T14:20:60Z", "2026-10-06T14:20:00.Z", "2026-10-06T14:20:00.1234567890Z",
            "2026-10-06T14:20:00+0300", "2026-10-06T14:20:00+24:00", "2026-10-06T14:20:00Z ", "２０２６-10-06T14:20:00Z",
            "+2026-10-06T14:20:00Z",
        ] {
            assert_eq!(parse_rfc3339_ms(text), None, "{text:?}");
        }
    }
}
