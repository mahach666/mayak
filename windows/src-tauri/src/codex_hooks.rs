// Optional Codex integration: Mayak's groups in ~/.codex/hooks.json (spec §8).
//
// Codex trusts user hooks by position (<file>:<event>:<group index>:<handler
// index>) and by a hash of command/timeout/matcher/statusMessage, so the merge
// never moves anybody else's group: install appends at the end of the event's
// array, reinstall replaces our group in place, uninstall removes our group only
// when it is last and otherwise leaves the placeholder {"hooks":[]}. Our group
// carries only type/command/timeout, and the command never changes between
// Mayak versions, so a reinstall keeps the user's Trust.
//
// The command is the relay's absolute path without quotes plus "--agent codex".
// Codex runs it through `powershell -Command` or `cmd /C`, so the path must mean
// the same to both: it is shortened with GetShortPathNameW and then checked
// against an allow-list. Codex 0.144/0.146 reject unknown top-level keys
// (deny_unknown_fields), so a file with anything but "description" and "hooks"
// at the top is refused rather than "fixed".

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::GetShortPathNameW;

use crate::hookfile;
use crate::ident;
use crate::message::Message;

/// Events Mayak installs, with the hook timeout in seconds.
pub const CODEX_EVENTS: &[(&str, u64)] = &[
    ("UserPromptSubmit", 10),
    ("PermissionRequest", 10),
    ("Stop", 10),
    ("Interrupt", 2),
    ("SessionEnd", 2),
];

const TOP_LEVEL_KEYS: &[&str] = &["description", "hooks"];

pub fn hooks_json_path(codex_home: &Path) -> PathBuf {
    codex_home.join("hooks.json")
}

/// GetShortPathNameW. Fails when the file does not exist.
pub fn short_path(long: &Path) -> Result<String, Message> {
    use std::os::windows::ffi::OsStrExt;

    let wide: Vec<u16> = long.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut buf = vec![0u16; 260];
    loop {
        // SAFETY: `wide` is NUL-terminated and outlives the call; the output
        // slice is passed with its own length.
        let n = unsafe { GetShortPathNameW(PCWSTR(wide.as_ptr()), Some(&mut buf)) } as usize;
        if n == 0 {
            let err = std::io::Error::last_os_error();
            return Err(Message::new(
                "msg.codex.shortPathFailed",
                format!("Can't resolve the short path of {}: {err}", long.display()),
            )
            .with("path", long.display().to_string())
            .with("err", err.to_string()));
        }
        if n < buf.len() {
            // Success: n characters, without the terminating NUL.
            buf.truncate(n);
            return String::from_utf16(&buf).map_err(|_| {
                Message::new(
                    "msg.codex.shortPathNotUtf16",
                    format!("The short path of {} isn't valid UTF-16.", long.display()),
                )
                .with("path", long.display().to_string())
            });
        }
        // Too small: n is the size needed, NUL included.
        buf.resize(n, 0);
    }
}

/// `^[A-Za-z]:(\\[\p{L}\p{N}._~-]+)+$`, with `char::is_alphabetic` for \p{L} and
/// `char::is_numeric` for \p{N} (D29). Each reason has its own code, so the whole sentence
/// translates; the offending character travels as `char`.
pub fn check_command_path(path: &str) -> Result<(), Message> {
    let refuse = |code: &'static str, why: String| -> Result<(), Message> {
        Err(Message::new(
            code,
            format!(
                "{path} can't be used as a Codex hook command: {why}. The relay path must be a drive \
                 letter followed by folder names made only of letters, digits, '.', '_', '~' and '-'."
            ),
        )
        .with("path", path))
    };
    let mut chars = path.chars();
    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => {}
        _ => return refuse("msg.codex.pathNoDrive", "it doesn't start with a drive letter".into()),
    }
    let Some(rest) = chars.as_str().strip_prefix('\\') else {
        return refuse("msg.codex.pathNotAbsolute", "it isn't an absolute path".into());
    };
    for segment in rest.split('\\') {
        if segment.is_empty() {
            return refuse("msg.codex.pathEmptyFolder", "it has an empty folder name".into());
        }
        let bad = segment
            .chars()
            .find(|c| !(c.is_alphabetic() || c.is_numeric() || matches!(c, '.' | '_' | '~' | '-')));
        if let Some(c) = bad {
            if c == ' ' {
                return refuse("msg.codex.pathSpace", "it contains a space".into());
            }
            return refuse("msg.codex.pathBadChar", format!("it contains '{c}'"))
                .map_err(|m| m.with("char", c.to_string()));
        }
    }
    Ok(())
}

