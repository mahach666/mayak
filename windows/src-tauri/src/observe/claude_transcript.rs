// Facts from Claude Code transcript records (`~/.claude/projects/<slug>/<sessionId>.jsonl`,
// spec §5.5 "Транскрипт", §7 "Вычисление итога"). Pure: the model passes parsed records in;
// nothing here reads files or keeps state.

use serde_json::Value;

use super::claude_usage::parse_rfc3339_ms;
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

/// The context fill of a read chunk: of its last record that tells one, `context_used`, or the
/// `postTokens` of a compaction (the context right after it, until the next request reports its own).
pub fn last_context_used(records: &[(u64, Value)]) -> Option<u64> {
    records.iter().rev().find_map(|(_, v)| context_used(v).or_else(|| compaction(v).and_then(|c| c.post)))
}

/// A context compaction (`/compact` or Claude Code's auto-compaction), from the
/// `system/compact_boundary` record that Claude Code writes when it ends. Only numbers and the time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Compaction {
    /// The record's `timestamp`, ms since the epoch; None when it is missing or not RFC 3339.
    pub at_ms: Option<i64>,
    /// `compactMetadata.preTokens`: the context before.
    pub pre: u64,
    /// `compactMetadata.postTokens`: the context after; Claude Code versions before it lack it.
    pub post: Option<u64>,
}

/// The compaction a record stands for: `type: system`, `subtype: compact_boundary`, not a
/// subagent's (`isSidechain: true` is a subagent compacting its own context), with a positive
/// integer `compactMetadata.preTokens`. None for anything else. The field is not documented: a
/// record without it is no compaction, never a guessed one.
pub fn compaction(v: &Value) -> Option<Compaction> {
    if v.get("type").and_then(Value::as_str) != Some("system")
        || v.get("subtype").and_then(Value::as_str) != Some("compact_boundary")
        || v.get("isSidechain").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let meta = v.get("compactMetadata")?;
    let pre = meta.get("preTokens").and_then(Value::as_u64).filter(|n| *n > 0)?;
    let post = meta.get("postTokens").and_then(Value::as_u64);
    let at_ms = v.get("timestamp").and_then(Value::as_str).and_then(parse_rfc3339_ms);
    Some(Compaction { at_ms, pre, post })
}

/// The last compaction of a read chunk.
pub fn last_compaction(records: &[(u64, Value)]) -> Option<Compaction> {
    records.iter().rev().find_map(|(_, v)| compaction(v))
}

/// Longest model id kept; anything longer is no model id.
const MODEL_MAX: usize = 64;

/// The model of a main-chain assistant record: `message.model` when it is a plausible model id
/// (1..=MODEL_MAX ASCII letters, digits and `-._:@/[]`). None for any other record, a missing or
/// odd value and the `<synthetic>` model of an API error record. Only this id is read.
pub fn model_of(v: &Value) -> Option<&str> {
    if v.get("type").and_then(Value::as_str) != Some("assistant") || !is_main_chain(v) {
        return None;
    }
    let model = v.pointer("/message/model")?.as_str()?;
    let valid = !model.is_empty()
        && model.len() <= MODEL_MAX
        && model.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._:@/[]".contains(&b));
    valid.then_some(model)
}

/// The model of a read chunk: `model_of` its last record that has one.
pub fn last_model(records: &[(u64, Value)]) -> Option<String> {
    records.iter().rev().find_map(|(_, v)| model_of(v)).map(str::to_owned)
}

/// Context windows of the Claude models (the API's model table; `max_input_tokens`). A model not
/// listed has no known window.
const WINDOWS: [(&str, u64); 14] = [
    ("claude-fable-5-1", 1_000_000),
    ("claude-fable-5", 1_000_000),
    ("claude-mythos-5-1", 1_000_000),
    ("claude-mythos-5", 1_000_000),
    ("claude-opus-5-5", 1_000_000),
    ("claude-opus-5", 1_000_000),
    ("claude-opus-4-8", 1_000_000),
    ("claude-opus-4-7", 1_000_000),
    ("claude-opus-4-6", 1_000_000),
    ("claude-sonnet-5-5", 1_000_000),
    ("claude-sonnet-5", 1_000_000),
    ("claude-sonnet-4-6", 1_000_000),
    ("claude-haiku-5-5", 1_000_000),
    ("claude-haiku-4-5", 200_000),
];

