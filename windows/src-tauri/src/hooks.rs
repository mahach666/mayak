// Hook installers (spec §8), one per target, both on the write protection in
// hookfile.rs:
// - "codex": Mayak's groups in ~/.codex/hooks.json (codex_hooks.rs), optional;
// - "antigravity": Mayak's named group in ~/.gemini/config/hooks.json
//   (antigravity_hooks.rs), optional, observe only;
// - "claude": the Allow/Deny relay in ~/.claude/settings.json, off by default.
//   It installs PermissionRequest and PreToolUse for AskUserQuestion (answers
//   from the island, spec §3); status and removal also recognise what an old
//   Coucou install left there.
//
// Apart from staging the relay (`ensure_hook_exe`), nothing here runs at app
// start: a tool config is read only when the settings window asks for a status,
// a preview or an apply. Writes happen only from an explicit click on a diff
// the user has seen.
//
// The Claude command is only the quoted exe path in forward slashes plus the
// event name: on Windows Claude Code runs hook commands through Git Bash, and
// anything with PowerShell or cmd in it breaks.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager};

use crate::antigravity_hooks;
use crate::codex_hooks;
use crate::hookfile::{self, PendingPlan, Plans, Target};
use crate::ident;
use crate::message::Message;
use crate::{platform, settings};

/// Claude target: (event, matcher, timeout in seconds). Both wait for a click in
/// the island: the relay's budget is 595 s, the pipe's 590 s (spec §2.5).
/// PermissionRequest has no matcher: the relay itself forwards only real
/// permission requests (no AskUserQuestion, a session_id required; D87).
/// AskUserQuestion is asked through PreToolUse instead, which can answer it.
pub const CLAUDE_EVENTS: &[(&str, Option<&str>, u64)] = &[
    ("PermissionRequest", None, 600),
    ("PreToolUse", Some("AskUserQuestion"), 600),
];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    pub target: Target,
    pub installed: bool,
    /// Installed, but an install now would change the file.
    pub outdated: bool,
    /// Config path of the target (settings.json or hooks.json); the name is kept
    /// from the single-target days.
    pub settings_path: String,
    pub hook_path: String,
    pub hook_ready: bool,
    pub notes: Vec<Message>,
    /// Codex only: whether the island answers Codex permissions (spec §3). Read from hooks.json,
    /// like `installed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answers: Option<Answers>,
}

/// The Codex answers switch as hooks.json has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Answers {
    /// Mayak's `PermissionRequest` hook waits 600 s for the island (`codex_hooks::answers_enabled`).
    On,
    /// Mayak's Codex hooks are installed, observe only.
    Off,
    /// Mayak's Codex hooks are not installed: the integration comes first.
    NotInstalled,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookPreview {
    pub diff: String,
    pub backup: String,
    pub settings_path: String,
    /// Handed back to apply, so only the reviewed bytes are ever written.
    pub fingerprint: String,
    pub notes: Vec<Message>,
    /// The file already holds exactly this; Apply has nothing to write (D53).
    pub unchanged: bool,
}

/// What the installer works on. Built once in lib.rs from `app_paths()`; tests
/// point it at a temp directory.
pub struct HookCtx {
    pub claude_dir: PathBuf,
    pub codex_home: PathBuf,
    /// `~/.gemini`: Antigravity's global hooks are in its `config\hooks.json`.
    pub gemini_dir: PathBuf,
    pub hook_exe: PathBuf,
    pub plans: Plans,
    /// The Codex answers switch's own plan (`answers_preview`), apart from the install plans so
    /// neither apply can write the other's diff.
    pub answer_plans: Plans,
}

fn config_path(ctx: &HookCtx, target: Target) -> PathBuf {
    match target {
        Target::Claude => ctx.claude_dir.join("settings.json"),
        Target::Codex => codex_hooks::hooks_json_path(&ctx.codex_home),
        Target::Antigravity => antigravity_hooks::hooks_json_path(&ctx.gemini_dir),
    }
}

// ── Claude target ─────────────────────────────────────────────────────────────

fn hook_command(exe: &Path, event: &str) -> String {
    let exe = exe.to_string_lossy().replace('\\', "/");
    format!("\"{exe}\" {event}")
}

fn entry_has_command(entry: &Value, pred: impl Fn(&str) -> bool) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .map_or(false, |hooks| {
            hooks
                .iter()
                .any(|h| h.get("command").and_then(Value::as_str).map_or(false, &pred))
        })
}

/// Ours: Mayak's marker, or the marker an older build of this app wrote.
fn entry_is_ours(entry: &Value) -> bool {
    entry_has_command(entry, |c| {
        c.contains(ident::MARKER) || ident::LEGACY_MARKERS.iter().any(|m| c.contains(m))
    })
}

fn entry_is_legacy(entry: &Value) -> bool {
    entry_has_command(entry, |c| ident::LEGACY_MARKERS.iter().any(|m| c.contains(m)))
}

fn any_entry(settings: &Value, pred: fn(&Value) -> bool) -> bool {
    settings
        .get("hooks")
        .and_then(Value::as_object)
        .map_or(false, |hooks| hooks.values().filter_map(Value::as_array).flatten().any(pred))
}

fn is_claude_event(event: &str) -> bool {
    CLAUDE_EVENTS.iter().any(|(name, _, _)| *name == event)
}

/// Every event list without our (or Coucou's) entries. A list we emptied goes,
/// unless `keep_empty(event)`: install keeps its own events' keys in place. A
/// list that was empty already is the user's and stays.
fn strip_ours(hooks: &Map<String, Value>, keep_empty: fn(&str) -> bool) -> Map<String, Value> {
    let mut out = Map::new();
    for (event, value) in hooks {
        match value.as_array() {
            Some(list) => {
                let kept: Vec<Value> = list.iter().filter(|e| !entry_is_ours(e)).cloned().collect();
                let emptied_by_us = kept.is_empty() && !list.is_empty();
                if !emptied_by_us || keep_empty(event) {
                    out.insert(event.clone(), Value::Array(kept));
                }
            }
            None => {
                out.insert(event.clone(), value.clone());
            }
        }
    }
    out
}

/// What `merged` needs before it may run: `"hooks"`, when present, an object, and each event
/// it writes (`CLAUDE_EVENTS`), when present, a list. `merged` would otherwise replace them with
/// its own, so it is refused instead, as Codex's `check_shape` does. Other events may hold anything:
/// they are kept as they are.
fn check_claude_shape(v: &Value, path: &str) -> Result<(), Message> {
    let bad = |key: &str| {
        Message::new(
            "msg.hook.badShape",
            format!("{path}: \"{key}\" isn't the shape Claude Code expects — {} won't touch it.", ident::APP_NAME),
        )
        .with("path", path)
        .with("key", key)
        .with("app", ident::APP_NAME)
    };
    let Some(hooks) = v.get("hooks") else { return Ok(()) };
    let Some(events) = hooks.as_object() else { return Err(bad("hooks")) };
    match CLAUDE_EVENTS.iter().find(|(event, _, _)| events.get(*event).is_some_and(|list| !list.is_array())) {
        Some((event, _, _)) => Err(bad(event)),
        None => Ok(()),
    }
}

