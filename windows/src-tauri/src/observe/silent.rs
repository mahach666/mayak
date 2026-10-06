//! «Silent» Codex (spec 2026-10-06-mayak-silent-codex-design.md): a Codex session that writes its
//! rollout while no hook reaches Mayak. The pure part: reading the first line of a rollout,
//! the host of an originator, the cause table and the liveness rule.

use serde_json::Value;

use super::keys::HostKind;

/// A silent row lives while the rollout was written to within this time (spec §3).
pub const SILENT_TTL_MS: i64 = 120_000;

/// What the first line of a rollout says about its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SilentMeta {
    pub id: String,
    pub cwd: Option<String>,
    pub originator: Option<String>,
    /// A subagent session (it has a parent thread): never shown.
    pub subagent: bool,
}

/// `session_meta` of the first rollout line; `None` for anything else (not JSON, another record,
/// no or empty string `payload.id`). Only these four facts are taken: nothing else of the line
/// is kept.
pub fn parse_session_meta(first_line: &str) -> Option<SilentMeta> {
    let v: Value = serde_json::from_str(first_line).ok()?;
    if v.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let p = v.get("payload")?.as_object()?;
    let text = |key: &str| p.get(key).and_then(Value::as_str).map(str::to_owned);
    let id = text("id").filter(|id| !id.is_empty())?;
    let nested_parent = p
        .get("source")
        .and_then(|s| s.get("subagent"))
        .and_then(|s| s.get("thread_spawn"))
        .and_then(|s| s.get("parent_thread_id"))
        .and_then(Value::as_str);
    let subagent = text("parent_thread_id").is_some() || nested_parent.is_some();
    Some(SilentMeta { id, cwd: text("cwd"), originator: text("originator"), subagent })
}

/// The host an originator names. VS Code and Cursor both write `codex_vscode`, so only the
/// Codex app is told apart; everything else is `Unknown`.
pub fn host_of(originator: Option<&str>) -> HostKind {
    match originator {
        Some("Codex Desktop") => HostKind::CodexDesktop,
        _ => HostKind::Unknown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SilentReason {
    HooksNotInstalled,
    FolderMissing,
    HooksNotReaching,
}

/// Why no hook arrives (spec §3.1), checked from the top.
pub fn reason_of(hooks_not_installed: bool, folder_exists: bool) -> SilentReason {
    if hooks_not_installed {
        SilentReason::HooksNotInstalled
    } else if !folder_exists {
        SilentReason::FolderMissing
    } else {
        SilentReason::HooksNotReaching
    }
}

/// Whether a row last written at `last_write_ms` is still shown at `now_ms`.
pub fn is_alive(last_write_ms: i64, now_ms: i64) -> bool {
    now_ms.saturating_sub(last_write_ms) <= SILENT_TTL_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: &str = r#"{"timestamp":"2026-10-06T10:00:00.000Z","ordinal":0,"type":"session_meta","payload":{"id":"11111111-1111-4111-8111-111111111111","cwd":"C:\\Users\\dev\\src\\demo","originator":"Codex Desktop","source":"vscode","cli_version":"0.160.0","base_instructions":{"text":"generic"}}}"#;

    #[test]
    fn a_session_meta_line_gives_id_cwd_and_originator() {
        let m = parse_session_meta(META).unwrap();
        assert_eq!(m.id, "11111111-1111-4111-8111-111111111111");
        assert_eq!(m.cwd.as_deref(), Some(r"C:\Users\dev\src\demo"));
        assert_eq!(m.originator.as_deref(), Some("Codex Desktop"));
        assert!(!m.subagent);
    }

    #[test]
    fn a_subagent_is_recognised_at_the_top_level_and_under_the_spawn() {
        let top = META.replace(r#""source":"vscode""#, r#""source":"vscode","parent_thread_id":"22222222-2222-4222-8222-222222222222""#);
        assert!(parse_session_meta(&top).unwrap().subagent);
        let nested = META.replace(
            r#""source":"vscode""#,
            r#""source":{"subagent":{"thread_spawn":{"parent_thread_id":"22222222-2222-4222-8222-222222222222","agent_role":null}}}"#,
        );
        assert!(parse_session_meta(&nested).unwrap().subagent);
    }

    #[test]
    fn lines_that_are_not_a_session_meta_with_an_id_give_nothing() {
        assert!(parse_session_meta("").is_none());
        assert!(parse_session_meta("not json").is_none());
        assert!(parse_session_meta(r#"{"type":"event_msg","payload":{"type":"task_started"}}"#).is_none());
        assert!(parse_session_meta(r#"{"type":"session_meta","payload":{"cwd":"C:\\x"}}"#).is_none());
        assert!(parse_session_meta(r#"{"type":"session_meta","payload":{"id":""}}"#).is_none());
        assert!(parse_session_meta(r#"{"type":"session_meta","payload":{"id":7}}"#).is_none());
    }

    #[test]
    fn only_the_codex_app_originator_names_a_host() {
        assert_eq!(host_of(Some("Codex Desktop")), HostKind::CodexDesktop);
        // VS Code and Cursor both write codex_vscode: the editor cannot be told from the rollout
        assert_eq!(host_of(Some("codex_vscode")), HostKind::Unknown);
        assert_eq!(host_of(Some("something else")), HostKind::Unknown);
        assert_eq!(host_of(None), HostKind::Unknown);
    }

    #[test]
    fn the_reason_follows_the_table_from_the_top() {
        assert_eq!(reason_of(true, true), SilentReason::HooksNotInstalled);
        assert_eq!(reason_of(true, false), SilentReason::HooksNotInstalled);
        assert_eq!(reason_of(false, false), SilentReason::FolderMissing);
        assert_eq!(reason_of(false, true), SilentReason::HooksNotReaching);
    }

    #[test]
    fn the_wire_names_of_the_reasons_are_camel_case() {
        let name = |r| serde_json::to_value(r).unwrap();
        assert_eq!(name(SilentReason::HooksNotInstalled), "hooksNotInstalled");
        assert_eq!(name(SilentReason::FolderMissing), "folderMissing");
        assert_eq!(name(SilentReason::HooksNotReaching), "hooksNotReaching");
    }

    #[test]
    fn a_row_is_alive_for_exactly_two_minutes() {
        assert!(is_alive(1_000, 1_000));
        assert!(is_alive(1_000, 1_000 + SILENT_TTL_MS));
        assert!(!is_alive(1_000, 1_000 + SILENT_TTL_MS + 1));
        // a clock that went back does not kill the row
        assert!(is_alive(5_000, 1_000));
    }
}
