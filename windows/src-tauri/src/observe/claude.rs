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

// ── claude-io thread (T11, spec §5.5) ─────────────────────────────────────────
//
// Everything below lives in a private child module so its imports never collide with the
// parsing part above; the public items are re-exported, so the paths stay
// `observe::claude::{spawn, ClaudeConfig, ClaudeInput}`.

mod io_thread {
    use std::collections::{BTreeSet, HashMap};
    use std::io;
    use std::os::windows::io::OwnedHandle;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{self, Sender};

    use serde_json::Value;

    use super::{
        accept, classify_host, is_session_file_name, parse_session_file, Accept, ClaudeSessionFile,
        SkipReason,
    };
    use crate::log;
    use crate::observe::keys::{claude_key, ProcId, SessionKey};
    use crate::observe::model::{Effect, Health, Signal, Source};
    use crate::observe::paths::{self, ProjectCache};
    use crate::observe::tail::{JsonlTail, WINDOW_BYTES};
    use crate::observe::win::proc::{self, ProcError};
    use crate::observe::win::{DirWatcher, FileAction, WatchEvent, MASK_NAME_WRITE_SIZE};

    /// D9 precedence: Error > Missing > Degraded > Ok.
    fn claude_health(error: Option<String>, missing: bool, unreadable: usize) -> Health {
        if let Some(msg) = error {
            Health::Error(msg)
        } else if missing {
            Health::Missing
        } else if unreadable > 0 {
            Health::Degraded { unreadable: u32::try_from(unreadable).unwrap_or(u32::MAX) }
        } else {
            Health::Ok
        }
    }

    /// D47: distinct session file names of a batch in first-seen order. Actions are ignored.
    fn session_names(changes: &[(FileAction, String)]) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for (_, rel) in changes {
            let name = rel.rsplit(['\\', '/']).next().unwrap_or(rel);
            if is_session_file_name(name) && !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
        names
    }

    /// Last component of a watcher-relative path, lowercase (NTFS compares names ignoring case).
    fn file_name_lower(rel: &str) -> String {
        rel.rsplit(['\\', '/']).next().unwrap_or(rel).to_lowercase()
    }

    fn transcript_name(session_id: &str) -> String {
        format!("{session_id}.jsonl").to_lowercase()
    }

    /// A session id names one file under `projects\*\`: a separator, a drive colon or `..`
    /// would reach outside it (spec §8: nothing but the transcripts is opened).
    fn is_plain_session_id(session_id: &str) -> bool {
        !session_id.contains(['\\', '/', ':']) && !session_id.contains("..")
    }

    /// One JSON value per line; unparsable lines are skipped (D26).
    fn parse_records(lines: Vec<(u64, String)>) -> Vec<(u64, Value)> {
        lines
            .into_iter()
            .filter_map(|(offset, text)| serde_json::from_str(&text).ok().map(|v| (offset, v)))
            .collect()
    }

    pub struct ClaudeConfig {
        /// The .claude dir (`sessions\` and `projects\` under it).
        pub base: PathBuf,
        /// %APPDATA%, for host classification.
        pub appdata: PathBuf,
        /// false in tests: no DirWatcher, inputs are fed manually.
        pub watch: bool,
    }

    #[derive(Debug)]
    pub enum ClaudeInput {
        Sessions(WatchEvent),
        Projects(WatchEvent),
        Effect(Effect),
        Retry,
    }