/// Settings with only Mayak's `CLAUDE_EVENTS` entries: every older entry of ours
/// (any event, Mayak or Coucou) is removed first; everything else is untouched.
/// Ours go after the user's entries of that event.
fn merged(existing: &Value, exe: &Path) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let current = root.get("hooks").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut hooks = strip_ours(&current, is_claude_event);
    for (event, matcher, timeout) in CLAUDE_EVENTS {
        let mut list = hooks.get(*event).and_then(Value::as_array).cloned().unwrap_or_default();
        let mut entry = Map::new();
        if let Some(matcher) = matcher {
            entry.insert("matcher".into(), json!(matcher));
        }
        entry.insert(
            "hooks".into(),
            json!([{
                "type": "command",
                "command": hook_command(exe, event),
                "timeout": timeout,
            }]),
        );
        list.push(Value::Object(entry));
        hooks.insert((*event).to_string(), Value::Array(list));
    }
    root.insert("hooks".into(), Value::Object(hooks));
    Value::Object(root)
}

/// Settings with every entry of ours (Mayak or Coucou) removed, and nothing else changed.
fn without_ours(existing: &Value) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return Value::Object(root);
    };
    let out = strip_ours(hooks, |_| false);
    if out.is_empty() && !hooks.is_empty() {
        // We emptied every event list, so "hooks" goes too; a "hooks": {} that
        // was empty already stays. shift_remove keeps the order of the user's
        // other keys.
        root.shift_remove("hooks");
    } else {
        root.insert("hooks".into(), Value::Object(out));
    }
    Value::Object(root)
}

// ── Shared ────────────────────────────────────────────────────────────────────

/// The config as an object (`{}` when the file is missing), or None with the
/// reason in `notes`.
fn read_value(path: &Path, notes: &mut Vec<Message>) -> Option<Value> {
    match hookfile::read_config(path) {
        Ok(None) => Some(json!({})),
        Ok(Some(bytes)) => match hookfile::parse_object(&bytes, &path.display().to_string()) {
            Ok(v) => Some(v),
            Err(err) => {
                notes.push(err);
                None
            }
        },
        Err(err) => {
            notes.push(err);
            None
        }
    }
}

fn claude_state(ctx: &HookCtx, path: &Path, notes: &mut Vec<Message>) -> (bool, bool) {
    let Some(current) = read_value(path, notes) else { return (false, false) };
    if any_entry(&current, entry_is_legacy) {
        notes.push(
            Message::new(
                "msg.hook.coucouLeft",
                format!(
                    "{} still has hook entries from Coucou. Reinstall replaces them; Remove deletes them.",
                    path.display()
                ),
            )
            .with("path", path.display().to_string()),
        );
    }
    let shape_ok = match check_claude_shape(&current, &path.display().to_string()) {
        Ok(()) => true,
        Err(err) => {
            notes.push(err);
            false
        }
    };
    let installed = any_entry(&current, entry_is_ours);
    // A reinstall is refused on that shape, so it can't be offered as the fix.
    let outdated = shape_ok && installed && ours_last(&merged(&current, &ctx.hook_exe)) != ours_last(&current);
    (installed, outdated)
}

/// `settings` with each event list's entries of ours moved after the user's, both kept in their
/// order. `merged` puts ours last, so a user's entry after ours is not a reason to update.
fn ours_last(settings: &Value) -> Value {
    let mut out = settings.clone();
    if let Some(hooks) = out.get_mut("hooks").and_then(Value::as_object_mut) {
        for list in hooks.values_mut().filter_map(Value::as_array_mut) {
            let (ours, mut foreign): (Vec<Value>, Vec<Value>) = list.drain(..).partition(|e| entry_is_ours(e));
            foreign.extend(ours);
            *list = foreign;
        }
    }
    out
}

/// Without a usable relay there is no current command, and the marker alone
/// does not match a command GetShortPathNameW turned into an 8.3 name.
fn relay_unusable_note() -> Message {
    Message::new(
        "msg.codex.relayUnusable",
        format!(
            "{0}'s relay isn't available, so entries written with its short (8.3) path can't be \
             recognised. Restart {0} to restore the relay, then Remove.",
            ident::APP_NAME
        ),
    )
    .with("app", ident::APP_NAME)
}

fn codex_state(ctx: &HookCtx, path: &Path, hook_ready: bool, notes: &mut Vec<Message>) -> (bool, bool, Answers) {
    // Without the relay there is no short path to compute; the missing-relay
    // note is already there.
    let our = if hook_ready {
        match codex_hooks::our_command(&ctx.hook_exe) {
            Ok(cmd) => Some(cmd),
            Err(err) => {
                notes.push(err);
                None
            }
        }
    } else {
        None
    };
    if our.is_none() {
        notes.push(relay_unusable_note());
    }
    let Some(current) = read_value(path, notes) else { return (false, false, Answers::NotInstalled) };
    if let Err(err) = codex_hooks::check_shape(&current) {
        notes.push(err);
    }
    let our = our.as_deref().unwrap_or("");
    let installed = codex_hooks::installed(&current, our);
    let outdated = !our.is_empty()
        && installed
        && codex_hooks::merge_install(&current, our).map_or(false, |next| next != current);
    // The switch's own test (`merge_answers` refuses without that group), not `installed`.
    let answers = if !codex_hooks::answers_switchable(&current, our) {
        Answers::NotInstalled
    } else if codex_hooks::answers_enabled(&current, our) {
        Answers::On
    } else {
        Answers::Off
    };
    (installed, outdated, answers)
}

/// Mayak's command for Antigravity, or None (with the reason in `notes`) without a usable relay.
fn antigravity_command(ctx: &HookCtx, hook_ready: bool, notes: &mut Vec<Message>) -> Option<String> {
    let our = if hook_ready {
        match antigravity_hooks::our_command(&ctx.hook_exe) {
            Ok(cmd) => Some(cmd),
            Err(err) => {
                notes.push(err);
                None
            }
        }
    } else {
        None
    };
    if our.is_none() {
        notes.push(relay_unusable_note());
    }
    our
}

