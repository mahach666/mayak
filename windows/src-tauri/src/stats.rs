//! Token statistics (night spec §C): how many tokens went by day, project, agent and model,
//! counted on request from the agents' own files, never from live counters.
//!
//! - Claude: `<claude dir>\projects\*\*.jsonl`, `assistant` lines of the main branch with a
//!   `message.usage`. Claude Code writes one line per content block and each repeats the same
//!   usage, so a `message.id` counts once in a file (its last line). A resumed or forked session
//!   copies earlier messages into a new transcript, so it also counts once across files (the
//!   first file in path order).
//! - Codex: `<codex home>\sessions\**\rollout-*.jsonl`, one `token_usage_record` per request. Its
//!   `input_tokens` includes `cached_input_tokens`: the input counted here is the difference.
//!
//! Only numbers, dates, model names, project paths and hashes of Claude message ids are read and
//! kept; a line's text never is. The cache (`stats-cache.json`) keeps each file's size, mtime and
//! aggregates (Codex) or messages (Claude), so a scan only re-reads the files that changed and
//! replaces their entries whole: nothing counts twice.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::log;
use crate::observe::claude_usage::parse_rfc3339_ms;
use crate::observe::paths::{basename, ProjectCache};
use crate::observe::silent::parse_session_meta;
use crate::platform::LocalTime;

const DAY_MS: i64 = 86_400_000;
/// 2: Claude transcripts keep their messages (id hashes) instead of per-file aggregates.
const CACHE_VERSION: u32 = 2;
/// How deep the Codex sessions tree is walked (`sessions\YYYY\MM\DD\rollout-….jsonl` is 4).
const CODEX_MAX_DEPTH: usize = 6;

// ── Numbers ───────────────────────────────────────────────────────────────────

/// The four counts of the spec, in the Claude shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Counts {
    /// «Всего» of the spec: the sum of the four.
    pub fn total(&self) -> u64 {
        self.input.saturating_add(self.output).saturating_add(self.cache_read).saturating_add(self.cache_write)
    }

    fn add(&mut self, other: &Counts) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    pub fn as_str(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }
}

/// One aggregate: the key (day, project root, agent, model) and its values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// Local day, `YYYY-MM-DD`.
    pub day: String,
    /// Project root (display form), "" when the line named no folder.
    pub project: String,
    pub agent: String,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub requests: u64,
}

impl Row {
    fn counts(&self) -> Counts {
        Counts { input: self.input, output: self.output, cache_read: self.cache_read, cache_write: self.cache_write }
    }
}

/// One Claude message, kept per message rather than summed: a resumed or forked session copies
/// earlier messages into a new transcript, so a `message.id` counts once across all files and
/// the files' rows cannot be summed. Only a hash of the id is kept, never the id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// FNV-1a of `message.id`; None for a line without one (it counts on its own).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub day: String,
    pub project: String,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Message {
    fn row(&self) -> Row {
        Row {
            day: self.day.clone(),
            project: self.project.clone(),
            agent: Agent::Claude.as_str().to_string(),
            model: self.model.clone(),
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
            requests: 1,
        }
    }
}

/// 64-bit FNV-1a: stable across Rust versions and runs, unlike `DefaultHasher`.
pub fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, b| (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// One request before aggregation: its day, raw folder, model and counts.
struct Usage {
    day: String,
    cwd: Option<String>,
    model: String,
    counts: Counts,
}

/// Sums requests by (day, project root, model) into rows of one agent, in key order.
fn aggregate(agent: Agent, usages: impl IntoIterator<Item = Usage>, project: &mut dyn FnMut(&str) -> String) -> Vec<Row> {
    let mut sums: BTreeMap<(String, String, String), (Counts, u64)> = BTreeMap::new();
    for u in usages {
        let root = u.cwd.as_deref().map(&mut *project).unwrap_or_default();
        let entry = sums.entry((u.day, root, u.model)).or_default();
        entry.0.add(&u.counts);
        entry.1 += 1;
    }
    sums.into_iter()
        .map(|((day, project, model), (c, requests))| Row {
            day,
            project,
            agent: agent.as_str().to_string(),
            model,
            input: c.input,
            output: c.output,
            cache_read: c.cache_read,
            cache_write: c.cache_write,
            requests,
        })
        .collect()
}

// ── Local days ────────────────────────────────────────────────────────────────

/// The local day (`YYYY-MM-DD`) of an instant, for a local offset in minutes east of UTC.
pub fn local_day(ms: i64, offset_minutes: i64) -> String {
    day_name((ms + offset_minutes * 60_000).div_euclid(DAY_MS))
}

/// `YYYY-MM-DD` of a day number since 1970-01-01 (H. Hinnant's `civil_from_days`).
fn day_name(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Minutes east of UTC of `local` (the wall clock at `utc_ms`), rounded to the quarter hour so
/// the moment between the two reads does not matter.
pub fn local_offset_minutes(local: &LocalTime, utc_ms: i64) -> i64 {
    let text = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        local.year, local.month, local.day, local.hour, local.minute, local.second
    );
    let Some(wall_ms) = parse_rfc3339_ms(&text) else { return 0 };
    let minutes = (wall_ms - utc_ms) as f64 / 60_000.0;
    ((minutes / 15.0).round() * 15.0) as i64
}

// ── Lines ─────────────────────────────────────────────────────────────────────

/// Calls `f` with every line that is valid UTF-8, streaming: a file is never held whole.
fn for_each_line(mut reader: impl BufRead, mut f: impl FnMut(&str)) -> io::Result<()> {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf)? == 0 {
            return Ok(());
        }
        if let Ok(line) = std::str::from_utf8(&buf) {
            f(line.trim_end_matches(['\r', '\n']));
        }
    }
}

/// The only keys of a Claude line that are read; serde skips every other key.
#[derive(Deserialize)]
struct ClaudeLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "isSidechain")]
    is_sidechain: Option<bool>,
    timestamp: Option<String>,
    cwd: Option<String>,
    message: Option<ClaudeMessage>,
}

