// Hook installers (spec §8), one per target, both on the write protection in
// hookfile.rs:
// - "codex": Mayak's groups in ~/.codex/hooks.json (codex_hooks.rs), optional;
// - "claude": the legacy Allow/Deny relay in ~/.claude/settings.json, under
//   Advanced and off by default. It installs PermissionRequest only; status and
//   removal also recognise what an old Coucou install left there.
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

use crate::codex_hooks;
use crate::hookfile::{self, Plans, Target};
use crate::ident;
use crate::{platform, settings};

/// Legacy Claude target. PermissionRequest waits for a human, so it gets the
/// island's decision timeout + 10 s. No matcher: the relay itself forwards only
/// real permission requests (no AskUserQuestion, a session_id required; D87).
pub const CLAUDE_EVENTS: &[(&str, u64)] = &[("PermissionRequest", 120)];

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
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookPreview {
    pub diff: String,
    pub backup: String,
    pub settings_path: String,
    /// Handed back to apply, so only the reviewed bytes are ever written.
    pub fingerprint: String,
    pub notes: Vec<String>,
    /// The file already holds exactly this; Apply has nothing to write (D53).
    pub unchanged: bool,
}

/// What the installer works on. Built once in lib.rs from `app_paths()`; tests
/// point it at a temp directory.
pub struct HookCtx {
    pub claude_dir: PathBuf,
    pub codex_home: PathBuf,
    pub hook_exe: PathBuf,
    pub plans: Plans,
}

fn config_path(ctx: &HookCtx, target: Target) -> PathBuf {
    match target {
        Target::Claude => ctx.claude_dir.join("settings.json"),
        Target::Codex => codex_hooks::hooks_json_path(&ctx.codex_home),
    }
}

// ── Legacy Claude target ──────────────────────────────────────────────────────

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
    CLAUDE_EVENTS.iter().any(|(name, _)| *name == event)
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

