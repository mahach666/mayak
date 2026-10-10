// Optional Antigravity integration: Mayak's hook group in ~/.gemini/config/hooks.json
// (antigravity.google/docs/hooks), the global hooks file of the Antigravity IDE, the `agy` CLI
// and Antigravity 2.0.
//
// Each top-level key of that file is a named hook group; Mayak's is `mayak`, and nothing else in
// the file is touched: install adds or replaces that one key in place, uninstall removes it. A
// `mayak` group Mayak did not write is neither replaced nor removed. Tool events (PostToolUse)
// hold matcher groups, lifecycle events (PreInvocation, Stop) hold handlers directly; timeouts
// are in seconds.
//
// No PreToolUse: Antigravity requires a decision on it, and Mayak never takes part in its
// permissions. The command is the relay's short path, as for Codex (codex_hooks.rs), plus
// `--agent antigravity <Event>`: the payload does not name its event.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::codex_hooks;
use crate::ident;
use crate::message::Message;

/// Mayak's top-level hook group.
pub const GROUP: &str = ident::APP_SLUG;
/// Tool events: one `{"matcher": "*", "hooks": [...]}` group each.
const TOOL_EVENTS: &[&str] = &["PostToolUse"];
/// Lifecycle events: handlers directly.
const LIFECYCLE_EVENTS: &[&str] = &["PreInvocation", "Stop"];
/// Seconds; the relay itself gives up after one.
const TIMEOUT: u64 = 10;
/// What makes a command ours, besides the marker or our relay path.
const AGENT_FLAG: &str = "--agent antigravity";

/// `<gemini dir>\config\hooks.json`.
pub fn hooks_json_path(gemini_dir: &Path) -> PathBuf {
    gemini_dir.join("config").join("hooks.json")
}

/// "<short relay path> --agent antigravity": the command of every event, before the event name.
pub fn our_command(hook_exe: &Path) -> Result<String, Message> {
    let short = codex_hooks::short_path(hook_exe)?;
    codex_hooks::check_command_path(&short)?;
    Ok(format!("{short} {AGENT_FLAG}"))
}

/// Mayak's group for `base` (`our_command`).
pub fn our_group(base: &str) -> Value {
    let handler = |event: &str| json!({ "type": "command", "command": format!("{base} {event}"), "timeout": TIMEOUT });
    let mut group = Map::new();
    for event in TOOL_EVENTS {
        group.insert((*event).into(), json!([{ "matcher": "*", "hooks": [handler(event)] }]));
    }
    for event in LIFECYCLE_EVENTS {
        group.insert((*event).into(), json!([handler(event)]));
    }
    Value::Object(group)
}

/// Every `command` string anywhere in `v`.
fn commands(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (key, value) in map {
                match value {
                    Value::String(s) if key == "command" => out.push(s.clone()),
                    other => commands(other, out),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| commands(item, out)),
        _ => {}
    }
}

/// A group Mayak wrote: it has a command, and each of its commands runs the relay — by the
/// marker, or by our current short path (GetShortPathNameW may have shortened the name) — as
/// `--agent antigravity`. An empty `base` matches by the marker only.
pub fn group_is_ours(group: &Value, base: &str) -> bool {
    let mut all = Vec::new();
    commands(group, &mut all);
    let base = base.to_lowercase();
    !all.is_empty()
        && all.iter().all(|command| {
            let c = command.to_lowercase();
            c.contains(AGENT_FLAG) && (c.contains(ident::MARKER) || (!base.is_empty() && c.starts_with(&base)))
        })
}

pub fn installed(v: &Value, base: &str) -> bool {
    v.get(GROUP).is_some_and(|g| group_is_ours(g, base))
}

fn group_taken(app: &str) -> Message {
    Message::new(
        "msg.antigravity.groupTaken",
        format!("hooks.json already has a hook group named \"{GROUP}\" that {app} didn't write — {app} won't touch it."),
    )
    .with("group", GROUP)
    .with("app", app)
}

/// Install / reinstall: Mayak's group, in place when it is there, else last. Missing file → `{}`.
pub fn merge_install(current: &Value, base: &str) -> Result<Value, Message> {
    let mut root = current.as_object().cloned().unwrap_or_default();
    if root.get(GROUP).is_some_and(|g| !group_is_ours(g, base)) {
        return Err(group_taken(ident::APP_NAME));
    }
    root.insert(GROUP.into(), our_group(base));
    Ok(Value::Object(root))
}

/// Uninstall: Mayak's group goes (the other keys keep their order); anything else stays.
pub fn merge_uninstall(current: &Value, base: &str) -> Value {
    let mut root = current.as_object().cloned().unwrap_or_default();
    if root.get(GROUP).is_some_and(|g| group_is_ours(g, base)) {
        root.shift_remove(GROUP);
    }
    Value::Object(root)
}

