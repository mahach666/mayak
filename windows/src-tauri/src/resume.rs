//! «Continue a session» (ideas §7): the Claude Code and Codex sessions of the last 24 hours that
//! are no longer on the session list, read only when the user opens the «Recent» fold, and the
//! two ways back into one: the resume command in a new console in the session's folder, or that
//! command on the clipboard. Nothing here runs on its own: every read and every launch is a click.
//!
//! What is read: the last `WINDOW` bytes of each recent transcript or rollout (its folder, title,
//! last phrase and how it ended) and the first line of a rollout (its folder). Nothing of it is
//! logged; a log line names a kind of failure only.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::observe::claude_transcript::{fact_of, terminal_outcome, Fact, TitleKind, TurnOutcome};
use crate::observe::codex::{index_title, parse_rollout_line, RolloutRecord};
use crate::observe::model::SessionsSnapshot;
use crate::observe::paths::basename;
use crate::observe::silent::{parse_session_meta, SilentMeta};
use crate::observe::text::{collapse, error_summary, SUMMARY_MAX};
use crate::stats::{self, Agent};

/// How far back «Recent» looks.
pub const RECENT_MS: i64 = 24 * 60 * 60 * 1000;
/// At most this many recent sessions are listed, newest first.
pub const RECENT_MAX: usize = 12;
/// The end of a transcript or rollout that is read for one row.
const WINDOW: u64 = 256 * 1024;
/// The first line of a rollout (`session_meta`) is read up to this size.
const FIRST_LINE_CAP: u64 = 256 * 1024;
/// `session_index.jsonl` is read up to this size; a bigger one gives no titles.
const INDEX_CAP: u64 = 4 * 1024 * 1024;

/// The agent whose CLI resumes a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResumeAgent {
    Claude,
    Codex,
}

impl ResumeAgent {
    /// The CLI looked up on PATH.
    pub fn cli(self) -> &'static str {
        match self {
            ResumeAgent::Claude => "claude",
            ResumeAgent::Codex => "codex",
        }
    }

    /// `claude --resume <id>` (Claude Code's `-r, --resume`) / `codex resume <id>` (Codex CLI).
    pub fn args(self, id: &str) -> [&str; 3] {
        match self {
            ResumeAgent::Claude => ["claude", "--resume", id],
            ResumeAgent::Codex => ["codex", "resume", id],
        }
    }
}

/// How a recent session's last turn ended, as far as its last `WINDOW` bytes tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RecentEnd {
    Finished,
    Error,
    Interrupted,
    /// The turn started and never ended: the agent was closed or crashed mid-turn.
    Unfinished,
}

/// One row of «Recent».
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentSession {
    pub agent: ResumeAgent,
    pub id: String,
    /// The folder the session ran in, where it is resumed.
    pub cwd: String,
    /// The folder's name.
    pub project: String,
    pub title: Option<String>,
    /// The last assistant phrase, or the error.
    pub last: Option<String>,
    /// None when the window holds no turn at all.
    pub end: Option<RecentEnd>,
    /// Last write, ms since the epoch.
    pub at: i64,
}

