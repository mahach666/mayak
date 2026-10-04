// Claude Code source (spec §5.5): the session files `~/.claude/sessions/<pid>.json` — name
// filter, lenient parsing, acceptance rules and host classification. These functions are
// pure: bytes, process image paths and %APPDATA% come in as arguments.

use std::path::Path;

use serde_json::{Map, Value};

use super::keys::HostKind;
use super::paths::{glob_match, normalize};

/// One `~/.claude/sessions/<pid>.json`. Built field by field from a `serde_json::Value`: a
/// missing key or a value of the wrong JSON type gives None for that field only, and unknown
/// keys are ignored. The format is undocumented and changes with Claude Code (spec §15).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeSessionFile {
    pub pid: Option<u32>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    /// Decimal creation FILETIME: a JSON string as is, or a JSON u64 number in decimal.
    pub proc_start: Option<String>,
    pub version: Option<String>,
    pub kind: Option<String>,
    pub entrypoint: Option<String>,
    pub name: Option<String>,
    pub name_source: Option<String>,
    pub status: Option<String>,
    pub waiting_for: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub status_updated_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub spare: Option<bool>,
    pub parked_job_id: Option<String>,
    pub pid_domain: Option<String>,
}

/// `^[0-9]+\.json$` without a regex crate. The `<pid>.<hash>.key` files next to the session
/// files hold peer tokens and must never match.
pub fn is_session_file_name(name: &str) -> bool {
    match name.strip_suffix(".json") {
        Some(stem) => !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// Empty or whitespace-only → Err("empty"); invalid JSON → Err("invalid JSON: …", position
/// only, never content); valid JSON that is not an object → Err("not an object"). Any JSON
/// object → Ok. Never panics.
pub fn parse_session_file(bytes: &[u8]) -> Result<ClaudeSessionFile, String> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Err("empty".to_string());
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|e| format!("invalid JSON: {e}"))?;
    let Value::Object(o) = value else {
        return Err("not an object".to_string());
    };
    Ok(ClaudeSessionFile {
        pid: o.get("pid").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
        session_id: text(&o, "sessionId"),
        cwd: text(&o, "cwd"),
        proc_start: match o.get("procStart") {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Number(n)) => n.as_u64().map(|n| n.to_string()),
            _ => None,
        },
        version: text(&o, "version"),
        kind: text(&o, "kind"),
        entrypoint: text(&o, "entrypoint"),
        name: text(&o, "name"),
        name_source: text(&o, "nameSource"),
        status: text(&o, "status"),
        waiting_for: text(&o, "waitingFor"),
        status_updated_at: o.get("statusUpdatedAt").and_then(Value::as_i64),
        updated_at: o.get("updatedAt").and_then(Value::as_i64),
        spare: o.get("spare").and_then(Value::as_bool),
        parked_job_id: text(&o, "parkedJobId"),
        pid_domain: text(&o, "pidDomain"),
    })
}