/// A `mayak` group someone else wrote, as a status note; None otherwise.
pub fn foreign_group_note(v: &Value, base: &str) -> Option<Message> {
    v.get(GROUP).is_some_and(|g| !group_is_ours(g, base)).then(|| group_taken(ident::APP_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r"C:\Users\dev\AppData\Local\Mayak\bin\mayak-hook.exe --agent antigravity";
    const SHORT: &str = r"C:\Users\DEV~1\AppData\Local\Mayak\bin\MAYAK-~1.EXE --agent antigravity";

    fn theirs() -> Value {
        json!({
            "guard-shell": {
                "enabled": true,
                "PreToolUse": [{ "matcher": "run_command", "hooks": [{ "command": "./scripts/guard.sh" }] }]
            },
            "context-reminder": { "PreInvocation": [{ "type": "command", "command": "./remind.sh" }] }
        })
    }

    #[test]
    fn install_adds_one_named_group_after_the_users_and_keeps_them_as_they_were() {
        let next = merge_install(&theirs(), BASE).unwrap();
        let keys: Vec<&str> = next.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["guard-shell", "context-reminder", GROUP]);
        assert_eq!(next["guard-shell"], theirs()["guard-shell"]);
        assert_eq!(next["context-reminder"], theirs()["context-reminder"]);
        let ours = &next[GROUP];
        assert_eq!(ours.as_object().unwrap().len(), 3);
        assert_eq!(ours["PostToolUse"][0]["matcher"], "*");
        assert_eq!(ours["PostToolUse"][0]["hooks"][0]["command"], format!("{BASE} PostToolUse"));
        for event in ["PreInvocation", "Stop"] {
            assert_eq!(ours[event][0], json!({ "type": "command", "command": format!("{BASE} {event}"), "timeout": 10 }));
        }
        assert!(ours.get("PreToolUse").is_none(), "Mayak never takes part in Antigravity's permissions");
    }

    #[test]
    fn a_missing_file_becomes_mayaks_group_alone() {
        let next = merge_install(&json!({}), BASE).unwrap();
        assert_eq!(next, json!({ GROUP: our_group(BASE) }));
        assert!(installed(&next, BASE));
    }

    #[test]
    fn install_is_idempotent_and_replaces_an_older_group_of_ours_in_place() {
        let once = merge_install(&theirs(), BASE).unwrap();
        assert_eq!(merge_install(&once, BASE).unwrap(), once);
        // An older group of ours (another relay path, fewer events) is replaced where it stood.
        let mut old = theirs();
        old.as_object_mut().unwrap().shift_insert(1, GROUP.into(), json!({
            "Stop": [{ "command": r"D:\old\mayak-hook.exe --agent antigravity Stop", "timeout": 5 }]
        }));
        let next = merge_install(&old, BASE).unwrap();
        let keys: Vec<&str> = next.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["guard-shell", GROUP, "context-reminder"]);
        assert_eq!(next[GROUP], our_group(BASE));
    }

    #[test]
    fn uninstall_takes_only_our_group_and_twice_is_once() {
        let installed_v = merge_install(&theirs(), BASE).unwrap();
        let removed = merge_uninstall(&installed_v, BASE);
        assert_eq!(removed, theirs());
        assert_eq!(merge_uninstall(&removed, BASE), removed);
        assert_eq!(merge_uninstall(&json!({}), BASE), json!({}));
        assert!(!installed(&removed, BASE));
    }

    #[test]
    fn a_mayak_group_someone_else_wrote_is_neither_replaced_nor_removed() {
        for foreign in [
            json!({ GROUP: { "Stop": [{ "command": "/bin/notify" }] } }),
            // Our relay, but not as Antigravity's: somebody's own use of it.
            json!({ GROUP: { "Stop": [{ "command": r"C:\x\mayak-hook.exe Stop" }] } }),
            // One foreign command next to ours.
            json!({ GROUP: { "Stop": [{ "command": format!("{BASE} Stop") }, { "command": "/bin/notify" }] } }),
            json!({ GROUP: "nope" }),
            json!({ GROUP: {} }),
        ] {
            let err = merge_install(&foreign, BASE).unwrap_err();
            assert_eq!(err.code, "msg.antigravity.groupTaken");
            assert_eq!(merge_uninstall(&foreign, BASE), foreign);
            assert!(!installed(&foreign, BASE));
            assert!(foreign_group_note(&foreign, BASE).is_some());
        }
        assert!(foreign_group_note(&theirs(), BASE).is_none());
    }

    #[test]
    fn ours_is_the_marker_or_our_short_path_with_the_agent_flag() {
        let short = json!({ "Stop": [{ "command": format!("{SHORT} Stop") }] });
        assert!(group_is_ours(&short, SHORT), "our 8.3 path");
        assert!(!group_is_ours(&short, ""), "an 8.3 name has no marker");
        let marked = json!({ "Stop": [{ "command": r"C:\Elsewhere\MAYAK-HOOK.EXE --agent antigravity Stop" }] });
        assert!(group_is_ours(&marked, ""));
        let codex = json!({ "Stop": [{ "command": r"C:\x\mayak-hook.exe --agent codex Stop" }] });
        assert!(!group_is_ours(&codex, BASE));
    }
}