/// The command written into hooks.json: "<short relay path> --agent codex".
pub fn our_command(hook_exe: &Path) -> Result<String, Message> {
    let short = short_path(hook_exe)?;
    check_command_path(&short)?;
    Ok(format!("{short} --agent codex"))
}

/// One group per event; no matcher, statusMessage, commandWindows or async, so
/// the trust hash covers as little as possible.
pub fn our_group(command: &str, timeout: u64) -> Value {
    json!({ "hooks": [ { "type": "command", "command": command, "timeout": timeout } ] })
}

fn placeholder() -> Value {
    json!({ "hooks": [] })
}

/// Top level holds only "description" and "hooks"; "hooks" is an object whose
/// values are arrays.
pub fn check_shape(v: &Value) -> Result<(), Message> {
    let app = ident::APP_NAME;
    let Some(root) = v.as_object() else {
        return Err(Message::new("msg.codex.notObject", format!("hooks.json isn't a JSON object — {app} won't touch it."))
            .with("app", app));
    };
    if let Some(key) = root.keys().find(|k| !TOP_LEVEL_KEYS.contains(&k.as_str())) {
        return Err(Message::new(
            "msg.codex.unknownKey",
            format!(
                "hooks.json has the top-level key \"{key}\". Codex only accepts \"description\" and \
                 \"hooks\" there, so {app} won't touch it."
            ),
        )
        .with("key", key.as_str())
        .with("app", app));
    }
    match root.get("hooks") {
        None => Ok(()),
        Some(Value::Object(events)) => match events.iter().find(|(_, list)| !list.is_array()) {
            Some((event, _)) => Err(Message::new(
                "msg.codex.eventNotList",
                format!("hooks.json: \"{event}\" isn't a list of hook groups — {app} won't touch it."),
            )
            .with("event", event.as_str())
            .with("app", app)),
            None => Ok(()),
        },
        Some(_) => Err(Message::new(
            "msg.codex.hooksNotObject",
            format!("hooks.json: \"hooks\" isn't an object — {app} won't touch it."),
        )
        .with("app", app)),
    }
}

/// Ours: the marker anywhere in the command (case-insensitive), or exactly our
/// current command ignoring ASCII case — GetShortPathNameW may have turned
/// "mayak-hook.exe" into an 8.3 name (D30). An empty `our_command` matches by
/// marker only.
pub fn is_ours(command: &str, our_command: &str) -> bool {
    command.to_lowercase().contains(ident::MARKER)
        || (!our_command.is_empty() && command.eq_ignore_ascii_case(our_command))
}

pub fn group_is_ours(group: &Value, our_command: &str) -> bool {
    group.get("hooks").and_then(Value::as_array).map_or(false, |handlers| {
        handlers.iter().any(|h| {
            h.get("command")
                .and_then(Value::as_str)
                .map_or(false, |c| is_ours(c, our_command))
        })
    })
}

pub fn installed(v: &Value, our_command: &str) -> bool {
    v.get("hooks").and_then(Value::as_object).map_or(false, |events| {
        events
            .values()
            .filter_map(Value::as_array)
            .flatten()
            .any(|g| group_is_ours(g, our_command))
    })
}

fn is_codex_event(event: &str) -> bool {
    CODEX_EVENTS.iter().any(|(name, _)| *name == event)
}

/// Uninstall rule for one event list: trailing groups of ours are removed, any
/// other group of ours becomes a placeholder, so no foreign group changes index.
fn remove_ours(list: &mut Vec<Value>, our_command: &str) -> bool {
    let mut changed = false;
    while list.last().map_or(false, |g| group_is_ours(g, our_command)) {
        list.pop();
        changed = true;
    }
    for group in list.iter_mut() {
        if group_is_ours(group, our_command) {
            *group = placeholder();
            changed = true;
        }
    }
    changed
}