#[derive(Deserialize)]
struct ClaudeMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<ClaudeUsage>,
}

#[derive(Deserialize)]
struct ClaudeUsage {
    input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

/// One counted Claude line: its message id (None when it has none) and its usage. None for
/// anything else: not JSON, not an assistant line, a sidechain, no usage or timestamp, a usage
/// that sums to 0 (an API error record).
fn claude_line(line: &str, offset_minutes: i64) -> Option<(Option<String>, Usage)> {
    // Most lines carry no usage at all; skip them before parsing.
    if !line.contains("\"usage\"") {
        return None;
    }
    let v: ClaudeLine = serde_json::from_str(line).ok()?;
    if v.kind.as_deref() != Some("assistant") || v.is_sidechain == Some(true) {
        return None;
    }
    let message = v.message?;
    let u = message.usage?;
    let counts = Counts {
        input: u.input_tokens.unwrap_or(0),
        output: u.output_tokens.unwrap_or(0),
        cache_read: u.cache_read_input_tokens.unwrap_or(0),
        cache_write: u.cache_creation_input_tokens.unwrap_or(0),
    };
    if counts.total() == 0 {
        return None;
    }
    let ms = parse_rfc3339_ms(v.timestamp.as_deref()?)?;
    let model = message.model.filter(|m| !m.is_empty()).unwrap_or_else(|| "unknown".to_string());
    Some((message.id, Usage { day: local_day(ms, offset_minutes), cwd: v.cwd, model, counts }))
}

/// The messages of one Claude transcript, in first-seen order. Inside the file a `message.id`
/// takes its last counted line; a line without an id counts on its own. Across files the report
/// keeps one message per id.
pub fn claude_messages(reader: impl BufRead, offset_minutes: i64, project: &mut dyn FnMut(&str) -> String) -> io::Result<Vec<Message>> {
    let mut usages: Vec<(Option<u64>, Usage)> = Vec::new();
    let mut by_id: HashMap<u64, usize> = HashMap::new();
    for_each_line(reader, |line| {
        let Some((id, usage)) = claude_line(line, offset_minutes) else { return };
        match id.as_deref().map(fnv1a) {
            Some(hash) => match by_id.get(&hash) {
                Some(&at) => usages[at].1 = usage,
                None => {
                    by_id.insert(hash, usages.len());
                    usages.push((Some(hash), usage));
                }
            },
            None => usages.push((None, usage)),
        }
    })?;
    Ok(usages
        .into_iter()
        .map(|(id, u)| Message {
            id,
            day: u.day,
            project: u.cwd.as_deref().map(&mut *project).unwrap_or_default(),
            model: u.model,
            input: u.counts.input,
            output: u.counts.output,
            cache_read: u.counts.cache_read,
            cache_write: u.counts.cache_write,
        })
        .collect())
}

/// The rows of one Codex rollout; nothing for a subagent's rollout (its session_meta names a
/// parent thread). The model of a record is the latest `turn_context.payload.model` before it,
/// else "codex"; the project is `session_meta.payload.cwd`.
pub fn codex_rows(reader: impl BufRead, offset_minutes: i64, project: &mut dyn FnMut(&str) -> String) -> io::Result<Vec<Row>> {
    let mut cwd: Option<String> = None;
    let mut subagent = false;
    let mut model: Option<String> = None;
    let mut usages: Vec<Usage> = Vec::new();
    for_each_line(reader, |line| {
        let wanted = ["\"token_usage_record\"", "\"turn_context\"", "\"session_meta\""];
        if !wanted.iter().any(|w| line.contains(w)) {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { return };
        match v.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                if let Some(meta) = parse_session_meta(line) {
                    subagent |= meta.subagent;
                    if cwd.is_none() {
                        cwd = meta.cwd;
                    }
                }
            }
            Some("turn_context") => {
                if let Some(m) = v.pointer("/payload/model").and_then(Value::as_str).filter(|m| !m.is_empty()) {
                    model = Some(m.to_string());
                }
            }
            Some("token_usage_record") => {
                let Some(u) = v.pointer("/payload/usage").and_then(Value::as_object) else { return };
                let Some(ms) = v.get("timestamp").and_then(Value::as_str).and_then(parse_rfc3339_ms) else { return };
                let n = |key: &str| u.get(key).and_then(Value::as_u64).unwrap_or(0);
                let cached = n("cached_input_tokens");
                usages.push(Usage {
                    day: local_day(ms, offset_minutes),
                    cwd: None,
                    model: model.clone().unwrap_or_else(|| "codex".to_string()),
                    counts: Counts {
                        input: n("input_tokens").saturating_sub(cached),
                        output: n("output_tokens"),
                        cache_read: cached,
                        cache_write: n("cache_write_input_tokens"),
                    },
                });
            }
            _ => {}
        }
    })?;
    if subagent {
        return Ok(Vec::new());
    }
    for u in &mut usages {
        u.cwd = cwd.clone();
    }
    Ok(aggregate(Agent::Codex, usages, project))
}

// ── Files and the cache ───────────────────────────────────────────────────────

/// Where the agents keep their files.
pub struct Roots {
    /// `<claude dir>\projects`
    pub claude_projects: PathBuf,
    /// `<codex home>\sessions`
    pub codex_sessions: PathBuf,
}

