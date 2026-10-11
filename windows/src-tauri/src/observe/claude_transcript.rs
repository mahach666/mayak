// Facts from Claude Code transcript records (`~/.claude/projects/<slug>/<sessionId>.jsonl`,
// spec §5.5 "Транскрипт", §7 "Вычисление итога"). Pure: the model passes parsed records in;
// nothing here reads files or keeps state.

use serde_json::Value;

use super::text::{collapse, tool_label, SUMMARY_MAX};

/// Prefix of the user record Claude Code writes when the user interrupts a turn
/// ("…by user]" or "…by user for tool use]").
pub const INTERRUPT_MARKER: &str = "[Request interrupted by user";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUse {
    pub id: String,
    /// `text::tool_label(name, input)`.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    /// The record's `error`, e.g. "rate_limit".
    pub code: Option<String>,
    /// The record's `apiErrorStatus`, e.g. 429.
    pub status: Option<u16>,
    /// Collapsed message text.
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleKind {
    /// `custom-title` / `customTitle`
    Custom,
    /// `agent-name` / `agentName`
    Agent,
    /// `ai-title` / `aiTitle`
    Ai,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    /// Main-chain `type: user`.
    User { interrupted: bool, tool_results: Vec<String> },
    /// Main-chain `type: assistant`.
    Assistant { stop_reason: Option<String>, api_error: Option<ApiError>, text: String, tool_uses: Vec<ToolUse> },
    /// `custom-title` / `agent-name` / `ai-title` (any chain).
    Title { kind: TitleKind, text: String },
}

/// `isSidechain != true` and `type` ∈ {user, assistant}. Subagent records carry
/// `isSidechain: true`.
pub fn is_main_chain(v: &Value) -> bool {
    v.get("isSidechain").and_then(Value::as_bool) != Some(true)
        && matches!(v.get("type").and_then(Value::as_str), Some("user" | "assistant"))
}

/// The fact a record contributes, or None for records that are neither main-chain
/// user/assistant nor title records (system, attachment, last-prompt, …). A title record
/// whose title is missing or blank is no title (None). Title text is `collapse(_, 160)`.
pub fn fact_of(v: &Value) -> Option<Fact> {
    match v.get("type").and_then(Value::as_str)? {
        "custom-title" => title_fact(v, TitleKind::Custom, "customTitle"),
        "agent-name" => title_fact(v, TitleKind::Agent, "agentName"),
        "ai-title" => title_fact(v, TitleKind::Ai, "aiTitle"),
        "user" if is_main_chain(v) => Some(user_fact(v.pointer("/message/content"))),
        "assistant" if is_main_chain(v) => Some(assistant_fact(v)),
        _ => None,
    }
}

/// The `message.usage` fields that together are the context of a request (context-fill spec §2.2).
const USAGE_FIELDS: [&str; 4] = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens", "output_tokens"];

/// How full the context was at a main-chain assistant record (context-fill spec §2.2): the sum of
/// the USAGE_FIELDS of `message.usage` that are unsigned integers, a missing one counting 0. None
/// for any other record, for a `usage` with none of the four, and for a sum of 0: the synthetic
/// record of an API error carries all four at 0, and it must not hide the real value. Only these
/// numbers are read.
pub fn context_used(v: &Value) -> Option<u64> {
    if v.get("type").and_then(Value::as_str) != Some("assistant") || !is_main_chain(v) {
        return None;
    }
    let usage = v.pointer("/message/usage")?.as_object()?;
    let mut sum: Option<u64> = None;
    for field in USAGE_FIELDS {
        if let Some(n) = usage.get(field).and_then(Value::as_u64) {
            sum = Some(sum.unwrap_or(0).saturating_add(n));
        }
    }
    sum.filter(|n| *n > 0)
}

/// The context fill of a read chunk: `context_used` of its last record that has one.
pub fn last_context_used(records: &[(u64, Value)]) -> Option<u64> {
    records.iter().rev().find_map(|(_, v)| context_used(v))
}

/// One file change of a turn (ideas §3 «Итог хода»), as a successful Edit / Write tool result
/// reports it. Only the path and line counts are kept: never the file's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEdit {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    /// 1-based line of the first change in the new file.
    pub line: u32,
}

