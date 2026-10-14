// Google Antigravity sessions (IDE agent and `agy` CLI), fed only by Mayak's hooks in
// ~/.gemini/config/hooks.json (antigravity_hooks.rs). The relay sends one line per
// PreInvocation, PostToolUse and Stop (hook/src/antigravity.rs); codex-io parses it here,
// proves the Antigravity process above the relay, reads the last answer from the conversation's
// transcript on Stop, and the store applies it (model.rs `antigravity_hook`). A session is its
// `conversationId`.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::keys::HostKind;
use super::paths::{self, ProjectRef};
use super::text;

/// One relay line of Antigravity, as `mayak-hook --agent antigravity` writes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AntigravityEvent {
    pub hook_event_name: String,
    /// Antigravity's `conversationId`.
    pub session_id: String,
    /// The first of `workspacePaths`; empty for a conversation without a workspace.
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<Value>,
    /// PostToolUse: the tool failed; Stop: the run ended on an error.
    #[serde(default)]
    pub error: Option<String>,
    /// Antigravity's `transcriptPath`.
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub agent_pid: Option<u32>,
    /// Creation FILETIME of `agent_pid`.
    #[serde(default)]
    pub agent_created: Option<u64>,
    #[serde(default)]
    pub agent_image: Option<String>,
    /// The Agent Orchestrator session Antigravity runs in, from its environment (the relay's
    /// `AO_CONTEXT`). Checked by `model::ao_ref`.
    #[serde(default, deserialize_with = "super::codex::string_or_none")]
    pub ao_session_id: Option<String>,
    #[serde(default, deserialize_with = "super::codex::string_or_none")]
    pub ao_project_id: Option<String>,
    /// Not on the wire: the last answer of the transcript, read by codex-io on Stop.
    #[serde(skip)]
    pub last_assistant_message: Option<String>,
    /// Relay clock, ms since the Unix epoch (UTC).
    pub relay_ts: i64,
}

/// The relay line as an event. The error names the problem and never repeats a payload value,
/// so it can be logged.
pub fn parse(payload: Value) -> Result<AntigravityEvent, String> {
    if !payload.is_object() {
        return Err("not an object".to_string());
    }
    let event: AntigravityEvent = serde_json::from_value(payload).map_err(|e| {
        let message = e.to_string();
        message.split(':').next().unwrap_or_default().trim().to_string()
    })?;
    if event.session_id.trim().is_empty() {
        return Err("empty session_id".to_string());
    }
    Ok(event)
}

/// The project of a conversation without a workspace: one row group named "Antigravity". Its
/// root is no folder, so nothing is ever opened from it.
pub fn no_workspace_project() -> ProjectRef {
    ProjectRef { key: "antigravity".into(), root: "Antigravity".into() }
}

/// The product a hook comes from. Each one keeps its data under its own folder of `~/.gemini`
/// (antigravity.google/docs/hooks: `antigravity-cli` for the CLI, `antigravity-ide` for the IDE,
/// `antigravity` for Antigravity 2.0), which the transcript path names; else the image of the
/// Antigravity process above the relay; else Unknown.
pub fn host_of(transcript_path: Option<&str>, agent_image: Option<&str>) -> HostKind {
    if let Some(path) = transcript_path {
        let key = paths::normalize(path).key;
        let mut parts = key.split(paths::SEP);
        if parts.any(|p| p == ".gemini") {
            match parts.next() {
                Some("antigravity-cli") => return HostKind::AntigravityCli,
                Some("antigravity-ide" | "antigravity") => return HostKind::AntigravityIde,
                _ => {}
            }
        }
    }
    let exe = agent_image.map(|image| paths::basename(&paths::normalize(image).display).to_lowercase());
    match exe.as_deref().map(|e| e.strip_suffix(".exe").unwrap_or(e)) {
        Some("agy") => HostKind::AntigravityCli,
        Some("antigravity") => HostKind::AntigravityIde,
        _ => HostKind::Unknown,
    }
}

/// Only a conversation transcript is ever opened: `…\.gemini\<app>\brain\<id>\…\transcript.jsonl`,
/// or `transcript_full.jsonl`, the one the IDE names in its hooks (same records). Any other path
/// a hook names is not read.
pub fn is_transcript_path(path: &str) -> bool {
    let key = paths::normalize(path).key;
    let parts: Vec<&str> = key.split(paths::SEP).collect();
    let Some(gemini) = parts.iter().position(|p| *p == ".gemini") else { return false };
    let app_ok = parts.get(gemini + 1).is_some_and(|app| app.starts_with("antigravity"));
    let brain_ok = parts.get(gemini + 2) == Some(&"brain");
    let name_ok = matches!(parts.last(), Some(&"transcript.jsonl" | &"transcript_full.jsonl"));
    app_ok && brain_ok && name_ok && Path::new(path).is_absolute()
}