fn antigravity_state(ctx: &HookCtx, path: &Path, hook_ready: bool, notes: &mut Vec<Message>) -> (bool, bool) {
    let our = antigravity_command(ctx, hook_ready, notes);
    let Some(current) = read_value(path, notes) else { return (false, false) };
    let our = our.as_deref().unwrap_or("");
    notes.extend(antigravity_hooks::foreign_group_note(&current, our));
    let installed = antigravity_hooks::installed(&current, our);
    let outdated = !our.is_empty()
        && installed
        && antigravity_hooks::merge_install(&current, our).map_or(false, |next| next != current);
    (installed, outdated)
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Read-only. Never fails: problems become `notes`.
pub fn status(ctx: &HookCtx, target: Target) -> HookStatus {
    let path = config_path(ctx, target);
    let hook_ready = ctx.hook_exe.exists();
    let mut notes = Vec::new();
    if !hook_ready {
        notes.push(
            Message::new(
                "msg.hook.relayMissing",
                format!("The relay {} is not in place yet. Restart {}.", ctx.hook_exe.display(), ident::APP_NAME),
            )
            .with("path", ctx.hook_exe.display().to_string())
            .with("app", ident::APP_NAME),
        );
    }
    let (installed, outdated, answers) = match target {
        Target::Claude => {
            let (installed, outdated) = claude_state(ctx, &path, &mut notes);
            (installed, outdated, None)
        }
        Target::Codex => {
            let (installed, outdated, answers) = codex_state(ctx, &path, hook_ready, &mut notes);
            (installed, outdated, Some(answers))
        }
        Target::Antigravity => {
            let (installed, outdated) = antigravity_state(ctx, &path, hook_ready, &mut notes);
            (installed, outdated, None)
        }
    };
    HookStatus {
        target,
        installed,
        outdated,
        settings_path: path.to_string_lossy().into_owned(),
        hook_path: ctx.hook_exe.to_string_lossy().into_owned(),
        hook_ready,
        notes,
        answers,
    }
}

/// Computes the exact bytes an install (or removal) would write, keeps the plan
/// for `apply` and returns the diff to show. `stamp` names the backup.
pub fn preview(ctx: &HookCtx, target: Target, install: bool, stamp: &str) -> Result<HookPreview, Message> {
    let path = config_path(ctx, target);
    let shown = path.display().to_string();
    let (current_bytes, current) = read_current(&path, &shown)?;
    let mut notes = Vec::new();
    let next = match target {
        Target::Claude => {
            if any_entry(&current, entry_is_legacy) {
                notes.push(Message::new("msg.hook.coucouRemoved", "Hook entries from Coucou are removed too."));
            }
            if install {
                check_claude_shape(&current, &shown)?;
                merged(&current, &ctx.hook_exe)
            } else {
                without_ours(&current)
            }
        }
        Target::Codex => {
            if install {
                codex_hooks::merge_install(&current, &codex_hooks::our_command(&ctx.hook_exe)?)?
            } else {
                // Without a usable relay the marker alone finds our groups,
                // which works for long-name commands only; the notes say so.
                let our = match codex_hooks::our_command(&ctx.hook_exe) {
                    Ok(cmd) => cmd,
                    Err(err) => {
                        notes.push(err);
                        notes.push(relay_unusable_note());
                        String::new()
                    }
                };
                codex_hooks::merge_uninstall(&current, &our)?
            }
        }
        Target::Antigravity => {
            if install {
                antigravity_hooks::merge_install(&current, &antigravity_hooks::our_command(&ctx.hook_exe)?)?
            } else {
                // As for Codex: without a usable relay only the marker finds our group.
                let our = match antigravity_hooks::our_command(&ctx.hook_exe) {
                    Ok(cmd) => cmd,
                    Err(err) => {
                        notes.push(err);
                        notes.push(relay_unusable_note());
                        String::new()
                    }
                };
                antigravity_hooks::merge_uninstall(&current, &our)
            }
        }
    };
    let made = hookfile::make_plan(target, install, &path, current_bytes.as_deref(), &next, stamp);
    Ok(store_preview(&ctx.plans, made, notes))
}

/// The config's raw bytes (None when missing) and its object (`{}` when missing).
fn read_current(path: &Path, shown: &str) -> Result<(Option<Vec<u8>>, Value), Message> {
    let bytes = hookfile::read_config(path)?;
    let current = match &bytes {
        Some(bytes) => hookfile::parse_object(bytes, shown)?,
        None => json!({}),
    };
    Ok((bytes, current))
}

/// Keeps a `hookfile::make_plan` result in `plans` for the matching apply, and returns what the
/// user has to see.
fn store_preview(
    plans: &Plans,
    (plan, diff, plan_notes): (PendingPlan, String, Vec<Message>),
    mut notes: Vec<Message>,
) -> HookPreview {
    notes.extend(plan_notes);
    let out = HookPreview {
        diff,
        backup: plan.backup_path.to_string_lossy().into_owned(),
        settings_path: plan.path.display().to_string(),
        fingerprint: plan.fingerprint.clone(),
        notes,
        unchanged: plan.unchanged,
    };
    plans.store(plan);
    out
}

/// Writes the plan the last preview of `target` stored, if `install` and
/// `fingerprint` match it. Ok(backup path), or Ok("") when there was no file.
pub fn apply(ctx: &HookCtx, target: Target, install: bool, fingerprint: &str) -> Result<String, Message> {
    let plan = ctx.plans.take(target, install, fingerprint)?;
    hookfile::write_plan(&plan)
}

/// The diff of switching Codex answers from the island on or off (spec §3,
/// `codex_hooks::merge_answers`); the plan stays in `answer_plans` for `answers_apply`.
/// Refused while Mayak's Codex hooks are not installed, or without a usable relay.
pub fn answers_preview(ctx: &HookCtx, on: bool, stamp: &str) -> Result<HookPreview, Message> {
    let path = config_path(ctx, Target::Codex);
    let shown = path.display().to_string();
    let (current_bytes, current) = read_current(&path, &shown)?;
    let next = codex_hooks::merge_answers(&current, &codex_hooks::our_command(&ctx.hook_exe)?, on)?;
    let made = hookfile::make_plan(Target::Codex, on, &path, current_bytes.as_deref(), &next, stamp);
    Ok(store_preview(&ctx.answer_plans, made, Vec::new()))
}

/// Writes the plan the last `answers_preview` stored, if `on` and `fingerprint` match it.
/// Ok(backup path).
pub fn answers_apply(ctx: &HookCtx, on: bool, fingerprint: &str) -> Result<String, Message> {
    let plan = ctx.answer_plans.take(Target::Codex, on, fingerprint)?;
    hookfile::write_plan(&plan)
}

/// Copies the relay (`platform::HOOK_EXE`) into the local data dir's
/// bin/ on launch. In a bundled install it comes from the app resources; in
/// `tauri dev` it sits next to the app binary in the workspace target directory.
///
/// Every candidate is tried rather than just the first, because getting this
/// wrong is silent and fatal: `resources` used to be a glob, which made NSIS
/// mirror the source path into `_up_\target\release\`, no candidate matched, and
/// the relay was simply never installed. It only looked healthy on a developer
/// machine, where a leftover copy from `tauri dev` was already sitting in bin/.
pub fn ensure_hook_exe(app: &AppHandle) {
    let dest = settings::hook_exe_path();
    let Some(dir) = dest.parent() else { return };
    // Nobody else may swap the relay Claude Code runs: its folder is ours only.
    if platform::ensure_private_dir(&settings::local_dir()).is_err()
        || std::fs::create_dir_all(dir).is_err()
    {
        return;
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = app.path().resolve(platform::HOOK_EXE, tauri::path::BaseDirectory::Resource) {
        candidates.push(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // Installed build, then `tauri dev` (target/debug) next to the
            // release hook the pre-build step produces.
            candidates.push(parent.join(platform::HOOK_EXE));
            candidates.push(parent.join("../release").join(platform::HOOK_EXE));
            // Belt and braces: where the old glob form used to land it.
            candidates.push(parent.join("_up_/target/release").join(platform::HOOK_EXE));
        }
    }

    let tried: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
    let Some(src) = candidates.into_iter().find(|p| p.exists()) else {
        crate::log::line(format!(
            "{} not found — Claude Code hooks cannot work. Looked in: {}",
            platform::HOOK_EXE,
            tried.join(", ")
        ));
        return;
    };
    install_relay(&src, &dest);
}

#[cfg(windows)]
fn install_relay(src: &Path, dest: &Path) {
    let same = match (std::fs::metadata(src), std::fs::metadata(dest)) {
        (Ok(a), Ok(b)) => a.len() == b.len() && a.modified().ok() == b.modified().ok(),
        _ => false,
    };
    if same {
        return;
    }
    // A hook may be running right now and hold the file open; keeping the old
    // copy is fine, it is the same relay.
    if let Err(err) = std::fs::copy(src, dest) {
        if !dest.exists() {
            crate::log::line(format!("could not install {}: {err}", platform::HOOK_EXE));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    const STAMP: &str = "20261001-120000";
    const EXE: &str = r"C:\Users\dev\AppData\Local\Mayak\bin\mayak-hook.exe";

    fn ctx_in(tmp: &TempDir) -> HookCtx {
        HookCtx {
            claude_dir: tmp.path.join(".claude"),
            codex_home: tmp.path.join(".codex"),
            gemini_dir: tmp.path.join(".gemini"),
            hook_exe: tmp.path.join("bin").join(ident::HOOK_EXE),
            plans: Plans::default(),
            answer_plans: Plans::default(),
        }
    }

    fn place_relay(ctx: &HookCtx) {
        std::fs::create_dir_all(ctx.hook_exe.parent().unwrap()).unwrap();
        std::fs::write(&ctx.hook_exe, b"relay").unwrap();
    }

    fn coucou(event: &str) -> Value {
        json!({ "hooks": [{
            "type": "command",
            "command": format!("\"C:/Users/dev/AppData/Local/Coucou/bin/coucou-hook.exe\" {event}"),
            "timeout": 10
        }] })
    }

    #[test]
    fn merging_keeps_every_other_setting_and_every_foreign_hook() {
        let existing = json!({
            "model": "claude-opus-5",
            "theme": "dark",
            "enabledPlugins": ["a", "b"],
            "hooks": {
                "PreToolUse": [
                    { "hooks": [{ "type": "command", "command": "someone-elses-tool.exe" }] }
                ],
                "SomeEventWeDoNotTouch": [
                    { "hooks": [{ "type": "command", "command": "keep-me.exe" }] }
                ]
            }
        });

        let after = merged(&existing, Path::new(EXE));
        assert_eq!(after["model"], "claude-opus-5");
        assert_eq!(after["theme"], "dark");
        assert_eq!(after["enabledPlugins"], json!(["a", "b"]));

        let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(
            pre.iter().any(|e| serde_json::to_string(e).unwrap().contains("someone-elses-tool.exe")),
            "another tool's hook was dropped"
        );
        assert!(after["hooks"]["SomeEventWeDoNotTouch"].is_array());
        let ours = after["hooks"]["PermissionRequest"].as_array().unwrap();
        assert!(ours.iter().any(entry_is_ours), "our own hook was not added");

        // And removing ours puts it back exactly as it was.
        let cleaned = without_ours(&after);
        assert_eq!(cleaned, existing);

        // An empty list, or an empty "hooks", the user left there is not ours to drop.
        let e = json!({ "hooks": { "Notification": [] } });
        let a = merged(&e, Path::new(EXE));
        assert!(a["hooks"]["Notification"].is_array(), "got: {a}");
        assert_eq!(without_ours(&a), e);
        let bare = json!({ "model": "x", "hooks": {} });
        assert_eq!(without_ours(&bare), bare);
    }

    #[test]
    fn the_claude_target_installs_permissions_and_questions_at_600_s() {
        // What a Coucou install of the twelve events left behind, plus a foreign hook.
        let mut hooks = Map::new();
        for event in ["SessionStart", "PreToolUse", "PermissionRequest", "Stop"] {
            hooks.insert(event.to_string(), json!([coucou(event)]));
        }
        hooks.insert(
            "Stop".to_string(),
            json!([coucou("Stop"), { "hooks": [{ "type": "command", "command": "keep-me.exe" }] }]),
        );
        let existing = json!({ "model": "x", "hooks": hooks });

        let after = merged(&existing, Path::new(EXE));
        let events = after["hooks"].as_object().unwrap();
        let names: Vec<&str> = events.keys().map(String::as_str).collect();
        assert_eq!(names, ["PreToolUse", "PermissionRequest", "Stop"], "Coucou-only events are gone");
        assert_eq!(events["PermissionRequest"], json!([permission_entry()]));
        assert_eq!(events["PreToolUse"], json!([question_entry()]));
        assert_eq!(events["Stop"], json!([{ "hooks": [{ "type": "command", "command": "keep-me.exe" }] }]));
        assert_eq!(
            serde_json::to_string(&question_entry()).unwrap(),
            concat!(
                r#"{"matcher":"AskUserQuestion","hooks":[{"type":"command","#,
                r#""command":"\"C:/Users/dev/AppData/Local/Mayak/bin/mayak-hook.exe\" PreToolUse","timeout":600}]}"#
            ),
            "key order is part of what the user reviews"
        );
    }

    /// Mayak's two Claude entries for `EXE`.
    fn permission_entry() -> Value {
        json!({ "hooks": [{
            "type": "command",
            "command": "\"C:/Users/dev/AppData/Local/Mayak/bin/mayak-hook.exe\" PermissionRequest",
            "timeout": 600
        }] })
    }

    fn question_entry() -> Value {
        json!({ "matcher": "AskUserQuestion", "hooks": [{
            "type": "command",
            "command": "\"C:/Users/dev/AppData/Local/Mayak/bin/mayak-hook.exe\" PreToolUse",
            "timeout": 600
        }] })
    }

    fn read_settings(ctx: &HookCtx) -> Value {
        hookfile::parse_object(&std::fs::read(ctx.claude_dir.join("settings.json")).unwrap(), "s").unwrap()
    }

    fn write_settings(ctx: &HookCtx, v: &Value) {
        std::fs::create_dir_all(&ctx.claude_dir).unwrap();
        std::fs::write(ctx.claude_dir.join("settings.json"), serde_json::to_vec(v).unwrap()).unwrap();
    }

    #[test]
    fn claude_fresh_install_writes_both_entries_and_removal_strips_both() {
        let tmp = TempDir::new("hooks-claude-fresh");
        let ctx = ctx_in(&tmp);
        place_relay(&ctx);
        let command = |event: &str| hook_command(&ctx.hook_exe, event);
        assert!(command("PreToolUse").ends_with("mayak-hook.exe\" PreToolUse") && !command("x").contains('\\'));

        // No settings.json yet.
        let install = preview(&ctx, Target::Claude, true, STAMP).unwrap();
        assert_eq!(apply(&ctx, Target::Claude, true, &install.fingerprint), Ok(String::new()));
        assert_eq!(
            read_settings(&ctx),
            json!({ "hooks": {
                "PermissionRequest": [{ "hooks": [{
                    "type": "command", "command": command("PermissionRequest"), "timeout": 600
                }] }],
                "PreToolUse": [{ "matcher": "AskUserQuestion", "hooks": [{
                    "type": "command", "command": command("PreToolUse"), "timeout": 600
                }] }]
            } })
        );
        let st = status(&ctx, Target::Claude);
        assert!(st.installed && !st.outdated, "{st:?}");
        assert_eq!(st.answers, None, "Claude has no separate answers switch");
        assert!(serde_json::to_value(&st).unwrap().get("answers").is_none());

        // A foreign PreToolUse entry, even on the same matcher, stays where it is; ours goes after it.
        let foreign = json!({ "matcher": "AskUserQuestion", "hooks": [{ "type": "command", "command": "other.exe" }] });
        let mut with_foreign = read_settings(&ctx);
        with_foreign["hooks"]["PreToolUse"].as_array_mut().unwrap().insert(0, foreign.clone());
        write_settings(&ctx, &with_foreign);
        assert!(!status(&ctx, Target::Claude).outdated);
        assert!(preview(&ctx, Target::Claude, true, "20261001-120001").unwrap().unchanged);

        // Removal takes both of ours and nothing else.
        let removal = preview(&ctx, Target::Claude, false, "20261001-120002").unwrap();
        assert!(removal.diff.contains("PermissionRequest") && removal.diff.contains("PreToolUse\""), "{}", removal.diff);
        apply(&ctx, Target::Claude, false, &removal.fingerprint).unwrap();
        assert_eq!(read_settings(&ctx), json!({ "hooks": { "PreToolUse": [foreign] } }));
        assert!(!status(&ctx, Target::Claude).installed);
    }

    #[test]
    fn an_old_claude_install_is_outdated_until_update() {
        let tmp = TempDir::new("hooks-claude-old");
        let ctx = ctx_in(&tmp);
        place_relay(&ctx);
        let entry = |event: &str, timeout: u64| {
            json!({ "hooks": [{ "type": "command", "command": hook_command(&ctx.hook_exe, event), "timeout": timeout }] })
        };
        let current = merged(&json!({}), &ctx.hook_exe);

        let old = json!({ "model": "x", "hooks": { "PermissionRequest": [entry("PermissionRequest", 120)] } });
        let no_question = json!({ "hooks": { "PermissionRequest": [entry("PermissionRequest", 600)] } });
        let mut short_question = current.clone();
        short_question["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"] = json!(120);
        let mut no_matcher = current.clone();
        no_matcher["hooks"]["PreToolUse"][0].as_object_mut().unwrap().shift_remove("matcher");
        for (name, before) in [
            ("120 s, no PreToolUse", old),
            ("no PreToolUse", no_question),
            ("PreToolUse at 120 s", short_question),
            ("PreToolUse without its matcher", no_matcher),
        ] {
            write_settings(&ctx, &before);
            let st = status(&ctx, Target::Claude);
            assert!(st.installed && st.outdated, "{name}: {st:?}");

            let update = preview(&ctx, Target::Claude, true, STAMP).unwrap();
            assert!(!update.unchanged, "{name}");
            let backup = apply(&ctx, Target::Claude, true, &update.fingerprint).unwrap();
            std::fs::remove_file(backup).unwrap();
            let after = read_settings(&ctx);
            assert_eq!(after["hooks"], current["hooks"], "{name}");
            assert_eq!(after.get("model"), before.get("model"), "{name}");
            let st = status(&ctx, Target::Claude);
            assert!(st.installed && !st.outdated, "{name}: {st:?}");
        }
    }

    #[test]
    fn a_foreign_entry_after_ours_is_not_outdated() {
        let tmp = TempDir::new("hooks-claude-foreign-after");
        let ctx = ctx_in(&tmp);
        place_relay(&ctx);
        let foreign = |cmd: &str| json!({ "hooks": [{ "type": "command", "command": cmd }] });
        let mut after_ours = merged(&json!({}), &ctx.hook_exe);
        after_ours["hooks"]["PermissionRequest"].as_array_mut().unwrap().push(foreign("mine.exe"));
        after_ours["hooks"]["PreToolUse"].as_array_mut().unwrap().push(foreign("also-mine.exe"));
        write_settings(&ctx, &after_ours);
        let st = status(&ctx, Target::Claude);
        assert!(st.installed && !st.outdated, "{st:?}");

        // Ours still count: an old timeout behind the user's entry is outdated.
        let mut old = after_ours.clone();
        old["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"] = json!(120);
        write_settings(&ctx, &old);
        assert!(status(&ctx, Target::Claude).outdated);
        // And so is an extra entry of ours.
        let mut extra = after_ours.clone();
        extra["hooks"]["PermissionRequest"].as_array_mut().unwrap().push(permission_entry_for(&ctx));
        write_settings(&ctx, &extra);
        assert!(status(&ctx, Target::Claude).outdated);
        // And a missing one.
        let mut missing = after_ours;
        missing["hooks"]["PreToolUse"].as_array_mut().unwrap().remove(0);
        write_settings(&ctx, &missing);
        assert!(status(&ctx, Target::Claude).outdated);
    }

    fn permission_entry_for(ctx: &HookCtx) -> Value {
        json!({ "hooks": [{
            "type": "command", "command": hook_command(&ctx.hook_exe, "PermissionRequest"), "timeout": 600
        }] })
    }

    #[test]
    fn coucou_entries_count_as_installed_and_outdated_and_removal_takes_them() {
        let tmp = TempDir::new("hooks-legacy");
        let ctx = ctx_in(&tmp);
        place_relay(&ctx);
        std::fs::create_dir_all(&ctx.claude_dir).unwrap();
        let existing = json!({ "hooks": { "PreToolUse": [coucou("PreToolUse")] }, "model": "x" });
        std::fs::write(ctx.claude_dir.join("settings.json"), serde_json::to_vec(&existing).unwrap()).unwrap();

        let st = status(&ctx, Target::Claude);
        assert!(st.installed && st.outdated);
        let settings = ctx.claude_dir.join("settings.json").display().to_string();
        assert_eq!(
            st.notes,
            [Message::new(
                "msg.hook.coucouLeft",
                format!("{settings} still has hook entries from Coucou. Reinstall replaces them; Remove deletes them."),
            )
            .with("path", settings)]
        );
        assert_eq!(st.target, Target::Claude);

        let removal = preview(&ctx, Target::Claude, false, STAMP).unwrap();
        assert_eq!(
            removal.notes,
            [Message::new("msg.hook.coucouRemoved", "Hook entries from Coucou are removed too.")]
        );
        assert!(removal.diff.contains("coucou-hook"), "the removed lines are shown");
        apply(&ctx, Target::Claude, false, &removal.fingerprint).unwrap();
        let after = hookfile::parse_object(&std::fs::read(ctx.claude_dir.join("settings.json")).unwrap(), "s").unwrap();
        assert_eq!(after, json!({ "model": "x" }));
        assert!(!status(&ctx, Target::Claude).installed);
    }

    /// The old filesystem test, now on a HookCtx in a temp dir instead of a
    /// process-wide USERPROFILE.
    #[test]
    fn writing_backs_up_preserves_and_refuses_a_changed_file() {
        let tmp = TempDir::new("hooks-claude");
        let ctx = ctx_in(&tmp);
        place_relay(&ctx);
        std::fs::create_dir_all(&ctx.claude_dir).unwrap();
        let path = ctx.claude_dir.join("settings.json");

        // A real-shaped file, written the way PowerShell 5 would: UTF-8 with BOM,
        // on one line, without a final newline.
        let original = r#"{"model":"claude-opus-5","theme":"dark","tui":{"x":1},"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"other-tool.exe"}]}]}}"#;
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(original.as_bytes());
        std::fs::write(&path, &bytes).unwrap();

        // Install.
        let plan = preview(&ctx, Target::Claude, true, STAMP).expect("a BOM must not stop the preview");
        assert!(plan.diff.contains(ident::MARKER), "the diff must show what changes");
        assert!(!plan.unchanged);
        let backup = apply(&ctx, Target::Claude, true, &plan.fingerprint).expect("install should succeed");

        // The backup holds the original bytes, BOM and all, next to the file.
        assert_eq!(backup, ctx.claude_dir.join("settings.json.mayak-backup-20261001-120000").to_string_lossy());
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);

        // The format survived: BOM kept, no CRLF invented, still no final newline.
        let written = std::fs::read(&path).unwrap();
        assert!(written.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert!(!written.windows(2).any(|w| w == b"\r\n"));
        assert!(!written.ends_with(b"\n"));

        // Everything else survived, and so did the other tool's hook.
        let after = hookfile::parse_object(&written, "settings.json").unwrap();
        assert_eq!(after["model"], "claude-opus-5");
        assert_eq!(after["theme"], "dark");
        assert_eq!(after["tui"]["x"], 1);
        let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(pre.iter().any(|e| serde_json::to_string(e).unwrap().contains("other-tool.exe")));
        let st = status(&ctx, Target::Claude);
        assert!(st.installed && !st.outdated && st.hook_ready);
        assert_eq!(st.settings_path, path.to_string_lossy());

        // A file that moved since the preview is refused, and left alone.
        let stale = preview(&ctx, Target::Claude, false, "20261001-120001").unwrap();
        std::fs::write(&path, br#"{"model":"someone-else-edited-this"}"#).unwrap();
        let err = apply(&ctx, Target::Claude, false, &stale.fingerprint).unwrap_err();
        assert_eq!(err.code, "msg.hook.changed");
        assert!(err.fallback.contains("changed since the preview"), "got: {err}");
        let untouched = hookfile::parse_object(&std::fs::read(&path).unwrap(), "s").unwrap();
        assert_eq!(untouched["model"], "someone-else-edited-this");

        // Content we cannot parse is refused before anything is written.
        std::fs::write(&path, b"{ broken").unwrap();
        assert!(preview(&ctx, Target::Claude, true, STAMP).is_err());
        assert!(apply(&ctx, Target::Claude, true, "whatever").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ broken");
        let st = status(&ctx, Target::Claude);
        assert!(!st.installed, "{st:?}");
        let invalid = st.notes.iter().find(|n| n.code == "msg.hook.invalidJson").expect("an invalid-JSON note");
        assert_eq!(invalid.params["path"], path.display().to_string());
        assert!(invalid.fallback.contains("isn't valid JSON"), "{invalid:?}");
    }

    #[test]
    fn claude_refuses_a_hooks_shape_it_would_have_to_overwrite() {
        let tmp = TempDir::new("hooks-claude-shape");
        let ctx = ctx_in(&tmp);
        place_relay(&ctx);
        std::fs::create_dir_all(&ctx.claude_dir).unwrap();
        let path = ctx.claude_dir.join("settings.json");
        let shown = path.display().to_string();

        for (content, key) in [
            (r#"{"model":"x","hooks":[]}"#, "hooks"),
            (r#"{"hooks":"none"}"#, "hooks"),
            (r#"{"hooks":{"PermissionRequest":{"hooks":[]}}}"#, "PermissionRequest"),
            (r#"{"hooks":{"PermissionRequest":null}}"#, "PermissionRequest"),
            (r#"{"hooks":{"PreToolUse":{"matcher":"AskUserQuestion","hooks":[]}}}"#, "PreToolUse"),
            (r#"{"hooks":{"PreToolUse":"x"}}"#, "PreToolUse"),
        ] {
            std::fs::write(&path, content).unwrap();
            let expected = Message::new(
                "msg.hook.badShape",
                format!("{shown}: \"{key}\" isn't the shape Claude Code expects — Mayak won't touch it."),
            )
            .with("path", shown.as_str())
            .with("key", key)
            .with("app", "Mayak");

            let err = preview(&ctx, Target::Claude, true, STAMP).unwrap_err();
            assert_eq!(err, expected, "{content}");
            assert!(apply(&ctx, Target::Claude, true, "whatever").is_err(), "nothing was planned: {content}");
            assert_eq!(std::fs::read(&path).unwrap(), content.as_bytes(), "left alone: {content}");

            let st = status(&ctx, Target::Claude);
            assert!(st.notes.contains(&expected), "{content}: {st:?}");
            assert!(!st.outdated, "a reinstall can't fix it: {content}");

            // Removal has nothing of ours to take out of it, and changes nothing.
            assert!(preview(&ctx, Target::Claude, false, STAMP).unwrap().unchanged, "{content}");
        }

        // An event Mayak does not touch may hold anything: it is kept as it is.
        std::fs::write(&path, r#"{"hooks":{"Stop":"someone else's"}}"#).unwrap();
        let plan = preview(&ctx, Target::Claude, true, STAMP).unwrap();
        assert!(status(&ctx, Target::Claude).notes.is_empty());
        apply(&ctx, Target::Claude, true, &plan.fingerprint).unwrap();
        let after = hookfile::parse_object(&std::fs::read(&path).unwrap(), "s").unwrap();
        assert_eq!(after["hooks"]["Stop"], "someone else's");
        assert!(after["hooks"]["PermissionRequest"].as_array().unwrap().iter().any(entry_is_ours));
    }

    #[test]
    fn codex_install_reinstall_and_remove_through_the_plan() {
        let tmp = TempDir::new("hooks-codex");
        let ctx = ctx_in(&tmp);
        place_relay(&ctx);
        let path = codex_hooks::hooks_json_path(&ctx.codex_home);
        let our = codex_hooks::our_command(&ctx.hook_exe).unwrap();

        let st = status(&ctx, Target::Codex);
        assert!(!st.installed && !st.outdated && st.hook_ready, "{st:?}");
        assert!(st.notes.is_empty(), "{:?}", st.notes);
        assert_eq!(st.settings_path, path.to_string_lossy());

        // No file yet: shown whole, written in the new-file format, no backup.
        let install = preview(&ctx, Target::Codex, true, STAMP).unwrap();
        assert!(!install.unchanged);
        assert!(install.diff.lines().all(|l| l.starts_with("+ ")), "{}", install.diff);
        assert_eq!(apply(&ctx, Target::Codex, true, &install.fingerprint), Ok(String::new()));
        let written = std::fs::read(&path).unwrap();
        let expected = codex_hooks::merge_install(&json!({}), &our).unwrap();
        assert_eq!(written, hookfile::render(&expected, hookfile::TextFormat::NEW_FILE));
        let shown: Vec<&str> = install.diff.lines().map(|l| &l[2..]).collect();
        assert_eq!(shown, String::from_utf8_lossy(&written).lines().collect::<Vec<_>>(), "diff = written bytes");
        assert!(codex_hooks::installed_at(&ctx.codex_home, Some(&our)));
        let st = status(&ctx, Target::Codex);
        assert!(st.installed && !st.outdated);

        // Reinstall of the identical entry: nothing to write (D53).
        let again = preview(&ctx, Target::Codex, true, "20261001-120001").unwrap();
        assert!(again.unchanged);
        assert_eq!(again.diff, "No change.");
        let err = apply(&ctx, Target::Codex, true, &again.fingerprint).unwrap_err();
        assert_eq!(err.code, "msg.hook.unchanged");
        assert!(err.fallback.starts_with("No change:"), "got: {err}");
        assert_eq!(std::fs::read(&path).unwrap(), written);

        // Remove: backup of the exact bytes; "hooks" stays as an empty object.
        let removal = preview(&ctx, Target::Codex, false, "20261001-120002").unwrap();
        let backup = apply(&ctx, Target::Codex, false, &removal.fingerprint).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), written);
        assert_eq!(std::fs::read(&path).unwrap(), b"{\n  \"hooks\": {}\n}\n".to_vec());
        assert!(!codex_hooks::installed_at(&ctx.codex_home, Some(&our)));
    }

    #[test]
    fn codex_status_is_outdated_while_a_stale_group_sits_outside_the_events() {
        // D55.
        let tmp = TempDir::new("hooks-codex-stale");
        let ctx = ctx_in(&tmp);
        place_relay(&ctx);
        let our = codex_hooks::our_command(&ctx.hook_exe).unwrap();
        let mut value = codex_hooks::merge_install(&json!({}), &our).unwrap();
        value["hooks"]["PreToolUse"] = json!([codex_hooks::our_group(&our, 10)]);
        std::fs::create_dir_all(&ctx.codex_home).unwrap();
        std::fs::write(codex_hooks::hooks_json_path(&ctx.codex_home), serde_json::to_vec(&value).unwrap()).unwrap();

        let st = status(&ctx, Target::Codex);
        assert!(st.installed && st.outdated, "{st:?}");
        let fix = preview(&ctx, Target::Codex, true, STAMP).unwrap();
        assert!(fix.diff.contains("- ") && fix.diff.contains("PreToolUse"), "{}", fix.diff);
        apply(&ctx, Target::Codex, true, &fix.fingerprint).unwrap();
        let st = status(&ctx, Target::Codex);
        assert!(st.installed && !st.outdated, "{st:?}");
    }

    #[test]
    fn codex_answers_switch_through_their_own_plan() {
        let tmp = TempDir::new("hooks-codex-answers");
        let ctx = ctx_in(&tmp);
        place_relay(&ctx);
        let path = codex_hooks::hooks_json_path(&ctx.codex_home);
        let our = codex_hooks::our_command(&ctx.hook_exe).unwrap();
        let answers_of = |ctx: &HookCtx| {
            let st = status(ctx, Target::Codex);
            assert_eq!(serde_json::to_value(&st).unwrap()["answers"], serde_json::to_value(st.answers).unwrap());
            st.answers
        };

        // Not installed: status says so, and the switch asks for the integration first.
        assert_eq!(answers_of(&ctx), Some(Answers::NotInstalled));
        assert_eq!(serde_json::to_value(Answers::NotInstalled).unwrap(), json!("notInstalled"));
        let err = answers_preview(&ctx, true, STAMP).unwrap_err();
        assert_eq!(err.code, "msg.codex.answersNotInstalled");
        assert!(!path.exists(), "nothing was written");

        // Installed by the integration, in a file with a BOM and CRLF.
        let off = codex_hooks::merge_install(&json!({}), &our).unwrap();
        let fmt = hookfile::TextFormat { bom: true, crlf: true, final_newline: true, mixed_eol: false };
        std::fs::create_dir_all(&ctx.codex_home).unwrap();
        std::fs::write(&path, hookfile::render(&off, fmt)).unwrap();
        assert_eq!(answers_of(&ctx), Some(Answers::Off));
        assert_eq!(serde_json::to_value(Answers::Off).unwrap(), json!("off"));

        // On: the diff shows the one handler that changes.
        let on = answers_preview(&ctx, true, STAMP).unwrap();
        assert!(!on.unchanged);
        let added = |text: &str| on.diff.lines().any(|l| l.starts_with("+ ") && l.trim_start_matches(['+', ' ']).starts_with(text));
        assert!(added("\"timeout\": 600,") && added("\"statusMessage\": \"Waiting for your answer in Mayak\""), "{}", on.diff);
        assert_eq!(on.diff.lines().filter(|l| l.starts_with("- ")).count(), 1, "only the timeout line goes: {}", on.diff);
        // Its plan is not the install plan, and the other direction does not match it.
        assert_eq!(apply(&ctx, Target::Codex, true, &on.fingerprint).unwrap_err().code, "msg.hook.noPreview");
        assert_eq!(answers_apply(&ctx, false, &on.fingerprint).unwrap_err().code, "msg.hook.previewMismatch");
        let before = std::fs::read(&path).unwrap();
        let backup = answers_apply(&ctx, true, &on.fingerprint).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), before, "byte-exact backup");
        let written = std::fs::read(&path).unwrap();
        assert_eq!(written, hookfile::render(&codex_hooks::merge_answers(&off, &our, true).unwrap(), fmt), "BOM and CRLF kept");
        assert!(codex_hooks::answers_enabled_at(&ctx.codex_home, Some(&our)));
        let st = status(&ctx, Target::Codex);
        assert!(st.installed && !st.outdated, "a reinstall keeps answers on: {st:?}");
        assert_eq!(st.answers, Some(Answers::On));
        assert_eq!(serde_json::to_value(Answers::On).unwrap(), json!("on"));
        assert!(answers_preview(&ctx, true, "20261001-120001").unwrap().unchanged);

        // A file that moved since the preview is refused.
        let stale = answers_preview(&ctx, false, "20261001-120002").unwrap();
        std::fs::write(&path, &before).unwrap();
        assert_eq!(answers_apply(&ctx, false, &stale.fingerprint).unwrap_err().code, "msg.hook.changed");
        std::fs::write(&path, &written).unwrap();

        // Off puts back exactly the integration's install.
        let back = answers_preview(&ctx, false, "20261001-120003").unwrap();
        answers_apply(&ctx, false, &back.fingerprint).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), hookfile::render(&off, fmt));
        assert_eq!(answers_of(&ctx), Some(Answers::Off));
    }

    #[test]
    fn codex_answers_need_our_permission_group_like_the_switch_does() {
        let tmp = TempDir::new("hooks-codex-answers-no-permission");
        let ctx = ctx_in(&tmp);
        place_relay(&ctx);
        let path = codex_hooks::hooks_json_path(&ctx.codex_home);
        let our = codex_hooks::our_command(&ctx.hook_exe).unwrap();
        // Our groups everywhere but PermissionRequest, where only a foreign group sits.
        let mut value = codex_hooks::merge_install(&json!({}), &our).unwrap();
        value["hooks"]["PermissionRequest"] = json!([codex_hooks::our_group("someone-else.exe", 10)]);
        std::fs::create_dir_all(&ctx.codex_home).unwrap();
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

        let st = status(&ctx, Target::Codex);
        assert!(st.installed, "{st:?}");
        assert_eq!(st.answers, Some(Answers::NotInstalled), "{st:?}");
        assert_eq!(answers_preview(&ctx, true, STAMP).unwrap_err().code, "msg.codex.answersNotInstalled");
    }

    #[test]
    fn codex_refuses_bad_files_and_a_missing_relay() {
        let tmp = TempDir::new("hooks-codex-bad");
        let ctx = ctx_in(&tmp);
        std::fs::create_dir_all(&ctx.codex_home).unwrap();
        let path = codex_hooks::hooks_json_path(&ctx.codex_home);

        // No relay: install is refused, status explains, and removal finds a
        // long-name command by the marker alone.
        let st = status(&ctx, Target::Codex);
        assert!(!st.hook_ready, "{st:?}");
        assert_eq!(
            st.notes[0],
            Message::new(
                "msg.hook.relayMissing",
                format!("The relay {} is not in place yet. Restart Mayak.", ctx.hook_exe.display()),
            )
            .with("path", ctx.hook_exe.display().to_string())
            .with("app", "Mayak")
        );
        assert!(preview(&ctx, Target::Codex, true, STAMP).is_err());
        let ours = json!({ "hooks": { "Stop": [codex_hooks::our_group(r"C:\x\mayak-hook.exe --agent codex", 10)] } });
        std::fs::write(&path, serde_json::to_vec(&ours).unwrap()).unwrap();
        let removal = preview(&ctx, Target::Codex, false, STAMP).unwrap();
        assert!(removal.diff.contains("mayak-hook.exe"));

        // An 8.3 command has no marker: without the relay it can't be recognised,
        // and status and the Remove preview say so instead of a bare "No change.".
        let short = json!({ "hooks": { "Stop": [codex_hooks::our_group(r"C:\x\MAYAK-~1.EXE --agent codex", 10)] } });
        std::fs::write(&path, serde_json::to_vec(&short).unwrap()).unwrap();
        let blind = preview(&ctx, Target::Codex, false, STAMP).unwrap();
        assert!(blind.unchanged, "{}", blind.diff);
        let unusable = Message::new(
            "msg.codex.relayUnusable",
            "Mayak's relay isn't available, so entries written with its short (8.3) path can't be recognised. \
             Restart Mayak to restore the relay, then Remove.",
        )
        .with("app", "Mayak");
        assert!(blind.notes.contains(&unusable), "{:?}", blind.notes);
        let st = status(&ctx, Target::Codex);
        assert!(!st.installed && st.notes.contains(&unusable), "{st:?}");

        place_relay(&ctx);
        std::fs::write(&path, br#"{"hooks":{},"state":{}}"#).unwrap();
        let err = preview(&ctx, Target::Codex, true, STAMP).unwrap_err();
        assert_eq!(err.code, "msg.codex.unknownKey");
        assert_eq!(err.params["key"], "state");
        assert!(err.fallback.contains("\"state\""), "got: {err}");
        assert!(status(&ctx, Target::Codex).notes.contains(&err));

        std::fs::write(&path, b"{ nope").unwrap();
        let err = preview(&ctx, Target::Codex, true, STAMP).unwrap_err();
        assert_eq!(err.code, "msg.hook.invalidJson");
        assert!(err.fallback.contains("isn't valid JSON"), "got: {err}");
        assert_eq!(std::fs::read(&path).unwrap(), b"{ nope");
    }

    #[test]
    fn unknown_target_and_unknown_fingerprint_are_refused() {
        let tmp = TempDir::new("hooks-refuse");
        let ctx = ctx_in(&tmp);
        place_relay(&ctx);
        assert!(Target::parse(Some("vscode")).is_err());
        assert!(apply(&ctx, Target::Codex, true, "0000000000000000").is_err(), "nothing previewed");

        let p = preview(&ctx, Target::Codex, true, STAMP).unwrap();
        assert!(apply(&ctx, Target::Codex, true, "0000000000000000").is_err(), "wrong fingerprint");
        assert!(apply(&ctx, Target::Claude, true, &p.fingerprint).is_err(), "other target");
        assert!(apply(&ctx, Target::Codex, false, &p.fingerprint).is_err(), "other direction");
        assert!(!codex_hooks::hooks_json_path(&ctx.codex_home).exists(), "nothing was written");
        assert!(apply(&ctx, Target::Codex, true, &p.fingerprint).is_ok(), "the matching plan still applies");
    }

    #[test]
    fn antigravity_install_keeps_the_users_hooks_backs_up_and_remove_restores_them() {
        let tmp = TempDir::new("hooks-antigravity");
        let ctx = ctx_in(&tmp);
        place_relay(&ctx);
        let path = antigravity_hooks::hooks_json_path(&ctx.gemini_dir);
        let our = antigravity_hooks::our_command(&ctx.hook_exe).unwrap();
        assert!(our.ends_with(" --agent antigravity"), "{our}");

        // No file yet (the folder holds other Antigravity files): created through the plan.
        let other = ctx.gemini_dir.join("config").join("config.json");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, b"{\"x\":1}").unwrap();
        let st = status(&ctx, Target::Antigravity);
        assert!(!st.installed && !st.outdated && st.hook_ready, "{st:?}");
        assert!(st.notes.is_empty(), "{:?}", st.notes);
        assert_eq!(st.settings_path, path.to_string_lossy());
        assert!(st.answers.is_none());
        let first = preview(&ctx, Target::Antigravity, true, STAMP).unwrap();
        assert!(!path.exists(), "a preview writes nothing");
        assert_eq!(apply(&ctx, Target::Antigravity, true, &first.fingerprint), Ok(String::new()));
        assert!(status(&ctx, Target::Antigravity).installed);
        assert_eq!(std::fs::read(&other).unwrap(), b"{\"x\":1}", "nothing else in the folder changes");

        // The user's own groups, written after: a reinstall keeps them, in their places.
        let theirs = json!({
            "guard-shell": { "PreToolUse": [{ "matcher": "run_command", "hooks": [{ "command": "./guard.sh" }] }] },
            "mayak": antigravity_hooks::our_group(&our),
            "notify": { "enabled": false, "Stop": [{ "command": "./notify.sh", "timeout": 5 }] }
        });
        let bytes = serde_json::to_vec_pretty(&theirs).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let st = status(&ctx, Target::Antigravity);
        assert!(st.installed && !st.outdated, "{st:?}");
        let again = preview(&ctx, Target::Antigravity, true, "20261001-120001").unwrap();
        assert!(again.unchanged, "{}", again.diff);
        assert_eq!(apply(&ctx, Target::Antigravity, true, &again.fingerprint).unwrap_err().code, "msg.hook.unchanged");

        // Remove: a backup of the exact bytes, only Mayak's group goes.
        let removal = preview(&ctx, Target::Antigravity, false, "20261001-120002").unwrap();
        assert!(removal.diff.contains("--agent antigravity"), "{}", removal.diff);
        let backup = apply(&ctx, Target::Antigravity, false, &removal.fingerprint).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);
        let after = hookfile::parse_object(&std::fs::read(&path).unwrap(), "h").unwrap();
        let mut expected = theirs.clone();
        expected.as_object_mut().unwrap().shift_remove("mayak");
        assert_eq!(after, expected);
        let keys: Vec<&str> = after.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["guard-shell", "notify"]);
        assert!(!status(&ctx, Target::Antigravity).installed);
        // Removing twice has nothing to write.
        assert!(preview(&ctx, Target::Antigravity, false, "20261001-120003").unwrap().unchanged);
    }

    #[test]
    fn antigravity_refuses_a_foreign_group_bad_files_and_a_missing_relay() {
        let tmp = TempDir::new("hooks-antigravity-bad");
        let ctx = ctx_in(&tmp);
        let path = antigravity_hooks::hooks_json_path(&ctx.gemini_dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        // No relay: no install, and status says why.
        let st = status(&ctx, Target::Antigravity);
        assert!(!st.hook_ready);
        assert!(st.notes.iter().any(|n| n.code == "msg.codex.relayUnusable"), "{st:?}");
        assert!(preview(&ctx, Target::Antigravity, true, STAMP).is_err());

        place_relay(&ctx);
        let foreign = br#"{"mayak":{"Stop":[{"command":"/bin/notify"}]}}"#;
        std::fs::write(&path, foreign).unwrap();
        let err = preview(&ctx, Target::Antigravity, true, STAMP).unwrap_err();
        assert_eq!(err.code, "msg.antigravity.groupTaken");
        assert!(status(&ctx, Target::Antigravity).notes.contains(&err));
        assert!(preview(&ctx, Target::Antigravity, false, STAMP).unwrap().unchanged, "not ours to remove");
        assert_eq!(std::fs::read(&path).unwrap(), foreign);

        for broken in [&b"{ nope"[..], b"[1,2]", b"\"text\""] {
            std::fs::write(&path, broken).unwrap();
            assert!(preview(&ctx, Target::Antigravity, true, STAMP).is_err());
            assert!(preview(&ctx, Target::Antigravity, false, STAMP).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), broken);
        }
    }
}