/// The context window of a model id, None when it is not known. A `[1m]` suffix (Claude Code's
/// 1M-context choice) is 1M. Otherwise the id is looked up exactly in WINDOWS after dropping a
/// provider prefix before `claude-` (Bedrock `us.anthropic.`), a version after `@` or `:`
/// (Vertex, Bedrock), a trailing `-v<digits>` and a trailing `-<8 digits>` date.
pub fn context_window(model: &str) -> Option<u64> {
    let start = model.find("claude-")?;
    let mut base = &model[start..];
    if let Some(stripped) = base.strip_suffix("[1m]") {
        return (stripped.len() > "claude-".len()).then_some(1_000_000);
    }
    if let Some(cut) = base.find(['@', ':']) {
        base = &base[..cut];
    }
    if let Some((head, tail)) = base.rsplit_once("-v") {
        if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) {
            base = head;
        }
    }
    if let Some((head, tail)) = base.rsplit_once('-') {
        if tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit()) {
            base = head;
        }
    }
    WINDOWS.iter().find(|(id, _)| *id == base).map(|(_, w)| *w)
}

/// The fill of a Claude context: the window of `model` when it is known and holds `used` (a count
/// beyond it means the table does not fit this session, so no window rather than a wrong one).
pub fn claude_fill(used: u64, model: Option<&str>) -> (u64, Option<u64>) {
    (used, model.and_then(context_window).filter(|w| used <= *w))
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
    const COMPACT: &str = include_str!("fixtures/claude/compact_boundary.jsonl");

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

    fn assistant_with_model(model: Value) -> Value {
        let mut v = assistant_with_usage(json!({ "input_tokens": 10 }));
        v["message"]["model"] = model;
        v
    }

    #[test]
    fn the_model_is_read_from_main_chain_assistant_records_only() {
        assert_eq!(model_of(&assistant_with_model(json!("claude-opus-5-5"))), Some("claude-opus-5-5"));
        assert_eq!(model_of(&assistant_with_model(json!("claude-opus-5-5[1m]"))), Some("claude-opus-5-5[1m]"));
        assert_eq!(model_of(&assistant_with_model(json!("<synthetic>"))), None, "an API error record");
        assert_eq!(model_of(&assistant_with_model(json!(""))), None);
        assert_eq!(model_of(&assistant_with_model(json!(42))), None);
        assert_eq!(model_of(&assistant_with_model(json!("x".repeat(65)))), None);
        assert_eq!(model_of(&assistant_with_model(json!("claude opus"))), None);
        assert_eq!(model_of(&assistant_with_usage(json!({ "input_tokens": 1 }))), None, "no model field");
        let mut side = assistant_with_model(json!("claude-haiku-4-5-20251001"));
        side["isSidechain"] = json!(true);
        assert_eq!(model_of(&side), None, "a subagent's model is not the session's");
        let mut user = assistant_with_model(json!("claude-opus-5"));
        user["type"] = json!("user");
        assert_eq!(model_of(&user), None);
    }

    #[test]
    fn the_last_model_in_the_chunk_counts() {
        let records = vec![
            (0, assistant_with_model(json!("claude-sonnet-5-5"))),
            (10, assistant_with_model(json!("claude-opus-5-5"))),
            (20, assistant_with_model(json!("<synthetic>"))),
            (30, json!({ "type": "user", "message": { "role": "user", "content": "next" } })),
        ];
        assert_eq!(last_model(&records).as_deref(), Some("claude-opus-5-5"));
        assert_eq!(last_model(&records[2..]), None);
    }

    #[test]
    fn context_windows_come_from_the_table() {
        for (model, window) in [
            ("claude-opus-5-5", Some(1_000_000)),
            ("claude-opus-5-5[1m]", Some(1_000_000)),
            ("claude-opus-5", Some(1_000_000)),
            ("claude-sonnet-5-5", Some(1_000_000)),
            ("claude-fable-5-1", Some(1_000_000)),
            ("claude-haiku-4-5", Some(200_000)),
            ("claude-haiku-4-5-20251001", Some(200_000)),
            ("claude-haiku-4-5[1m]", Some(1_000_000)),
            ("us.anthropic.claude-haiku-4-5-20251001-v1:0", Some(200_000)),
            ("claude-opus-4-8@20260101", Some(1_000_000)),
            // unknown models keep today's count without a ring
            ("claude-sonnet-4-5", None),
            ("claude-opus-5-5x", None),
            ("claude-unknown-9", None),
            ("opus", None),
            ("gpt-5", None),
            ("[1m]", None),
            ("claude-[1m]", None),
        ] {
            assert_eq!(context_window(model), window, "{model}");
        }
    }

    #[test]
    fn a_count_beyond_the_window_drops_the_window() {
        assert_eq!(claude_fill(612_000, Some("claude-opus-5-5")), (612_000, Some(1_000_000)));
        assert_eq!(claude_fill(200_000, Some("claude-haiku-4-5")), (200_000, Some(200_000)));
        assert_eq!(claude_fill(250_000, Some("claude-haiku-4-5")), (250_000, None));
        assert_eq!(claude_fill(612_000, Some("claude-x")), (612_000, None));
        assert_eq!(claude_fill(612_000, None), (612_000, None));
    }

    fn records(src: &str) -> Vec<(u64, Value)> {
        let mut out = Vec::new();
        let mut offset = 0u64;
        for line in src.split_inclusive('\n') {
            let text = line.trim_end_matches(['\r', '\n']);
            if !text.is_empty() {
                out.push((offset, serde_json::from_str(text).expect("fixture lines are JSON")));
            }
            offset += line.len() as u64;
        }
        out
    }

    #[test]
    fn a_compact_boundary_record_is_a_compaction() {
        let list = records(COMPACT);
        let main = compaction(&list[3].1).expect("the main-chain boundary");
        // 2026-10-01T14:02:10.500Z
        assert_eq!(main, Compaction { at_ms: Some(1_790_863_330_500), pre: 967_536, post: Some(27_326) });
        assert_eq!(compaction(&list[2].1), None, "a subagent's compaction is not the session's");
        assert_eq!(last_compaction(&list), Some(main));
        assert_eq!(last_compaction(&list[..3]), None);
        assert_eq!(fact_of(&list[3].1), None, "a boundary is no conversation fact");
    }

    #[test]
    fn a_boundary_without_numbers_is_no_compaction() {
        let boundary = |meta: Value| json!({ "type": "system", "subtype": "compact_boundary", "compactMetadata": meta,
            "timestamp": "2026-10-01T14:02:10Z" });
        assert_eq!(compaction(&boundary(json!({ "trigger": "manual" }))), None);
        assert_eq!(compaction(&boundary(json!({ "preTokens": "967536" }))), None, "only integers count");
        assert_eq!(compaction(&boundary(json!({ "preTokens": 0 }))), None);
        assert_eq!(compaction(&boundary(json!(42))), None);
        assert_eq!(
            compaction(&boundary(json!({ "preTokens": 150_000 }))),
            Some(Compaction { at_ms: Some(1_790_863_330_000), pre: 150_000, post: None }),
            "an older record without postTokens"
        );
        let no_time = json!({ "type": "system", "subtype": "compact_boundary", "compactMetadata": { "preTokens": 5, "postTokens": 1 } });
        assert_eq!(compaction(&no_time), Some(Compaction { at_ms: None, pre: 5, post: Some(1) }));
        let other = json!({ "type": "system", "subtype": "stop_hook_summary", "compactMetadata": { "preTokens": 5 } });
        assert_eq!(compaction(&other), None);
        let mut as_user = boundary(json!({ "preTokens": 5 }));
        as_user["type"] = json!("user");
        assert_eq!(compaction(&as_user), None);
    }

    #[test]
    fn after_a_compaction_the_context_is_its_post_tokens_until_the_next_request() {
        let list = records(COMPACT);
        assert_eq!(last_context_used(&list[..2]), Some(966_536));
        assert_eq!(last_context_used(&list), Some(27_326), "the boundary is newer than the last usage");
        let mut later = list.clone();
        later.push((99_999, assistant_with_usage(json!({ "input_tokens": 30_000 }))));
        assert_eq!(last_context_used(&later), Some(30_000));
        let without_post = vec![(0, assistant_with_usage(json!({ "input_tokens": 100 }))),
            (10, json!({ "type": "system", "subtype": "compact_boundary", "compactMetadata": { "preTokens": 100 } }))];
        assert_eq!(last_context_used(&without_post), Some(100), "no postTokens: the last usage stays");
    }
}