fn text(o: &Map<String, Value>, key: &str) -> Option<String> {
    o.get(key).and_then(Value::as_str).map(str::to_owned)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Spare,
    Parked,
    ForeignDomain,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accept {
    Track { pid: u32, proc_start: u64 },
    Skip(SkipReason),
}

/// Spec §5.5 item 2, in order: spare → Spare; non-empty `parkedJobId` → Parked; `pidDomain`
/// present and not starting with "win32:" → ForeignDomain; pid missing, procStart missing or
/// not a u64, sessionId or cwd missing/empty → Incomplete (the source counts it as unreadable).
pub fn accept(f: &ClaudeSessionFile) -> Accept {
    if f.spare == Some(true) {
        return Accept::Skip(SkipReason::Spare);
    }
    if f.parked_job_id.as_deref().is_some_and(|j| !j.is_empty()) {
        return Accept::Skip(SkipReason::Parked);
    }
    if f.pid_domain.as_deref().is_some_and(|d| !d.starts_with("win32:")) {
        return Accept::Skip(SkipReason::ForeignDomain);
    }
    let filled = |s: &Option<String>| s.as_deref().is_some_and(|v| !v.is_empty());
    let proc_start = f.proc_start.as_deref().and_then(|s| s.parse::<u64>().ok());
    match (f.pid, proc_start) {
        (Some(pid), Some(proc_start)) if filled(&f.session_id) && filled(&f.cwd) => {
            Accept::Track { pid, proc_start }
        }
        _ => Accept::Skip(SkipReason::Incomplete),
    }
}

/// Claude Desktop (MSIX) runs `claude.exe` from its package folder; virtualized views
/// (`Get-Process`) show the same file under `%APPDATA%\Claude\claude-code\<ver>\`.
const DESKTOP_MSIX: &str = r"**\packages\claude_*\localcache\roaming\claude\claude-code\*\claude.exe";
const CURSOR_EXTENSION: &str = r"**\.cursor\extensions\anthropic.claude-code-*\**\claude.exe";
const VSCODE_EXTENSION: &str = r"**\.vscode\extensions\anthropic.claude-code-*\**\claude.exe";

/// Host of a Claude process from its raw image path (QueryFullProcessImageNameW), compared as
/// `paths::normalize(image).key` (spec §5.5 item 5). A Desktop image counts as ClaudeDesktop
/// only together with `entrypoint == "claude-desktop"`, else Unknown. `appdata` is %APPDATA%,
/// passed in by the caller (no environment reads here).
pub fn classify_host(image: &str, entrypoint: Option<&str>, appdata: &Path) -> HostKind {
    let key = normalize(image).key;
    let roaming_desktop = !appdata.as_os_str().is_empty()
        && glob_match(
            &key,
            &format!(r"{}\claude\claude-code\*\claude.exe", normalize(&appdata.to_string_lossy()).key),
        );
    if glob_match(&key, DESKTOP_MSIX) || roaming_desktop {
        return if entrypoint == Some("claude-desktop") { HostKind::ClaudeDesktop } else { HostKind::Unknown };
    }
    if glob_match(&key, CURSOR_EXTENSION) {
        HostKind::Cursor
    } else if glob_match(&key, VSCODE_EXTENSION) {
        HostKind::VsCode
    } else {
        HostKind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUSY: &str = include_str!("fixtures/claude/session_busy.json");
    const IDLE: &str = include_str!("fixtures/claude/session_idle.json");
    const WAITING_PERMISSION: &str = include_str!("fixtures/claude/session_waiting_permission.json");
    const WAITING_INPUT: &str = include_str!("fixtures/claude/session_waiting_input.json");
    const WAITING_OTHER: &str = include_str!("fixtures/claude/session_waiting_other.json");
    const SPARE: &str = include_str!("fixtures/claude/session_spare.json");
    const PARKED: &str = include_str!("fixtures/claude/session_parked.json");
    const LINUX_DOMAIN: &str = include_str!("fixtures/claude/session_linux_domain.json");
    const NO_PROCSTART: &str = include_str!("fixtures/claude/session_no_procstart.json");
    const GARBAGE: &str = include_str!("fixtures/claude/session_garbage.json");
    const MISTYPED: &str = include_str!("fixtures/claude/session_mistyped_field.json");

    const APPDATA: &str = r"C:\Users\dev\AppData\Roaming";

    fn parse(src: &str) -> ClaudeSessionFile {
        parse_session_file(src.as_bytes()).expect("fixture parses")
    }

    #[test]
    fn session_file_names_are_digits_dot_json_only() {
        for good in ["30584.json", "1.json"] {
            assert!(is_session_file_name(good), "{good}");
        }
        for bad in ["30584.abc.key", ".json", "30584.json.tmp", "abc.json", "30584.JSON", "30584"] {
            assert!(!is_session_file_name(bad), "{bad}");
        }
    }

    #[test]
    fn parses_a_busy_session_file() {
        let expected = ClaudeSessionFile {
            pid: Some(4242),
            session_id: Some("11111111-1111-4111-8111-111111111111".into()),
            cwd: Some(r"C:\Users\dev\src\demo".into()),
            proc_start: Some("134040000000000000".into()),
            version: Some("2.1.286".into()),
            kind: Some("interactive".into()),
            entrypoint: Some("claude-vscode".into()),
            name: Some("demo-tests".into()),
            name_source: Some("derived".into()),
            status: Some("busy".into()),
            waiting_for: None,
            status_updated_at: Some(1_790_000_060_000),
            updated_at: Some(1_790_000_060_000),
            spare: None,
            parked_job_id: None,
            pid_domain: Some("win32:devbox".into()),
        };
        assert_eq!(parse(BUSY), expected);
    }

    #[test]
    fn parses_idle_and_the_three_waiting_kinds() {
        let idle = parse(IDLE);
        assert_eq!(idle.status.as_deref(), Some("idle"));
        assert_eq!(idle.waiting_for, None);
        assert_eq!(idle.status_updated_at, Some(1_790_000_090_000));

        for (src, waiting_for) in [
            (WAITING_PERMISSION, "permission prompt"),
            (WAITING_INPUT, "input needed"),
            (WAITING_OTHER, "plan review"),
        ] {
            let f = parse(src);
            assert_eq!(f.status.as_deref(), Some("waiting"));
            assert_eq!(f.waiting_for.as_deref(), Some(waiting_for));
            assert_eq!(f.status_updated_at, Some(1_790_000_070_000));
        }
        assert_eq!(parse(WAITING_INPUT).entrypoint.as_deref(), Some("claude-desktop"));
    }

    #[test]
    fn complete_win32_sessions_are_tracked() {
        assert_eq!(accept(&parse(BUSY)), Accept::Track { pid: 4242, proc_start: 134_040_000_000_000_000 });
        assert_eq!(accept(&parse(WAITING_INPUT)), Accept::Track { pid: 5353, proc_start: 134_040_000_000_001_111 });
    }

    #[test]
    fn spare_parked_foreign_and_incomplete_files_are_skipped() {
        assert_eq!(accept(&parse(SPARE)), Accept::Skip(SkipReason::Spare));
        assert_eq!(accept(&parse(PARKED)), Accept::Skip(SkipReason::Parked));
        assert_eq!(accept(&parse(LINUX_DOMAIN)), Accept::Skip(SkipReason::ForeignDomain));
        assert_eq!(accept(&parse(NO_PROCSTART)), Accept::Skip(SkipReason::Incomplete));
    }

    #[test]
    fn tracking_needs_pid_numeric_procstart_session_id_and_cwd() {
        let base = parse(BUSY);
        let cases = [
            ClaudeSessionFile { pid: None, ..base.clone() },
            ClaudeSessionFile { proc_start: Some("13404x".into()), ..base.clone() },
            ClaudeSessionFile { session_id: None, ..base.clone() },
            ClaudeSessionFile { session_id: Some(String::new()), ..base.clone() },
            ClaudeSessionFile { cwd: Some(String::new()), ..base.clone() },
        ];
        for f in cases {
            assert_eq!(accept(&f), Accept::Skip(SkipReason::Incomplete), "{f:?}");
        }
        // No pidDomain at all (older versions) and an empty parkedJobId are fine.
        let plain = ClaudeSessionFile { pid_domain: None, parked_job_id: Some(String::new()), ..base };
        assert_eq!(accept(&plain), Accept::Track { pid: 4242, proc_start: 134_040_000_000_000_000 });
    }

    #[test]
    fn empty_truncated_and_non_object_files_are_errors() {
        assert_eq!(parse_session_file(b""), Err("empty".to_string()));
        assert_eq!(parse_session_file(b"  \r\n\t"), Err("empty".to_string()));
        let truncated = parse_session_file(GARBAGE.as_bytes()).unwrap_err();
        assert!(truncated.starts_with("invalid JSON: "), "{truncated}");
        assert!(!truncated.contains("Users"), "the reason never repeats file content: {truncated}");
        assert_eq!(parse_session_file(b"[1,2]"), Err("not an object".to_string()));
        assert!(parse_session_file(&[0xFF, 0xFE, 0x00]).is_err());
    }

    #[test]
    fn a_mistyped_field_loses_only_itself() {
        let f = parse(MISTYPED);
        assert_eq!(f.waiting_for, None, "an object where a string belongs is dropped");
        assert_eq!(f.proc_start.as_deref(), Some("134040000000000000"), "a numeric procStart is kept in decimal");
        assert_eq!(f.status.as_deref(), Some("waiting"));
        assert_eq!(accept(&f), Accept::Track { pid: 4242, proc_start: 134_040_000_000_000_000 });
    }

    #[test]
    fn a_pid_outside_u32_is_missing() {
        assert_eq!(parse_session_file(br#"{"pid":4294967296}"#).unwrap().pid, None);
        assert_eq!(parse_session_file(br#"{"pid":-1}"#).unwrap().pid, None);
        assert_eq!(parse_session_file(br#"{"pid":"4242"}"#).unwrap().pid, None);
    }

    #[test]
    fn desktop_images_need_the_desktop_entrypoint() {
        let msix = r"C:\Users\dev\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\claude-code\2.1.284\claude.exe";
        let roaming = r"C:\Users\dev\AppData\Roaming\Claude\claude-code\2.1.284\claude.exe";
        for image in [msix, roaming] {
            assert_eq!(classify_host(image, Some("claude-desktop"), Path::new(APPDATA)), HostKind::ClaudeDesktop, "{image}");
            assert_eq!(classify_host(image, Some("claude-vscode"), Path::new(APPDATA)), HostKind::Unknown, "{image}");
            assert_eq!(classify_host(image, None, Path::new(APPDATA)), HostKind::Unknown, "{image}");
        }
    }

    #[test]
    fn extension_images_classify_as_cursor_or_vscode() {
        // QueryFullProcessImageNameW reports Cursor's binary with a lowercase drive letter.
        let cursor = r"c:\Users\dev\.cursor\extensions\anthropic.claude-code-2.1.286-win32-x64\resources\native-binary\claude.exe";
        let vscode = r"C:\Users\dev\.vscode\extensions\anthropic.claude-code-2.1.280-win32-x64\resources\native-binary\claude.exe";
        assert_eq!(classify_host(cursor, Some("claude-vscode"), Path::new(APPDATA)), HostKind::Cursor);
        assert_eq!(classify_host(vscode, Some("claude-vscode"), Path::new(APPDATA)), HostKind::VsCode);
    }

    #[test]
    fn other_images_are_unknown() {
        let appdata = Path::new(APPDATA);
        for image in [
            r"C:\Users\dev\.local\bin\claude.exe",
            r"c:\Users\dev\.cursor\extensions\openai.chatgpt-0.146.0-win32-x64\bin\windows-x86_64\codex.exe",
            r"C:\Users\dev\AppData\Roaming\Claude\claude-code\claude.exe",
            "",
        ] {
            assert_eq!(classify_host(image, Some("claude-desktop"), appdata), HostKind::Unknown, "{image}");
        }
        // Without %APPDATA% only the package path identifies Desktop.
        let roaming = r"C:\Users\dev\AppData\Roaming\Claude\claude-code\2.1.284\claude.exe";
        assert_eq!(classify_host(roaming, Some("claude-desktop"), Path::new("")), HostKind::Unknown);
    }
}
