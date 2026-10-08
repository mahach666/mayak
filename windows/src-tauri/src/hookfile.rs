// Write protection shared by every tool config Mayak may edit (spec §8):
// ~/.codex/hooks.json (the optional Codex integration) and ~/.claude/settings.json
// (answers from the island for Claude: permissions and questions, under Settings →
// "Answers from the island"), plus the Codex answers switch in hooks.json.
//
// The rules, for both targets:
// - the diff is computed between the current raw bytes and the exact bytes that
//   will be written; a missing file is shown whole;
// - the preview's plan (those bytes, the fingerprint of the file they were made
//   from, the backup path) stays here in Rust; the webview hands back only a
//   target and a fingerprint, never a path or content;
// - the file's own format is kept: BOM, CRLF/LF, final newline;
// - right before writing, the file is read again and must still match the
//   fingerprint; then a byte-exact backup, a flushed temp file next to the
//   target, and a rename over it, so a crash leaves the original intact.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::message::Message;
use crate::{ident, platform};

const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Claude,
    Codex,
}

impl Target {
    /// No target means "claude": that is what the commands did before targets existed.
    pub fn parse(raw: Option<&str>) -> Result<Target, Message> {
        match raw {
            None | Some("claude") => Ok(Target::Claude),
            Some("codex") => Ok(Target::Codex),
            Some(other) => Err(Message::new(
                "msg.hook.unknownTarget",
                format!("unknown target \"{other}\" (expected \"claude\" or \"codex\")"),
            )
            .with("target", other)),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Target::Claude => "claude",
            Target::Codex => "codex",
        }
    }
}

/// How an existing file is laid out, so a rewrite keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFormat {
    pub bom: bool,
    /// Line endings to write: the style of the file's first line ending.
    pub crlf: bool,
    pub final_newline: bool,
    /// The file has both "\r\n" and bare "\n"; the rewrite uses `crlf` everywhere.
    pub mixed_eol: bool,
}

impl TextFormat {
    /// A file created from scratch: no BOM, LF, final newline (D28).
    pub const NEW_FILE: TextFormat =
        TextFormat { bom: false, crlf: false, final_newline: true, mixed_eol: false };
}

pub fn detect_format(bytes: &[u8]) -> TextFormat {
    let bom = bytes.starts_with(&BOM);
    let body = strip_bom(bytes);
    let mut crlf = None;
    let (mut has_crlf, mut has_lf) = (false, false);
    for (i, &b) in body.iter().enumerate() {
        if b != b'\n' {
            continue;
        }
        let is_crlf = i > 0 && body[i - 1] == b'\r';
        crlf.get_or_insert(is_crlf);
        if is_crlf {
            has_crlf = true;
        } else {
            has_lf = true;
        }
    }
    TextFormat {
        bom,
        crlf: crlf.unwrap_or(false),
        final_newline: body.ends_with(b"\n"),
        mixed_eol: has_crlf && has_lf,
    }
}

/// One leading UTF-8 BOM, which PowerShell's `Set-Content -Encoding utf8` writes
/// and serde_json refuses.
pub fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&BOM).unwrap_or(bytes)
}

/// The config as a JSON object. Empty or whitespace-only means "start from
/// nothing"; anything else that is not a JSON object is refused, because
/// treating somebody's unreadable config as empty and writing that back over
/// it would destroy it.
pub fn parse_object(bytes: &[u8], path: &str) -> Result<Value, Message> {
    let text = strip_bom(bytes);
    if text.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Object(Map::new()));
    }
    let app = ident::APP_NAME;
    match serde_json::from_slice::<Value>(text) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(Message::new("msg.hook.notObject", format!("{path} isn't a JSON object — {app} won't touch it."))
            .with("path", path)
            .with("app", app)),
        Err(err) => Err(Message::new(
            "msg.hook.invalidJson",
            format!("{path} isn't valid JSON ({err}). Fix or move it, then try again — {app} won't overwrite it."),
        )
        .with("path", path)
        .with("err", err.to_string())
        .with("app", app)),
    }
}

