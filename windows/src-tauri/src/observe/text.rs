// Short one-line texts derived from agent data: summaries and tool labels (spec §7).
// Pure: no I/O, no clock.

use serde_json::Value;

/// Longest summary, label or title the observer keeps (spec §7: "первые 160 символов").
pub const SUMMARY_MAX: usize = 160;

/// Tool input fields that name what a tool acts on, most specific first (the order of the
/// island's approval card).
pub const TARGET_FIELDS: &[&str] = &["command", "file_path", "path", "url", "query", "pattern", "prompt"];

/// Whitespace runs (including newlines) become one space, the ends are trimmed, and the
/// first `max_chars` characters are kept. No ellipsis.
pub fn collapse(s: &str, max_chars: usize) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(max_chars).collect()
}

/// Label of one tool call. `AskUserQuestion` → its first question; otherwise
/// `"<tool> · <target>"` with the first non-blank `TARGET_FIELDS` value (an array of strings
/// is joined with ' '), or `"<tool>"` without one. The result is `collapse(_, SUMMARY_MAX)`.
pub fn tool_label(tool: &str, input: Option<&Value>) -> String {
    if tool == "AskUserQuestion" {
        let question = input
            .and_then(|i| i.pointer("/questions/0/question"))
            .and_then(Value::as_str)
            .filter(|q| !q.trim().is_empty());
        if let Some(q) = question {
            return collapse(q, SUMMARY_MAX);
        }
    }
    let target = input.and_then(|i| TARGET_FIELDS.iter().find_map(|field| target_text(i.get(*field)?)));
    match target {
        Some(t) => collapse(&format!("{tool} · {t}"), SUMMARY_MAX),
        None => collapse(tool, SUMMARY_MAX),
    }
}

fn target_text(v: &Value) -> Option<String> {
    let text = match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        _ => return None,
    };
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn collapse_joins_whitespace_runs_and_trims() {
        assert_eq!(collapse("  All 12 tests\n\n pass.\t Done  ", 160), "All 12 tests pass. Done");
        assert_eq!(collapse("one\r\ntwo", 160), "one two");
    }

    #[test]
    fn collapse_keeps_the_first_chars_without_ellipsis() {
        assert_eq!(collapse("abcdef", 3), "abc");
        // Characters, not bytes: Cyrillic is two bytes per letter in UTF-8.
        assert_eq!(collapse("Привет мир", 6), "Привет");
        assert_eq!(collapse(&"x".repeat(500), SUMMARY_MAX).chars().count(), 160);
    }

    #[test]
    fn collapse_of_blank_text_is_empty() {
        assert_eq!(collapse(" \n\t ", 160), "");
        assert_eq!(collapse("", 160), "");
    }

    #[test]
    fn tool_label_takes_the_first_target_field_in_order() {
        let grep = json!({ "pattern": "TODO", "path": "src" });
        assert_eq!(tool_label("Grep", Some(&grep)), "Grep · src");
        let bash = json!({ "command": "cargo test", "description": "Run the tests" });
        assert_eq!(tool_label("Bash", Some(&bash)), "Bash · cargo test");
        let fetch = json!({ "prompt": "Summarise", "url": "https://example.com/docs" });
        assert_eq!(tool_label("WebFetch", Some(&fetch)), "WebFetch · https://example.com/docs");
    }

    #[test]
    fn tool_label_skips_blank_and_non_string_fields() {
        let edit = json!({ "command": 5, "file_path": "   ", "path": "src/main.rs" });
        assert_eq!(tool_label("Edit", Some(&edit)), "Edit · src/main.rs");
    }

    #[test]
    fn tool_label_joins_string_arrays() {
        let shell = json!({ "type": "exec", "command": ["cargo", "test", "--offline"] });
        assert_eq!(tool_label("shell", Some(&shell)), "shell · cargo test --offline");
    }

    #[test]
    fn tool_label_without_a_target_is_the_tool_name() {
        let todo = json!({ "todos": [{ "content": "x", "status": "pending" }] });
        assert_eq!(tool_label("TodoWrite", Some(&todo)), "TodoWrite");
        assert_eq!(tool_label("Read", None), "Read");
        assert_eq!(tool_label("exec", Some(&json!("not an object"))), "exec");
    }

    #[test]
    fn ask_user_question_label_is_the_first_question() {
        let input = json!({ "questions": [
            { "question": "Which database should the demo use?", "header": "Database",
              "multiSelect": false, "options": [{ "label": "SQLite", "description": "File based" }] },
            { "question": "Second question?", "header": "More", "multiSelect": false, "options": [] }
        ] });
        assert_eq!(tool_label("AskUserQuestion", Some(&input)), "Which database should the demo use?");
    }

    #[test]
    fn ask_user_question_without_questions_falls_back_to_the_name() {
        // Seen in real transcripts: the input could not be parsed by Claude Code.
        let input = json!({ "__unparsedToolInput": {} });
        assert_eq!(tool_label("AskUserQuestion", Some(&input)), "AskUserQuestion");
    }

    #[test]
    fn tool_label_is_one_line_of_at_most_summary_max_chars() {
        let long = format!("echo start\n{}\necho end", "a ".repeat(200));
        let label = tool_label("Bash", Some(&json!({ "command": long })));
        assert!(label.starts_with("Bash · echo start a a"), "{label}");
        assert!(!label.contains('\n'));
        assert_eq!(label.chars().count(), SUMMARY_MAX);
    }
}
