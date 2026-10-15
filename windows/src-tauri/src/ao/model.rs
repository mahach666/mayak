// The pure half of the Agent Orchestrator (AO) integration: what the daemon's run file and its
// HTTP answers say, as the island shows it. No file, no network, no clock, so every case below runs
// on the real output kept in `fixtures/` (captured from AO 0.13.8-nightly, names and paths made
// anonymous) and on the failures that output can turn into.
//
// The daemon documents its own contract at `GET /api/v1/openapi.yaml` (ControllersSessionView,
// SessionPRFacts). Everything here reads it tolerantly: a field or enum value the contract adds
// later is ignored or passed through, a row that cannot be understood is skipped and counted, and
// a body that is no session list at all is a problem of its own, never an empty list.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::observe::paths;

/// Longest display name sent to the island, in characters.
pub const MAX_NAME_CHARS: usize = 120;
/// Longest id, project id, role, status and harness kept, in characters.
const MAX_WORD_CHARS: usize = 64;

// ── The run file ──────────────────────────────────────────────────────────────

/// `~/.ao/running.json`: the daemon that is (or was) running. The file has more fields (`startedAt`,
/// `owner`, `appRunId`, `browserRuntimeAddress`); only the two below are used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunFile {
    pub pid: u32,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunFileProblem {
    NotJson,
    NoPid,
    NoPort,
}

impl std::fmt::Display for RunFileProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RunFileProblem::NotJson => "the run file is not JSON",
            RunFileProblem::NoPid => "the run file has no usable pid",
            RunFileProblem::NoPort => "the run file has no usable port",
        })
    }
}

pub fn parse_run_file(text: &str) -> Result<RunFile, RunFileProblem> {
    let value: Value = serde_json::from_str(text).map_err(|_| RunFileProblem::NotJson)?;
    let positive = |key: &str| value.get(key).and_then(Value::as_u64).filter(|n| *n > 0);
    let pid = positive("pid").and_then(|n| u32::try_from(n).ok()).ok_or(RunFileProblem::NoPid)?;
    let port = positive("port").and_then(|n| u16::try_from(n).ok()).ok_or(RunFileProblem::NoPort)?;
    Ok(RunFile { pid, port })
}

// ── Sessions ──────────────────────────────────────────────────────────────────