/// Pretty JSON (2 spaces, key order kept) in the given format.
pub fn render(value: &Value, fmt: TextFormat) -> Vec<u8> {
    // serde_json escapes newlines inside strings, so every '\n' it emits is a
    // line break and can be rewritten as one.
    let pretty = serde_json::to_vec_pretty(value).expect("serializing a serde_json::Value cannot fail");
    let eol: &[u8] = if fmt.crlf { b"\r\n" } else { b"\n" };
    let mut out = Vec::with_capacity(pretty.len() + pretty.len() / 16 + 8);
    if fmt.bom {
        out.extend_from_slice(&BOM);
    }
    for &b in &pretty {
        if b == b'\n' {
            out.extend_from_slice(eol);
        } else {
            out.push(b);
        }
    }
    if fmt.final_newline {
        out.extend_from_slice(eol);
    }
    out
}

/// Identifies the exact bytes a preview was computed from. FNV-1a is plenty:
/// the question is only "is this still the file I showed the user?". A missing
/// file has the fingerprint of no bytes.
pub fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Configs are short, so a plain O(n·m) LCS is the simplest honest diff.
pub fn unified_diff(before: &str, after: &str) -> String {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let (n, m) = (a.len(), b.len());

    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut out: Vec<String> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(format!("  {}", a[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(format!("- {}", a[i]));
            i += 1;
        } else {
            out.push(format!("+ {}", b[j]));
            j += 1;
        }
    }
    while i < n {
        out.push(format!("- {}", a[i]));
        i += 1;
    }
    while j < m {
        out.push(format!("+ {}", b[j]));
        j += 1;
    }

    // Keep three lines of context around each change so the panel stays readable.
    let changed: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with('+') || l.starts_with('-'))
        .map(|(i, _)| i)
        .collect();
    if changed.is_empty() {
        return "No change.".into();
    }
    let mut keep = vec![false; out.len()];
    for idx in changed {
        let lo = idx.saturating_sub(3);
        let hi = (idx + 4).min(out.len());
        for k in lo..hi {
            keep[k] = true;
        }
    }
    let mut result = String::new();
    let mut gap = false;
    for (idx, line) in out.iter().enumerate() {
        if keep[idx] {
            result.push_str(line);
            result.push('\n');
            gap = false;
        } else if !gap {
            result.push_str("  …\n");
            gap = true;
        }
    }
    result
}

/// The raw bytes, or None when the file does not exist. Any other failure — a
/// lock, a permission problem — is an error: not knowing is not the same as empty.
pub fn read_config(path: &Path) -> Result<Option<Vec<u8>>, Message> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(Message::new("msg.hook.readFailed", format!("Can't read {}: {err}", path.display()))
            .with("path", path.display().to_string())
            .with("err", err.to_string())),
    }
}

/// `<config>.mayak-backup-<stamp>`, next to the config.
pub fn backup_path(config: &Path, stamp: &str) -> PathBuf {
    let mut name = config.as_os_str().to_os_string();
    name.push(format!(".{}-backup-{stamp}", ident::APP_SLUG));
    PathBuf::from(name)
}

/// Local time down to the second, "YYYYMMDD-HHMMSS": installing then removing
/// in the same minute must not collide on one backup name.
pub fn local_stamp() -> String {
    let t = platform::local_time();
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

/// `<config>.<ext>.mayak-<pid>`: next to the target, so the rename stays on one volume.
fn temp_path(config: &Path) -> PathBuf {
    let ext = config.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let suffix = format!("{}-{}", ident::APP_SLUG, std::process::id());
    if ext.is_empty() {
        config.with_extension(suffix)
    } else {
        config.with_extension(format!("{ext}.{suffix}"))
    }
}

/// What `hooks_preview` showed, kept in Rust until `hooks_apply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPlan {
    pub target: Target,
    pub install: bool,
    pub path: PathBuf,
    /// Of the bytes the plan was computed from (`fingerprint(b"")` for a missing file).
    pub fingerprint: String,
    /// Exactly what will be written.
    pub bytes: Vec<u8>,
    pub backup_path: PathBuf,
    /// The file already holds `next`: nothing will be written.
    pub unchanged: bool,
}