/// How much of the end of a transcript is read for the last answer.
pub const TRANSCRIPT_TAIL: u64 = 512 * 1024;

/// The last answer of the model in transcript text (JSON lines): the `content` of the last record
/// with `source` "MODEL" and `type` "PLANNER_RESPONSE" (the shape of Antigravity's transcript.jsonl
/// as found on disk; the docs do not describe it), as `collapse(_, SUMMARY_MAX)`. Lines that are
/// not JSON (a line cut by the tail read) are skipped.
pub fn last_answer(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| {
            v.get("source").and_then(Value::as_str) == Some("MODEL")
                && v.get("type").and_then(Value::as_str) == Some("PLANNER_RESPONSE")
        })
        .filter_map(|v| v.get("content").and_then(Value::as_str).map(|c| text::collapse(c, text::SUMMARY_MAX)))
        .find(|answer| !answer.is_empty())
}

/// `last_answer` of the transcript at `path`, its last `TRANSCRIPT_TAIL` bytes only. None for a
/// path that is no transcript, a missing or unreadable file; the error is the kind only (no path,
/// no content), for the log.
pub fn read_last_answer(path: &str) -> Result<Option<String>, String> {
    if !is_transcript_path(path) {
        return Err("not a transcript path".into());
    }
    let mut file = std::fs::File::open(path).map_err(|e| format!("{:?}", e.kind()))?;
    let len = file.metadata().map_err(|e| format!("{:?}", e.kind()))?.len();
    let start = len.saturating_sub(TRANSCRIPT_TAIL);
    file.seek(SeekFrom::Start(start)).map_err(|e| format!("{:?}", e.kind()))?;
    let mut bytes = Vec::new();
    file.take(TRANSCRIPT_TAIL).read_to_end(&mut bytes).map_err(|e| format!("{:?}", e.kind()))?;
    Ok(last_answer(&String::from_utf8_lossy(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_relay_line_becomes_an_event() {
        let ev = parse(json!({
            "agent": "antigravity", "hook_event_name": "PostToolUse", "relay": 2, "relay_ts": 9,
            "session_id": "conv-1", "cwd": r"C:\p", "tool_name": "run_command",
            "tool_input": { "command": "cargo test" }, "error": "exit 1",
            "transcript_path": r"C:\t.jsonl", "agent_pid": 42, "agent_created": 7, "agent_image": r"C:\A\agy.exe",
            "last_assistant_message": "never from the wire"
        }))
        .unwrap();
        assert_eq!(ev.hook_event_name, "PostToolUse");
        assert_eq!(ev.session_id, "conv-1");
        assert_eq!(ev.cwd, r"C:\p");
        assert_eq!(ev.tool_name.as_deref(), Some("run_command"));
        assert_eq!(ev.tool_input, Some(json!({ "command": "cargo test" })));
        assert_eq!(ev.error.as_deref(), Some("exit 1"));
        assert_eq!((ev.agent_pid, ev.agent_created, ev.agent_image.as_deref()), (Some(42), Some(7), Some(r"C:\A\agy.exe")));
        assert_eq!(ev.last_assistant_message, None);
        assert_eq!(ev.relay_ts, 9);
        let bare = parse(json!({ "hook_event_name": "Stop", "session_id": "c", "relay_ts": 1, "agent_pid": null })).unwrap();
        assert_eq!((bare.cwd.as_str(), bare.tool_name, bare.error, bare.agent_pid), ("", None, None, None));
    }

    #[test]
    fn broken_lines_name_the_problem_without_their_values() {
        assert_eq!(parse(json!([1])).unwrap_err(), "not an object");
        assert_eq!(parse(json!({ "hook_event_name": "Stop", "session_id": " ", "relay_ts": 1 })).unwrap_err(), "empty session_id");
        let missing = parse(json!({ "hook_event_name": "Stop", "relay_ts": 1, "cwd": "secret-path" })).unwrap_err();
        assert!(missing.contains("session_id") && !missing.contains("secret"), "{missing}");
    }

    use crate::testutil::wp;

    const IDE: &str = r"C:\Users\dev\.gemini\antigravity\brain\c-1\.system_generated\logs\transcript.jsonl";
    const CLI: &str = r"C:\Users\dev\.gemini\antigravity-cli\brain\c-1\.system_generated\logs\transcript.jsonl";

    /// The host of a transcript and an image given as Windows fixture paths.
    fn host(transcript: Option<&str>, image: Option<&str>) -> HostKind {
        host_of(transcript.map(wp).as_deref(), image.map(wp).as_deref())
    }

    #[test]
    fn the_host_comes_from_the_transcript_folder_else_the_process() {
        assert_eq!(host(Some(IDE), None), HostKind::AntigravityIde);
        assert_eq!(host(Some(&IDE.replace("antigravity", "antigravity-ide")), None), HostKind::AntigravityIde);
        assert_eq!(host(Some(CLI), Some(r"C:\A\Antigravity.exe")), HostKind::AntigravityCli, "the folder first");
        assert_eq!(host(None, Some(r"C:\Users\dev\AppData\Local\agy\bin\agy.exe")), HostKind::AntigravityCli);
        assert_eq!(host(Some(r"C:\elsewhere\t.jsonl"), Some(r"C:\P\antigravity\Antigravity.exe")), HostKind::AntigravityIde);
        assert_eq!(host(None, Some(r"C:\x\language_server.exe")), HostKind::Unknown);
        assert_eq!(host(None, None), HostKind::Unknown);
    }

    #[test]
    fn only_a_conversation_transcript_is_read() {
        assert!(is_transcript_path(&wp(IDE)));
        assert!(is_transcript_path(&wp(CLI)));
        // The IDE names the full transcript, with forward slashes.
        assert!(is_transcript_path(&wp(
            "C:/Users/dev/.gemini/antigravity/brain/c-1/.system_generated/logs/transcript_full.jsonl"
        )));
        for other in [
            r"C:\Users\dev\.gemini\antigravity\brain\c-1\transcript_full.jsonl.bak",
            r"C:\Users\dev\.gemini\config\hooks.json",
            r"C:\Users\dev\.gemini\oauth_creds.json",
            r"C:\Users\dev\.gemini\antigravity\brain\c-1\secret.jsonl",
            r"C:\Users\dev\.codex\brain\transcript.jsonl",
            r"C:\Users\dev\.gemini\antigravity\conversations\transcript.jsonl",
            r"relative\.gemini\antigravity\brain\transcript.jsonl",
            "",
        ] {
            let other = wp(other);
            assert!(!is_transcript_path(&other), "{other}");
            assert!(read_last_answer(&other).is_err(), "{other}");
        }
    }

    fn record(source: &str, kind: &str, content: &str) -> String {
        json!({ "step_index": 1, "source": source, "type": kind, "status": "DONE", "content": content }).to_string()
    }

    #[test]
    fn the_last_answer_is_the_last_planner_response_of_the_model() {
        let text = [
            record("USER_EXPLICIT", "USER_INPUT", "question"),
            record("MODEL", "PLANNER_RESPONSE", "first answer"),
            record("USER_EXPLICIT", "USER_INPUT", "again"),
            record("MODEL", "PLANNER_RESPONSE", "All   12 tests\n\npass."),
            record("MODEL", "PLANNER_RESPONSE", "  "),
            record("MODEL", "GENERIC", "an error record"),
            record("SYSTEM", "SYSTEM_MESSAGE", "system"),
            "{\"cut".to_string(),
        ]
        .join("\n");
        assert_eq!(last_answer(&text).as_deref(), Some("All 12 tests pass."));
        assert_eq!(last_answer(&record("USER_EXPLICIT", "USER_INPUT", "q")), None);
        assert_eq!(last_answer(""), None);
        let long = "x".repeat(500);
        assert_eq!(last_answer(&record("MODEL", "PLANNER_RESPONSE", &long)).unwrap().chars().count(), text::SUMMARY_MAX);
    }

    #[test]
    fn read_last_answer_reads_the_end_of_the_file() {
        let tmp = crate::testutil::TempDir::new("antigravity-transcript");
        let dir = tmp.path.join(".gemini").join("antigravity").join("brain").join("c-1").join("logs");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        let filler = record("MODEL", "PLANNER_RESPONSE", &"old ".repeat(1000));
        let mut text = std::iter::repeat(filler).take(200).collect::<Vec<_>>().join("\n");
        text.push('\n');
        text.push_str(&record("MODEL", "PLANNER_RESPONSE", "Done."));
        text.push('\n');
        assert!(text.len() as u64 > TRANSCRIPT_TAIL);
        std::fs::write(&path, text).unwrap();
        let shown = path.to_string_lossy().into_owned();
        assert_eq!(read_last_answer(&shown), Ok(Some("Done.".into())));
        std::fs::remove_file(&path).unwrap();
        assert!(read_last_answer(&shown).is_err(), "missing file");
    }
}