/// Settings with only Mayak's PermissionRequest entry: every older entry of ours
/// (any event, Mayak or Coucou) is removed first; everything else is untouched.
fn merged(existing: &Value, exe: &Path) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let current = root.get("hooks").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut hooks = strip_ours(&current, is_claude_event);
    for (event, timeout) in CLAUDE_EVENTS {
        let mut list = hooks.get(*event).and_then(Value::as_array).cloned().unwrap_or_default();
        list.push(json!({
            "hooks": [{
                "type": "command",
                "command": hook_command(exe, event),
                "timeout": timeout,
            }]
        }));
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
fn read_value(path: &Path, notes: &mut Vec<String>) -> Option<Value> {
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

fn claude_state(ctx: &HookCtx, path: &Path, notes: &mut Vec<String>) -> (bool, bool) {
    let Some(current) = read_value(path, notes) else { return (false, false) };
    if any_entry(&current, entry_is_legacy) {
        notes.push(format!(
            "{} still has hook entries from Coucou. Reinstall replaces them; Remove deletes them.",
            path.display()
        ));
    }
    let installed = any_entry(&current, entry_is_ours);
    let outdated = installed && merged(&current, &ctx.hook_exe) != current;
    (installed, outdated)
}

/// Without a usable relay there is no current command, and the marker alone
/// does not match a command GetShortPathNameW turned into an 8.3 name.
fn relay_unusable_note() -> String {
    format!(
        "{0}'s relay isn't available, so entries written with its short (8.3) path can't be \
         recognised. Restart {0} to restore the relay, then Remove.",
        ident::APP_NAME
    )
}

fn codex_state(ctx: &HookCtx, path: &Path, hook_ready: bool, notes: &mut Vec<String>) -> (bool, bool) {
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
    let Some(current) = read_value(path, notes) else { return (false, false) };
    if let Err(err) = codex_hooks::check_shape(&current) {
        notes.push(err);
    }
    let installed = codex_hooks::installed(&current, our.as_deref().unwrap_or(""));
    let outdated = match &our {
        Some(cmd) if installed => {
            codex_hooks::merge_install(&current, cmd).map_or(false, |next| next != current)
        }
        _ => false,
    };
    (installed, outdated)
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Read-only. Never fails: problems become `notes`.
pub fn status(ctx: &HookCtx, target: Target) -> HookStatus {
    let path = config_path(ctx, target);
    let hook_ready = ctx.hook_exe.exists();
    let mut notes = Vec::new();
    if !hook_ready {
        notes.push(format!(
            "The relay {} is not in place yet. Restart {}.",
            ctx.hook_exe.display(),
            ident::APP_NAME
        ));
    }
    let (installed, outdated) = match target {
        Target::Claude => claude_state(ctx, &path, &mut notes),
        Target::Codex => codex_state(ctx, &path, hook_ready, &mut notes),
    };
    HookStatus {
        target,
        installed,
        outdated,
        settings_path: path.to_string_lossy().into_owned(),
        hook_path: ctx.hook_exe.to_string_lossy().into_owned(),
        hook_ready,
        notes,
    }
}

/// Computes the exact bytes an install (or removal) would write, keeps the plan
/// for `apply` and returns the diff to show. `stamp` names the backup.
pub fn preview(ctx: &HookCtx, target: Target, install: bool, stamp: &str) -> Result<HookPreview, String> {
    let path = config_path(ctx, target);
    let shown = path.display().to_string();
    let current_bytes = hookfile::read_config(&path)?;
    let current = match &current_bytes {
        Some(bytes) => hookfile::parse_object(bytes, &shown)?,
        None => json!({}),
    };
    let mut notes = Vec::new();
    let next = match target {
        Target::Claude => {
            if any_entry(&current, entry_is_legacy) {
                notes.push("Hook entries from Coucou are removed too.".to_string());
            }
            if install {
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
    };
    let (plan, diff, plan_notes) =
        hookfile::make_plan(target, install, &path, current_bytes.as_deref(), &next, stamp);
    notes.extend(plan_notes);
    let out = HookPreview {
        diff,
        backup: plan.backup_path.to_string_lossy().into_owned(),
        settings_path: shown,
        fingerprint: plan.fingerprint.clone(),
        notes,
        unchanged: plan.unchanged,
    };
    ctx.plans.store(plan);
    Ok(out)
}

/// Writes the plan the last preview of `target` stored, if `install` and
/// `fingerprint` match it. Ok(backup path), or Ok("") when there was no file.
pub fn apply(ctx: &HookCtx, target: Target, install: bool, fingerprint: &str) -> Result<String, String> {
    let plan = ctx.plans.take(target, install, fingerprint)?;
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
            hook_exe: tmp.path.join("bin").join(ident::HOOK_EXE),
            plans: Plans::default(),
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
    fn the_legacy_target_installs_permission_request_only() {
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
        assert_eq!(names, ["PermissionRequest", "Stop"], "Coucou-only events are gone");
        assert_eq!(
            events["PermissionRequest"],
            json!([{ "hooks": [{
                "type": "command",
                "command": "\"C:/Users/dev/AppData/Local/Mayak/bin/mayak-hook.exe\" PermissionRequest",
                "timeout": 120
            }] }])
        );
        assert_eq!(events["Stop"], json!([{ "hooks": [{ "type": "command", "command": "keep-me.exe" }] }]));
        assert_eq!(CLAUDE_EVENTS, &[("PermissionRequest", 120)]);
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
        assert!(st.notes.iter().any(|n| n.contains("Coucou")), "notes: {:?}", st.notes);
        assert_eq!(st.target, Target::Claude);

        let removal = preview(&ctx, Target::Claude, false, STAMP).unwrap();
        assert!(removal.notes.iter().any(|n| n.contains("Coucou")));
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
        assert!(err.contains("changed since the preview"), "got: {err}");
        let untouched = hookfile::parse_object(&std::fs::read(&path).unwrap(), "s").unwrap();
        assert_eq!(untouched["model"], "someone-else-edited-this");

        // Content we cannot parse is refused before anything is written.
        std::fs::write(&path, b"{ broken").unwrap();
        assert!(preview(&ctx, Target::Claude, true, STAMP).is_err());
        assert!(apply(&ctx, Target::Claude, true, "whatever").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ broken");
        let st = status(&ctx, Target::Claude);
        assert!(!st.installed && st.notes.iter().any(|n| n.contains("isn't valid JSON")));
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
        assert!(err.starts_with("No change:"), "got: {err}");
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
    fn codex_refuses_bad_files_and_a_missing_relay() {
        let tmp = TempDir::new("hooks-codex-bad");
        let ctx = ctx_in(&tmp);
        std::fs::create_dir_all(&ctx.codex_home).unwrap();
        let path = codex_hooks::hooks_json_path(&ctx.codex_home);

        // No relay: install is refused, status explains, and removal finds a
        // long-name command by the marker alone.
        let st = status(&ctx, Target::Codex);
        assert!(!st.hook_ready && st.notes.iter().any(|n| n.contains("not in place")), "{st:?}");
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
        assert!(blind.notes.iter().any(|n| n.contains("short (8.3) path")), "{:?}", blind.notes);
        let st = status(&ctx, Target::Codex);
        assert!(!st.installed && st.notes.iter().any(|n| n.contains("short (8.3) path")), "{st:?}");

        place_relay(&ctx);
        std::fs::write(&path, br#"{"hooks":{},"state":{}}"#).unwrap();
        let err = preview(&ctx, Target::Codex, true, STAMP).unwrap_err();
        assert!(err.contains("\"state\""), "got: {err}");
        assert!(status(&ctx, Target::Codex).notes.iter().any(|n| n.contains("\"state\"")));

        std::fs::write(&path, b"{ nope").unwrap();
        assert!(preview(&ctx, Target::Codex, true, STAMP).unwrap_err().contains("isn't valid JSON"));
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
}