/// Plan, diff text and notes for writing `next` over `current` (D53, D54).
pub fn make_plan(
    target: Target,
    install: bool,
    path: &Path,
    current: Option<&[u8]>,
    next: &Value,
    stamp: &str,
) -> (PendingPlan, String, Vec<Message>) {
    let before = current.unwrap_or_default();
    // Structural: serde_json maps compare without key order and arrays by
    // position (which is what Codex trust follows), so the same entry in a file
    // with other indentation or line endings is not a write.
    let unchanged = match current {
        Some(bytes) => parse_object(bytes, "").map_or(false, |v| v == *next),
        None => next.as_object().map_or(false, |o| o.is_empty()),
    };
    let mut notes = Vec::new();
    let (bytes, diff) = if unchanged {
        (before.to_vec(), "No change.".to_string())
    } else {
        let fmt = current.map(detect_format).unwrap_or(TextFormat::NEW_FILE);
        if fmt.mixed_eol {
            // BOM and final newline are kept, so this is the one byte change the
            // line diff cannot show.
            let style = if fmt.crlf { "CRLF" } else { "LF" };
            notes.push(
                Message::new("msg.hook.lineEndings", format!("Line endings will be normalised to {style}."))
                    .with("style", style),
            );
        }
        let bytes = render(next, fmt);
        let diff = unified_diff(
            &String::from_utf8_lossy(strip_bom(before)),
            &String::from_utf8_lossy(strip_bom(&bytes)),
        );
        (bytes, diff)
    };
    let plan = PendingPlan {
        target,
        install,
        path: path.to_path_buf(),
        fingerprint: fingerprint(before),
        bytes,
        backup_path: backup_path(path, stamp),
        unchanged,
    };
    (plan, diff, notes)
}

/// One pending plan per target, between preview and apply.
#[derive(Default)]
pub struct Plans(Mutex<HashMap<Target, PendingPlan>>);

impl Plans {
    /// Replaces the plan of that target.
    pub fn store(&self, plan: PendingPlan) {
        self.lock().insert(plan.target, plan);
    }