    /// State of one directory watcher.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Watch {
        /// Not started (watch: false, or before `start`).
        Off,
        Running,
        Missing,
        Failed(String),
    }

    /// A session file that produced a ClaudeFile.
    struct Known {
        key: SessionKey,
        proc: ProcId,
        /// Last file sent (D25: unchanged files are not sent again).
        last: ClaudeSessionFile,
    }

    /// A TrackTranscript binding. `tail == None`: not bound yet. `existed == false` means the
    /// name is expected: the file did not exist and binds at offset 0 once it appears (D48).
    /// `existed == true` means the file existed but could not be opened: it binds with the
    /// window at its EOF once it opens, never at 0 (the history is not replayed).
    struct Track {
        /// `<sessionId>.jsonl`, lowercase.
        name: String,
        tail: Option<JsonlTail>,
        existed: bool,
    }

    struct ClaudeIo {
        cfg: ClaudeConfig,
        signals: Sender<Signal>,
        send_failed: bool,
        registry: proc::Registry,
        watcher_tx: Option<Sender<ClaudeInput>>,
        sessions_watch: Watch,
        projects_watch: Watch,
        /// By session file name.
        known: HashMap<String, Known>,
        /// Session file names currently unreadable (D9): unparsable known file, Incomplete file,
        /// process that denies access.
        unreadable: BTreeSet<String>,
        tracks: HashMap<SessionKey, Track>,
        cache: ProjectCache,
        /// Last health sent; the store starts at Ok.
        health: Health,
    }

    impl ClaudeIo {
        fn new(
            cfg: ClaudeConfig,
            signals: Sender<Signal>,
            registry: proc::Registry,
            watcher_tx: Option<Sender<ClaudeInput>>,
        ) -> Self {
            ClaudeIo {
                cfg,
                signals,
                send_failed: false,
                registry,
                watcher_tx,
                sessions_watch: Watch::Off,
                projects_watch: Watch::Off,
                known: HashMap::new(),
                unreadable: BTreeSet::new(),
                tracks: HashMap::new(),
                cache: ProjectCache::new(),
                health: Health::Ok,
            }
        }

        fn sessions_dir(&self) -> PathBuf {
            self.cfg.base.join("sessions")
        }

        fn projects_dir(&self) -> PathBuf {
            self.cfg.base.join("projects")
        }

        /// Never blocks; the first failed send (store gone) is logged once.
        fn emit(&mut self, signal: Signal) {
            if self.signals.send(signal).is_err() && !self.send_failed {
                self.send_failed = true;
                log::line("claude-io: store is gone, signals are dropped");
            }
        }

        fn start(&mut self) {
            self.ensure_watchers();
            self.rescan();
            self.report_health();
        }

        fn handle(&mut self, input: ClaudeInput) {
            match input {
                ClaudeInput::Sessions(WatchEvent::Changes(changes)) => {
                    for name in session_names(&changes) {
                        self.look_at(&name);
                    }
                }
                ClaudeInput::Projects(WatchEvent::Changes(changes)) => self.projects_changed(&changes),
                ClaudeInput::Sessions(WatchEvent::Overflow)
                | ClaudeInput::Projects(WatchEvent::Overflow) => {
                    log::line("claude-io: watcher overflow, rescanning");
                    self.rescan();
                }
                ClaudeInput::Sessions(WatchEvent::Error(err)) => {
                    log::line(format!("claude-io: sessions watcher stopped: {err}"));
                    self.sessions_watch = Watch::Failed(format!("sessions watcher: {err}"));
                }
                ClaudeInput::Projects(WatchEvent::Error(err)) => {
                    log::line(format!("claude-io: projects watcher stopped: {err}"));
                    self.projects_watch = Watch::Failed(format!("projects watcher: {err}"));
                }
                ClaudeInput::Effect(effect) => self.effect(effect),
                ClaudeInput::Retry => self.retry(),
            }
            self.report_health();
        }

        /// Starts every watcher that is not running. True if at least one started now.
        fn ensure_watchers(&mut self) -> bool {
            let Some(tx) = self.watcher_tx.clone() else { return false };
            let mut started = false;
            if self.sessions_watch != Watch::Running {
                self.sessions_watch =
                    start_watcher(&self.sessions_dir(), false, "sessions", tx.clone(), ClaudeInput::Sessions);
                started |= self.sessions_watch == Watch::Running;
            }
            if self.projects_watch != Watch::Running {
                self.projects_watch =
                    start_watcher(&self.projects_dir(), true, "projects", tx, ClaudeInput::Projects);
                started |= self.projects_watch == Watch::Running;
            }
            started
        }

        /// Visible(true): restart missing/failed watchers; on success behave as Overflow. Without
        /// watchers (tests) there is nothing to restart, so Retry always rescans.
        fn retry(&mut self) {
            if self.watcher_tx.is_none() || self.ensure_watchers() {
                self.rescan();
            }
        }

        /// Sent on change only. Precedence Error > Missing > Degraded > Ok (D9).
        fn report_health(&mut self) {
            let error = [&self.sessions_watch, &self.projects_watch]
                .into_iter()
                .find_map(|w| match w {
                    Watch::Failed(msg) => Some(msg.clone()),
                    _ => None,
                });
            let missing = dir_missing(&self.sessions_watch, &self.sessions_dir())
                || dir_missing(&self.projects_watch, &self.projects_dir());
            let health = claude_health(error, missing, self.unreadable.len());
            if health != self.health {
                log::line(format!("claude-io: health {health:?}"));
                self.health = health.clone();
                self.emit(Signal::SourceHealth { source: Source::Claude, health });
            }
        }
    }

    impl ClaudeIo {
        /// D47: the file system decides what a name means, never the action code.
        fn look_at(&mut self, name: &str) {
            let path = self.sessions_dir().join(name);
            match std::fs::read(&path) {
                Ok(bytes) => self.process(name, &bytes),
                Err(err) if err.kind() == io::ErrorKind::NotFound => self.gone(name),
                Err(err) => self.unreadable_file(name, &format!("read failed: {:?}", err.kind())),
            }
        }

        fn gone(&mut self, name: &str) {
            self.unreadable.remove(name);
            if let Some(known) = self.known.remove(name) {
                log::line(format!("claude {name} removed: {}", known.key));
                self.emit(Signal::ClaudeGone { key: known.key });
            }
        }

        /// A known file keeps its last state and counts as unreadable; a new file is skipped
        /// (spec §5.5 step 1).
        fn unreadable_file(&mut self, name: &str, reason: &str) {
            log::line(format!("claude {name} unreadable: {reason}"));
            if self.known.contains_key(name) {
                self.unreadable.insert(name.to_string());
            }
        }

        fn process(&mut self, name: &str, bytes: &[u8]) {
            let file = match parse_session_file(bytes) {
                Ok(file) => file,
                Err(reason) => return self.unreadable_file(name, &reason),
            };
            let (pid, proc_start) = match accept(&file) {
                Accept::Track { pid, proc_start } => (pid, proc_start),
                Accept::Skip(SkipReason::Incomplete) => {
                    log::line(format!("claude {name} skipped: incomplete"));
                    self.unreadable.insert(name.to_string());
                    return;
                }
                Accept::Skip(reason) => {
                    // Spare / parked / foreign domain: not a session to show (spec §5.5 step 2).
                    log::line(format!("claude {name} skipped: {reason:?}"));
                    self.unreadable.remove(name);
                    if let Some(known) = self.known.remove(name) {
                        self.emit(Signal::ClaudeGone { key: known.key });
                    }
                    return;
                }
            };
            self.unreadable.remove(name);
            if let Some(known) = self.known.get(name) {
                let same = known.proc.pid == pid && known.proc.created_ft == proc_start;
                if !same {
                    // procStart changed for this pid: the old session is gone (spec §5.5 step 3).
                    let known = self.known.remove(name).expect("entry checked above");
                    self.emit(Signal::ClaudeGone { key: known.key });
                } else if !self.registry.is_watched(pid, proc_start) && !self.rewatch(name, pid, proc_start) {
                    // The process exited (its waiter fired; ProcExit removes the session in the
                    // model): the file is proved again below, and that proof fails.
                    self.known.remove(name);
                }
            }
            if let Some(known) = self.known.get_mut(name) {
                if known.last == file {
                    return; // D25
                }
                let status_changed = known.last.status != file.status
                    || known.last.status_updated_at != file.status_updated_at;
                known.last = file.clone();
                let (key, proc) = (known.key.clone(), known.proc.clone());
                self.send_file(key, proc, file, status_changed);
                return;
            }
            self.prove_new(name, pid, proc_start, file);
        }

        /// Process proof of a new session file (spec §5.5 steps 3–4).
        fn prove_new(&mut self, name: &str, pid: u32, proc_start: u64, file: ClaudeSessionFile) {
            match proc::identify(pid) {
                Ok((id, handle)) => {
                    if id.created_ft != proc_start {
                        // A file left behind by a dead process whose pid was reused.
                        log::line(format!("claude {name} skipped: procStart does not match pid {pid}"));
                        return;
                    }
                    let key = claude_key(id.pid, id.created_ft);
                    log::line(format!("claude {name} tracked as {key}"));
                    self.known.insert(
                        name.to_string(),
                        Known { key: key.clone(), proc: id.clone(), last: file.clone() },
                    );
                    self.send_file(key, id.clone(), file, false);
                    // Registered only after ClaudeFile went out, so ProcExit always follows it in
                    // the FIFO channel.
                    self.watch_exit(&id, handle);
                }
                Err(ProcError::NotFound) | Err(ProcError::Exited) => {
                    log::line(format!("claude {name} skipped: process {pid} is not running"));
                }
                Err(ProcError::AccessDenied) => {
                    log::line(format!("claude {name}: process {pid} denies access"));
                    self.unreadable.insert(name.to_string());
                }
                Err(ProcError::Os(err)) => {
                    log::line(format!("claude {name}: process {pid} not identified: {err}"));
                    self.unreadable.insert(name.to_string());
                }
            }
        }

        /// A known process missing from the Registry: its exit waiter fired, or the waiter
        /// thread did not start or its wait failed (the Registry drops the entry then). Still
        /// the same process → watched again and the session stays (true); gone → false.
        fn rewatch(&mut self, name: &str, pid: u32, created_ft: u64) -> bool {
            match proc::identify(pid) {
                Ok((id, handle)) if id.created_ft == created_ft => {
                    log::line(format!("claude {name}: process {pid} watched again"));
                    self.watch_exit(&id, handle);
                    true
                }
                _ => false,
            }
        }

        /// ProcExit when the process ends. `false` from the Registry needs nothing more here:
        /// the process is already watched, or its waiter thread could not be started, and then
        /// the Registry has removed the entry and logged it, so the next change of the file
        /// watches it again (`rewatch`).
        fn watch_exit(&self, id: &ProcId, handle: OwnedHandle) {
            let signals = self.signals.clone();
            let _ = self.registry.watch(id, handle, move |pid, created_ft| {
                if signals.send(Signal::ProcExit { pid, created_ft }).is_err() {
                    log::line(format!("proc-exit {pid}: store is gone"));
                }
            });
        }

        /// ClaudeFile, preceded by the transcript records it depends on (spec §5.5 step 6): an
        /// unbound transcript is searched again first (D48), and a status change drains the
        /// bound one and reports its EOF.
        fn send_file(
            &mut self,
            key: SessionKey,
            proc: ProcId,
            file: ClaudeSessionFile,
            status_changed: bool,
        ) {
            self.search_unbound(&key);
            let transcript_eof = if status_changed { self.drain(&key) } else { None };
            let host = classify_host(&proc.image, file.entrypoint.as_deref(), &self.cfg.appdata);
            let project = self.cache.get(file.cwd.as_deref().unwrap_or_default(), paths::git_probe);
            self.emit(Signal::ClaudeFile { key, proc, host, project, file, transcript_eof });
        }

        /// Start, Overflow, successful Retry: every session file plus every name that vanished
        /// unseen (a known file gives ClaudeGone, an unreadable one leaves the D9 count), then
        /// every unbound transcript name, then every bound tail.
        fn rescan(&mut self) {
            let mut names = list_session_files(&self.sessions_dir());
            let mut vanished: Vec<String> = self
                .known
                .keys()
                .chain(self.unreadable.iter())
                .filter(|name| !names.contains(*name))
                .cloned()
                .collect();
            vanished.sort();
            vanished.dedup();
            names.extend(vanished);
            for name in names {
                self.look_at(&name);
            }
            let mut keys: Vec<SessionKey> = self.tracks.keys().cloned().collect();
            keys.sort();
            for key in &keys {
                self.search_unbound(key);
            }
            for key in &keys {
                self.drain(key);
            }
        }
    }

    impl ClaudeIo {
        /// Reads a bound transcript; new records go out as ClaudeTranscript{rebind_at: None}.
        /// Returns the tail's EOF, or None when no transcript is bound for `key`.
        fn drain(&mut self, key: &str) -> Option<u64> {
            let (records, eof) = {
                let tail = self.tracks.get_mut(key)?.tail.as_mut()?;
                read_records(tail)
            };
            if !records.is_empty() {
                self.emit(Signal::ClaudeTranscript { key: key.to_string(), rebind_at: None, records, eof });
            }
            Some(eof)
        }

        /// D48: a tracked session whose transcript is not bound looks for it again.
        fn search_unbound(&mut self, key: &str) {
            let name = match self.tracks.get(key) {
                Some(track) if track.tail.is_none() => track.name.clone(),
                _ => return,
            };
            if let Some(path) = find_transcript(&self.projects_dir(), &name) {
                self.bind(key, path);
            }
        }

        /// Binds an unbound name: from offset 0 when its file did not exist at TrackTranscript
        /// (D48), else with the window at its EOF.
        fn bind(&mut self, key: &str, path: PathBuf) {
            match self.tracks.get(key) {
                Some(Track { tail: None, existed: false, .. }) => self.bind_from_start(key, path),
                Some(Track { tail: None, existed: true, .. }) => self.bind_at_eof(key, path),
                _ => {}
            }
        }

        fn bind_from_start(&mut self, key: &str, path: PathBuf) {
            let mut tail = JsonlTail::new(path);
            let (records, eof) = read_records(&mut tail);
            match self.tracks.get_mut(key) {
                Some(track) => track.tail = Some(tail),
                None => return,
            }
            log::line(format!("claude transcript bound for {key} at 0"));
            self.emit(Signal::ClaudeTranscript { key: key.to_string(), rebind_at: Some(0), records, eof });
        }

        /// An existing transcript: the last WINDOW_BYTES are read and the binding starts at its
        /// EOF. A file that cannot be opened stays unbound, never bound at offset 0 (that would
        /// replay its whole history): the next search, notification or rescan tries again.
        fn bind_at_eof(&mut self, key: &str, path: PathBuf) {
            let mut tail = JsonlTail::new(path);
            if let Err(err) = tail.start_with_window(WINDOW_BYTES) {
                log::line(format!("claude transcript window failed for {key}: {:?}", err.kind()));
                return;
            }
            let (records, eof) = read_records(&mut tail);
            match self.tracks.get_mut(key) {
                Some(track) => track.tail = Some(tail),
                None => return,
            }
            log::line(format!("claude transcript bound for {key} at {eof}"));
            self.emit(Signal::ClaudeTranscript { key: key.to_string(), rebind_at: Some(eof), records, eof });
        }
    }

    fn start_watcher(
        dir: &Path,
        recursive: bool,
        label: &str,
        tx: Sender<ClaudeInput>,
        wrap: fn(WatchEvent) -> ClaudeInput,
    ) -> Watch {
        match DirWatcher::spawn(dir, recursive, MASK_NAME_WRITE_SIZE, move |event| tx.send(wrap(event)).is_ok()) {
            Ok(()) => Watch::Running,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Watch::Missing,
            Err(err) => {
                log::line(format!("claude-io: {label} watcher failed: {err}"));
                Watch::Failed(format!("{label} watcher: {err}"))
            }
        }
    }

    fn dir_missing(watch: &Watch, dir: &Path) -> bool {
        match watch {
            Watch::Missing => true,
            Watch::Off => !dir.is_dir(),
            Watch::Running | Watch::Failed(_) => false,
        }
    }

    fn list_session_files(dir: &Path) -> Vec<String> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) => {
                if err.kind() != io::ErrorKind::NotFound {
                    log::line(format!("claude-io: sessions dir unreadable: {:?}", err.kind()));
                }
                return Vec::new();
            }
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| is_session_file_name(name))
            .collect();
        names.sort();
        names
    }

    /// `projects\*\<name>`: the slug rule is not documented, the session UUID is unique.
    fn find_transcript(projects: &Path, name: &str) -> Option<PathBuf> {
        let entries = std::fs::read_dir(projects).ok()?;
        entries
            .flatten()
            .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|entry| entry.path().join(name))
            .find(|candidate| candidate.is_file())
    }

    fn read_records(tail: &mut JsonlTail) -> (Vec<(u64, Value)>, u64) {
        match tail.read_new() {
            Ok((lines, eof)) => (parse_records(lines), eof),
            Err(err) => {
                log::line(format!("claude transcript read failed: {:?}", err.kind()));
                (Vec::new(), tail.eof())
            }
        }
    }

    impl ClaudeIo {
        fn effect(&mut self, effect: Effect) {
            match effect {
                Effect::TrackTranscript { key, session_id } => self.track(key, &session_id),
                Effect::UntrackTranscript { key } => {
                    if self.tracks.remove(&key).is_some() {
                        log::line(format!("claude transcript untracked for {key}"));
                    }
                }
                other => log::line(format!("claude-io: ignored effect {other:?}")),
            }
        }

        /// An existing transcript binds with the window at its EOF; a missing one becomes an
        /// expected name that binds at 0 once it appears (spec §5.5 "Транскрипт", D48). A
        /// session id that is not a plain file name is never tracked.
        fn track(&mut self, key: SessionKey, session_id: &str) {
            if !is_plain_session_id(session_id) {
                log::line(format!("claude transcript not tracked for {key}: the session id is not a file name"));
                return;
            }
            let name = transcript_name(session_id);
            let path = find_transcript(&self.projects_dir(), &name);
            self.tracks.insert(key.clone(), Track { name, tail: None, existed: path.is_some() });
            match path {
                Some(path) => self.bind_at_eof(&key, path),
                None => log::line(format!("claude transcript for {key} not created yet")),
            }
        }

        /// Files are matched by name only (spec §5.5): an unbound name binds on
        /// Added/Modified/RenamedNew (at 0 when it was expected, D48); a bound name is read on
        /// Modified.
        fn projects_changed(&mut self, changes: &[(FileAction, String)]) {
            for (action, rel) in changes {
                let name = file_name_lower(rel);
                if !name.ends_with(".jsonl") {
                    continue;
                }
                let discovered =
                    matches!(action, FileAction::Added | FileAction::Modified | FileAction::RenamedNew);
                let mut keys: Vec<SessionKey> = self
                    .tracks
                    .iter()
                    .filter(|(_, track)| track.name == name)
                    .map(|(key, _)| key.clone())
                    .collect();
                keys.sort();
                for key in keys {
                    let bound = self.tracks.get(&key).is_some_and(|track| track.tail.is_some());
                    if bound && *action == FileAction::Modified {
                        self.drain(&key);
                    } else if !bound && discovered {
                        let path = self.projects_dir().join(rel);
                        self.bind(&key, path);
                    }
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::testutil::TempDir;
        use crate::observe::model::{Alert, AttentionKind, BotState, Store};
        use std::sync::mpsc::Receiver;
        use std::time::Duration;

        fn offsets(records: &[(u64, Value)]) -> Vec<u64> {
            records.iter().map(|(o, _)| *o).collect()
        }

        #[test]
        fn session_names_are_distinct_session_files_in_first_seen_order() {
            let changes = vec![
                (FileAction::Removed, "12.json".to_string()),
                (FileAction::RenamedNew, "12.json".to_string()),
                (FileAction::Modified, "30584.abc.key".to_string()),
                (FileAction::Added, "7.json".to_string()),
                (FileAction::Modified, "12.json.tmp".to_string()),
            ];
            assert_eq!(session_names(&changes), ["12.json", "7.json"]);
        }

        #[test]
        fn transcript_names_compare_lowercase() {
            assert_eq!(file_name_lower(r"C--Work-App\ABC-1.JSONL"), "abc-1.jsonl");
            assert_eq!(transcript_name("ABC-1"), "abc-1.jsonl");
        }

        #[test]
        fn session_ids_with_path_parts_are_rejected() {
            assert!(is_plain_session_id("0f6c2a51-0000-4000-8000-00000000c1a0"));
            for id in [r"..\..\escape", "a/b", r"a\b", "C:x", "..", "a..b"] {
                assert!(!is_plain_session_id(id), "{id}");
            }
        }

        #[test]
        fn unparsable_lines_are_skipped() {
            let lines = vec![(0, r#"{"a":1}"#.to_string()), (8, "{broken".to_string()), (15, "[1]".to_string())];
            assert_eq!(offsets(&parse_records(lines)), [0, 15]);
        }

        #[test]
        fn health_precedence_is_error_missing_degraded_ok() {
            assert_eq!(claude_health(Some("x".into()), true, 2), Health::Error("x".into()));
            assert_eq!(claude_health(None, true, 2), Health::Missing);
            assert_eq!(claude_health(None, false, 2), Health::Degraded { unreadable: 2 });
            assert_eq!(claude_health(None, false, 0), Health::Ok);
        }

        const SID: &str = "0f6c2a51-0000-4000-8000-00000000c1a0";
        const SLUG: &str = "C--work-app";

        fn me() -> ProcId {
            proc::identify(std::process::id()).expect("the test process identifies itself").0
        }

        fn session_json(pid: u32, proc_start: u64, cwd: &str, status: &str, at: i64) -> String {
            serde_json::json!({
                "pid": pid, "sessionId": SID, "cwd": cwd, "procStart": proc_start.to_string(),
                "version": "2.1.286", "kind": "interactive", "entrypoint": "claude-vscode",
                "status": status, "statusUpdatedAt": at, "updatedAt": at
            })
            .to_string()
        }

        struct Rig {
            dir: TempDir,
            io: ClaudeIo,
            rx: Receiver<Signal>,
            me: ProcId,
        }

        fn rig(tag: &str) -> Rig {
            let dir = TempDir::new(tag);
            std::fs::create_dir_all(dir.path.join("sessions")).unwrap();
            std::fs::create_dir_all(dir.path.join("projects").join(SLUG)).unwrap();
            // The project root must not depend on a `.git` above %TEMP% (git_probe walks up).
            std::fs::create_dir_all(dir.path.join("work").join(".git")).unwrap();
            let (tx, rx) = mpsc::channel();
            let cfg = ClaudeConfig { base: dir.path.clone(), appdata: dir.path.join("appdata"), watch: false };
            let io = ClaudeIo::new(cfg, tx, proc::Registry::new(), None);
            Rig { dir, io, rx, me: me() }
        }

        impl Rig {
            fn name(&self) -> String {
                format!("{}.json", self.me.pid)
            }
            fn key(&self) -> SessionKey {
                claude_key(self.me.pid, self.me.created_ft)
            }
            fn cwd(&self) -> String {
                self.dir.path.join("work").to_string_lossy().into_owned()
            }
            fn write_session(&self, status: &str, at: i64) {
                let text = session_json(self.me.pid, self.me.created_ft, &self.cwd(), status, at);
                std::fs::write(self.dir.path.join("sessions").join(self.name()), text).unwrap();
            }
            fn signals(&self) -> Vec<Signal> {
                self.rx.try_iter().collect()
            }
            fn session_changed(&mut self, actions: &[FileAction]) {
                let name = self.name();
                let changes = actions.iter().map(|a| (*a, name.clone())).collect();
                self.io.handle(ClaudeInput::Sessions(WatchEvent::Changes(changes)));
            }
            /// Start with the live session file written and the first ClaudeFile consumed.
            fn started(tag: &str) -> Rig {
                let mut rig = rig(tag);
                rig.write_session("idle", 100);
                rig.io.start();
                let first = rig.signals();
                assert_eq!(names(&first), ["ClaudeFile"], "{first:?}");
                rig
            }
        }

        fn names(signals: &[Signal]) -> Vec<&'static str> {
            signals
                .iter()
                .map(|s| match s {
                    Signal::ClaudeFile { .. } => "ClaudeFile",
                    Signal::ClaudeGone { .. } => "ClaudeGone",
                    Signal::ClaudeTranscript { .. } => "ClaudeTranscript",
                    Signal::SourceHealth { .. } => "SourceHealth",
                    Signal::ProcExit { .. } => "ProcExit",
                    _ => "other",
                })
                .collect()
        }

        #[test]
        fn start_sends_live_sessions_and_skips_stale_spare_and_incomplete_files() {
            let mut rig = rig("cio-start");
            rig.write_session("busy", 100);
            let sessions = rig.dir.path.join("sessions");
            let cwd = rig.cwd();
            // Same live pid but another procStart: a stale file of a reused pid.
            std::fs::write(sessions.join("999999.json"), session_json(rig.me.pid, rig.me.created_ft + 1, &cwd, "busy", 1)).unwrap();
            std::fs::write(sessions.join("4.json"), r#"{"pid":4,"spare":true}"#).unwrap();
            std::fs::write(sessions.join("5.json"), r#"{"pid":5,"sessionId":"x","cwd":"C:\\w"}"#).unwrap();
            std::fs::write(sessions.join("30584.abc.key"), "secret").unwrap();
            rig.io.start();
            let signals = rig.signals();
            assert_eq!(names(&signals), ["ClaudeFile", "SourceHealth"], "{signals:?}");
            match &signals[0] {
                Signal::ClaudeFile { key, proc, project, file, transcript_eof, .. } => {
                    assert_eq!(key, &rig.key());
                    assert_eq!(proc, &rig.me);
                    assert_eq!(project.root, paths::normalize(&cwd).display);
                    assert_eq!(file.status.as_deref(), Some("busy"));
                    assert_eq!(*transcript_eof, None);
                }
                other => panic!("{other:?}"),
            }
            // 5.json has no procStart: Incomplete counts as unreadable (D9).
            assert_eq!(signals[1], Signal::SourceHealth { source: Source::Claude, health: Health::Degraded { unreadable: 1 } });
        }

        #[test]
        fn a_rename_over_write_is_an_update_not_a_removal() {
            let mut rig = Rig::started("cio-rename");
            rig.write_session("waiting", 200);
            rig.session_changed(&[FileAction::Removed, FileAction::RenamedNew]);
            let signals = rig.signals();
            assert_eq!(names(&signals), ["ClaudeFile"], "{signals:?}");
            rig.write_session("busy", 300);
            rig.session_changed(&[FileAction::RenamedOld, FileAction::RenamedNew]);
            assert_eq!(names(&rig.signals()), ["ClaudeFile"]);
        }

        #[test]
        fn a_deleted_file_gives_claude_gone() {
            let mut rig = Rig::started("cio-gone");
            std::fs::remove_file(rig.dir.path.join("sessions").join(rig.name())).unwrap();
            rig.session_changed(&[FileAction::Removed]);
            assert_eq!(rig.signals(), [Signal::ClaudeGone { key: rig.key() }]);
        }

        #[test]
        fn an_unchanged_file_is_not_sent_again() {
            let mut rig = Rig::started("cio-same");
            rig.session_changed(&[FileAction::Modified]);
            assert_eq!(rig.signals(), []);
        }

        #[test]
        fn garbage_in_a_known_file_keeps_the_session_and_degrades_health() {
            let mut rig = Rig::started("cio-garbage");
            std::fs::write(rig.dir.path.join("sessions").join(rig.name()), "{ not json").unwrap();
            rig.session_changed(&[FileAction::Modified]);
            assert_eq!(
                rig.signals(),
                [Signal::SourceHealth { source: Source::Claude, health: Health::Degraded { unreadable: 1 } }]
            );
            rig.write_session("busy", 200);
            rig.session_changed(&[FileAction::Modified]);
            let signals = rig.signals();
            assert_eq!(names(&signals), ["ClaudeFile", "SourceHealth"], "{signals:?}");
            assert_eq!(signals[1], Signal::SourceHealth { source: Source::Claude, health: Health::Ok });
        }

        #[test]
        fn a_new_proc_start_for_a_known_pid_gives_claude_gone() {
            let mut rig = Rig::started("cio-procstart");
            let text = session_json(rig.me.pid, rig.me.created_ft + 1, &rig.cwd(), "busy", 200);
            std::fs::write(rig.dir.path.join("sessions").join(rig.name()), text).unwrap();
            rig.session_changed(&[FileAction::Modified]);
            // The new procStart does not match the live process, so nothing replaces it.
            assert_eq!(rig.signals(), [Signal::ClaudeGone { key: rig.key() }]);
        }

        #[test]
        fn a_known_file_that_becomes_spare_gives_claude_gone() {
            let mut rig = Rig::started("cio-spare");
            let mut file: Value = serde_json::from_str(&session_json(rig.me.pid, rig.me.created_ft, &rig.cwd(), "idle", 200)).unwrap();
            file["spare"] = true.into();
            std::fs::write(rig.dir.path.join("sessions").join(rig.name()), file.to_string()).unwrap();
            rig.session_changed(&[FileAction::Modified]);
            assert_eq!(rig.signals(), [Signal::ClaudeGone { key: rig.key() }]);
        }

        #[test]
        fn a_live_process_missing_from_the_registry_keeps_its_session() {
            // The Registry drops a process whose waiter thread did not start or whose wait failed.
            let mut rig = Rig::started("cio-rewatch");
            rig.io.registry = proc::Registry::new();
            rig.session_changed(&[FileAction::Modified]);
            assert_eq!(rig.signals(), [], "still known: the unchanged file is not sent again (D25)");
            assert!(rig.io.registry.is_watched(rig.me.pid, rig.me.created_ft), "watched again");
        }

        #[test]
        fn an_overflow_forgets_unreadable_files_that_are_gone() {
            let mut rig = rig("cio-unreadable-gone");
            let incomplete = rig.dir.path.join("sessions").join("5.json");
            std::fs::write(&incomplete, r#"{"pid":5,"sessionId":"x","cwd":"C:\\w"}"#).unwrap();
            rig.io.start();
            assert_eq!(
                rig.signals(),
                [Signal::SourceHealth { source: Source::Claude, health: Health::Degraded { unreadable: 1 } }]
            );
            // Deleted while notifications overflowed: only the rescan sees it (D9).
            std::fs::remove_file(&incomplete).unwrap();
            rig.io.handle(ClaudeInput::Sessions(WatchEvent::Overflow));
            assert_eq!(rig.signals(), [Signal::SourceHealth { source: Source::Claude, health: Health::Ok }]);
        }

        #[test]
        fn a_watcher_error_reports_source_health_error() {
            let mut rig = rig("cio-error");
            rig.io.handle(ClaudeInput::Projects(WatchEvent::Error(io::Error::other("gone"))));
            assert_eq!(
                rig.signals(),
                [Signal::SourceHealth { source: Source::Claude, health: Health::Error("projects watcher: gone".into()) }]
            );
        }

        #[test]
        fn missing_dirs_report_missing() {
            let dir = TempDir::new("cio-missing");
            let (tx, rx) = mpsc::channel();
            let cfg = ClaudeConfig { base: dir.path.join("absent"), appdata: dir.path.clone(), watch: false };
            let mut io = ClaudeIo::new(cfg, tx, proc::Registry::new(), None);
            io.start();
            assert_eq!(
                rx.try_iter().collect::<Vec<_>>(),
                [Signal::SourceHealth { source: Source::Claude, health: Health::Missing }]
            );
        }

        #[test]
        fn retry_without_watchers_rescans() {
            let mut rig = rig("cio-retry");
            rig.io.start();
            assert_eq!(rig.signals(), []);
            rig.write_session("busy", 100);
            rig.io.handle(ClaudeInput::Retry);
            assert_eq!(names(&rig.signals()), ["ClaudeFile"]);
        }

        #[test]
        fn an_exited_process_gets_proc_exit_after_its_file_and_is_not_sent_again() {
            let mut rig = rig("cio-exit");
            let mut child = std::process::Command::new("cmd")
                .args(["/c", "ping -n 3 127.0.0.1 >nul"])
                .spawn()
                .expect("cmd starts");
            let id = proc::identify(child.id()).expect("child is running").0;
            let name = format!("{}.json", id.pid);
            let text = session_json(id.pid, id.created_ft, &rig.cwd(), "busy", 100);
            std::fs::write(rig.dir.path.join("sessions").join(&name), text).unwrap();
            rig.io.start();
            assert_eq!(names(&rig.signals()), ["ClaudeFile"]);
            child.wait().unwrap();
            let exit = rig.rx.recv_timeout(Duration::from_secs(10)).expect("ProcExit");
            assert_eq!(exit, Signal::ProcExit { pid: id.pid, created_ft: id.created_ft });
            // The file is still there (a crash leaves it): it must not bring the session back.
            let text = session_json(id.pid, id.created_ft, &rig.cwd(), "idle", 200);
            std::fs::write(rig.dir.path.join("sessions").join(&name), text).unwrap();
            rig.io.handle(ClaudeInput::Sessions(WatchEvent::Changes(vec![(FileAction::Modified, name)])));
            assert_eq!(rig.signals(), []);
        }

        const L1: &str = r#"{"type":"user","message":{"content":"one"}}"#;
        const L2: &str = r#"{"type":"assistant","message":{"content":[]}}"#;
        const L3: &str = r#"{"type":"user","message":{"content":"three"}}"#;

        impl Rig {
            fn transcript(&self) -> PathBuf {
                self.dir.path.join("projects").join(SLUG).join(format!("{SID}.jsonl"))
            }
            fn transcript_rel(&self) -> String {
                format!("{SLUG}\\{SID}.jsonl")
            }
            fn append(&self, lines: &[&str]) {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.transcript()).unwrap();
                for line in lines {
                    writeln!(f, "{line}").unwrap();
                }
            }
            fn len(&self) -> u64 {
                std::fs::metadata(self.transcript()).unwrap().len()
            }
            fn track(&mut self) {
                let key = self.key();
                self.io.handle(ClaudeInput::Effect(Effect::TrackTranscript { key, session_id: SID.into() }));
            }
        }

        #[test]
        fn track_binds_an_existing_transcript_at_its_eof_then_reads_appends() {
            let mut rig = Rig::started("cio-track");
            rig.append(&[L1, L2]);
            let end = rig.len();
            rig.track();
            match rig.signals().as_slice() {
                [Signal::ClaudeTranscript { key, rebind_at, records, eof }] => {
                    assert_eq!(key, &rig.key());
                    assert_eq!(*rebind_at, Some(end));
                    assert_eq!(*eof, end);
                    assert_eq!(offsets(records), [0, L1.len() as u64 + 1]);
                }
                other => panic!("{other:?}"),
            }
            rig.append(&[L3]);
            let rel = rig.transcript_rel();
            // A bound name is read on Modified only (contract §2.16).
            rig.io.handle(ClaudeInput::Projects(WatchEvent::Changes(vec![(FileAction::Added, rel.clone())])));
            assert_eq!(rig.signals(), []);
            rig.io.handle(ClaudeInput::Projects(WatchEvent::Changes(vec![(FileAction::Modified, rel)])));
            match rig.signals().as_slice() {
                [Signal::ClaudeTranscript { rebind_at: None, records, eof, .. }] => {
                    assert_eq!(offsets(records), [end]);
                    assert_eq!(*eof, rig.len());
                }
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn an_expected_transcript_binds_at_zero_when_its_file_is_notified() {
            let mut rig = Rig::started("cio-expected");
            rig.track();
            assert_eq!(rig.signals(), []);
            rig.append(&[L1, L2]);
            let rel = rig.transcript_rel();
            rig.io.handle(ClaudeInput::Projects(WatchEvent::Changes(vec![(FileAction::Added, rel)])));
            match rig.signals().as_slice() {
                [Signal::ClaudeTranscript { rebind_at: Some(0), records, .. }] => {
                    assert_eq!(offsets(records), [0, L1.len() as u64 + 1]);
                }
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn an_expected_transcript_found_by_the_research_on_claude_file_binds_at_zero() {
            // Required regression (D48).
            let mut rig = Rig::started("cio-research");
            rig.track();
            assert_eq!(rig.signals(), []);
            rig.append(&[L1, L2]);
            rig.write_session("busy", 200);
            rig.session_changed(&[FileAction::Modified]);
            let signals = rig.signals();
            assert_eq!(names(&signals), ["ClaudeTranscript", "ClaudeFile"], "{signals:?}");
            match &signals[0] {
                Signal::ClaudeTranscript { rebind_at, records, .. } => {
                    assert_eq!(*rebind_at, Some(0));
                    assert_eq!(records.first().map(|(o, _)| *o), Some(0));
                }
                other => panic!("{other:?}"),
            }
            match &signals[1] {
                Signal::ClaudeFile { transcript_eof, .. } => assert_eq!(*transcript_eof, Some(rig.len())),
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn an_expected_transcript_found_by_an_overflow_rescan_binds_at_zero() {
            // The third way an expected name is found (D48): the rescan after Overflow/Retry.
            let mut rig = Rig::started("cio-rescan");
            rig.track();
            assert_eq!(rig.signals(), []);
            rig.append(&[L1, L2]);
            rig.io.handle(ClaudeInput::Projects(WatchEvent::Overflow));
            match rig.signals().as_slice() {
                [Signal::ClaudeTranscript { rebind_at: Some(0), records, .. }] => {
                    assert_eq!(offsets(records), [0, L1.len() as u64 + 1]);
                }
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn a_status_change_drains_the_transcript_before_the_file() {
            let mut rig = Rig::started("cio-order");
            rig.append(&[L1]);
            rig.track();
            rig.signals();
            let before = rig.len();
            rig.append(&[L2]);
            rig.write_session("busy", 200);
            rig.session_changed(&[FileAction::Modified]);
            let signals = rig.signals();
            assert_eq!(names(&signals), ["ClaudeTranscript", "ClaudeFile"], "{signals:?}");
            match &signals[0] {
                Signal::ClaudeTranscript { rebind_at: None, records, .. } => assert_eq!(offsets(records), [before]),
                other => panic!("{other:?}"),
            }
            match &signals[1] {
                Signal::ClaudeFile { transcript_eof, .. } => assert_eq!(*transcript_eof, Some(rig.len())),
                other => panic!("{other:?}"),
            }
            // Same status and statusUpdatedAt, another field changed: no drain, no EOF.
            let text = session_json(rig.me.pid, rig.me.created_ft, &rig.cwd(), "busy", 200).replace("2.1.286", "2.1.287");
            std::fs::write(rig.dir.path.join("sessions").join(rig.name()), text).unwrap();
            rig.append(&[L3]);
            rig.session_changed(&[FileAction::Modified]);
            match rig.signals().as_slice() {
                [Signal::ClaudeFile { transcript_eof: None, .. }] => {}
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn untrack_stops_reading_the_transcript() {
            let mut rig = Rig::started("cio-untrack");
            rig.append(&[L1]);
            rig.track();
            rig.signals();
            let key = rig.key();
            rig.io.handle(ClaudeInput::Effect(Effect::UntrackTranscript { key }));
            rig.append(&[L2]);
            let rel = rig.transcript_rel();
            rig.io.handle(ClaudeInput::Projects(WatchEvent::Changes(vec![(FileAction::Modified, rel)])));
            assert_eq!(rig.signals(), []);
        }

        #[test]
        fn overflow_rescans_sessions_and_tails() {
            let mut rig = Rig::started("cio-overflow");
            rig.append(&[L1]);
            rig.track();
            rig.signals();
            std::fs::remove_file(rig.dir.path.join("sessions").join(rig.name())).unwrap();
            rig.append(&[L2]);
            rig.io.handle(ClaudeInput::Projects(WatchEvent::Overflow));
            let signals = rig.signals();
            assert_eq!(names(&signals), ["ClaudeGone", "ClaudeTranscript"], "{signals:?}");
        }

        #[test]
        fn a_transcript_that_cannot_be_opened_is_bound_later_at_its_eof() {
            use std::os::windows::fs::OpenOptionsExt;
            let mut rig = Rig::started("cio-locked");
            rig.append(&[L1, L2]);
            // Held open without sharing: the tail's own open fails (sharing violation).
            let lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(rig.transcript()).unwrap();
            rig.track();
            assert_eq!(rig.signals(), [], "not bound, and never at offset 0");
            drop(lock);
            rig.write_session("busy", 200);
            rig.session_changed(&[FileAction::Modified]);
            let signals = rig.signals();
            assert_eq!(names(&signals), ["ClaudeTranscript", "ClaudeFile"], "{signals:?}");
            match &signals[0] {
                Signal::ClaudeTranscript { rebind_at, eof, .. } => {
                    assert_eq!(*rebind_at, Some(rig.len()));
                    assert_eq!(*eof, rig.len());
                }
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn a_session_id_with_path_parts_is_not_tracked() {
            let mut rig = Rig::started("cio-escape");
            // `projects\<slug>\..\escape.jsonl` is `projects\escape.jsonl`, outside every project.
            std::fs::write(rig.dir.path.join("projects").join("escape.jsonl"), format!("{L1}\n")).unwrap();
            let key = rig.key();
            rig.io.handle(ClaudeInput::Effect(Effect::TrackTranscript { key, session_id: r"..\escape".into() }));
            assert_eq!(rig.signals(), []);
            rig.write_session("busy", 200);
            rig.session_changed(&[FileAction::Modified]);
            match rig.signals().as_slice() {
                [Signal::ClaudeFile { transcript_eof: None, .. }] => {}
                other => panic!("{other:?}"),
            }
        }

        // ── Review focus: claude-io and the model together (what the store thread does, T13) ──

        const USER_OLD: &str = r#"{"type":"user","isSidechain":false,"message":{"role":"user","content":"old question"}}"#;
        const REPLY_OLD: &str = r#"{"type":"assistant","isSidechain":false,"message":{"role":"assistant","content":[{"type":"text","text":"Old answer."}],"stop_reason":"end_turn"}}"#;
        const REPLY_STALE: &str = r#"{"type":"assistant","isSidechain":false,"message":{"role":"assistant","content":[{"type":"text","text":"Stale answer."}],"stop_reason":"end_turn"}}"#;
        const USER_QUICK: &str = r#"{"type":"user","isSidechain":false,"message":{"role":"user","content":"quick one"}}"#;
        const REPLY_QUICK: &str = r#"{"type":"assistant","isSidechain":false,"message":{"role":"assistant","content":[{"type":"text","text":"Quick answer."}],"stop_reason":"end_turn"}}"#;
        const TITLE: &str = r#"{"type":"custom-title","customTitle":"Release prep"}"#;
        const TOOL_PUSH: &str = r#"{"type":"assistant","isSidechain":false,"message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git push"}}],"stop_reason":"tool_use"}}"#;
        const CLEARED_SID: &str = "0f6c2a51-0000-4000-8000-00000000c1a1";

        /// claude-io's signals go to a real Store and the Store's effects back to claude-io until
        /// both are quiet (the store thread's job, T13). Returns every alert raised.
        fn pump(rig: &mut Rig, store: &mut Store) -> Vec<Alert> {
            let mut alerts = Vec::new();
            loop {
                let signals = rig.signals();
                if signals.is_empty() {
                    return alerts;
                }
                for signal in signals {
                    let applied = store.apply(signal, 9_000);
                    alerts.extend(applied.alerts);
                    for effect in applied.effects {
                        rig.io.handle(ClaudeInput::Effect(effect));
                    }
                }
            }
        }

        fn texts(alerts: &[Alert]) -> Vec<(AttentionKind, &str)> {
            alerts.iter().map(|a| (a.kind, a.text.as_str())).collect()
        }

        impl Rig {
            /// Rewrites the session file with `edit` applied to its JSON.
            fn edit_session(&self, edit: impl FnOnce(&mut Value)) {
                let path = self.dir.path.join("sessions").join(self.name());
                let mut file: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                edit(&mut file);
                std::fs::write(path, file.to_string()).unwrap();
            }
            fn cleared_transcript(&self) -> PathBuf {
                self.dir.path.join("projects").join(SLUG).join(format!("{CLEARED_SID}.jsonl"))
            }
        }

        #[test]
        fn sessions_already_waiting_at_start_are_listed_and_held_without_alerts() {
            // Mayak starts (or restarts) while a session waits on a permission prompt.
            let mut rig = rig("cio-boot");
            rig.append(&[TITLE, USER_OLD, TOOL_PUSH]);
            rig.write_session("waiting", 100);
            rig.edit_session(|f| f["waitingFor"] = "permission prompt".into());
            rig.io.start();
            let mut store = Store::new();
            assert_eq!(texts(&pump(&mut rig, &mut store)), [], "first sight never alerts");
            let snap = store.snapshot(9_000);
            assert_eq!(snap.attention, [rig.key()]);
            assert_eq!((snap.counts.waiting, snap.hold, snap.aggregate), (1, true, BotState::Approval));
            let row = &snap.projects[0].sessions[0];
            assert_eq!(row.title.as_deref(), Some("Release prep"), "the start window's title is read");
            assert_eq!(row.since, 100);
            // Claude Code rewrites the file within the same episode: still no alert, still held.
            rig.edit_session(|f| f["updatedAt"] = 5_000.into());
            rig.session_changed(&[FileAction::Modified]);
            assert_eq!(texts(&pump(&mut rig, &mut store)), []);
            let snap = store.snapshot(9_000);
            assert_eq!((snap.attention.len(), snap.hold, snap.projects[0].sessions[0].since), (1, true, 100));
        }

        #[test]
        fn a_collapsed_turn_is_read_before_its_file_and_alerts_once() {
            // busy → idle between two reads (a quick answer, an API error at once): claude-io sees
            // only idle → idle with a new statusUpdatedAt (spec §7 collapsed turn).
            let mut rig = rig("cio-collapsed");
            rig.append(&[USER_OLD, REPLY_OLD]);
            rig.write_session("idle", 100);
            rig.io.start();
            let mut store = Store::new();
            assert_eq!(texts(&pump(&mut rig, &mut store)), []);
            rig.append(&[USER_QUICK, REPLY_QUICK]);
            rig.write_session("idle", 200);
            rig.session_changed(&[FileAction::Modified, FileAction::Modified]);
            let alerts = pump(&mut rig, &mut store);
            assert_eq!(texts(&alerts), [(AttentionKind::Finished, "Claude · work finished: Quick answer.")]);
            // The same writes reported again, the transcript's too: nothing more.
            rig.session_changed(&[FileAction::Modified]);
            let rel = rig.transcript_rel();
            rig.io.handle(ClaudeInput::Projects(WatchEvent::Changes(vec![(FileAction::Modified, rel)])));
            assert_eq!(texts(&pump(&mut rig, &mut store)), []);
            let snap = store.snapshot(9_000);
            let outcome = snap.projects[0].sessions[0].outcome.as_ref().map(|o| o.summary.as_str());
            assert_eq!(outcome, Some("Quick answer."));
        }

        #[test]
        fn a_cleared_session_lets_the_old_transcript_go_and_counts_only_the_new_one() {
            // /clear: same process, new sessionId; the new transcript appears with the first prompt.
            let mut rig = rig("cio-clear");
            rig.append(&[USER_OLD, REPLY_OLD]);
            rig.write_session("idle", 100);
            rig.io.start();
            let mut store = Store::new();
            assert_eq!(texts(&pump(&mut rig, &mut store)), []);
            rig.edit_session(|f| f["sessionId"] = CLEARED_SID.into());
            rig.session_changed(&[FileAction::Modified]);
            assert_eq!(texts(&pump(&mut rig, &mut store)), []);
            // The old transcript is written once more, then removed: claude-io no longer reads it.
            rig.append(&[REPLY_STALE]);
            let old = rig.transcript_rel();
            rig.io.handle(ClaudeInput::Projects(WatchEvent::Changes(vec![(FileAction::Modified, old.clone())])));
            std::fs::remove_file(rig.transcript()).unwrap();
            rig.io.handle(ClaudeInput::Projects(WatchEvent::Changes(vec![(FileAction::Removed, old)])));
            assert_eq!(rig.signals(), []);
            // First prompt of the cleared session: its file is created, the turn runs and ends.
            std::fs::write(rig.cleared_transcript(), format!("{USER_QUICK}\n")).unwrap();
            let new = format!("{SLUG}\\{CLEARED_SID}.jsonl");
            rig.io.handle(ClaudeInput::Projects(WatchEvent::Changes(vec![(FileAction::Added, new)])));
            rig.edit_session(|f| {
                f["status"] = "busy".into();
                f["statusUpdatedAt"] = 200.into();
            });
            rig.session_changed(&[FileAction::Modified]);
            assert_eq!(texts(&pump(&mut rig, &mut store)), []);
            let mut file = std::fs::OpenOptions::new().append(true).open(rig.cleared_transcript()).unwrap();
            std::io::Write::write_all(&mut file, format!("{REPLY_QUICK}\n").as_bytes()).unwrap();
            drop(file);
            rig.edit_session(|f| {
                f["status"] = "idle".into();
                f["statusUpdatedAt"] = 300.into();
            });
            rig.session_changed(&[FileAction::Modified]);
            let alerts = pump(&mut rig, &mut store);
            assert_eq!(texts(&alerts), [(AttentionKind::Finished, "Claude · work finished: Quick answer.")]);
        }
    }
}