/// One AO session as the island gets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AoSession {
    pub id: String,
    /// None for a projectless (standalone) session.
    pub project: Option<String>,
    /// AO's `kind`, lower case: `worker` or `orchestrator`; anything else AO adds later as it is.
    pub role: String,
    pub name: Option<String>,
    /// AO's `status` code (`working`, `ci_failed`, `merged`…), as it is: the island translates the known ones.
    pub status: String,
    pub activity: Option<String>,
    pub harness: Option<String>,
    /// Pull requests of every state, and how many of them are a draft or open.
    pub prs: usize,
    pub prs_open: usize,
    /// The CI of the open ones, worst first: `failing`, `pending`, `passing`, else `unknown`; None without an open PR.
    pub ci: Option<String>,
    /// The session's workspace as the observer writes a project key (`observe::paths`), once known.
    pub workspace: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Sessions {
    pub list: Vec<AoSession>,
    /// Rows left out: malformed, or an id that is no safe path segment.
    pub skipped: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SessionsProblem {
    NotJson,
    NoList,
    /// A list with rows, none of which could be read: the contract has drifted.
    NoReadableRow,
}

impl std::fmt::Display for SessionsProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SessionsProblem::NotJson => "the session list is not JSON",
            SessionsProblem::NoList => "the answer has no `sessions` list",
            SessionsProblem::NoReadableRow => "no row of the session list could be read",
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSession {
    id: Option<String>,
    project_id: Option<String>,
    kind: Option<String>,
    display_name: Option<String>,
    status: Option<String>,
    activity: Option<RawActivity>,
    harness: Option<String>,
    is_terminated: Option<bool>,
    #[serde(default)]
    prs: Vec<RawPr>,
}

#[derive(Deserialize)]
struct RawActivity {
    state: Option<String>,
}

#[derive(Deserialize)]
struct RawPr {
    state: Option<String>,
    ci: Option<String>,
}

/// Control characters become spaces, runs of white space one space; cut at `max` characters (with
/// an ellipsis). None when nothing is left. AO's names come from the user's prompts: they are shown,
/// never interpreted.
pub fn clean_text(raw: &str, max: usize) -> Option<String> {
    let flat: String = raw.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let joined = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.is_empty() {
        return None;
    }
    if joined.chars().count() <= max {
        return Some(joined);
    }
    let cut: String = joined.chars().take(max.saturating_sub(1)).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// A session id is put into a URL path (`/api/v1/desktop/sessions/{id}/workspace`): letters, digits,
/// `-`, `_` and `.` only, never `.` or `..`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_WORD_CHARS
        && id != "."
        && id != ".."
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn word(raw: Option<String>) -> Option<String> {
    raw.and_then(|s| clean_text(&s, MAX_WORD_CHARS))
}

fn rank_ci(ci: &str) -> u8 {
    match ci {
        "failing" => 3,
        "pending" => 2,
        "passing" => 1,
        _ => 0,
    }
}

fn from_raw(raw: RawSession) -> Option<AoSession> {
    let id = raw.id.filter(|id| valid_id(id))?;
    let role = word(raw.kind)?.to_lowercase();
    let status = word(raw.status)?;
    let open: Vec<&RawPr> = raw.prs.iter().filter(|pr| matches!(pr.state.as_deref(), Some("draft" | "open"))).collect();
    let ci = open
        .iter()
        .map(|pr| pr.ci.as_deref().unwrap_or("unknown"))
        .max_by_key(|ci| rank_ci(ci))
        .map(|ci| if rank_ci(ci) > 0 { ci.to_string() } else { "unknown".into() });
    Some(AoSession {
        id,
        project: word(raw.project_id),
        role,
        name: raw.display_name.and_then(|n| clean_text(&n, MAX_NAME_CHARS)),
        status,
        activity: word(raw.activity.and_then(|a| a.state)),
        harness: word(raw.harness),
        prs: raw.prs.len(),
        prs_open: open.len(),
        ci,
        workspace: None,
    })
}

/// The answer of `GET /api/v1/sessions?active=true`. Terminated sessions are left out even if the
/// daemon sends one.
pub fn parse_sessions(body: &[u8]) -> Result<Sessions, SessionsProblem> {
    let value: Value = serde_json::from_slice(body).map_err(|_| SessionsProblem::NotJson)?;
    let rows = value.get("sessions").and_then(Value::as_array).ok_or(SessionsProblem::NoList)?;
    let mut list = Vec::new();
    let mut skipped = 0;
    for row in rows {
        match serde_json::from_value::<RawSession>(row.clone()) {
            Ok(raw) if raw.is_terminated == Some(true) => {}
            Ok(raw) => match from_raw(raw) {
                Some(session) => list.push(session),
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    if list.is_empty() && skipped > 0 {
        return Err(SessionsProblem::NoReadableRow);
    }
    Ok(Sessions { list, skipped })
}

/// The answer of `GET /api/v1/desktop/sessions/{id}/workspace` as a project key, None when it is
/// not about `id` or has no path.
pub fn parse_workspace(body: &[u8], id: &str) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    if value.get("sessionId").and_then(Value::as_str) != Some(id) {
        return None;
    }
    let path = value.get("workspacePath").and_then(Value::as_str)?;
    let key = paths::normalize(path).key;
    (!key.is_empty()).then_some(key)
}

/// Puts each known workspace on its session.
pub fn attach_workspaces(list: &mut [AoSession], keys: &HashMap<String, String>) {
    for session in list {
        session.workspace = keys.get(&session.id).cloned();
    }
}

// ── What the island receives ──────────────────────────────────────────────────

/// Where AO stands. `ok` has sessions; the others have none and say why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    NotInstalled,
    NotRunning,
    /// The run file names a daemon that does not answer.
    Unreachable,
    /// A daemon answered, but not the way its contract says (or the run file is not what it was).
    Unsupported,
    Ok,
}

impl Status {
    pub fn code(self) -> &'static str {
        match self {
            Status::NotInstalled => "notInstalled",
            Status::NotRunning => "notRunning",
            Status::Unreachable => "unreachable",
            Status::Unsupported => "unsupported",
            Status::Ok => "ok",
        }
    }
}

/// The `data` of the pill's `integration` update. `cli`: an `ao` is on PATH.
pub fn payload(status: Status, cli: bool, sessions: &[AoSession]) -> Value {
    serde_json::json!({ "status": status.code(), "cli": cli, "sessions": sessions })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN: &str = include_str!("fixtures/running.json");
    const SESSIONS: &str = include_str!("fixtures/sessions.json");

    fn session(extra: &str) -> String {
        format!(r#"{{"id":"x-1","kind":"worker","status":"working"{extra}}}"#)
    }

    fn list(rows: &[String]) -> Vec<u8> {
        format!(r#"{{"sessions":[{}]}}"#, rows.join(",")).into_bytes()
    }

    #[test]
    fn the_real_run_file_gives_its_pid_and_port() {
        assert_eq!(parse_run_file(RUN), Ok(RunFile { pid: 11576, port: 3001 }));
    }

    #[test]
    fn a_run_file_without_a_usable_pid_or_port_is_a_problem_not_a_default() {
        assert_eq!(parse_run_file(""), Err(RunFileProblem::NotJson));
        assert_eq!(parse_run_file("{"), Err(RunFileProblem::NotJson));
        assert_eq!(parse_run_file("[]"), Err(RunFileProblem::NoPid));
        assert_eq!(parse_run_file(r#"{"port":3001}"#), Err(RunFileProblem::NoPid));
        assert_eq!(parse_run_file(r#"{"pid":0,"port":3001}"#), Err(RunFileProblem::NoPid));
        assert_eq!(parse_run_file(r#"{"pid":"7","port":3001}"#), Err(RunFileProblem::NoPid));
        assert_eq!(parse_run_file(r#"{"pid":7}"#), Err(RunFileProblem::NoPort));
        assert_eq!(parse_run_file(r#"{"pid":7,"port":0}"#), Err(RunFileProblem::NoPort));
        assert_eq!(parse_run_file(r#"{"pid":7,"port":70000}"#), Err(RunFileProblem::NoPort));
        assert_eq!(parse_run_file(r#"{"pid":4294967296,"port":3001}"#), Err(RunFileProblem::NoPid));
    }

    #[test]
    fn the_real_session_list_is_read_row_by_row() {
        let parsed = parse_sessions(SESSIONS.as_bytes()).unwrap();
        assert_eq!(parsed.skipped, 0);
        let ids: Vec<&str> = parsed.list.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["mayak-1", "mayak-2", "mayak-3", "mayak-4", "mayak-5", "mayak-6", "orbit-1", "orbit-2"]);

        let orchestrator = &parsed.list[0];
        assert_eq!(orchestrator.role, "orchestrator");
        assert_eq!(orchestrator.project.as_deref(), Some("mayak"));
        assert_eq!(orchestrator.name, None, "an orchestrator has no display name");
        assert_eq!(orchestrator.status, "idle");
        assert_eq!(orchestrator.harness.as_deref(), Some("claude-code"));
        assert_eq!((orchestrator.prs, orchestrator.prs_open, orchestrator.ci.as_deref()), (0, 0, None));

        let me = &parsed.list[5];
        assert_eq!(me.id, "mayak-6");
        assert_eq!(me.role, "worker");
        assert_eq!(me.name.as_deref(), Some("Agent Orchestrator recognition and session detection"));
        assert_eq!((me.status.as_str(), me.activity.as_deref()), ("working", Some("active")));

        // Two merged pull requests: counted, but nothing open and so no CI.
        let merged = &parsed.list[1];
        assert_eq!((merged.prs, merged.prs_open, merged.ci.as_deref()), (2, 0, None));
        assert_eq!(parsed.list[3].name.as_deref(), Some("Сверка с upstream: новое, баги и идеи"));
    }

    #[test]
    fn open_pull_requests_roll_their_ci_up_worst_first() {
        let pr = |state: &str, ci: &str| format!(r#"{{"state":"{state}","ci":"{ci}","number":1}}"#);
        let ci_of = |prs: Vec<String>| {
            let body = list(&[session(&format!(r#","prs":[{}]"#, prs.join(",")))]);
            parse_sessions(&body).unwrap().list.remove(0).ci
        };
        assert_eq!(ci_of(vec![pr("open", "passing"), pr("draft", "failing"), pr("open", "pending")]).as_deref(), Some("failing"));
        assert_eq!(ci_of(vec![pr("open", "passing"), pr("open", "pending")]).as_deref(), Some("pending"));
        assert_eq!(ci_of(vec![pr("open", "passing")]).as_deref(), Some("passing"));
        assert_eq!(ci_of(vec![pr("open", "unknown")]).as_deref(), Some("unknown"));
        // A value the contract may add later is `unknown`, not a made-up colour.
        assert_eq!(ci_of(vec![pr("open", "exploding")]).as_deref(), Some("unknown"));
        // A merged or closed pull request's CI is history.
        assert_eq!(ci_of(vec![pr("merged", "failing"), pr("closed", "failing")]), None);
        assert_eq!(ci_of(vec![]), None);
    }

    #[test]
    fn terminated_sessions_are_left_out_and_rows_that_cannot_be_read_are_counted() {
        let body = list(&[
            session(""),
            session(r#","isTerminated":true"#),
            r#"{"id":"no-kind","status":"idle"}"#.into(),
            r#"{"id":"../../etc","kind":"worker","status":"idle"}"#.into(),
            r#"{"id":"bad-prs","kind":"worker","status":"idle","prs":"none"}"#.into(),
            "42".into(),
        ]);
        let parsed = parse_sessions(&body).unwrap();
        assert_eq!(parsed.list.len(), 1);
        assert_eq!(parsed.skipped, 4);
    }

    #[test]
    fn a_body_that_is_no_session_list_is_a_problem_not_an_empty_list() {
        assert_eq!(parse_sessions(b""), Err(SessionsProblem::NotJson));
        assert_eq!(parse_sessions(b"<html>"), Err(SessionsProblem::NotJson));
        assert_eq!(parse_sessions(br#"{"error":"not_found"}"#), Err(SessionsProblem::NoList));
        assert_eq!(parse_sessions(br#"{"sessions":{}}"#), Err(SessionsProblem::NoList));
        assert_eq!(parse_sessions(b"[]"), Err(SessionsProblem::NoList));
        assert_eq!(parse_sessions(br#"{"sessions":[{"x":1},{"y":2}]}"#), Err(SessionsProblem::NoReadableRow));
        // No sessions at all is a normal state.
        assert_eq!(parse_sessions(br#"{"sessions":[]}"#), Ok(Sessions { list: vec![], skipped: 0 }));
        assert_eq!(parse_sessions(br#"{"sessions":null}"#), Err(SessionsProblem::NoList));
    }

    #[test]
    fn unknown_fields_and_values_pass_through_untouched() {
        let body = list(&[session(r#","somethingNew":{"a":[1]},"alsoNew":"x""#)]);
        assert_eq!(parse_sessions(&body).unwrap().list.len(), 1);
        let body = list(&[r#"{"id":"r-1","kind":"Reviewer","status":"brand_new_status"}"#.into()]);
        let s = parse_sessions(&body).unwrap().list.remove(0);
        assert_eq!((s.role.as_str(), s.status.as_str()), ("reviewer", "brand_new_status"));
    }

    #[test]
    fn names_are_flattened_cut_and_never_empty() {
        assert_eq!(clean_text("  a\u{0}b\r\n\tc   d ", 50).as_deref(), Some("a b c d"));
        assert_eq!(clean_text("\u{7}\u{1b}  ", 50), None);
        assert_eq!(clean_text("", 50), None);
        assert_eq!(clean_text("abcdef", 6).as_deref(), Some("abcdef"));
        assert_eq!(clean_text("abcdefg", 6).as_deref(), Some("abcde…"));
        // Characters, not bytes: a Cyrillic name is not cut mid-letter.
        assert_eq!(clean_text("Сверка с upstream", 7).as_deref(), Some("Сверка…"));
        let long = "я".repeat(500);
        assert_eq!(clean_text(&long, MAX_NAME_CHARS).unwrap().chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn only_ids_that_are_safe_in_a_url_path_are_kept() {
        for ok in ["mayak-6", "a", "x_1.2", "ABC-9"] {
            assert!(valid_id(ok), "{ok}");
        }
        let too_long = "x".repeat(65);
        for bad in ["", ".", "..", "a/b", "a\\b", "a b", "a?b", "a#b", "a%2fb", "é", too_long.as_str()] {
            assert!(!valid_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_workspace_answer_becomes_the_observers_project_key() {
        let key = |file: &str, id: &str| parse_workspace(file.as_bytes(), id);
        let me = include_str!("fixtures/workspace-mayak-6.json");
        let orchestrator = include_str!("fixtures/workspace-mayak-1.json");
        #[cfg(windows)]
        {
            assert_eq!(key(me, "mayak-6").as_deref(), Some(r"c:\users\dev\.ao\data\worktrees\mayak\mayak-6"));
            // An orchestrator's worktree is nested one level deeper than a worker's.
            assert_eq!(
                key(orchestrator, "mayak-1").as_deref(),
                Some(r"c:\users\dev\.ao\data\worktrees\mayak\orchestrator\mayak-orchestrator")
            );
        }
        #[cfg(not(windows))]
        {
            assert!(key(me, "mayak-6").is_some());
            assert!(key(orchestrator, "mayak-1").is_some());
        }
        // The answer of another session, or without a path, is no answer.
        assert_eq!(key(me, "mayak-1"), None);
        assert_eq!(key(r#"{"sessionId":"a"}"#, "a"), None);
        assert_eq!(key(r#"{"sessionId":"a","workspacePath":"  "}"#, "a"), None);
        assert_eq!(key("nope", "a"), None);
    }

    #[test]
    fn workspaces_attach_by_session_id() {
        let mut parsed = parse_sessions(SESSIONS.as_bytes()).unwrap();
        let keys = HashMap::from([("mayak-6".to_string(), "k6".to_string())]);
        attach_workspaces(&mut parsed.list, &keys);
        assert_eq!(parsed.list.iter().filter(|s| s.workspace.is_some()).count(), 1);
        assert_eq!(parsed.list[5].workspace.as_deref(), Some("k6"));
    }

    /// `fixtures/payload.json` is what the island receives for the real session list with four
    /// folders known; scripts/checks/ao.mjs reads the same file, so the two sides cannot drift apart.
    #[cfg(windows)]
    #[test]
    fn the_payload_is_the_file_the_island_check_reads() {
        let mut parsed = parse_sessions(SESSIONS.as_bytes()).unwrap();
        let mut keys = HashMap::new();
        for (id, file) in [
            ("mayak-1", include_str!("fixtures/workspace-mayak-1.json")),
            ("mayak-2", include_str!("fixtures/workspace-mayak-2.json")),
            ("mayak-6", include_str!("fixtures/workspace-mayak-6.json")),
            ("orbit-1", include_str!("fixtures/workspace-orbit-1.json")),
        ] {
            keys.insert(id.to_string(), parse_workspace(file.as_bytes(), id).unwrap());
        }
        attach_workspaces(&mut parsed.list, &keys);
        let file: Value = serde_json::from_str(include_str!("fixtures/payload.json")).unwrap();
        assert_eq!(payload(Status::Ok, true, &parsed.list), file);
    }

    #[test]
    fn the_payload_names_the_status_the_cli_and_the_sessions() {
        let parsed = parse_sessions(SESSIONS.as_bytes()).unwrap();
        let ok = payload(Status::Ok, true, &parsed.list);
        assert_eq!(ok["status"], "ok");
        assert_eq!(ok["cli"], true);
        assert_eq!(ok["sessions"].as_array().unwrap().len(), 8);
        assert_eq!(ok["sessions"][5]["prsOpen"], 0);
        assert_eq!(ok["sessions"][0]["name"], Value::Null);
        for (status, code) in [
            (Status::NotInstalled, "notInstalled"),
            (Status::NotRunning, "notRunning"),
            (Status::Unreachable, "unreachable"),
            (Status::Unsupported, "unsupported"),
        ] {
            let p = payload(status, false, &[]);
            assert_eq!((p["status"].as_str(), p["cli"].as_bool(), p["sessions"].as_array().map(Vec::len)), (Some(code), Some(false), Some(0)));
        }
    }
}