/// Applies `remove_ours` to one event; a list it leaves empty takes its key
/// with it (key order of the other events kept).
fn uninstall_event(events: &mut Map<String, Value>, event: &str, our_command: &str) {
    let Some(Value::Array(list)) = events.get_mut(event) else { return };
    if remove_ours(list, our_command) && list.is_empty() {
        events.shift_remove(event);
    }
}

/// Install / reinstall. Missing file → call with `{}`.
pub fn merge_install(current: &Value, our_command: &str) -> Result<Value, Message> {
    check_shape(current)?;
    let mut root = current.as_object().cloned().unwrap_or_default();
    let mut events = root.get("hooks").and_then(Value::as_object).cloned().unwrap_or_default();
    for (event, timeout) in CODEX_EVENTS {
        let mut list = match events.get(*event) {
            Some(Value::Array(list)) => list.clone(),
            _ => Vec::new(),
        };
        // First group of ours is replaced in place; any further one becomes a
        // placeholder; none → append. Placeholders are never reused (D31).
        let mut placed = false;
        for group in list.iter_mut() {
            if group_is_ours(group, our_command) {
                *group = if placed { placeholder() } else { our_group(our_command, *timeout) };
                placed = true;
            }
        }
        if !placed {
            list.push(our_group(our_command, *timeout));
        }
        events.insert((*event).to_string(), Value::Array(list));
    }
    // Groups of ours under events Mayak no longer installs (e.g. after the
    // default set shrinks) get the uninstall rule (D55).
    let stale: Vec<String> = events.keys().filter(|k| !is_codex_event(k)).cloned().collect();
    for event in stale {
        uninstall_event(&mut events, &event, our_command);
    }
    root.insert("hooks".into(), Value::Object(events));
    Ok(Value::Object(root))
}

/// Uninstall. "hooks" stays even when it ends up empty (D31).
pub fn merge_uninstall(current: &Value, our_command: &str) -> Result<Value, Message> {
    check_shape(current)?;
    let mut root = current.as_object().cloned().unwrap_or_default();
    if let Some(Value::Object(events)) = root.get_mut("hooks") {
        let names: Vec<String> = events.keys().cloned().collect();
        for event in names {
            uninstall_event(events, &event, our_command);
        }
    }
    Ok(Value::Object(root))
}