/// The file change a `type: user` record reports in its `toolUseResult` (undocumented, read
/// defensively): `filePath` and the `structuredPatch` hunks (`newStart`, `lines` prefixed with
/// '+', '-' or ' '). A Write that created the file has no hunks and `type: "create"`: every line
/// of its `content` is added, the change starts at line 1. A failed tool (its result is a string),
/// a result without both fields, and a patch that changes nothing give None.
pub fn file_edit(v: &Value) -> Option<FileEdit> {
    if v.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let result = v.get("toolUseResult")?.as_object()?;
    let path = result.get("filePath")?.as_str()?;
    if path.trim().is_empty() {
        return None;
    }
    let hunks = result.get("structuredPatch")?.as_array()?;
    let (mut added, mut removed, mut line) = (0u32, 0u32, None::<u32>);
    for hunk in hunks {
        let start = hunk.get("newStart").and_then(Value::as_u64).map_or(1, |n| u32::try_from(n).unwrap_or(u32::MAX));
        let mut context = 0u32;
        for text in hunk.get("lines").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            match text.as_bytes().first() {
                Some(b'+') => added = added.saturating_add(1),
                Some(b'-') => removed = removed.saturating_add(1),
                Some(b' ') => {
                    context = context.saturating_add(1);
                    continue;
                }
                _ => continue, // "\ No newline at end of file" and the like
            }
            line.get_or_insert(start.saturating_add(context).max(1));
        }
    }
    if added == 0 && removed == 0 {
        if hunks.is_empty() && result.get("type").and_then(Value::as_str) == Some("create") {
            let lines = result.get("content").and_then(Value::as_str).map_or(0, |c| c.lines().count());
            return Some(FileEdit { path: path.to_owned(), added: u32::try_from(lines).unwrap_or(u32::MAX), removed: 0, line: 1 });
        }
        return None;
    }
    Some(FileEdit { path: path.to_owned(), added, removed, line: line.unwrap_or(1) })
}

/// The files a turn changed, one entry per path in the order of their first change: the line
/// counts summed, the line of the first change kept. Paths are compared ignoring ASCII case and
/// with `/` read as `\`, as Windows does; the first spelling is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub added: u64,
    pub removed: u64,
    pub line: u32,
}

pub fn turn_files<'a>(edits: impl IntoIterator<Item = &'a FileEdit>) -> Vec<FileChange> {
    let mut out: Vec<FileChange> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for edit in edits {
        let key = edit.path.to_ascii_lowercase().replace('/', "\\");
        match index.get(&key) {
            Some(&i) => {
                out[i].added += u64::from(edit.added);
                out[i].removed += u64::from(edit.removed);
            }
            None => {
                index.insert(key, out.len());
                out.push(FileChange {
                    path: edit.path.clone(),
                    added: u64::from(edit.added),
                    removed: u64::from(edit.removed),
                    line: edit.line,
                });
            }
        }
    }
    out
}

fn title_fact(v: &Value, kind: TitleKind, field: &str) -> Option<Fact> {
    let text = collapse(v.get(field)?.as_str()?, SUMMARY_MAX);
    (!text.is_empty()).then_some(Fact::Title { kind, text })
}