/// Every agent file under `roots`, sorted by path. A missing folder gives nothing.
pub fn list_files(roots: &Roots) -> Vec<(PathBuf, Agent)> {
    let mut files = Vec::new();
    for project in dir_entries(&roots.claude_projects) {
        if project.1 {
            for (file, is_dir) in dir_entries(&project.0) {
                if !is_dir && file.extension().is_some_and(|e| e == "jsonl") {
                    files.push((file, Agent::Claude));
                }
            }
        }
    }
    walk_rollouts(&roots.codex_sessions, 0, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// (path, is a real folder) of a folder's entries; symlinks count as files.
fn dir_entries(dir: &Path) -> Vec<(PathBuf, bool)> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    read.flatten()
        .filter_map(|e| {
            let kind = e.file_type().ok()?;
            Some((e.path(), kind.is_dir()))
        })
        .collect()
}

fn walk_rollouts(dir: &Path, depth: usize, out: &mut Vec<(PathBuf, Agent)>) {
    if depth > CODEX_MAX_DEPTH {
        return;
    }
    for (path, is_dir) in dir_entries(dir) {
        if is_dir {
            walk_rollouts(&path, depth + 1, out);
        } else {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                out.push((path, Agent::Codex));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub mtime_ms: i64,
    /// A Codex rollout's aggregates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Row>,
    /// A Claude transcript's messages (deduplicated across files by the report).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Message>,
}

/// `stats-cache.json`: numbers, dates, model names, project and agent-file paths only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cache {
    pub version: u32,
    /// The local offset the days were bucketed with; another offset starts the cache over.
    pub offset_minutes: i64,
    pub files: Vec<FileEntry>,
}

/// The cache on disk; empty when it is missing, unreadable or of another version.
pub fn load_cache(path: &Path) -> Cache {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Cache>(&bytes) {
            Ok(cache) if cache.version == CACHE_VERSION => cache,
            Ok(_) => Cache::default(),
            Err(err) => {
                log::line(format!("stats: cache unreadable ({:?}), counting again", err.classify()));
                Cache::default()
            }
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => Cache::default(),
        Err(err) => {
            log::line(format!("stats: cache unreadable ({:?}), counting again", err.kind()));
            Cache::default()
        }
    }
}

/// Writes the cache atomically: a temp file next to it, then a rename over it.
pub fn save_cache(path: &Path, cache: &Cache) -> io::Result<()> {
    let bytes = serde_json::to_vec(cache).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// What one refresh did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scan {
    /// Files read because they are new or their size or mtime changed.
    pub scanned: usize,
    /// Files whose aggregates came from the cache.
    pub cached: usize,
}

fn size_and_mtime(path: &Path) -> io::Result<(u64, i64)> {
    let meta = std::fs::metadata(path)?;
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64);
    Ok((meta.len(), mtime))
}

/// Brings `cache` up to `files`: an unchanged file keeps its aggregates, a new or changed one is
/// read again and its aggregates replaced whole, a file that is gone is dropped. A file that
/// cannot be read keeps what the cache had for it.
pub fn refresh(cache: &mut Cache, files: &[(PathBuf, Agent)], offset_minutes: i64, probe: impl Fn(&Path) -> bool) -> Scan {
    if cache.version != CACHE_VERSION || cache.offset_minutes != offset_minutes {
        *cache = Cache { version: CACHE_VERSION, offset_minutes, files: Vec::new() };
    }
    let mut old: HashMap<String, FileEntry> = std::mem::take(&mut cache.files).into_iter().map(|f| (f.path.clone(), f)).collect();
    let mut projects = ProjectCache::new();
    let mut project = |cwd: &str| projects.get(cwd, &probe).root;
    let mut scan = Scan { scanned: 0, cached: 0 };
    let mut failed: Option<io::ErrorKind> = None;
    let mut failures = 0usize;
    for (path, agent) in files {
        let key = path.to_string_lossy().into_owned();
        let previous = old.remove(&key);
        let (size, mtime_ms) = match size_and_mtime(path) {
            Ok(found) => found,
            Err(err) => {
                failures += 1;
                failed.get_or_insert(err.kind());
                cache.files.extend(previous);
                continue;
            }
        };
        if let Some(entry) = previous.as_ref().filter(|e| e.size == size && e.mtime_ms == mtime_ms) {
            cache.files.push(entry.clone());
            scan.cached += 1;
            continue;
        }
        let read = std::fs::File::open(path).and_then(|file| {
            let reader = BufReader::with_capacity(256 * 1024, file);
            match agent {
                Agent::Claude => claude_messages(reader, offset_minutes, &mut project).map(|m| (Vec::new(), m)),
                Agent::Codex => codex_rows(reader, offset_minutes, &mut project).map(|r| (r, Vec::new())),
            }
        });
        match read {
            Ok((rows, messages)) => {
                scan.scanned += 1;
                cache.files.push(FileEntry { path: key, size, mtime_ms, rows, messages });
            }
            Err(err) => {
                failures += 1;
                failed.get_or_insert(err.kind());
                cache.files.extend(previous);
            }
        }
    }
    if let Some(kind) = failed {
        log::line(format!("stats: {failures} agent files could not be read ({kind:?})"));
    }
    cache.files.sort_by(|a, b| a.path.cmp(&b.path));
    scan
}

// ── The report ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DayTotal {
    pub day: String,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectTotal {
    pub project: String,
    pub name: String,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModel {
    pub agent: String,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub requests: u64,
    pub total: u64,
}

/// What `stats_query` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsReport {
    /// Every day of the period, oldest first, today last; 0 for a day without use.
    pub days: Vec<DayTotal>,
    /// Projects by total, largest first.
    pub projects: Vec<ProjectTotal>,
    /// (agent, model) pairs by total, largest first.
    pub agents: Vec<AgentModel>,
    pub total: Totals,
    pub scanned_files: usize,
    pub from_cache: usize,
}

/// The period of `stats_query`: 7 or 30 days, nothing else.
pub fn period(days: u32) -> Result<u32, String> {
    match days {
        7 | 30 => Ok(days),
        _ => Err("days must be 7 or 30".to_string()),
    }
}

/// Every row of the cached files: a Codex file's rows, and one row per Claude message, where a
/// `message.id` seen in several transcripts counts once, from the first file in path order.
pub fn file_rows(files: &[FileEntry]) -> Vec<Row> {
    let mut order: Vec<&FileEntry> = files.iter().collect();
    order.sort_by(|a, b| a.path.cmp(&b.path));
    let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let mut rows = Vec::new();
    for file in order {
        rows.extend(file.rows.iter().cloned());
        rows.extend(file.messages.iter().filter(|m| m.id.is_none_or(|id| seen.insert(id))).map(Message::row));
    }
    rows
}

/// The report over the `days` local days that end today.
pub fn report<'a>(rows: impl IntoIterator<Item = &'a Row>, days: u32, now_ms: i64, offset_minutes: i64, scan: Scan) -> StatsReport {
    let today = (now_ms + offset_minutes * 60_000).div_euclid(DAY_MS);
    let names: Vec<String> = (0..i64::from(days)).rev().map(|back| day_name(today - back)).collect();
    let mut by_day: BTreeMap<&str, u64> = names.iter().map(|d| (d.as_str(), 0)).collect();
    // Grouped by the lowercase root (one folder, any spelling); the first spelling is shown.
    let mut by_project: HashMap<String, (String, u64)> = HashMap::new();
    let mut by_model: BTreeMap<(String, String), Counts> = BTreeMap::new();
    let mut total = Totals::default();
    for row in rows {
        let Some(day_total) = by_day.get_mut(row.day.as_str()) else { continue };
        let counts = row.counts();
        *day_total = day_total.saturating_add(counts.total());
        let project = by_project.entry(row.project.to_lowercase()).or_insert_with(|| (row.project.clone(), 0));
        project.1 = project.1.saturating_add(counts.total());
        by_model.entry((row.agent.clone(), row.model.clone())).or_default().add(&counts);
        total.input = total.input.saturating_add(counts.input);
        total.output = total.output.saturating_add(counts.output);
        total.cache_read = total.cache_read.saturating_add(counts.cache_read);
        total.cache_write = total.cache_write.saturating_add(counts.cache_write);
        total.requests = total.requests.saturating_add(row.requests);
        total.total = total.total.saturating_add(counts.total());
    }
    let mut projects: Vec<ProjectTotal> = by_project
        .into_values()
        .map(|(project, total)| {
            let name = if project.is_empty() { "Other".to_string() } else { basename(&project) };
            ProjectTotal { project, name, total }
        })
        .collect();
    projects.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.project.cmp(&b.project)));
    let mut agents: Vec<(u64, AgentModel)> = by_model
        .into_iter()
        .map(|((agent, model), c)| {
            (c.total(), AgentModel { agent, model, input: c.input, output: c.output, cache_read: c.cache_read, cache_write: c.cache_write })
        })
        .collect();
    agents.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| (&a.1.agent, &a.1.model).cmp(&(&b.1.agent, &b.1.model))));
    StatsReport {
        days: by_day.into_iter().map(|(day, total)| DayTotal { day: day.to_string(), total }).collect(),
        projects,
        agents: agents.into_iter().map(|(_, a)| a).collect(),
        total,
        scanned_files: scan.scanned,
        from_cache: scan.cached,
    }
}