/// A session id that may go on a command line: 1 to 64 ASCII letters, digits and dashes (Claude
/// and Codex ids are UUIDs). Nothing a shell or a console reads as syntax.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Every character PowerShell reads as a single quote: the ASCII one and the typographic ones
/// (U+2018–U+201B). Inside a single-quoted string each of them ends the string unless doubled.
#[cfg(windows)]
const PS_SINGLE_QUOTES: [char; 5] = ['\'', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'];

/// The text «Copy» puts on the clipboard: PowerShell (the console Windows opens by default) that
/// goes to the folder and resumes there. The folder is a literal path in single quotes, every
/// PowerShell single quote in it (`PS_SINGLE_QUOTES`) doubled. None for an id that is not
/// `valid_id` or a folder with a control character.
#[cfg(windows)]
pub fn copy_text(agent: ResumeAgent, id: &str, cwd: &str) -> Option<String> {
    if !valid_id(id) || cwd.is_empty() || cwd.chars().any(char::is_control) {
        return None;
    }
    let mut folder = String::with_capacity(cwd.len());
    for c in cwd.chars() {
        folder.push(c);
        if PS_SINGLE_QUOTES.contains(&c) {
            folder.push(c);
        }
    }
    Some(format!("Set-Location -LiteralPath '{folder}'; {}", agent.args(id).join(" ")))
}

/// `cwd` as one POSIX shell word: in single quotes, each single quote in it written `'\''`.
#[cfg(not(windows))]
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The text «Copy» puts on the clipboard: POSIX shell (zsh, the macOS default, and bash) that
/// goes to the folder and resumes there. None for an id that is not `valid_id` or a folder with a
/// control character.
#[cfg(not(windows))]
pub fn copy_text(agent: ResumeAgent, id: &str, cwd: &str) -> Option<String> {
    if !valid_id(id) || cwd.is_empty() || cwd.chars().any(char::is_control) {
        return None;
    }
    Some(format!("cd -- {} && {}", sh_quote(cwd), agent.args(id).join(" ")))
}

/// The resume target of a live session in the snapshot (the session card's «Copy»): Claude and
/// Codex only, with a folder and an id.
pub fn live_target(snapshot: &SessionsSnapshot, key: &str) -> Option<(ResumeAgent, String, String)> {
    use crate::observe::model::Source;
    snapshot.projects.iter().flat_map(|p| p.sessions.iter()).find(|s| s.key == key).and_then(|s| {
        let agent = match s.agent {
            Source::Claude => ResumeAgent::Claude,
            Source::Codex => ResumeAgent::Codex,
            Source::Antigravity => return None,
        };
        let id = s.logical_id.clone()?;
        let cwd = s.cwd.clone().filter(|c| !c.is_empty())?;
        Some((agent, id, cwd))
    })
}

/// The ids of the sessions on the list now (Claude `sessionId`, Codex `session_id`, silent Codex
/// rows included): «Recent» leaves them out.
pub fn live_ids(snapshot: &SessionsSnapshot) -> HashSet<String> {
    snapshot.projects.iter().flat_map(|p| p.sessions.iter()).filter_map(|s| s.logical_id.clone()).collect()
}

// ── One Claude transcript ─────────────────────────────────────────────────────

/// A recent row from the end of a Claude transcript `<id>.jsonl` (records in file order). None for
/// a session run by a program through the Agent SDK (`entrypoint` "sdk-…", `claude -p` included),
/// and when no record names a folder. The folder is the `cwd` of the last record of the window that
/// has one, else `head_cwd` (the first one at the start of the file, read only then); the title
/// the last custom title, else agent name, else AI title (the list's order); the end and the last
/// phrase come from `terminal_outcome` over the window.
pub fn claude_recent(
    id: &str,
    records: &[(u64, Value)],
    head_cwd: impl FnOnce() -> Option<String>,
    at: i64,
) -> Option<RecentSession> {
    if !valid_id(id) {
        return None;
    }
    let last_str = |field: &str| {
        records.iter().rev().find_map(|(_, v)| v.get(field).and_then(Value::as_str).filter(|s| !s.is_empty()))
    };
    if last_str("entrypoint").is_some_and(|e| e.starts_with("sdk-")) {
        return None;
    }
    let cwd = last_str("cwd").map(str::to_string).or_else(head_cwd)?;
    let facts: Vec<(u64, Fact)> = records.iter().filter_map(|(offset, v)| fact_of(v).map(|f| (*offset, f))).collect();
    let title_of = |kind: TitleKind| {
        facts.iter().rev().find_map(|(_, f)| match f {
            Fact::Title { kind: k, text } if *k == kind => Some(text.clone()),
            _ => None,
        })
    };
    let title = title_of(TitleKind::Custom).or_else(|| title_of(TitleKind::Agent)).or_else(|| title_of(TitleKind::Ai));
    let last_text = facts.iter().rev().find_map(|(_, f)| match f {
        Fact::Assistant { text, api_error: None, .. } if !text.is_empty() => Some(text.clone()),
        _ => None,
    });
    let turns = facts.iter().any(|(_, f)| !matches!(f, Fact::Title { .. }));
    let (end, last) = match terminal_outcome(&facts, 0).0 {
        _ if !turns => (None, None),
        TurnOutcome::Finished { summary } => (Some(RecentEnd::Finished), Some(summary).filter(|s| !s.is_empty()).or(last_text)),
        TurnOutcome::Error(e) => (Some(RecentEnd::Error), Some(error_summary(e.code.as_deref(), &e.text))),
        TurnOutcome::Interrupted => (Some(RecentEnd::Interrupted), last_text),
        TurnOutcome::Pending => (Some(RecentEnd::Unfinished), last_text),
    };
    Some(RecentSession {
        agent: ResumeAgent::Claude,
        id: id.to_string(),
        project: basename(&cwd),
        cwd,
        title,
        last: last.map(|s| collapse(&s, SUMMARY_MAX)),
        end,
        at,
    })
}

// ── One Codex rollout ─────────────────────────────────────────────────────────

/// A recent row from a Codex rollout: its `session_meta` (first line) and the records of its end.
/// None for a subagent's rollout, an id that is not `valid_id` and a session without a folder.
/// The end is the last of task_started (unfinished), task_complete (finished, or error when it
/// has one) and turn_aborted (interrupted); the title comes from `session_index.jsonl`.
pub fn codex_recent(meta: &SilentMeta, records: &[Value], index_text: &str, at: i64) -> Option<RecentSession> {
    if meta.subagent || !valid_id(&meta.id) {
        return None;
    }
    let cwd = meta.cwd.clone().filter(|c| !c.is_empty())?;
    let mut end = None;
    let mut last = None;
    for v in records {
        match parse_rollout_line(v) {
            RolloutRecord::TaskStarted { .. } => end = Some(RecentEnd::Unfinished),
            RolloutRecord::TaskComplete { last_agent_message, error, .. } => match error {
                Some(error) => {
                    end = Some(RecentEnd::Error);
                    last = Some(error);
                }
                None => {
                    end = Some(RecentEnd::Finished);
                    if let Some(text) = last_agent_message.filter(|t| !t.trim().is_empty()) {
                        last = Some(text);
                    }
                }
            },
            RolloutRecord::TurnAborted { .. } => end = Some(RecentEnd::Interrupted),
            _ => {}
        }
    }
    Some(RecentSession {
        agent: ResumeAgent::Codex,
        id: meta.id.clone(),
        project: basename(&cwd),
        cwd,
        title: index_title(index_text, &meta.id),
        last: last.map(|s| collapse(&s, SUMMARY_MAX)).filter(|s| !s.is_empty()),
        end,
        at,
    })
}

// ── The scan ──────────────────────────────────────────────────────────────────

/// Where the agents keep their files.
pub struct RecentRoots {
    /// `<claude dir>\projects`
    pub claude_projects: PathBuf,
    /// `<codex home>`: `sessions\` and `session_index.jsonl` are in it.
    pub codex_home: PathBuf,
}

/// The sessions written to within `RECENT_MS` before `now_ms`, newest first, at most `RECENT_MAX`,
/// leaving out the ids in `live`. Only the files that could make the list are opened, newest first.
/// An unreadable file is skipped and logged by kind.
pub fn scan(roots: &RecentRoots, now_ms: i64, live: &HashSet<String>) -> Vec<RecentSession> {
    let files = stats::list_files(&stats::Roots {
        claude_projects: roots.claude_projects.clone(),
        codex_sessions: roots.codex_home.join("sessions"),
    });
    let mut recent: Vec<(i64, PathBuf, Agent)> = files
        .into_iter()
        .filter_map(|(path, agent)| {
            let at = mtime_ms(&path)?;
            (at >= now_ms - RECENT_MS && at <= now_ms + 60_000).then_some((at, path, agent))
        })
        .collect();
    recent.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut index: Option<String> = None;
    let mut out: Vec<RecentSession> = Vec::new();
    for (at, path, agent) in recent {
        if out.len() >= RECENT_MAX {
            break;
        }
        let row = match agent {
            Agent::Claude => {
                let Some(id) = path.file_stem().and_then(|s| s.to_str()) else { continue };
                if live.contains(id) {
                    continue;
                }
                match tail_records(&path) {
                    Ok(records) => claude_recent(id, &records, || head_cwd(&path), at),
                    Err(err) => {
                        crate::log::line(format!("recent: transcript unreadable: {:?}", err.kind()));
                        None
                    }
                }
            }
            Agent::Codex => {
                let Some(meta) = first_line(&path).as_deref().and_then(parse_session_meta) else { continue };
                if live.contains(&meta.id) {
                    continue;
                }
                let index = index.get_or_insert_with(|| read_index(&roots.codex_home.join("session_index.jsonl")));
                match tail_records(&path) {
                    Ok(records) => {
                        let values: Vec<Value> = records.into_iter().map(|(_, v)| v).collect();
                        codex_recent(&meta, &values, index, at)
                    }
                    Err(err) => {
                        crate::log::line(format!("recent: rollout unreadable: {:?}", err.kind()));
                        None
                    }
                }
            }
        };
        // One session can have several files (a Codex session resumed into a new rollout).
        if let Some(row) = row.filter(|r| !out.iter().any(|o| o.agent == r.agent && o.id == r.id)) {
            out.push(row);
        }
    }
    out
}

fn mtime_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let ms = modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    i64::try_from(ms).ok()
}