/// `interrupted`: the string content or a text block is the interrupt record, i.e. starts with
/// INTERRUPT_MARKER after leading whitespace (D62: a prompt quoting the marker mid-sentence is
/// not an interrupt). tool_result content does not count: tool output may quote the marker.
fn user_fact(content: Option<&Value>) -> Fact {
    let mut interrupted = false;
    let mut tool_results = Vec::new();
    match content {
        Some(Value::String(s)) => interrupted = is_interrupt(s),
        Some(Value::Array(blocks)) => {
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                        interrupted |= is_interrupt(text);
                    }
                    Some("tool_result") => {
                        if let Some(id) = block.get("tool_use_id").and_then(Value::as_str) {
                            tool_results.push(id.to_owned());
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    Fact::User { interrupted, tool_results }
}

fn is_interrupt(text: &str) -> bool {
    text.trim_start().starts_with(INTERRUPT_MARKER)
}

/// Text blocks joined by ' ' and collapsed to SUMMARY_MAX; thinking blocks are ignored.
/// `isApiErrorMessage: true` → ApiError { error, apiErrorStatus, the same text }.
fn assistant_fact(v: &Value) -> Fact {
    let mut texts: Vec<&str> = Vec::new();
    let mut tool_uses = Vec::new();
    match v.pointer("/message/content") {
        Some(Value::String(s)) => texts.push(s),
        Some(Value::Array(blocks)) => {
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => texts.extend(block.get("text").and_then(Value::as_str)),
                    Some("tool_use") => {
                        if let Some(id) = block.get("id").and_then(Value::as_str) {
                            let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                            tool_uses.push(ToolUse { id: id.to_owned(), label: tool_label(name, block.get("input")) });
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    let text = collapse(&texts.join(" "), SUMMARY_MAX);
    let api_error = (v.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true)).then(|| ApiError {
        code: v.get("error").and_then(Value::as_str).map(str::to_owned),
        status: v.get("apiErrorStatus").and_then(Value::as_u64).and_then(|s| u16::try_from(s).ok()),
        text: text.clone(),
    });
    Fact::Assistant {
        stop_reason: v.pointer("/message/stop_reason").and_then(Value::as_str).map(str::to_owned),
        api_error,
        text,
        tool_uses,
    }
}

/// Tool pairing, one fact at a time: an Assistant fact adds its tool_uses as (id, label);
/// a User fact removes the ids in its tool_results. Title facts change nothing.
pub fn apply_tools(pending: &mut Vec<(String, String)>, fact: &Fact) {
    match fact {
        Fact::Assistant { tool_uses, .. } => {
            pending.extend(tool_uses.iter().map(|t| (t.id.clone(), t.label.clone())));
        }
        Fact::User { tool_results, .. } => pending.retain(|(id, _)| !tool_results.contains(id)),
        Fact::Title { .. } => {}
    }
}

/// Unresolved tool calls (id, label) in call order, computed from scratch.
pub fn pair_tools<'a>(facts: impl IntoIterator<Item = &'a Fact>) -> Vec<(String, String)> {
    let mut pending = Vec::new();
    for fact in facts {
        apply_tools(&mut pending, fact);
    }
    pending
}

/// Label of the last tool_use among `facts` (the session's `activity`).
pub fn last_tool_label<'a>(facts: impl IntoIterator<Item = &'a Fact>) -> Option<String> {
    facts
        .into_iter()
        .filter_map(|fact| match fact {
            Fact::Assistant { tool_uses, .. } => tool_uses.last(),
            _ => None,
        })
        .last()
        .map(|t| t.label.clone())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    Pending,
    Interrupted,
    Error(ApiError),
    Finished { summary: String },
}

/// Outcome of the turn made of the facts with offset >= `after` (spec §7). Assistant records
/// without a stop_reason (streaming fragments) are skipped, so a later successful retry beats
/// an earlier API error:
/// 1. the last User fact is `interrupted` → Interrupted;
/// 2. A = the last Assistant fact with a stop_reason or an API error; A has an API error → Error;
/// 3. A.stop_reason != "tool_use" and no later fact has tool_uses or tool_results →
///    Finished { A.text, or, when empty, the text of the nearest earlier Assistant fact in range
///    with non-empty text, else "" };
/// 4. otherwise Pending (also when there is no such Assistant fact).
/// The second value is the offset of the deciding fact (the User fact for Interrupted, A for
/// Error/Finished), None for Pending; the model uses it so that a final record read after the
/// session went idle is not read again by the next turn.
pub fn terminal_outcome(facts: &[(u64, Fact)], after: u64) -> (TurnOutcome, Option<u64>) {
    let turn: Vec<&(u64, Fact)> = facts.iter().filter(|(offset, _)| *offset >= after).collect();

    let last_user = turn.iter().rev().find_map(|(offset, fact)| match fact {
        Fact::User { interrupted, .. } => Some((*offset, *interrupted)),
        _ => None,
    });
    if let Some((offset, true)) = last_user {
        return (TurnOutcome::Interrupted, Some(offset));
    }

    let decisive = turn.iter().enumerate().rev().find_map(|(index, (offset, fact))| match fact {
        Fact::Assistant { stop_reason, api_error, text, .. } if stop_reason.is_some() || api_error.is_some() => {
            Some((index, *offset, stop_reason, api_error, text))
        }
        _ => None,
    });
    let Some((index, offset, stop_reason, api_error, text)) = decisive else {
        return (TurnOutcome::Pending, None);
    };
    if let Some(error) = api_error {
        return (TurnOutcome::Error(error.clone()), Some(offset));
    }

    let tool_activity_after = turn[index + 1..].iter().any(|(_, fact)| match fact {
        Fact::Assistant { tool_uses, .. } => !tool_uses.is_empty(),
        Fact::User { tool_results, .. } => !tool_results.is_empty(),
        Fact::Title { .. } => false,
    });
    if stop_reason.as_deref() == Some("tool_use") || tool_activity_after {
        return (TurnOutcome::Pending, None);
    }
    let summary = if text.is_empty() {
        turn[..index]
            .iter()
            .rev()
            .find_map(|(_, fact)| match fact {
                Fact::Assistant { text, .. } if !text.is_empty() => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default()
    } else {
        text.clone()
    };
    (TurnOutcome::Finished { summary }, Some(offset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const END_TURN: &str = include_str!("fixtures/claude/turn_end_turn.jsonl");
    const REFUSAL: &str = include_str!("fixtures/claude/turn_refusal.jsonl");
    const API_ERROR: &str = include_str!("fixtures/claude/turn_api_error.jsonl");
    const INTERRUPTED: &str = include_str!("fixtures/claude/turn_interrupted.jsonl");
    const TOOL_USE_AFTER_END: &str = include_str!("fixtures/claude/turn_tool_use_after_end.jsonl");
    const NO_ASSISTANT: &str = include_str!("fixtures/claude/turn_no_assistant.jsonl");
    const TOOLS_PAIRING: &str = include_str!("fixtures/claude/tools_pairing.jsonl");
    const TITLES: &str = include_str!("fixtures/claude/titles.jsonl");
    const EDITS: &str = include_str!("fixtures/claude/turn_edits.jsonl");

    /// (byte offset of the line, fact) for every line that yields a fact — what the model
    /// gets from `ClaudeTranscript.records`. Works with LF and CRLF checkouts.
    fn facts(src: &str) -> Vec<(u64, Fact)> {
        let mut out = Vec::new();
        let mut offset = 0u64;
        for line in src.split_inclusive('\n') {
            let text = line.trim_end_matches(['\r', '\n']);
            if !text.is_empty() {
                let v: Value = serde_json::from_str(text).expect("fixture lines are JSON");
                if let Some(fact) = fact_of(&v) {
                    out.push((offset, fact));
                }
            }
            offset += line.len() as u64;
        }
        out
    }

    fn plain(list: &[(u64, Fact)]) -> Vec<Fact> {
        list.iter().map(|(_, fact)| fact.clone()).collect()
    }

    fn user(tool_results: &[&str]) -> Fact {
        Fact::User { interrupted: false, tool_results: tool_results.iter().map(|s| s.to_string()).collect() }
    }

    fn assistant(stop_reason: Option<&str>, text: &str, tool_uses: &[(&str, &str)]) -> Fact {
        Fact::Assistant {
            stop_reason: stop_reason.map(str::to_owned),
            api_error: None,
            text: text.to_owned(),
            tool_uses: tool_uses.iter().map(|(id, label)| ToolUse { id: id.to_string(), label: label.to_string() }).collect(),
        }
    }

    #[test]
    fn main_chain_is_user_and_assistant_outside_sidechains() {
        assert!(is_main_chain(&json!({ "type": "user", "isSidechain": false })));
        assert!(is_main_chain(&json!({ "type": "assistant" })));
        assert!(!is_main_chain(&json!({ "type": "assistant", "isSidechain": true })));
        for other in ["system", "attachment", "last-prompt", "queue-operation", "file-history-snapshot", "ai-title"] {
            assert!(!is_main_chain(&json!({ "type": other, "isSidechain": false })), "{other}");
        }
    }

    #[test]
    fn records_outside_the_conversation_give_no_fact() {
        let records: Vec<Value> = END_TURN.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(records.len(), 8);
        assert_eq!(fact_of(&records[6]), None, "system stop_hook_summary");
        assert_eq!(fact_of(&records[7]), None, "last-prompt");
        assert_eq!(fact_of(&json!({ "type": "attachment", "isSidechain": false })), None);
        assert_eq!(fact_of(&json!([1, 2])), None);
        assert_eq!(facts(END_TURN).len(), 6);
    }

    #[test]
    fn end_turn_fixture_yields_user_and_assistant_facts() {
        let list = plain(&facts(END_TURN));
        assert_eq!(list[0], user(&[]));
        assert_eq!(list[1], assistant(Some("tool_use"), "", &[]), "thinking-only record");
        assert_eq!(list[2], assistant(Some("tool_use"), "", &[("toolu_01", "Bash · cargo test")]));
        assert_eq!(list[3], user(&["toolu_01"]));
        assert_eq!(list[5], assistant(Some("end_turn"), "All 12 tests pass. Nothing else changed.", &[]));
    }

    #[test]
    fn labels_follow_the_target_field_order() {
        let labels: Vec<String> = plain(&facts(TOOLS_PAIRING))
            .into_iter()
            .flat_map(|fact| match fact {
                Fact::Assistant { tool_uses, .. } => tool_uses.into_iter().map(|t| t.label).collect(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(
            labels,
            [
                "Bash · git status",
                r"Read · C:\Users\dev\src\demo\README.md",
                "Grep · src",
                "Which database should the demo use?",
            ]
        );
    }

    #[test]
    fn pairing_ignores_sidechain_records() {
        let list = facts(TOOLS_PAIRING);
        assert_eq!(list.len(), 7, "the two sidechain records give no fact");
        let pending = pair_tools(list.iter().map(|(_, fact)| fact));
        assert_eq!(
            pending,
            vec![
                ("toolu_B".to_string(), r"Read · C:\Users\dev\src\demo\README.md".to_string()),
                ("toolu_D".to_string(), "Which database should the demo use?".to_string()),
            ],
            "toolu_B stays open: only a sidechain record answered it"
        );
    }

    #[test]
    fn apply_tools_step_by_step_matches_pair_tools() {
        let list = facts(TOOLS_PAIRING);
        let mut pending = Vec::new();
        let mut sizes = Vec::new();
        for (_, fact) in &list {
            apply_tools(&mut pending, fact);
            sizes.push(pending.len());
        }
        assert_eq!(sizes, [0, 1, 2, 3, 2, 1, 2]);
        assert_eq!(pending, pair_tools(list.iter().map(|(_, fact)| fact)));
    }

    #[test]
    fn last_tool_label_is_the_latest_call() {
        let list = facts(TOOLS_PAIRING);
        assert_eq!(last_tool_label(list.iter().map(|(_, fact)| fact)).as_deref(), Some("Which database should the demo use?"));
        let none = facts(NO_ASSISTANT);
        assert_eq!(last_tool_label(none.iter().map(|(_, fact)| fact)), None);
    }

    #[test]
    fn the_interrupt_marker_counts_in_text_not_in_tool_output() {
        let string_content = json!({ "type": "user", "message": { "role": "user", "content": "[Request interrupted by user]" } });
        assert_eq!(fact_of(&string_content), Some(Fact::User { interrupted: true, tool_results: vec![] }));
        let last = facts(INTERRUPTED).pop().unwrap().1;
        assert_eq!(last, Fact::User { interrupted: true, tool_results: vec![] });
        // tools_pairing line 6: a tool_result whose output quotes the marker.
        assert_eq!(facts(TOOLS_PAIRING)[5].1, user(&["toolu_C"]));
    }

    #[test]
    fn a_prompt_quoting_the_marker_mid_sentence_is_no_interrupt() {
        // D62: the interrupt record starts with the marker; a prompt that mentions it is a prompt.
        let quoted = "Why is the turn lost? see the [Request interrupted by user marker in the log";
        let as_string = json!({ "type": "user", "message": { "role": "user", "content": quoted } });
        let as_block = json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "text", "text": quoted }] } });
        assert_eq!(fact_of(&as_string), Some(user(&[])));
        assert_eq!(fact_of(&as_block), Some(user(&[])));
        let indented = json!({ "type": "user", "message": { "role": "user", "content": "\n [Request interrupted by user]" } });
        assert_eq!(fact_of(&indented), Some(Fact::User { interrupted: true, tool_results: vec![] }));
        let list = vec![(0, fact_of(&as_string).unwrap()), (10, assistant(Some("end_turn"), "Only a leading marker counts.", &[]))];
        assert_eq!(
            terminal_outcome(&list, 0),
            (TurnOutcome::Finished { summary: "Only a leading marker counts.".into() }, Some(10))
        );
    }

    #[test]
    fn titles_come_from_title_records() {
        assert_eq!(
            plain(&facts(TITLES)),
            vec![
                Fact::Title { kind: TitleKind::Ai, text: "Investigate flaky test".into() },
                Fact::Title { kind: TitleKind::Custom, text: "Release prep".into() },
                Fact::Title { kind: TitleKind::Agent, text: "reviewer".into() },
            ],
            "a blank custom title and the mode record give nothing"
        );
        let sidechain_title = json!({ "type": "ai-title", "aiTitle": "Side", "isSidechain": true });
        assert_eq!(fact_of(&sidechain_title), Some(Fact::Title { kind: TitleKind::Ai, text: "Side".into() }));
    }

    #[test]
    fn end_turn_finishes_with_the_collapsed_text() {
        let list = facts(END_TURN);
        let deciding = list.last().unwrap().0;
        assert_eq!(
            terminal_outcome(&list, 0),
            (TurnOutcome::Finished { summary: "All 12 tests pass. Nothing else changed.".into() }, Some(deciding))
        );
    }

    #[test]
    fn refusal_finishes() {
        let list = facts(REFUSAL);
        let deciding = list.last().unwrap().0;
        assert_eq!(
            terminal_outcome(&list, 0),
            (TurnOutcome::Finished { summary: "I can't help with that request.".into() }, Some(deciding))
        );
    }

    #[test]
    fn an_api_error_record_is_an_error() {
        let list = facts(API_ERROR);
        let deciding = list.last().unwrap().0;
        let expected = ApiError {
            code: Some("rate_limit".into()),
            status: Some(429),
            text: "API Error: Rate limit reached. Try again later.".into(),
        };
        assert_eq!(terminal_outcome(&list, 0), (TurnOutcome::Error(expected), Some(deciding)));
    }

    #[test]
    fn a_successful_retry_after_an_api_error_finishes() {
        let mut list = facts(API_ERROR);
        list.push((10_000, assistant(Some("end_turn"), "Here is the summary.", &[])));
        assert_eq!(
            terminal_outcome(&list, 0),
            (TurnOutcome::Finished { summary: "Here is the summary.".into() }, Some(10_000))
        );
    }

    #[test]
    fn the_interrupt_marker_interrupts() {
        let list = facts(INTERRUPTED);
        let deciding = list.last().unwrap().0;
        assert_eq!(terminal_outcome(&list, 0), (TurnOutcome::Interrupted, Some(deciding)));
    }

    #[test]
    fn tool_activity_after_end_turn_keeps_the_turn_pending() {
        assert_eq!(terminal_outcome(&facts(TOOL_USE_AFTER_END), 0), (TurnOutcome::Pending, None));
        let result_after_end = vec![(0, assistant(Some("end_turn"), "Done.", &[])), (10, user(&["toolu_9"]))];
        assert_eq!(terminal_outcome(&result_after_end, 0), (TurnOutcome::Pending, None));
    }

    #[test]
    fn a_turn_without_assistant_records_is_pending() {
        assert_eq!(terminal_outcome(&facts(NO_ASSISTANT), 0), (TurnOutcome::Pending, None));
        assert_eq!(terminal_outcome(&[], 0), (TurnOutcome::Pending, None));
    }

    #[test]
    fn fragments_without_stop_reason_are_skipped_and_lend_their_text() {
        let list = vec![
            (0, user(&[])),
            (10, assistant(None, "Partial answer", &[])),
            (20, assistant(Some("end_turn"), "", &[])),
        ];
        assert_eq!(terminal_outcome(&list, 0), (TurnOutcome::Finished { summary: "Partial answer".into() }, Some(20)));
    }

    #[test]
    fn a_refusal_on_a_tool_use_record_finishes() {
        // Seen in real transcripts: stop_reason "refusal" on a record holding a tool_use block.
        let list = vec![
            (0, user(&[])),
            (10, assistant(Some("tool_use"), "Checking the logs.", &[])),
            (20, assistant(Some("refusal"), "", &[("toolu_X", "Bash · rm -rf build")])),
        ];
        assert_eq!(terminal_outcome(&list, 0), (TurnOutcome::Finished { summary: "Checking the logs.".into() }, Some(20)));
    }

    #[test]
    fn only_facts_from_the_turn_start_count() {
        let both = format!("{END_TURN}{NO_ASSISTANT}");
        let list = facts(&both);
        let second_turn = END_TURN.len() as u64;
        assert!(matches!(terminal_outcome(&list, 0).0, TurnOutcome::Finished { .. }));
        assert_eq!(terminal_outcome(&list, second_turn), (TurnOutcome::Pending, None));
        let one = facts(END_TURN);
        let after_final = one.last().unwrap().0 + 1;
        assert_eq!(terminal_outcome(&one, after_final), (TurnOutcome::Pending, None));
    }

    fn assistant_with_usage(usage: Value) -> Value {
        json!({ "type": "assistant", "isSidechain": false, "message": { "role": "assistant",
            "content": [{ "type": "text", "text": "ok" }], "stop_reason": "end_turn", "usage": usage } })
    }

    #[test]
    fn the_context_is_the_sum_of_the_four_usage_fields() {
        let usage = json!({
            "input_tokens": 3, "cache_creation_input_tokens": 1_200, "cache_read_input_tokens": 610_000,
            "output_tokens": 797, "cache_creation": { "ephemeral_5m_input_tokens": 1_200 }, "service_tier": "standard"
        });
        assert_eq!(context_used(&assistant_with_usage(usage)), Some(612_000));
    }

    #[test]
    fn a_missing_usage_field_counts_as_zero_and_no_field_gives_nothing() {
        assert_eq!(context_used(&assistant_with_usage(json!({ "input_tokens": 950 }))), Some(950));
        assert_eq!(context_used(&assistant_with_usage(json!({ "cache_read_input_tokens": 10, "output_tokens": 5 }))), Some(15));
        assert_eq!(context_used(&assistant_with_usage(json!({ "service_tier": "standard" }))), None);
        assert_eq!(
            context_used(&assistant_with_usage(json!({ "input_tokens": "950", "output_tokens": null }))),
            None,
            "only integers count"
        );
        assert_eq!(context_used(&assistant_with_usage(json!(42))), None);
        let without = json!({ "type": "assistant", "message": { "role": "assistant", "content": [] } });
        assert_eq!(context_used(&without), None);
    }

    #[test]
    fn only_main_chain_assistant_records_with_a_real_usage_count() {
        let usage = json!({ "input_tokens": 10, "output_tokens": 5 });
        let mut side = assistant_with_usage(usage.clone());
        side["isSidechain"] = json!(true);
        assert_eq!(context_used(&side), None, "a subagent's context is not the session's");
        let mut as_user = assistant_with_usage(usage);
        as_user["type"] = json!("user");
        assert_eq!(context_used(&as_user), None);
        // The synthetic record of an API error carries a usage whose four fields are all 0.
        let zeros = json!({ "input_tokens": 0, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0, "output_tokens": 0 });
        let mut synthetic = assistant_with_usage(zeros);
        synthetic["isApiErrorMessage"] = json!(true);
        synthetic["message"]["model"] = json!("<synthetic>");
        assert_eq!(context_used(&synthetic), None);
    }

    #[test]
    fn the_last_record_with_a_usage_in_the_chunk_counts() {
        let records = vec![
            (0, assistant_with_usage(json!({ "input_tokens": 100 }))),
            (10, assistant_with_usage(json!({ "input_tokens": 200 }))),
            (20, json!({ "type": "user", "message": { "role": "user", "content": "next" } })),
            (30, json!({ "type": "system", "subtype": "stop_hook_summary" })),
        ];
        assert_eq!(last_context_used(&records), Some(200));
        assert_eq!(last_context_used(&records[2..]), None);
        assert_eq!(last_context_used(&[]), None);
    }
    fn edits(src: &str) -> Vec<FileEdit> {
        src.lines().filter_map(|l| file_edit(&serde_json::from_str(l).unwrap())).collect()
    }

    #[test]
    fn edit_and_write_results_give_file_edits() {
        let lib = r"C:\Users\dev\src\demo\src\lib.rs";
        let plan = r"C:\Users\dev\src\demo\notes\plan.md";
        let cargo = r"C:\Users\dev\src\demo\Cargo.toml";
        let edit = |path: &str, added, removed, line| FileEdit { path: path.into(), added, removed, line };
        assert_eq!(
            edits(EDITS),
            vec![
                edit(lib, 2, 1, 12),  // two context lines before the first change at newStart 10
                edit(plan, 3, 0, 1),  // created: every line of its content
                edit(lib, 1, 2, 42),  // two hunks summed; the first change of the first hunk
                edit(cargo, 1, 1, 1), // a Write over an existing file has hunks like an Edit
            ],
            "the failed Edit (a string result) gives nothing"
        );
    }

    #[test]
    fn turn_files_sum_per_path_in_first_change_order() {
        let list = edits(EDITS);
        let files = turn_files(&list);
        let summary: Vec<(&str, u64, u64, u32)> =
            files.iter().map(|f| (f.path.rsplit('\\').next().unwrap(), f.added, f.removed, f.line)).collect();
        assert_eq!(summary, [("lib.rs", 3, 3, 12), ("plan.md", 3, 0, 1), ("Cargo.toml", 1, 1, 1)]);
        let mixed = [
            FileEdit { path: r"C:\A\x.rs".into(), added: 1, removed: 0, line: 5 },
            FileEdit { path: r"c:\a\X.rs".into(), added: 2, removed: 1, line: 1 },
            FileEdit { path: "C:/A/x.rs".into(), added: 1, removed: 0, line: 9 },
        ];
        assert_eq!(
            turn_files(&mixed),
            vec![FileChange { path: r"C:\A\x.rs".into(), added: 4, removed: 1, line: 5 }],
            "Windows paths differ only in case or separator: one file"
        );
        assert!(turn_files(&[]).is_empty());
    }

    #[test]
    fn records_without_a_usable_patch_give_no_edit() {
        let with = |result: Value| json!({ "type": "user", "message": { "role": "user", "content": [] }, "toolUseResult": result });
        // nothing changed: no hunks, not a creation
        assert_eq!(file_edit(&with(json!({ "type": "update", "filePath": "C:/a.txt", "content": "x", "structuredPatch": [] }))), None);
        assert_eq!(file_edit(&with(json!({ "filePath": "C:/a.txt", "structuredPatch": [{ "newStart": 1, "lines": [" same"] }] }))), None);
        // missing or mistyped fields
        assert_eq!(file_edit(&with(json!({ "filePath": "C:/a.txt" }))), None);
        assert_eq!(file_edit(&with(json!({ "structuredPatch": [{ "newStart": 1, "lines": ["+x"] }] }))), None);
        assert_eq!(file_edit(&with(json!({ "filePath": "  ", "structuredPatch": [{ "newStart": 1, "lines": ["+x"] }] }))), None);
        assert_eq!(file_edit(&with(json!({ "filePath": 7, "structuredPatch": [] }))), None);
        assert_eq!(file_edit(&with(json!("Error: file not found"))), None);
        assert_eq!(file_edit(&with(json!([{ "type": "text", "text": "Bash output" }]))), None);
        // only user records carry tool results
        let mut assistant = with(json!({ "filePath": "C:/a.txt", "structuredPatch": [{ "newStart": 1, "lines": ["+x"] }] }));
        assistant["type"] = json!("assistant");
        assert_eq!(file_edit(&assistant), None);
        // an empty creation is still a new file; a hunk without newStart starts at line 1
        assert_eq!(
            file_edit(&with(json!({ "type": "create", "filePath": "C:/e.txt", "content": "", "structuredPatch": [] }))),
            Some(FileEdit { path: "C:/e.txt".into(), added: 0, removed: 0, line: 1 })
        );
        assert_eq!(
            file_edit(&with(json!({ "filePath": "C:/a.txt", "structuredPatch": [{ "lines": ["\\ No newline at end of file", "-x", 5] }] }))),
            Some(FileEdit { path: "C:/a.txt".into(), added: 0, removed: 1, line: 1 })
        );
    }
}