/// One scan at a time: two quick period switches must not write the cache over each other.
static SCAN: Mutex<()> = Mutex::new(());

/// Loads the cache, refreshes it from the agents' files, saves it and builds the report. Blocking:
/// the caller runs it on its own thread.
pub fn query(
    days: u32,
    roots: &Roots,
    cache_path: &Path,
    now_ms: i64,
    offset_minutes: i64,
    probe: impl Fn(&Path) -> bool,
) -> Result<StatsReport, String> {
    let days = period(days)?;
    let _one = SCAN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut cache = load_cache(cache_path);
    let before = cache.files.len();
    let scan = refresh(&mut cache, &list_files(roots), offset_minutes, probe);
    // Saved when a file was read again or an entry was dropped (a file that is gone).
    if scan.scanned > 0 || cache.files.len() != before {
        if let Err(err) = save_cache(cache_path, &cache) {
            log::line(format!("stats: cache not saved ({:?})", err.kind()));
        }
    }
    Ok(report(&file_rows(&cache.files), days, now_ms, offset_minutes, scan))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::wp;
    use crate::testutil::TempDir;
    use serde_json::json;

    /// No folder is a git repository: the project is the folder itself, the disk is never probed.
    fn no_git(_: &Path) -> bool {
        false
    }

    fn root_of(cwd: &str) -> String {
        crate::observe::paths::project_of(cwd, no_git).root
    }

    fn claude(id: &str, ts: &str, input: u64, output: u64, cache_read: u64, cache_write: u64) -> Value {
        json!({
            "parentUuid": "p", "isSidechain": false, "type": "assistant", "cwd": wp(r"C:\work\demo"),
            "sessionId": "s", "timestamp": ts,
            "message": {
                "id": id, "model": "claude-opus-4", "role": "assistant",
                "content": [{ "type": "text", "text": "generic" }],
                "usage": {
                    "input_tokens": input, "cache_read_input_tokens": cache_read,
                    "cache_creation_input_tokens": cache_write, "output_tokens": output,
                    "server_tool_use": { "web_search_requests": 0 }
                }
            }
        })
    }

    fn lines(values: &[Value]) -> String {
        values.iter().map(|v| v.to_string() + "\n").collect()
    }

    /// Rows summed by (day, project, agent, model), in key order.
    fn summed(rows: Vec<Row>) -> Vec<Row> {
        let mut sums: BTreeMap<(String, String, String, String), Row> = BTreeMap::new();
        for r in rows {
            let key = (r.day.clone(), r.project.clone(), r.agent.clone(), r.model.clone());
            match sums.get_mut(&key) {
                Some(s) => {
                    s.input += r.input;
                    s.output += r.output;
                    s.cache_read += r.cache_read;
                    s.cache_write += r.cache_write;
                    s.requests += r.requests;
                }
                None => {
                    sums.insert(key, r);
                }
            }
        }
        sums.into_values().collect()
    }

    fn entry(path: &str, messages: Vec<Message>) -> FileEntry {
        FileEntry { path: path.into(), size: 0, mtime_ms: 0, rows: Vec::new(), messages }
    }

    /// One transcript's messages as summed rows.
    fn claude_bytes(bytes: &[u8], offset: i64) -> Vec<Row> {
        let messages = claude_messages(bytes, offset, &mut |c| root_of(c)).unwrap();
        summed(file_rows(&[entry("t.jsonl", messages)]))
    }

    fn claude_of(text: &str, offset: i64) -> Vec<Row> {
        claude_bytes(text.as_bytes(), offset)
    }

    fn codex_of(text: &str, offset: i64) -> Vec<Row> {
        codex_rows(text.as_bytes(), offset, &mut |c| root_of(c)).unwrap()
    }

    fn row(day: &str, project: &str, agent: &str, model: &str, c: [u64; 4], requests: u64) -> Row {
        Row {
            day: day.into(),
            project: project.into(),
            agent: agent.into(),
            model: model.into(),
            input: c[0],
            output: c[1],
            cache_read: c[2],
            cache_write: c[3],
            requests,
        }
    }

    #[test]
    fn a_claude_message_counts_once_from_its_last_line() {
        // Claude Code writes one line per content block; each repeats the message's usage, the
        // last with the final output count.
        let text = lines(&[
            claude("msg_1", "2026-10-06T10:00:00.000Z", 3, 1, 100, 50),
            claude("msg_1", "2026-10-06T10:00:01.000Z", 3, 1, 100, 50),
            claude("msg_1", "2026-10-06T10:00:02.000Z", 3, 40, 100, 50),
            claude("msg_2", "2026-10-06T10:01:00.000Z", 5, 7, 200, 0),
        ]);
        assert_eq!(
            claude_of(&text, 0),
            vec![row("2026-10-06", &wp(r"C:\work\demo"), "claude", "claude-opus-4", [8, 47, 300, 50], 2)]
        );
    }

    #[test]
    fn claude_sidechains_zero_usage_and_other_lines_are_skipped() {
        let mut side = claude("msg_s", "2026-10-06T10:00:00Z", 9, 9, 9, 9);
        side["isSidechain"] = json!(true);
        let zero = claude("msg_z", "2026-10-06T10:00:00Z", 0, 0, 0, 0);
        let mut user = claude("msg_u", "2026-10-06T10:00:00Z", 1, 1, 1, 1);
        user["type"] = json!("user");
        let no_time = {
            let mut v = claude("msg_t", "x", 1, 1, 1, 1);
            v.as_object_mut().unwrap().remove("timestamp");
            v
        };
        let counted = claude("msg_ok", "2026-10-06T10:00:00Z", 1, 2, 0, 0);
        // A zero line of a counted id (an error retry) does not replace its usage.
        let zero_again = claude("msg_ok", "2026-10-06T10:00:05Z", 0, 0, 0, 0);
        let text = lines(&[side, zero, user, no_time, counted, zero_again]);
        assert_eq!(claude_of(&text, 0), vec![row("2026-10-06", &wp(r"C:\work\demo"), "claude", "claude-opus-4", [1, 2, 0, 0], 1)]);
    }

    #[test]
    fn corrupt_lines_are_skipped() {
        let good = claude("msg_1", "2026-10-06T10:00:00Z", 1, 2, 3, 4).to_string();
        let torn = &good[..good.len() / 2];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"not json \"usage\"\n");
        bytes.extend_from_slice(torn.as_bytes());
        bytes.push(b'\n');
        bytes.extend_from_slice(b"{\"usage\": \xff\xfe}\n");
        bytes.extend_from_slice(good.as_bytes());
        bytes.extend_from_slice(b"\r\n");
        bytes.extend_from_slice(torn.as_bytes()); // a last line still being written
        let rows = claude_bytes(&bytes, 0);
        assert_eq!(rows, vec![row("2026-10-06", &wp(r"C:\work\demo"), "claude", "claude-opus-4", [1, 2, 3, 4], 1)]);

        let codex = format!(
            "{}\n{{\"type\":\"token_usage_record\",\"payload\":\n{}\n",
            codex_meta(None),
            json!({ "timestamp": "2026-10-06T10:00:00Z", "type": "token_usage_record", "payload": { "usage": { "input_tokens": 10, "output_tokens": 1 } } })
        );
        assert_eq!(codex_of(&codex, 0), vec![row("2026-10-06", &wp(r"C:\work\codex"), "codex", "codex", [10, 1, 0, 0], 1)]);
    }

    #[test]
    fn a_claude_line_without_an_id_counts_on_its_own_and_a_missing_model_is_unknown() {
        let mut a = claude("x", "2026-10-06T10:00:00Z", 1, 0, 0, 0);
        a["message"].as_object_mut().unwrap().remove("id");
        a["message"].as_object_mut().unwrap().remove("model");
        let text = lines(&[a.clone(), a]);
        assert_eq!(claude_of(&text, 0), vec![row("2026-10-06", &wp(r"C:\work\demo"), "claude", "unknown", [2, 0, 0, 0], 2)]);
    }

    fn codex_meta(parent: Option<&str>) -> Value {
        let mut payload = json!({ "id": "11111111-1111-4111-8111-111111111111", "cwd": wp(r"C:\work\codex"), "originator": "codex_vscode" });
        if let Some(p) = parent {
            payload["source"] = json!({ "subagent": { "thread_spawn": { "parent_thread_id": p } } });
        }
        json!({ "timestamp": "2026-10-06T09:00:00Z", "type": "session_meta", "payload": payload })
    }

    fn codex_turn(model: &str) -> Value {
        json!({ "timestamp": "2026-10-06T09:00:01Z", "type": "turn_context", "payload": { "cwd": wp(r"C:\work\codex"), "model": model } })
    }

    fn codex_usage(ts: &str, input: u64, cached: u64, write: u64, output: u64) -> Value {
        json!({
            "ordinal": 3, "timestamp": ts, "type": "token_usage_record",
            "payload": {
                "turn_id": "u",
                "usage": {
                    "input_tokens": input, "cached_input_tokens": cached, "cache_write_input_tokens": write,
                    "output_tokens": output, "reasoning_output_tokens": 2, "total_tokens": input + output
                },
                "thread_token_usage": { "total_tokens": 999_999 }
            }
        })
    }

    #[test]
    fn codex_input_excludes_the_cached_part_and_the_model_comes_from_the_turn() {
        let text = lines(&[
            codex_meta(None),
            codex_usage("2026-10-06T09:00:00Z", 1_000, 900, 0, 10),
            codex_turn("gpt-5-codex"),
            codex_usage("2026-10-06T09:01:00Z", 2_000, 1_500, 30, 20),
            codex_usage("2026-10-06T09:02:00Z", 100, 0, 0, 5),
            codex_turn("gpt-5.1"),
            // cached above input never goes below zero
            codex_usage("2026-10-06T09:03:00Z", 10, 50, 0, 1),
        ]);
        assert_eq!(
            codex_of(&text, 0),
            vec![
                row("2026-10-06", &wp(r"C:\work\codex"), "codex", "codex", [100, 10, 900, 0], 1),
                row("2026-10-06", &wp(r"C:\work\codex"), "codex", "gpt-5-codex", [600, 25, 1_500, 30], 2),
                row("2026-10-06", &wp(r"C:\work\codex"), "codex", "gpt-5.1", [0, 1, 50, 0], 1),
            ]
        );
    }

    #[test]
    fn a_codex_subagent_rollout_counts_nothing() {
        let text = lines(&[codex_meta(Some("22222222-2222-4222-8222-222222222222")), codex_usage("2026-10-06T09:00:00Z", 10, 0, 0, 1)]);
        assert!(codex_of(&text, 0).is_empty());
    }

    #[test]
    fn days_are_local_across_midnight() {
        assert_eq!(local_day(parse_rfc3339_ms("2026-10-06T21:30:00Z").unwrap(), 0), "2026-10-06");
        assert_eq!(local_day(parse_rfc3339_ms("2026-10-06T21:30:00Z").unwrap(), 180), "2026-10-07");
        assert_eq!(local_day(parse_rfc3339_ms("2026-10-07T00:30:00Z").unwrap(), -120), "2026-10-06");
        assert_eq!(local_day(parse_rfc3339_ms("2024-02-29T23:59:59Z").unwrap(), 0), "2024-02-29");
        assert_eq!(local_day(parse_rfc3339_ms("2026-12-31T23:00:00Z").unwrap(), 60), "2027-01-01");
        assert_eq!(local_day(-1, 0), "1969-12-31");
        // The same two Claude requests land on one day or two depending on the offset.
        let text = lines(&[claude("a", "2026-10-06T20:30:00Z", 1, 0, 0, 0), claude("b", "2026-10-06T21:30:00Z", 2, 0, 0, 0)]);
        let days = |offset| claude_of(&text, offset).into_iter().map(|r| (r.day, r.input)).collect::<Vec<_>>();
        assert_eq!(days(0), vec![("2026-10-06".to_string(), 3)]);
        assert_eq!(days(180), vec![("2026-10-06".to_string(), 1), ("2026-10-07".to_string(), 2)]);
    }

    #[test]
    fn the_local_offset_is_the_wall_clock_minus_utc_to_the_quarter_hour() {
        let utc = parse_rfc3339_ms("2026-10-06T21:30:00Z").unwrap();
        let wall = |y, mo, d, h, mi, s| LocalTime { year: y, month: mo, day: d, hour: h, minute: mi, second: s };
        assert_eq!(local_offset_minutes(&wall(2026, 10, 7, 0, 30, 0), utc), 180);
        assert_eq!(local_offset_minutes(&wall(2026, 10, 7, 0, 30, 2), utc + 1_500), 180, "the reads a moment apart");
        assert_eq!(local_offset_minutes(&wall(2026, 10, 6, 16, 30, 0), utc), -300);
        assert_eq!(local_offset_minutes(&wall(2026, 10, 7, 3, 15, 0), utc), 345, "Nepal");
        assert_eq!(local_offset_minutes(&wall(2026, 10, 6, 21, 29, 59), utc), 0);
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn rig() -> (TempDir, Roots) {
        let tmp = TempDir::new("stats");
        let roots = Roots { claude_projects: tmp.path.join("claude").join("projects"), codex_sessions: tmp.path.join("codex").join("sessions") };
        (tmp, roots)
    }

    #[test]
    fn only_transcripts_and_rollouts_are_listed() {
        let (_tmp, roots) = rig();
        let p = roots.claude_projects.join("C--work-demo");
        write(&p.join("a.jsonl"), "");
        write(&p.join("notes.txt"), "");
        write(&p.join("a").join("subagents").join("agent-1.jsonl"), "");
        write(&roots.claude_projects.join("top.jsonl"), "");
        let day = roots.codex_sessions.join("2026").join("10").join("06");
        write(&day.join("rollout-2026-10-06T09-00-00-x.jsonl"), "");
        write(&day.join("other.jsonl"), "");
        let files: Vec<(String, Agent)> = list_files(&roots)
            .into_iter()
            .map(|(p, a)| (p.file_name().unwrap().to_string_lossy().into_owned(), a))
            .collect();
        assert_eq!(files, vec![("a.jsonl".into(), Agent::Claude), ("rollout-2026-10-06T09-00-00-x.jsonl".into(), Agent::Codex)]);
        assert!(list_files(&Roots { claude_projects: roots.claude_projects.join("none"), codex_sessions: roots.codex_sessions.join("none") }).is_empty());
    }

    fn totals(cache: &Cache) -> u64 {
        file_rows(&cache.files).iter().map(|r| r.counts().total()).sum()
    }

    #[test]
    fn the_cache_rereads_only_changed_files_and_drops_missing_ones() {
        let (_tmp, roots) = rig();
        let transcript = roots.claude_projects.join("C--work-demo").join("s.jsonl");
        let rollout = roots.codex_sessions.join("2026").join("10").join("06").join("rollout-x.jsonl");
        write(&transcript, &lines(&[claude("m1", "2026-10-06T10:00:00Z", 1, 2, 3, 4)]));
        write(&rollout, &lines(&[codex_meta(None), codex_usage("2026-10-06T10:00:00Z", 100, 40, 0, 5)]));

        let mut cache = Cache::default();
        assert_eq!(refresh(&mut cache, &list_files(&roots), 0, no_git), Scan { scanned: 2, cached: 0 });
        assert_eq!(totals(&cache), 10 + 105);

        // Nothing changed: nothing is read again.
        assert_eq!(refresh(&mut cache, &list_files(&roots), 0, no_git), Scan { scanned: 0, cached: 2 });
        assert_eq!(totals(&cache), 115);

        // The transcript grew (its message streamed on, and a new one came): only it is read
        // again, and its aggregates are replaced, not added to.
        write(
            &transcript,
            &lines(&[claude("m1", "2026-10-06T10:00:00Z", 1, 2, 3, 4), claude("m1", "2026-10-06T10:00:01Z", 1, 20, 3, 4), claude("m2", "2026-10-06T10:05:00Z", 1, 1, 0, 0)]),
        );
        assert_eq!(refresh(&mut cache, &list_files(&roots), 0, no_git), Scan { scanned: 1, cached: 1 });
        assert_eq!(totals(&cache), 28 + 2 + 105);

        // The rollout is gone: its entry goes with it.
        std::fs::remove_file(&rollout).unwrap();
        assert_eq!(refresh(&mut cache, &list_files(&roots), 0, no_git), Scan { scanned: 0, cached: 1 });
        assert_eq!(cache.files.len(), 1);
        assert_eq!(totals(&cache), 30);

        // Another local offset buckets the days differently: everything is counted again.
        assert_eq!(refresh(&mut cache, &list_files(&roots), 60, no_git), Scan { scanned: 1, cached: 0 });
        assert_eq!(totals(&cache), 30);
    }

    #[test]
    fn a_claude_message_copied_into_another_transcript_counts_once() {
        // A resumed or forked session copies earlier messages into its new transcript.
        let (tmp, roots) = rig();
        let mut copy = claude("msg_1", "2026-10-06T10:00:00Z", 1, 2, 3, 4);
        copy["cwd"] = json!(wp(r"C:\work\two"));
        write(
            &roots.claude_projects.join("p1").join("a.jsonl"),
            &lines(&[claude("msg_1", "2026-10-06T10:00:00Z", 1, 2, 3, 4), claude("msg_2", "2026-10-06T10:01:00Z", 10, 0, 0, 0)]),
        );
        write(&roots.claude_projects.join("p2").join("b.jsonl"), &lines(&[copy, claude("msg_3", "2026-10-06T11:00:00Z", 100, 0, 0, 0)]));
        let now = parse_rfc3339_ms("2026-10-07T12:00:00Z").unwrap();
        let r = query(7, &roots, &tmp.path.join("stats-cache.json"), now, 0, no_git).unwrap();
        assert_eq!(r.total.total, 10 + 10 + 100, "msg_1 once");
        assert_eq!(r.total.requests, 3);
        // The first file in path order keeps it: its project (demo), not the copy's (two).
        assert_eq!(r.projects, vec![ProjectTotal { project: wp(r"C:\work\demo"), name: "demo".into(), total: 120 }]);
        // The order of the cache entries does not matter.
        let a = entry("a", vec![Message { id: Some(fnv1a("msg_1")), day: "d".into(), project: "A".into(), model: "m".into(), input: 1, output: 0, cache_read: 0, cache_write: 0 }]);
        let b = entry("b", vec![Message { project: "B".into(), ..a.messages[0].clone() }]);
        let rows = file_rows(&[b.clone(), a.clone()]);
        assert_eq!(rows.iter().map(|r| r.project.as_str()).collect::<Vec<_>>(), vec!["A"]);
        // Lines without an id are never merged.
        let none = entry("c", vec![Message { id: None, ..a.messages[0].clone() }; 2]);
        assert_eq!(file_rows(&[none]).len(), 2);
    }

    #[test]
    fn message_ids_hash_with_fnv_1a() {
        assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(fnv1a("msg_01"), fnv1a("msg_02"));
    }

    #[test]
    fn a_deleted_file_alone_saves_the_cache() {
        let (tmp, roots) = rig();
        let gone = roots.claude_projects.join("p").join("gone.jsonl");
        write(&roots.claude_projects.join("p").join("kept.jsonl"), &lines(&[claude("m1", "2026-10-06T10:00:00Z", 1, 0, 0, 0)]));
        write(&gone, &lines(&[claude("m2", "2026-10-06T10:00:00Z", 2, 0, 0, 0)]));
        let cache_path = tmp.path.join("stats-cache.json");
        let now = parse_rfc3339_ms("2026-10-07T12:00:00Z").unwrap();
        assert_eq!(query(7, &roots, &cache_path, now, 0, no_git).unwrap().total.total, 3);
        assert_eq!(load_cache(&cache_path).files.len(), 2);
        std::fs::remove_file(&gone).unwrap();
        let r = query(7, &roots, &cache_path, now, 0, no_git).unwrap();
        assert_eq!((r.scanned_files, r.from_cache, r.total.total), (0, 1, 1));
        assert_eq!(load_cache(&cache_path).files.len(), 1, "the dropped entry is saved");
    }

    #[test]
    fn the_cache_survives_a_save_and_load_and_a_broken_one_starts_over() {
        let (tmp, roots) = rig();
        write(&roots.claude_projects.join("p").join("s.jsonl"), &lines(&[claude("m1", "2026-10-06T10:00:00Z", 1, 2, 3, 4)]));
        let mut cache = Cache::default();
        refresh(&mut cache, &list_files(&roots), 0, no_git);
        let path = tmp.path.join("stats-cache.json");
        save_cache(&path, &cache).unwrap();
        assert!(!path.with_extension("json.tmp").exists(), "the temp file is renamed over the cache");
        assert_eq!(load_cache(&path), cache);
        // Only numbers, days, models and paths are in it: none of the line's text.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("generic") && !text.contains("msg") && text.contains("claude-opus-4"));
        save_cache(&path, &cache).unwrap();
        assert_eq!(load_cache(&path), cache, "a second save replaces the first");

        std::fs::write(&path, b"{\"version\":2,\"files\":[").unwrap();
        assert_eq!(load_cache(&path), Cache::default());
        // A cache of the first format (per-file Claude aggregates) is rebuilt.
        std::fs::write(&path, br#"{"version":1,"offsetMinutes":0,"files":[{"path":"x","size":1,"mtimeMs":1,"rows":[]}]}"#).unwrap();
        assert_eq!(load_cache(&path), Cache::default());
        assert_eq!(load_cache(&tmp.path.join("absent.json")), Cache::default());
    }

    #[test]
    fn the_report_covers_the_period_ending_today() {
        let rows = vec![
            row("2026-09-30", &wp(r"C:\work\old"), "claude", "claude-opus-4", [1_000, 0, 0, 0], 1),
            row("2026-10-01", &wp(r"C:\work\demo"), "claude", "claude-opus-4", [10, 20, 30, 40], 2),
            row("2026-10-07", &wp(r"C:\Work\Demo"), "codex", "gpt-5-codex", [5, 5, 0, 0], 1),
            row("2026-10-07", &wp(r"C:\work\app"), "claude", "claude-sonnet-4", [1, 1, 1, 1], 1),
            row("2026-10-07", "", "claude", "claude-opus-4", [0, 1, 0, 0], 1),
            row("2026-10-08", &wp(r"C:\work\future"), "claude", "claude-opus-4", [9_999, 0, 0, 0], 1),
        ];
        // 2026-10-07T10:00 local at +03:00
        let now = parse_rfc3339_ms("2026-10-07T07:00:00Z").unwrap();
        let r = report(&rows, 7, now, 180, Scan { scanned: 3, cached: 4 });
        assert_eq!(r.days.len(), 7);
        assert_eq!(r.days.first().unwrap(), &DayTotal { day: "2026-10-01".into(), total: 100 });
        assert_eq!(r.days.last().unwrap(), &DayTotal { day: "2026-10-07".into(), total: 15 });
        assert!(r.days[1..6].iter().all(|d| d.total == 0));
        assert_eq!(
            r.projects,
            vec![
                ProjectTotal { project: wp(r"C:\work\demo"), name: "demo".into(), total: 110 },
                ProjectTotal { project: wp(r"C:\work\app"), name: "app".into(), total: 4 },
                ProjectTotal { project: "".into(), name: "Other".into(), total: 1 },
            ]
        );
        assert_eq!(
            r.agents,
            vec![
                AgentModel { agent: "claude".into(), model: "claude-opus-4".into(), input: 10, output: 21, cache_read: 30, cache_write: 40 },
                AgentModel { agent: "codex".into(), model: "gpt-5-codex".into(), input: 5, output: 5, cache_read: 0, cache_write: 0 },
                AgentModel { agent: "claude".into(), model: "claude-sonnet-4".into(), input: 1, output: 1, cache_read: 1, cache_write: 1 },
            ]
        );
        assert_eq!(r.total, Totals { input: 16, output: 27, cache_read: 31, cache_write: 41, requests: 5, total: 115 });
        assert_eq!((r.scanned_files, r.from_cache), (3, 4));

        let month = report(&rows, 30, now, 180, Scan { scanned: 0, cached: 0 });
        assert_eq!(month.days.len(), 30);
        assert_eq!(month.days[0].day, "2026-09-08");
        assert_eq!(month.total.total, 1_115);
    }

    #[test]
    fn the_report_serializes_with_the_wire_names() {
        let r = report(&[row("2026-10-07", &wp(r"C:\w\a"), "codex", "m", [1, 2, 3, 4], 1)], 7, parse_rfc3339_ms("2026-10-07T12:00:00Z").unwrap(), 0, Scan { scanned: 1, cached: 0 });
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["agents"][0], json!({ "agent": "codex", "model": "m", "input": 1, "output": 2, "cacheRead": 3, "cacheWrite": 4 }));
        assert_eq!(v["projects"][0], json!({ "project": wp(r"C:\w\a"), "name": "a", "total": 10 }));
        assert_eq!(v["days"][6], json!({ "day": "2026-10-07", "total": 10 }));
        assert_eq!(v["total"], json!({ "input": 1, "output": 2, "cacheRead": 3, "cacheWrite": 4, "requests": 1, "total": 10 }));
        assert_eq!((v["scannedFiles"].clone(), v["fromCache"].clone()), (json!(1), json!(0)));
    }

    #[test]
    fn only_seven_and_thirty_days_are_asked() {
        assert_eq!(period(7), Ok(7));
        assert_eq!(period(30), Ok(30));
        for bad in [0, 1, 8, 31, 365] {
            assert!(period(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_query_scans_saves_and_reports() {
        let (tmp, roots) = rig();
        write(&roots.claude_projects.join("p").join("s.jsonl"), &lines(&[claude("m1", "2026-10-06T10:00:00Z", 1, 2, 3, 4)]));
        let cache_path = tmp.path.join("stats-cache.json");
        let now = parse_rfc3339_ms("2026-10-07T12:00:00Z").unwrap();
        let first = query(7, &roots, &cache_path, now, 0, no_git).unwrap();
        assert_eq!((first.scanned_files, first.from_cache, first.total.total), (1, 0, 10));
        assert!(cache_path.exists());
        let second = query(30, &roots, &cache_path, now, 0, no_git).unwrap();
        assert_eq!((second.scanned_files, second.from_cache, second.total.total), (0, 1, 10));
        assert!(query(14, &roots, &cache_path, now, 0, no_git).is_err());
    }
}