/// The JSON records of the last `WINDOW` bytes, as (offset, value); a line that does not parse
/// is skipped.
fn tail_records(path: &Path) -> std::io::Result<Vec<(u64, Value)>> {
    let mut tail = crate::observe::tail::JsonlTail::new(path);
    tail.start_with_window(WINDOW)?;
    let (lines, _) = tail.read_new()?;
    Ok(lines.into_iter().filter_map(|(offset, line)| serde_json::from_str(&line).ok().map(|v| (offset, v))).collect())
}

/// The first line of `path` when it ends within `FIRST_LINE_CAP` bytes.
fn first_line(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    BufReader::new(file.take(FIRST_LINE_CAP + 1)).read_until(b'\n', &mut buf).ok()?;
    if buf.last() != Some(&b'\n') {
        return None;
    }
    buf.pop();
    Some(String::from_utf8_lossy(&buf).trim_end_matches('\r').to_owned())
}

/// The first non-empty `cwd` among the complete lines of the first `FIRST_LINE_CAP` bytes of a
/// Claude transcript: its folder when no record at its end names one (one huge last record).
fn head_cwd(path: &Path) -> Option<String> {
    let mut head = Vec::new();
    std::fs::File::open(path).ok()?.take(FIRST_LINE_CAP).read_to_end(&mut head).ok()?;
    let complete = head.iter().rposition(|&b| b == b'\n')?;
    String::from_utf8_lossy(&head[..complete]).lines().find_map(|line| {
        let v: Value = serde_json::from_str(line).ok()?;
        v.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_owned)
    })
}

/// `session_index.jsonl` as text; empty when it is missing, unreadable or bigger than `INDEX_CAP`.
fn read_index(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() <= INDEX_CAP => std::fs::read_to_string(path).unwrap_or_default(),
        _ => String::new(),
    }
}

// ── Resume ────────────────────────────────────────────────────────────────────

/// What «Resume» did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResumeOutcome {
    /// A console runs the resume command in the session's folder.
    Started,
    /// The agent's CLI is not on PATH: the command went to the clipboard instead.
    Copied,
    /// Nothing happened (a refused id or folder, or the console did not start and the clipboard
    /// was busy).
    Failed,
}