/// Read-only status for `Signal::CodexHooksInstalled`. A missing, unreadable or
/// invalid file counts as not installed.
pub fn installed_at(codex_home: &Path, our_command: Option<&str>) -> bool {
    let path = hooks_json_path(codex_home);
    match hookfile::read_config(&path) {
        Ok(Some(bytes)) => hookfile::parse_object(&bytes, &path.display().to_string())
            .map_or(false, |v| installed(&v, our_command.unwrap_or(""))),
        Ok(None) | Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    const OUR: &str = r"C:\Users\dev\AppData\Local\Mayak\bin\mayak-hook.exe --agent codex";

    fn foreign(command: &str) -> Value {
        json!({ "hooks": [ { "type": "command", "command": command } ] })
    }

    fn event<'a>(v: &'a Value, name: &str) -> &'a Vec<Value> {
        v["hooks"][name].as_array().unwrap_or_else(|| panic!("{name} is not an array in {v}"))
    }

    #[test]
    fn command_paths_are_checked_against_the_allow_list() {
        for ok in [
            r"C:\Users\HP\AppData\Local\Mayak\bin\mayak-hook.exe",
            r"C:\Users\Пётр\AppData\Local\Mayak\bin\MAYAK-~1.EXE",
            r"d:\x_1\y.z",
        ] {
            assert_eq!(check_command_path(ok), Ok(()), "{ok} should pass");
        }
        for (bad, code, why) in [
            (r"C:\Users\A[1]\mayak-hook.exe", "msg.codex.pathBadChar", "'['"),
            (r"C:\Users\{x}\mayak-hook.exe", "msg.codex.pathBadChar", "'{'"),
            (r"C:\Users\a@b\mayak-hook.exe", "msg.codex.pathBadChar", "'@'"),
            (r"C:\Users\a=b\mayak-hook.exe", "msg.codex.pathBadChar", "'='"),
            (r"C:\Users\John Smith\mayak-hook.exe", "msg.codex.pathSpace", "a space"),
            (r#"C:\Users\"q"\mayak-hook.exe"#, "msg.codex.pathBadChar", "'\"'"),
            (r"C:\Users\a&b\mayak-hook.exe", "msg.codex.pathBadChar", "'&'"),
            (r"mayak-hook.exe", "msg.codex.pathNoDrive", "drive letter"),
            (r"1:\x", "msg.codex.pathNoDrive", "drive letter"),
            (r"C:", "msg.codex.pathNotAbsolute", "absolute path"),
            (r"C:x\y", "msg.codex.pathNotAbsolute", "absolute path"),
            (r"C:\", "msg.codex.pathEmptyFolder", "empty folder name"),
            (r"C:\a\\b", "msg.codex.pathEmptyFolder", "empty folder name"),
            (r"C:\a\", "msg.codex.pathEmptyFolder", "empty folder name"),
        ] {
            let err = check_command_path(bad).unwrap_err();
            assert_eq!(err.code, code, "{bad}: {err:?}");
            assert_eq!(err.params["path"], bad);
            assert!(err.fallback.starts_with(&format!("{bad} can't be used as a Codex hook command: ")), "{err:?}");
            assert!(err.fallback.contains(why), "{bad}: expected {why} in: {err}");
        }
        let err = check_command_path(r"C:\Users\a&b\mayak-hook.exe").unwrap_err();
        assert_eq!(err.params["char"], "&");
        assert_eq!(
            err.fallback,
            concat!(
                r"C:\Users\a&b\mayak-hook.exe can't be used as a Codex hook command: it contains '&'. ",
                "The relay path must be a drive letter followed by folder names made only of letters, digits, ",
                "'.', '_', '~' and '-'."
            )
        );
    }

    #[test]
    fn a_missing_file_becomes_one_group_per_event() {
        let next = merge_install(&json!({}), OUR).unwrap();
        let events = next["hooks"].as_object().unwrap();
        let names: Vec<&str> = events.keys().map(String::as_str).collect();
        assert_eq!(names, ["UserPromptSubmit", "PermissionRequest", "Stop", "Interrupt", "SessionEnd"]);
        for (name, timeout) in CODEX_EVENTS {
            assert_eq!(event(&next, name), &vec![our_group(OUR, *timeout)]);
        }
        assert_eq!(
            our_group(OUR, 2),
            json!({ "hooks": [ { "type": "command", "command": OUR, "timeout": 2 } ] })
        );
        assert_eq!(
            serde_json::to_string(&our_group("c", 10)).unwrap(),
            r#"{"hooks":[{"type":"command","command":"c","timeout":10}]}"#,
            "key order is part of what the user reviews"
        );
    }

    #[test]
    fn install_appends_after_foreign_groups_and_keeps_their_indices() {
        let current = json!({
            "description": "mine",
            "hooks": {
                "Stop": [ foreign("a.exe"), foreign("b.exe") ],
                "PreToolUse": [ foreign("c.exe") ]
            }
        });
        let next = merge_install(&current, OUR).unwrap();
        assert_eq!(next["description"], "mine");
        assert_eq!(event(&next, "Stop"), &vec![foreign("a.exe"), foreign("b.exe"), our_group(OUR, 10)]);
        assert_eq!(event(&next, "PreToolUse"), &vec![foreign("c.exe")]);
        // Existing event keys keep their place; new ones follow.
        let names: Vec<&str> = next["hooks"].as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(names, ["Stop", "PreToolUse", "UserPromptSubmit", "PermissionRequest", "Interrupt", "SessionEnd"]);
    }

    #[test]
    fn reinstall_replaces_our_group_in_place_and_placeholders_duplicates() {
        let old = r"C:\Users\dev\AppData\Local\Mayak\bin\mayak-hook.exe --agent codex --old";
        let current = json!({ "hooks": { "Stop": [
            foreign("a.exe"),
            json!({ "hooks": [ { "type": "command", "command": old, "timeout": 99 } ] }),
            foreign("b.exe"),
            json!({ "hooks": [ { "type": "command", "command": old } ] }),
            foreign("c.exe")
        ] } });
        let next = merge_install(&current, OUR).unwrap();
        assert_eq!(
            event(&next, "Stop"),
            &vec![foreign("a.exe"), our_group(OUR, 10), foreign("b.exe"), json!({ "hooks": [] }), foreign("c.exe")]
        );
    }

    #[test]
    fn reinstalling_the_same_command_changes_nothing() {
        let once = merge_install(&json!({ "hooks": { "Stop": [ foreign("a.exe") ] } }), OUR).unwrap();
        let twice = merge_install(&once, OUR).unwrap();
        assert_eq!(twice, once);
        assert!(installed(&once, OUR));
    }

    #[test]
    fn our_groups_outside_the_event_set_are_cleaned_on_install() {
        // D55: a group of ours under an event Mayak no longer installs.
        let installed_now = merge_install(&json!({}), OUR).unwrap();
        let mut current = installed_now.clone();
        current["hooks"]["PreToolUse"] = json!([ foreign("a.exe"), our_group(OUR, 10) ]);
        current["hooks"]["PostToolUse"] = json!([ our_group(OUR, 10), foreign("b.exe") ]);
        current["hooks"]["SubagentStop"] = json!([ our_group(OUR, 10) ]);

        let next = merge_install(&current, OUR).unwrap();
        assert_ne!(next, current, "so status reports outdated");
        assert_eq!(event(&next, "PreToolUse"), &vec![foreign("a.exe")]);
        assert_eq!(event(&next, "PostToolUse"), &vec![json!({ "hooks": [] }), foreign("b.exe")]);
        assert!(next["hooks"].get("SubagentStop").is_none());
        assert_eq!(merge_install(&next, OUR).unwrap(), next);
    }

    #[test]
    fn uninstall_removes_a_last_group_and_placeholders_the_others() {
        let current = json!({ "description": "d", "hooks": {
            "Stop": [ foreign("a.exe"), our_group(OUR, 10) ],
            "UserPromptSubmit": [ our_group(OUR, 10), foreign("b.exe") ],
            "SessionEnd": [ our_group(OUR, 2) ],
            "Interrupt": [ foreign("c.exe"), our_group(OUR, 2), our_group(OUR, 2) ],
            "PreCompact": []
        } });
        let next = merge_uninstall(&current, OUR).unwrap();
        assert_eq!(event(&next, "Stop"), &vec![foreign("a.exe")]);
        assert_eq!(event(&next, "UserPromptSubmit"), &vec![json!({ "hooks": [] }), foreign("b.exe")]);
        assert!(next["hooks"].get("SessionEnd").is_none(), "an array we emptied goes");
        assert_eq!(event(&next, "Interrupt"), &vec![foreign("c.exe")]);
        assert_eq!(event(&next, "PreCompact"), &Vec::<Value>::new(), "an array we did not touch stays");
        assert!(!installed(&next, OUR));

        let only_ours = merge_install(&json!({}), OUR).unwrap();
        assert_eq!(merge_uninstall(&only_ours, OUR).unwrap(), json!({ "hooks": {} }), "\"hooks\" is kept");
        assert_eq!(merge_uninstall(&json!({}), OUR).unwrap(), json!({}));
    }

    #[test]
    fn unknown_top_level_keys_and_wrong_shapes_are_refused() {
        let unknown = json!({ "hooks": {}, "state": {} });
        let err = merge_install(&unknown, OUR).unwrap_err();
        assert_eq!(err.code, "msg.codex.unknownKey");
        assert_eq!(err.params["key"], "state");
        assert_eq!(
            err.fallback,
            r#"hooks.json has the top-level key "state". Codex only accepts "description" and "hooks" there, so Mayak won't touch it."#
        );
        assert!(merge_uninstall(&unknown, OUR).is_err());

        let err = merge_install(&json!({ "hooks": [] }), OUR).unwrap_err();
        assert_eq!(err.code, "msg.codex.hooksNotObject");
        assert_eq!(err.fallback, r#"hooks.json: "hooks" isn't an object — Mayak won't touch it."#);
        let err = merge_install(&json!({ "hooks": { "Stop": {} } }), OUR).unwrap_err();
        assert_eq!(err.code, "msg.codex.eventNotList");
        assert_eq!(err.params["event"], "Stop");
        assert_eq!(err.fallback, r#"hooks.json: "Stop" isn't a list of hook groups — Mayak won't touch it."#);
        assert_eq!(check_shape(&json!([])).unwrap_err().code, "msg.codex.notObject");
        assert_eq!(check_shape(&json!({ "description": "x" })), Ok(()));
    }

    #[test]
    fn ours_is_the_marker_or_the_exact_short_command() {
        assert!(is_ours(r"C:\X\MAYAK-HOOK.EXE --agent codex", OUR));
        let short = r"C:\Users\dev\AppData\Local\Mayak\bin\MAYAK-~1.EXE --agent codex";
        assert!(is_ours(&short.to_lowercase(), short), "8.3 name: equal ignoring ASCII case");
        assert!(!is_ours(r"C:\Users\dev\AppData\Local\Mayak\bin\MAYAK-~2.EXE --agent codex", short));
        assert!(!is_ours("someone-elses-tool.exe", OUR));
        assert!(!is_ours("", ""), "an empty command is never ours by equality");
        assert!(!group_is_ours(&json!({ "hooks": [] }), OUR), "a placeholder is not ours");
    }

    fn place_relay(tmp: &TempDir) -> PathBuf {
        let bin = tmp.path.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join(ident::HOOK_EXE);
        std::fs::write(&exe, b"relay").unwrap();
        exe
    }

    #[test]
    fn short_path_resolves_an_existing_file_and_fails_on_a_missing_one() {
        let tmp = TempDir::new("codex-short");
        let exe = place_relay(&tmp);
        let short = short_path(&exe).unwrap();
        assert!(Path::new(&short).is_file(), "{short} must name the same file");
        let missing = tmp.path.join("bin").join("missing.exe");
        let err = short_path(&missing).unwrap_err();
        assert_eq!(err.code, "msg.codex.shortPathFailed");
        assert_eq!(err.params["path"], missing.display().to_string());
        assert_eq!(
            err.fallback,
            format!("Can't resolve the short path of {}: {}", missing.display(), err.params["err"])
        );
    }

    #[test]
    fn our_command_is_the_short_path_plus_agent_flag() {
        let tmp = TempDir::new("codex-command");
        let exe = place_relay(&tmp);
        let cmd = our_command(&exe).unwrap();
        let path = cmd.strip_suffix(" --agent codex").expect("ends with the agent flag");
        assert_eq!(check_command_path(path), Ok(()));
        assert!(Path::new(path).is_file());
        assert!(our_command(&tmp.path.join("nope.exe")).is_err());
    }

    #[test]
    fn installed_at_reads_the_file_and_treats_problems_as_not_installed() {
        let tmp = TempDir::new("codex-installed");
        let home = tmp.path.join(".codex");
        assert!(!installed_at(&home, Some(OUR)), "no folder, no file");
        std::fs::create_dir_all(&home).unwrap();
        let path = hooks_json_path(&home);
        assert_eq!(path, home.join("hooks.json"));

        std::fs::write(&path, b"{ broken").unwrap();
        assert!(!installed_at(&home, Some(OUR)));

        let with_ours = merge_install(&json!({}), OUR).unwrap();
        std::fs::write(&path, serde_json::to_vec(&with_ours).unwrap()).unwrap();
        assert!(installed_at(&home, Some(OUR)));
        assert!(installed_at(&home, None), "the marker alone is enough");

        let short = r"C:\Users\dev\AppData\Local\Mayak\bin\MAYAK-~1.EXE --agent codex";
        std::fs::write(&path, serde_json::to_vec(&merge_install(&json!({}), short).unwrap()).unwrap()).unwrap();
        assert!(installed_at(&home, Some(short)));
        assert!(!installed_at(&home, None), "an 8.3 command needs the current command to be recognised");
    }
}