    /// Removes and returns the plan only if target, install and fingerprint all
    /// match what the user previewed; otherwise the stored plan stays and
    /// nothing is returned.
    pub fn take(&self, target: Target, install: bool, fingerprint: &str) -> Result<PendingPlan, Message> {
        let name = target.as_str();
        // One lock for the whole take: a refused plan is back in place before anyone else looks.
        let mut plans = self.lock();
        let Some(plan) = plans.remove(&target) else {
            return Err(Message::new(
                "msg.hook.noPreview",
                format!("There is no {name} preview to apply. Open the preview again."),
            )
            .with("target", name));
        };
        if plan.install != install || plan.fingerprint != fingerprint {
            plans.insert(target, plan);
            return Err(Message::new(
                "msg.hook.previewMismatch",
                format!("This doesn't match the last {name} preview. Nothing was written — open the preview again."),
            )
            .with("target", name));
        }
        Ok(plan)
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Target, PendingPlan>> {
        // Every operation under this lock leaves the map whole (`take` removes a plan and
        // puts it back with nothing that can panic in between), so a poisoned lock cannot
        // hold a half-updated map.
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Writes a previewed plan. Ok(backup path), or Ok("") only when there was no
/// previous file (so nothing to back up).
pub fn write_plan(plan: &PendingPlan) -> Result<String, Message> {
    let path = &plan.path;
    let shown = path.display().to_string();
    // Read again: what is on disk now is what must match the preview and what
    // the backup must hold.
    let current = read_config(path)?;
    if fingerprint(current.as_deref().unwrap_or_default()) != plan.fingerprint {
        return Err(Message::new(
            "msg.hook.changed",
            format!("{shown} changed since the preview. Nothing was written — review the new diff."),
        )
        .with("path", shown));
    }
    if plan.unchanged {
        return Err(Message::new(
            "msg.hook.unchanged",
            format!("No change: {shown} already has this entry; nothing was written."),
        )
        .with("path", shown));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| {
            Message::new("msg.hook.createFailed", format!("Can't create {}: {e}", dir.display()))
                .with("path", dir.display().to_string())
                .with("err", e.to_string())
        })?;
    }
    let backup = match &current {
        Some(bytes) => {
            write_new(&plan.backup_path, bytes).map_err(|e| {
                let backup = plan.backup_path.display().to_string();
                Message::new("msg.hook.backupFailed", format!("Backup to {backup} failed ({e}). Nothing was written."))
                    .with("path", backup)
                    .with("err", e.to_string())
            })?;
            plan.backup_path.to_string_lossy().into_owned()
        }
        None => String::new(),
    };
    let write_failed = |err: std::io::Error| {
        Message::new("msg.hook.writeFailed", format!("write failed: {err}")).with("err", err.to_string())
    };
    // A dotfiles setup often makes the config a symlink: write to the file it points at, so
    // the link survives the rename.
    let target = resolve_link(path);
    // Write beside the target, flush, and rename over it: a crash, a power loss
    // or a full disk leaves the original intact rather than half a file.
    let temp = temp_path(&target);
    if let Err(err) = write_synced(&temp, &plan.bytes) {
        let _ = std::fs::remove_file(&temp);
        return Err(write_failed(err));
    }
    if let Err(err) = std::fs::rename(&temp, &target) {
        let _ = std::fs::remove_file(&temp);
        return Err(write_failed(err));
    }
    Ok(backup)
}

/// The file a symlinked config points at, else `path` itself. A link that leads nowhere stays
/// `path`: renaming over it then replaces the dead link, as writing a new file would.
fn resolve_link(path: &Path) -> PathBuf {
    let is_link = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    if is_link {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

/// Creates `path` and writes `bytes`. An existing file is an error, never
/// overwritten: a second backup in the same second must not replace the first.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Creates or truncates `path`, writes `bytes` and flushes them to disk before
/// returning, so the rename that follows can never put a file whose data is
/// not on disk yet in place of the config.
pub(crate) fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use serde_json::json;

    const WHERE: &str = "settings.json";
    const STAMP: &str = "20261001-120000";

    #[test]
    fn a_utf8_bom_is_stripped_not_treated_as_corruption() {
        // PowerShell 5's `Set-Content -Encoding utf8` produces exactly this.
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"model":"opus","hooks":{}}"#);
        let parsed = parse_object(&bytes, WHERE).expect("a BOM must not defeat the parser");
        assert_eq!(parsed["model"], "opus");
    }

    #[test]
    fn unreadable_content_is_an_error_never_an_empty_object() {
        // Returning {} here would let a merge produce a file holding nothing but
        // our hooks, and the write would replace everything the user had.
        for bad in [&b"{ not json"[..], &b"[1,2,3]"[..], &b"\"a string\""[..]] {
            let err = parse_object(bad, WHERE).expect_err("content we cannot use must refuse, not come back empty");
            assert!(["msg.hook.notObject", "msg.hook.invalidJson"].contains(&err.code), "{err:?}");
        }
    }

    #[test]
    fn not_an_object_is_a_message_with_the_path_and_the_old_sentence() {
        let err = parse_object(b"[1,2,3]", r"C:\Users\dev\.claude\settings.json").unwrap_err();
        assert_eq!(err.code, "msg.hook.notObject");
        assert_eq!(
            err.params,
            std::collections::BTreeMap::from([
                ("app", "Mayak".to_string()),
                ("path", r"C:\Users\dev\.claude\settings.json".to_string()),
            ])
        );
        assert_eq!(err.fallback, r"C:\Users\dev\.claude\settings.json isn't a JSON object — Mayak won't touch it.");
        assert_eq!(err.to_string(), err.fallback);
    }

    #[test]
    fn empty_and_whitespace_files_start_from_nothing() {
        assert_eq!(parse_object(b"", WHERE).unwrap(), json!({}));
        assert_eq!(parse_object(b"  \n\t ", WHERE).unwrap(), json!({}));
    }

    #[test]
    fn a_fingerprint_notices_any_change() {
        assert_eq!(fingerprint(b"{}"), fingerprint(b"{}"));
        assert_ne!(fingerprint(b"{}"), fingerprint(b"{ }"));
        assert_ne!(fingerprint(b""), fingerprint(b"{}"));
    }

    #[test]
    fn targets_parse_with_claude_as_the_default() {
        assert_eq!(Target::parse(None), Ok(Target::Claude));
        assert_eq!(Target::parse(Some("claude")), Ok(Target::Claude));
        assert_eq!(Target::parse(Some("codex")), Ok(Target::Codex));
        let err = Target::parse(Some("cursor")).unwrap_err();
        assert_eq!(err.code, "msg.hook.unknownTarget");
        assert_eq!(err.params["target"], "cursor");
        assert_eq!(err.fallback, r#"unknown target "cursor" (expected "claude" or "codex")"#);
        assert_eq!(Target::Codex.as_str(), "codex");
        assert_eq!(serde_json::to_value(Target::Claude).unwrap(), json!("claude"));
    }

    #[test]
    fn the_format_of_an_existing_file_is_detected() {
        let f = detect_format(b"\xEF\xBB\xBF{\r\n  \"a\": 1\r\n}\r\n");
        assert_eq!(f, TextFormat { bom: true, crlf: true, final_newline: true, mixed_eol: false });

        let f = detect_format(b"{\n  \"a\": 1\n}");
        assert_eq!(f, TextFormat { bom: false, crlf: false, final_newline: false, mixed_eol: false });

        // First line ending decides the style; both kinds present → mixed.
        let f = detect_format(b"{\r\n  \"a\": 1\n}\r\n");
        assert_eq!(f, TextFormat { bom: false, crlf: true, final_newline: true, mixed_eol: true });

        let f = detect_format(br#"{"a":1}"#);
        assert_eq!(f, TextFormat { bom: false, crlf: false, final_newline: false, mixed_eol: false });
    }

    #[test]
    fn rendering_keeps_bom_crlf_and_the_final_newline() {
        let v = json!({ "b": 1, "a": "x\ny" });
        let crlf = TextFormat { bom: true, crlf: true, final_newline: true, mixed_eol: false };
        assert_eq!(render(&v, crlf), b"\xEF\xBB\xBF{\r\n  \"b\": 1,\r\n  \"a\": \"x\\ny\"\r\n}\r\n".to_vec());

        let bare = TextFormat { bom: false, crlf: false, final_newline: false, mixed_eol: false };
        assert_eq!(render(&v, bare), b"{\n  \"b\": 1,\n  \"a\": \"x\\ny\"\n}".to_vec());

        assert_eq!(render(&json!({}), TextFormat::NEW_FILE), b"{}\n".to_vec());
    }

    #[test]
    fn a_missing_file_is_shown_whole_and_created_in_the_new_file_format() {
        let next = json!({ "hooks": { "Stop": [] } });
        let path = Path::new(r"C:\Users\dev\.codex\hooks.json");
        let (plan, diff, notes) = make_plan(Target::Codex, true, path, None, &next, STAMP);

        assert_eq!(plan.bytes, b"{\n  \"hooks\": {\n    \"Stop\": []\n  }\n}\n".to_vec());
        assert_eq!(diff, "+ {\n+   \"hooks\": {\n+     \"Stop\": []\n+   }\n+ }\n");
        assert!(notes.is_empty());
        assert!(!plan.unchanged);
        assert_eq!(plan.fingerprint, fingerprint(b""));
        assert_eq!(plan.path, path);
        assert_eq!(plan.backup_path, PathBuf::from(r"C:\Users\dev\.codex\hooks.json.mayak-backup-20261001-120000"));
    }

    #[test]
    fn the_diff_is_made_from_the_bytes_that_will_be_written() {
        let current = b"\xEF\xBB\xBF{\r\n    \"model\": \"x\"\r\n}\r\n".to_vec();
        let next = json!({ "model": "x", "hooks": {} });
        let (plan, diff, _) =
            make_plan(Target::Claude, true, Path::new("settings.json"), Some(&current), &next, STAMP);

        // The new file keeps BOM, CRLF and the final newline...
        assert_eq!(plan.bytes, b"\xEF\xBB\xBF{\r\n  \"model\": \"x\",\r\n  \"hooks\": {}\r\n}\r\n".to_vec());
        // ...and the context and "+" lines of the diff are exactly its lines.
        let after: Vec<&str> = diff
            .lines()
            .filter(|l| !l.starts_with('-'))
            .map(|l| &l[2..])
            .collect();
        let written = String::from_utf8_lossy(strip_bom(&plan.bytes)).into_owned();
        assert_eq!(after, written.lines().collect::<Vec<_>>());
        assert!(diff.contains("-     \"model\": \"x\""), "the old 4-space line must show as removed: {diff}");
        assert_eq!(plan.fingerprint, fingerprint(&current));
    }

    #[test]
    fn an_identical_entry_in_another_layout_is_unchanged_and_never_written() {
        // D53: same content, 4-space indent, CRLF, keys in another order.
        let tmp = TempDir::new("hookfile-same");
        let dir = tmp.path.join("cfg");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hooks.json");
        let current = b"{\r\n    \"hooks\": {\r\n        \"Stop\": [ { \"hooks\": [] } ]\r\n    },\r\n    \"description\": \"d\"\r\n}\r\n".to_vec();
        std::fs::write(&path, &current).unwrap();
        let next = json!({ "description": "d", "hooks": { "Stop": [ { "hooks": [] } ] } });

        let (plan, diff, notes) = make_plan(Target::Codex, true, &path, Some(&current), &next, STAMP);
        assert!(plan.unchanged);
        assert_eq!(diff, "No change.");
        assert!(notes.is_empty());
        assert_eq!(plan.bytes, current);

        let err = write_plan(&plan).unwrap_err();
        assert_eq!(err.code, "msg.hook.unchanged");
        assert_eq!(err.params["path"], path.display().to_string());
        assert_eq!(err.fallback, format!("No change: {} already has this entry; nothing was written.", path.display()));
        assert_eq!(std::fs::read(&path).unwrap(), current);
        assert!(!plan.backup_path.exists(), "no backup for a write that did not happen");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temp file left behind");
    }

    #[test]
    fn a_missing_file_and_an_empty_object_is_unchanged() {
        let (plan, diff, _) = make_plan(Target::Codex, false, Path::new("hooks.json"), None, &json!({}), STAMP);
        assert!(plan.unchanged);
        assert_eq!(diff, "No change.");
        assert!(plan.bytes.is_empty());
    }

    #[test]
    fn mixed_line_endings_are_announced_in_the_notes() {
        // D54: the line diff cannot show EOL normalisation, so the notes do.
        let current = b"{\r\n  \"a\": 1\n}\r\n".to_vec();
        let (plan, _, notes) =
            make_plan(Target::Claude, true, Path::new("settings.json"), Some(&current), &json!({ "a": 2 }), STAMP);
        assert_eq!(
            notes,
            vec![Message::new("msg.hook.lineEndings", "Line endings will be normalised to CRLF.").with("style", "CRLF")]
        );
        assert_eq!(plan.bytes, b"{\r\n  \"a\": 2\r\n}\r\n".to_vec());

        let current = b"{\n  \"a\": 1\r\n}\n".to_vec();
        let (_, _, notes) =
            make_plan(Target::Claude, true, Path::new("settings.json"), Some(&current), &json!({ "a": 2 }), STAMP);
        assert_eq!(
            notes,
            vec![Message::new("msg.hook.lineEndings", "Line endings will be normalised to LF.").with("style", "LF")]
        );
    }

    #[test]
    fn local_stamp_is_date_dash_time() {
        let s = local_stamp();
        assert_eq!(s.len(), 15, "got: {s}");
        assert_eq!(&s[8..9], "-");
        assert!(s.chars().enumerate().all(|(i, c)| i == 8 || c.is_ascii_digit()), "got: {s}");
    }

    fn plan_for(target: Target, install: bool, fp: &str) -> PendingPlan {
        PendingPlan {
            target,
            install,
            path: PathBuf::from("x.json"),
            fingerprint: fp.into(),
            bytes: b"{}\n".to_vec(),
            backup_path: PathBuf::from("x.json.mayak-backup-1"),
            unchanged: false,
        }
    }

    #[test]
    fn plans_are_taken_once_and_only_with_matching_install_and_fingerprint() {
        let plans = Plans::default();
        let none = plans.take(Target::Codex, true, "aa").unwrap_err();
        assert_eq!(none.code, "msg.hook.noPreview", "nothing previewed yet");
        assert_eq!(none.params["target"], "codex");
        assert_eq!(none.fallback, "There is no codex preview to apply. Open the preview again.");

        plans.store(plan_for(Target::Codex, true, "aa"));
        plans.store(plan_for(Target::Claude, false, "bb"));
        let other = plans.take(Target::Codex, true, "zz").unwrap_err();
        assert_eq!(other.code, "msg.hook.previewMismatch", "unknown fingerprint");
        assert_eq!(
            other.fallback,
            "This doesn't match the last codex preview. Nothing was written — open the preview again."
        );
        assert!(plans.take(Target::Codex, false, "aa").is_err(), "install flag differs");
        // A refused take leaves the plan in place.
        assert_eq!(plans.take(Target::Codex, true, "aa").unwrap(), plan_for(Target::Codex, true, "aa"));
        assert!(plans.take(Target::Codex, true, "aa").is_err(), "a plan is applied once");
        // Targets do not share plans.
        assert_eq!(plans.take(Target::Claude, false, "bb").unwrap(), plan_for(Target::Claude, false, "bb"));

        // A newer preview replaces the older one of the same target.
        plans.store(plan_for(Target::Codex, true, "aa"));
        plans.store(plan_for(Target::Codex, true, "cc"));
        assert!(plans.take(Target::Codex, true, "aa").is_err());
        assert!(plans.take(Target::Codex, true, "cc").is_ok());
    }

    #[test]
    fn write_plan_backs_up_byte_for_byte_and_replaces_the_file() {
        let tmp = TempDir::new("hookfile-write");
        let dir = tmp.path.join("cfg");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let original = b"\xEF\xBB\xBF{\r\n  \"model\": \"x\"\r\n}\r\n".to_vec();
        std::fs::write(&path, &original).unwrap();

        let next = json!({ "model": "x", "hooks": {} });
        let (plan, _, _) = make_plan(Target::Claude, true, &path, Some(&original), &next, STAMP);
        let backup = write_plan(&plan).expect("the write should succeed");

        assert_eq!(backup, dir.join("settings.json.mayak-backup-20261001-120000").to_string_lossy());
        assert_eq!(std::fs::read(&backup).unwrap(), original, "the backup is the exact old bytes");
        assert_eq!(std::fs::read(&path).unwrap(), plan.bytes, "the file is the exact previewed bytes");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "file + backup, no temp file left");

        // The same backup path is never overwritten.
        let (again, _, _) = make_plan(Target::Claude, false, &path, Some(&plan.bytes), &json!({}), STAMP);
        let err = write_plan(&again).unwrap_err();
        assert_eq!(err.code, "msg.hook.backupFailed");
        assert_eq!(err.params["path"], backup);
        // The OS error stays verbatim, in the params and in the sentence.
        assert_eq!(
            err.fallback,
            format!("Backup to {backup} failed ({}). Nothing was written.", err.params["err"])
        );
        assert_eq!(std::fs::read(&path).unwrap(), plan.bytes, "a failed backup writes nothing");
        assert_eq!(std::fs::read(&backup).unwrap(), original);
    }

    #[test]
    fn write_plan_refuses_a_file_that_changed_since_the_preview() {
        let tmp = TempDir::new("hookfile-stale");
        let dir = tmp.path.join("cfg");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hooks.json");
        std::fs::write(&path, b"{}").unwrap();
        let (plan, _, _) = make_plan(Target::Codex, true, &path, Some(b"{}"), &json!({ "hooks": {} }), STAMP);

        std::fs::write(&path, br#"{"description":"edited by hand"}"#).unwrap();
        let err = write_plan(&plan).unwrap_err();
        assert_eq!(err.code, "msg.hook.changed");
        assert_eq!(err.params["path"], path.display().to_string());
        assert_eq!(
            err.fallback,
            format!("{} changed since the preview. Nothing was written — review the new diff.", path.display())
        );
        assert_eq!(std::fs::read(&path).unwrap(), br#"{"description":"edited by hand"}"#.to_vec());
        assert!(!plan.backup_path.exists());
    }

    #[test]
    fn write_plan_writes_through_a_symlinked_config_and_keeps_the_link() {
        let tmp = TempDir::new("hookfile-link");
        let real_dir = tmp.path.join("dotfiles");
        let dir = tmp.path.join("cfg");
        std::fs::create_dir_all(&real_dir).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let real = real_dir.join("settings.json");
        std::fs::write(&real, b"{}").unwrap();
        let path = dir.join("settings.json");
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&real, &path);
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&real, &path);
        if let Err(err) = linked {
            // File links on Windows need Developer Mode or the right to create them.
            eprintln!("skipped: this account cannot create a file symlink ({err})");
            return;
        }

        let (plan, _, _) = make_plan(Target::Claude, true, &path, Some(b"{}"), &json!({ "hooks": {} }), STAMP);
        write_plan(&plan).expect("the write should succeed");

        assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink(), "the link is still a link");
        assert_eq!(std::fs::read(&real).unwrap(), plan.bytes, "the file it points at has the new bytes");
        assert_eq!(std::fs::read_dir(&real_dir).unwrap().count(), 1, "no temp file left beside the real file");
    }

    #[test]
    fn write_plan_with_no_previous_file_returns_an_empty_backup() {
        let tmp = TempDir::new("hookfile-new");
        let path = tmp.path.join("not-yet").join("hooks.json");
        let (plan, _, _) = make_plan(Target::Codex, true, &path, None, &json!({ "hooks": {} }), STAMP);

        assert_eq!(write_plan(&plan), Ok(String::new()));
        assert_eq!(std::fs::read(&path).unwrap(), b"{\n  \"hooks\": {}\n}\n".to_vec());
        assert!(!plan.backup_path.exists());
    }
}