/// The text after `cmd.exe /S /K` that runs `cli` (the absolute path `find_on_path` resolved) with
/// the resume arguments: `"<cli>" --resume <id>` in one more pair of quotes, which `/S` strips. The
/// CLI is always named by its full path, never by a bare stem cmd.exe would look up in the
/// session's folder first. None for an id that is not `valid_id`, a relative path, and a path with
/// a quote, `%` or `!`, which cmd.exe reads as syntax even inside quotes.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn console_line(agent: ResumeAgent, cli: &Path, id: &str) -> Option<String> {
    let path = cli.to_str()?;
    if !valid_id(id) || !cli.is_absolute() || path.contains(['"', '%', '!', '\r', '\n']) {
        return None;
    }
    let [_, verb, id] = agent.args(id);
    Some(format!("\"\"{path}\" {verb} {id}\""))
}

/// Opens a new console in `cwd` running the resume command (`cmd.exe /K`, so the console stays
/// when the agent exits and runs `claude.cmd` as well as `claude.exe`). cmd.exe is started from
/// System32 and runs the CLI by the absolute path found on PATH, with
/// `NoDefaultCurrentDirectoryInExePath` set, so nothing in the session's folder (a `claude.cmd`
/// in a cloned repo) is ever run in its place. The id is `valid_id`, so nothing in it is console
/// syntax; the folder is the process's working folder, never part of a command line. Without the
/// CLI on PATH, or with a path cmd.exe cannot be given safely, the command is copied instead.
#[cfg(windows)]
pub fn start(agent: ResumeAgent, id: &str, cwd: &str) -> ResumeOutcome {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let folder = Path::new(cwd);
    if !valid_id(id) || !folder.is_absolute() || !folder.is_dir() {
        return ResumeOutcome::Failed;
    }
    match crate::platform::find_on_path(agent.cli()).and_then(|cli| console_line(agent, &cli, id)) {
        Some(line) => {
            let system = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
            let mut cmd = std::process::Command::new(system.join("System32").join("cmd.exe"));
            cmd.args(["/S", "/K"])
                .raw_arg(line)
                .env("NoDefaultCurrentDirectoryInExePath", "1")
                .current_dir(folder)
                .creation_flags(CREATE_NEW_CONSOLE);
            match cmd.spawn() {
                Ok(_) => {
                    crate::log::line(format!("resume: {} started", agent.cli()));
                    return ResumeOutcome::Started;
                }
                Err(err) => crate::log::line(format!("resume: console did not start: {:?}", err.kind())),
            }
        }
        None => crate::log::line(format!("resume: {} not on PATH or its path refused, copied", agent.cli())),
    }
    match copy(agent, id, cwd) {
        true => ResumeOutcome::Copied,
        false => ResumeOutcome::Failed,
    }
}

/// The script a new Terminal window runs: to the folder, the CLI by its absolute path (never a
/// name looked up from the session's folder), then a login shell, so the window stays when the
/// agent exits, as `cmd.exe /K` does on Windows.
#[cfg(not(windows))]
pub fn terminal_script(agent: ResumeAgent, cli: &Path, id: &str, cwd: &str) -> Option<String> {
    let path = cli.to_str()?;
    if !valid_id(id) || !cli.is_absolute() || path.chars().any(char::is_control) || cwd.chars().any(char::is_control) {
        return None;
    }
    let [_, verb, id] = agent.args(id);
    Some(format!(
        "#!/bin/sh\ncd -- {} || exit 1\n{} {verb} {id}\nexec \"${{SHELL:-/bin/zsh}}\" -l\n",
        sh_quote(cwd),
        sh_quote(path)
    ))
}

/// The terminals a resumed session may open in, by app bundle name, each with how `open` hands
/// it the script: as a document (`.command`, which they run), or after these arguments of a new
/// instance. In this order the first one running wins; none running, Terminal.
#[cfg(any(target_os = "macos", test))]
const MAC_TERMINALS: &[(&str, Option<&[&str]>)] = &[
    ("iTerm", None),
    ("Ghostty", Some(&["-e"])),
    ("WezTerm", Some(&["start", "--"])),
    ("kitty", Some(&[])),
    ("Alacritty", Some(&["-e"])),
];

/// The arguments of `/usr/bin/open` that run `script` in the terminal the user works in: the first
/// of `MAC_TERMINALS` among the `running` app bundle names, else Terminal.
#[cfg(any(target_os = "macos", test))]
pub fn open_args(running: &[String], script: &Path) -> Vec<String> {
    let script = script.to_string_lossy().into_owned();
    let found = MAC_TERMINALS.iter().find(|(name, _)| running.iter().any(|r| r == name));
    match found {
        Some((name, None)) => vec!["-a".into(), (*name).into(), script],
        Some((name, Some(args))) => {
            let mut out: Vec<String> = vec!["-na".into(), (*name).into(), "--args".into()];
            out.extend(args.iter().map(|a| (*a).to_string()));
            out.push(script);
            out
        }
        None => vec!["-a".into(), "Terminal".into(), script],
    }
}

/// The app bundles running now, by name.
#[cfg(target_os = "macos")]
fn running_apps() -> Vec<String> {
    crate::platform::all_pids()
        .into_iter()
        .filter_map(|pid| crate::platform::image_path(pid).and_then(|image| crate::platform::app_bundle(&image)))
        .map(|(_, name)| name)
        .collect()
}

/// The `claude` Claude Desktop keeps for itself when no CLI is installed: the newest of
/// `~/Library/Application Support/Claude/claude-code/<ver>/<hash>/claude.app/Contents/MacOS/claude`.
#[cfg(target_os = "macos")]
fn desktop_claude() -> Option<PathBuf> {
    let root = crate::platform::home_dir().join("Library/Application Support/Claude/claude-code");
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for version in std::fs::read_dir(root).ok()?.flatten() {
        let Ok(builds) = std::fs::read_dir(version.path()) else { continue };
        for build in builds.flatten() {
            let exe = build.path().join("claude.app/Contents/MacOS/claude");
            if let Ok(modified) = std::fs::metadata(&exe).and_then(|m| m.modified()) {
                found.push((modified, exe));
            }
        }
    }
    found.into_iter().max_by_key(|(at, _)| *at).map(|(_, exe)| exe)
}

/// Opens a new window of the user's terminal (`open_args`) in `cwd` running the resume command: a
/// `.command` script of ours (`terminal_script`) under the app's private folder. No Apple Events,
/// so no Automation prompt. Without the CLI on PATH the command is copied instead.
#[cfg(target_os = "macos")]
pub fn start(agent: ResumeAgent, id: &str, cwd: &str) -> ResumeOutcome {
    use std::os::unix::fs::PermissionsExt;
    let folder = Path::new(cwd);
    if !valid_id(id) || !folder.is_absolute() || !folder.is_dir() {
        return ResumeOutcome::Failed;
    }
    // Codex's CLI may be only the one inside ChatGPT.app, as for the limits.
    let cli = match agent {
        ResumeAgent::Codex => crate::codex_limits::find_codex(Path::new("")),
        ResumeAgent::Claude => crate::platform::find_on_path(agent.cli()).or_else(desktop_claude),
    };
    let script = cli.and_then(|cli| terminal_script(agent, &cli, id, cwd));
    match script {
        Some(text) => {
            let dir = crate::settings::local_dir().join("resume");
            let file = dir.join(format!("{}-{id}.command", agent.cli()));
            let written = crate::platform::ensure_private_dir(&dir)
                .and_then(|_| std::fs::write(&file, text))
                .and_then(|_| std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)));
            let opened = written.and_then(|_| {
                std::process::Command::new("/usr/bin/open").args(open_args(&running_apps(), &file)).status()
            });
            match opened {
                Ok(status) if status.success() => {
                    crate::log::line(format!("resume: {} started", agent.cli()));
                    return ResumeOutcome::Started;
                }
                Ok(status) => crate::log::line(format!("resume: the terminal did not open: {status}")),
                Err(err) => crate::log::line(format!("resume: the terminal did not open: {:?}", err.kind())),
            }
        }
        None => crate::log::line(format!("resume: {} not on PATH or its path refused, copied", agent.cli())),
    }
    match copy(agent, id, cwd) {
        true => ResumeOutcome::Copied,
        false => ResumeOutcome::Failed,
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn start(_agent: ResumeAgent, _id: &str, _cwd: &str) -> ResumeOutcome {
    ResumeOutcome::Failed
}

/// Puts `copy_text` on the clipboard (kept out of clipboard history). False when the text is
/// refused or the clipboard is busy.
#[cfg(windows)]
pub fn copy(agent: ResumeAgent, id: &str, cwd: &str) -> bool {
    let Some(text) = copy_text(agent, id, cwd) else { return false };
    match crate::launcher::win::set_clipboard_text(&text) {
        Ok(()) => true,
        Err(_) => {
            crate::log::line("resume: clipboard busy");
            false
        }
    }
}

/// Puts `copy_text` on the clipboard through `pbcopy`. False when the text is refused or
/// `pbcopy` fails.
#[cfg(target_os = "macos")]
pub fn copy(agent: ResumeAgent, id: &str, cwd: &str) -> bool {
    let Some(text) = copy_text(agent, id, cwd) else { return false };
    crate::platform::set_clipboard_text(&text).is_ok()
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn copy(_agent: ResumeAgent, _id: &str, _cwd: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::wp;
    use serde_json::json;

    const ID: &str = "3235e946-0d64-4356-98c9-f7b8c8949416";

    fn at(records: Vec<Value>) -> Vec<(u64, Value)> {
        records.into_iter().enumerate().map(|(i, v)| (i as u64 * 100, v)).collect()
    }

    fn user(text: &str) -> Value {
        json!({ "type": "user", "cwd": wp(r"C:\work\app"), "entrypoint": "cli", "message": { "content": text } })
    }

    fn assistant(text: &str, stop: &str) -> Value {
        json!({
            "type": "assistant", "cwd": r"C:\work\app", "entrypoint": "cli",
            "message": { "stop_reason": stop, "content": [{ "type": "text", "text": text }] }
        })
    }

    #[test]
    fn ids_are_letters_digits_and_dashes_only() {
        assert!(valid_id(ID));
        assert!(valid_id("01a11bee-5e59-73d0-a256-491c8bb6d877"));
        for bad in ["", "a b", "a&b", "a;b", "a\"b", "a|b", "--x\n", "a%PATH%", &"a".repeat(65)] {
            assert!(!valid_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_commands_are_the_documented_resume_forms() {
        assert_eq!(ResumeAgent::Claude.args(ID), ["claude", "--resume", ID]);
        assert_eq!(ResumeAgent::Codex.args("x-1"), ["codex", "resume", "x-1"]);
    }

    #[test]
    #[cfg(not(windows))]
    fn the_copied_text_is_posix_shell_with_the_folder_quoted() {
        assert_eq!(
            copy_text(ResumeAgent::Claude, ID, "/Users/dev/my app").as_deref(),
            Some("cd -- '/Users/dev/my app' && claude --resume 3235e946-0d64-4356-98c9-f7b8c8949416")
        );
        assert_eq!(
            copy_text(ResumeAgent::Codex, "abc", "/x/it's; rm -rf ~").as_deref(),
            Some("cd -- '/x/it'\\''s; rm -rf ~' && codex resume abc")
        );
        assert_eq!(copy_text(ResumeAgent::Codex, "a b", "/x"), None);
        assert_eq!(copy_text(ResumeAgent::Codex, "abc", "/x\ny"), None);
    }

    #[test]
    fn a_resumed_session_opens_in_the_running_terminal_else_in_terminal() {
        let script = Path::new("/x/claude-1.command");
        let running = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        assert_eq!(open_args(&running(&["Finder"]), script), ["-a", "Terminal", "/x/claude-1.command"]);
        assert_eq!(open_args(&running(&["Ghostty", "iTerm"]), script), ["-a", "iTerm", "/x/claude-1.command"], "iTerm2 first");
        assert_eq!(open_args(&running(&["Ghostty"]), script), ["-na", "Ghostty", "--args", "-e", "/x/claude-1.command"]);
        assert_eq!(open_args(&running(&["WezTerm"]), script), ["-na", "WezTerm", "--args", "start", "--", "/x/claude-1.command"]);
        assert_eq!(open_args(&running(&["kitty"]), script), ["-na", "kitty", "--args", "/x/claude-1.command"]);
    }

    #[test]
    #[cfg(not(windows))]
    fn the_terminal_runs_the_cli_by_its_full_path_then_keeps_a_shell() {
        let script = terminal_script(ResumeAgent::Claude, Path::new("/opt/homebrew/bin/claude"), ID, "/Users/dev/a'b").unwrap();
        assert_eq!(
            script,
            "#!/bin/sh\ncd -- '/Users/dev/a'\\''b' || exit 1\n'/opt/homebrew/bin/claude' --resume 3235e946-0d64-4356-98c9-f7b8c8949416\nexec \"${SHELL:-/bin/zsh}\" -l\n"
        );
        assert_eq!(terminal_script(ResumeAgent::Claude, Path::new("claude"), ID, "/x"), None, "never by its name");
        assert_eq!(terminal_script(ResumeAgent::Claude, Path::new("/bin/claude"), "x;y", "/x"), None);
    }

    #[test]
    #[cfg(windows)]
    fn the_copied_text_goes_to_the_literal_folder_first() {
        assert_eq!(
            copy_text(ResumeAgent::Claude, ID, r"C:\work\it's [app]").as_deref(),
            Some(r"Set-Location -LiteralPath 'C:\work\it''s [app]'; claude --resume 3235e946-0d64-4356-98c9-f7b8c8949416")
        );
        assert_eq!(
            copy_text(ResumeAgent::Codex, "abc", r"C:\w").as_deref(),
            Some(r"Set-Location -LiteralPath 'C:\w'; codex resume abc")
        );
        assert_eq!(copy_text(ResumeAgent::Claude, "a;b", r"C:\w"), None);
        assert_eq!(copy_text(ResumeAgent::Claude, ID, ""), None);
        assert_eq!(copy_text(ResumeAgent::Claude, ID, "C:\\w\nx"), None);
        assert_eq!(copy_text(ResumeAgent::Claude, ID, "C:\\w\u{7}x"), None);
    }

    #[test]
    #[cfg(windows)]
    fn typographic_single_quotes_in_the_folder_are_doubled_too() {
        // PowerShell ends a single-quoted string on ’ ‘ ‚ ‛ as on ', so a folder named
        // `x’; calc; ’` must stay one literal path.
        let text = copy_text(ResumeAgent::Codex, "abc", "C:\\x\u{2019}; calc; \u{2018}y\u{201A}\u{201B}").unwrap();
        assert_eq!(
            text,
            "Set-Location -LiteralPath 'C:\\x\u{2019}\u{2019}; calc; \u{2018}\u{2018}y\u{201A}\u{201A}\u{201B}\u{201B}'; codex resume abc"
        );
    }

    #[test]
    fn a_finished_claude_session_has_its_folder_title_and_last_phrase() {
        let records = at(vec![
            json!({ "type": "ai-title", "aiTitle": "Fix the parser" }),
            json!({ "type": "custom-title", "customTitle": "parser" }),
            user("fix it"),
            assistant("Done, the parser is fixed.", "end_turn"),
            json!({ "type": "system", "cwd": wp(r"C:\work\app\sub"), "entrypoint": "cli" }),
        ]);
        let row = claude_recent(ID, &records, || None, 42).unwrap();
        assert_eq!(row.agent, ResumeAgent::Claude);
        assert_eq!(row.cwd, wp(r"C:\work\app\sub"));
        assert_eq!(row.project, "sub");
        assert_eq!(row.title.as_deref(), Some("parser"));
        assert_eq!(row.last.as_deref(), Some("Done, the parser is fixed."));
        assert_eq!(row.end, Some(RecentEnd::Finished));
        assert_eq!(row.at, 42);
    }

    #[test]
    fn claude_ends_error_interrupted_and_unfinished() {
        let error = at(vec![
            user("go"),
            json!({
                "type": "assistant", "cwd": r"C:\w", "isApiErrorMessage": true, "error": "rate_limit",
                "message": { "content": [{ "type": "text", "text": "Limit reached" }] }
            }),
        ]);
        let row = claude_recent(ID, &error, || None, 0).unwrap();
        assert_eq!(row.end, Some(RecentEnd::Error));
        assert_eq!(row.last.as_deref(), Some("rate_limit: Limit reached"));

        let interrupted = at(vec![assistant("Working on it", "tool_use"), user("[Request interrupted by user]")]);
        let row = claude_recent(ID, &interrupted, || None, 0).unwrap();
        assert_eq!(row.end, Some(RecentEnd::Interrupted));
        assert_eq!(row.last.as_deref(), Some("Working on it"));

        let unfinished = at(vec![user("go"), assistant("Reading the files", "tool_use")]);
        assert_eq!(claude_recent(ID, &unfinished, || None, 0).unwrap().end, Some(RecentEnd::Unfinished));
    }

    #[test]
    fn claude_sessions_without_a_folder_or_run_by_the_sdk_are_left_out() {
        let no_cwd = at(vec![json!({ "type": "ai-title", "aiTitle": "x" })]);
        assert_eq!(claude_recent(ID, &no_cwd, || None, 0), None);
        let sdk = at(vec![json!({ "type": "user", "cwd": r"C:\w", "entrypoint": "sdk-py", "message": { "content": "x" } })]);
        assert_eq!(claude_recent(ID, &sdk, || None, 0), None);
        assert_eq!(claude_recent("bad id", &at(vec![user("x")]), || None, 0), None);
        // Only titles: listed, with no end.
        let titled = at(vec![json!({ "type": "ai-title", "aiTitle": "x", "cwd": r"C:\w" })]);
        assert_eq!(claude_recent(ID, &titled, || None, 0).unwrap().end, None);
    }

    fn meta(subagent: bool) -> SilentMeta {
        SilentMeta {
            id: "22222222-2222-4222-8222-222222222222".into(),
            cwd: Some(wp(r"C:\work\codex")),
            originator: Some("codex_cli_rs".into()),
            subagent,
        }
    }

    fn event(kind: &str, extra: Value) -> Value {
        let mut payload = json!({ "type": kind, "turn_id": "t" });
        for (k, v) in extra.as_object().unwrap() {
            payload[k] = v.clone();
        }
        json!({ "type": "event_msg", "payload": payload })
    }

    const INDEX: &str = include_str!("observe/fixtures/codex/session_index.jsonl");

    #[test]
    fn a_codex_rollout_gives_its_end_last_message_and_index_title() {
        let records = vec![
            event("task_started", json!({})),
            event("task_complete", json!({ "last_agent_message": "All tests pass." })),
        ];
        let row = codex_recent(&meta(false), &records, INDEX, 7).unwrap();
        assert_eq!(row.agent, ResumeAgent::Codex);
        assert_eq!(row.cwd, wp(r"C:\work\codex"));
        assert_eq!(row.project, "codex");
        assert_eq!(row.title.as_deref(), Some("Run the tests offline"));
        assert_eq!(row.last.as_deref(), Some("All tests pass."));
        assert_eq!(row.end, Some(RecentEnd::Finished));

        let crashed = vec![
            event("task_complete", json!({ "last_agent_message": "first" })),
            event("task_started", json!({})),
        ];
        let row = codex_recent(&meta(false), &crashed, "", 0).unwrap();
        assert_eq!(row.end, Some(RecentEnd::Unfinished));
        assert_eq!(row.last.as_deref(), Some("first"));
        assert_eq!(row.title, None);

        let aborted = vec![event("task_started", json!({})), event("turn_aborted", json!({}))];
        assert_eq!(codex_recent(&meta(false), &aborted, "", 0).unwrap().end, Some(RecentEnd::Interrupted));

        let failed = vec![event("task_complete", json!({ "error": { "message": "stream disconnected" } }))];
        let row = codex_recent(&meta(false), &failed, "", 0).unwrap();
        assert_eq!(row.end, Some(RecentEnd::Error));
        assert_eq!(row.last.as_deref(), Some("stream disconnected"));
    }

    #[test]
    fn codex_subagents_and_sessions_without_a_folder_are_left_out() {
        assert_eq!(codex_recent(&meta(true), &[], "", 0), None);
        let mut no_cwd = meta(false);
        no_cwd.cwd = None;
        assert_eq!(codex_recent(&no_cwd, &[], "", 0), None);
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn the_scan_lists_recent_files_newest_first_without_live_or_stale_ones() {
        let dir = crate::testutil::TempDir::new("recent");
        let roots = RecentRoots { claude_projects: dir.path.join("projects"), codex_home: dir.path.join("codex") };
        let line = |v: Value| format!("{v}\n");
        let a = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let b = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
        write(&roots.claude_projects.join("C--w").join(format!("{a}.jsonl")), &line(user("hi")));
        write(&roots.claude_projects.join("C--w").join(format!("{b}.jsonl")), &line(user("hi")));
        write(&roots.claude_projects.join("C--w").join("notes.txt"), "x");
        let sid = "22222222-2222-4222-8222-222222222222";
        let rollout = roots.codex_home.join("sessions").join("2026").join("10").join("11").join(format!("rollout-x-{sid}.jsonl"));
        let meta_line = json!({ "type": "session_meta", "payload": { "id": sid, "cwd": r"C:\work\codex", "originator": "codex_cli_rs" } });
        write(&rollout, &format!("{}{}", line(meta_line), line(event("task_started", json!({})))));
        write(&roots.codex_home.join("session_index.jsonl"), INDEX);

        let now = mtime_ms(&rollout).unwrap() + 1000;
        let rows = scan(&roots, now, &HashSet::from([b.to_string()]));
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids.len(), 2, "{ids:?}");
        assert!(ids.contains(&a) && ids.contains(&sid) && !ids.contains(&b));
        assert!(rows.windows(2).all(|w| w[0].at >= w[1].at));
        let codex = rows.iter().find(|r| r.id == sid).unwrap();
        assert_eq!(codex.end, Some(RecentEnd::Unfinished));
        assert_eq!(codex.title.as_deref(), Some("Run the tests offline"));

        // A day later nothing is recent.
        assert!(scan(&roots, now + RECENT_MS + 120_000, &HashSet::new()).is_empty());
    }

    #[test]
    #[cfg(windows)]
    fn the_console_runs_the_cli_by_its_full_path_never_by_its_name() {
        let cli = Path::new(r"C:\Program Files (x86)\nodejs\claude.cmd");
        assert_eq!(
            console_line(ResumeAgent::Claude, cli, ID).as_deref(),
            Some(r#"""C:\Program Files (x86)\nodejs\claude.cmd" --resume 3235e946-0d64-4356-98c9-f7b8c8949416""#)
        );
        assert_eq!(
            console_line(ResumeAgent::Codex, Path::new(r"C:\bin\codex.exe"), "x-1").as_deref(),
            Some(r#"""C:\bin\codex.exe" resume x-1""#)
        );
        // A bare name (cmd.exe would look in the session's folder first) and a relative path are refused.
        assert_eq!(console_line(ResumeAgent::Claude, Path::new("claude"), ID), None);
        assert_eq!(console_line(ResumeAgent::Claude, Path::new(r"bin\claude.cmd"), ID), None);
        for bad in [r"C:\a%PATH%\claude.cmd", r"C:\a!x!\claude.cmd", r#"C:\a"b\claude.cmd"#] {
            assert_eq!(console_line(ResumeAgent::Claude, Path::new(bad), ID), None, "{bad}");
        }
        assert_eq!(console_line(ResumeAgent::Claude, Path::new(r"C:\bin\claude.cmd"), "a&b"), None);
    }

    fn view(key: &str, agent: crate::observe::model::Source, id: Option<&str>, cwd: Option<&str>) -> crate::observe::model::SessionView {
        crate::observe::model::SessionView {
            key: key.into(),
            agent,
            host: crate::observe::keys::HostKind::Unknown,
            title: None,
            phase: crate::observe::model::PhaseView::Idle,
            since: 0,
            activity: None,
            detail: None,
            outcome: None,
            attention: None,
            acked: true,
            silent: None,
            context: None,
            model: None,
            compacted: None,
            turn: None,
            ao: None,
            agent_pid: None,
            host_session_id: None,
            logical_id: id.map(str::to_string),
            cwd: cwd.map(str::to_string),
            last_reply: None,
        }
    }

    #[test]
    fn live_sessions_give_their_ids_and_claude_and_codex_their_resume_target() {
        use crate::observe::model::{ProjectView, Source, Store};
        let mut snapshot = Store::new().snapshot(0);
        snapshot.projects.push(ProjectView {
            key: r"c:\w".into(),
            name: "w".into(),
            root: r"C:\w".into(),
            sessions: vec![
                view("claude:1:2", Source::Claude, Some(ID), Some(r"C:\w")),
                view("codex:t", Source::Codex, Some("x-1"), Some(r"C:\w\sub")),
                view("antigravity:a", Source::Antigravity, Some("ag"), Some(r"C:\w")),
                view("silent:s-1", Source::Codex, Some("s-1"), None),
                view("claude:3:4", Source::Claude, None, Some(r"C:\w")),
                view("claude:5:6", Source::Claude, Some("y"), Some("")),
            ],
        });
        assert_eq!(
            live_ids(&snapshot),
            HashSet::from([ID, "x-1", "ag", "s-1", "y"].map(str::to_string))
        );
        assert_eq!(live_target(&snapshot, "claude:1:2"), Some((ResumeAgent::Claude, ID.to_string(), r"C:\w".to_string())));
        assert_eq!(live_target(&snapshot, "codex:t"), Some((ResumeAgent::Codex, "x-1".to_string(), r"C:\w\sub".to_string())));
        for refused in ["antigravity:a", "silent:s-1", "claude:3:4", "claude:5:6", "nope"] {
            assert_eq!(live_target(&snapshot, refused), None, "{refused}");
        }
    }

    #[test]
    fn a_claude_transcript_with_no_folder_at_its_end_takes_the_first_one() {
        let dir = crate::testutil::TempDir::new("recent-head");
        let path = dir.path.join(format!("{ID}.jsonl"));
        let first = json!({ "type": "summary", "summary": "x" });
        let big = json!({ "type": "user", "message": { "content": "y".repeat(300 * 1024) } });
        write(&path, &format!("{first}\n{}\n{big}\n", user("hi")));
        assert_eq!(head_cwd(&path), Some(wp(r"C:\work\app")));
        let records = tail_records(&path).unwrap();
        assert!(records.iter().all(|(_, v)| v.get("cwd").is_none()));
        assert_eq!(claude_recent(ID, &records, || None, 0), None);
        assert_eq!(claude_recent(ID, &records, || head_cwd(&path), 0).unwrap().cwd, wp(r"C:\work\app"));
    }
}
