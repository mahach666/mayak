// The question card an MCP agent's `ask_user` puts on the island (src-tauri mcp_server/backend.rs
// `card_payload`) is read by the island as it reads a Claude Code AskUserQuestion. Rust builds this
// very file in its own test (`the_card_is_what_the_islands_page_takes_for_a_question`); here the
// page's side takes it: the router's `isCard` filter (island/hooks.ts) and `parseQuestions`.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseQuestions, answersOnClick, buildAnswers } from "../../src/island/answers.ts";

const card = JSON.parse(readFileSync(new URL("./fixtures/mcp-ask-card.json", import.meta.url), "utf8"));

// island/hooks.ts isCard(): a question card is `kind: "question"`, agent "claude", PreToolUse.
assert.equal(card.kind, "question");
assert.equal(card.agent, "claude");
assert.equal(card.hook_event_name, "PreToolUse");
assert.equal(typeof card.request_id, "string");
assert.notEqual(card.request_id, "");
// The card shows `project_name` as its name line; the question is not a permission to keep a rule for.
assert.equal(typeof card.project_name, "string");
assert.equal(card.canAlways, false);

const questions = parseQuestions(card.tool_input);
assert.notEqual(questions, null, "the page can show this question");
assert.equal(questions.length, 1);
assert.deepEqual(questions[0], {
  question: "Deploy now?",
  header: "Deploy",
  options: [{ label: "Yes", description: "" }, { label: "No", description: "Wait for CI" }],
  multiSelect: false,
});
// One single-select question: a click on an option answers it, keyed by the question's own text,
// which is what the MCP tool looks the answer up by.
assert.equal(answersOnClick(questions), true);
assert.deepEqual(buildAnswers(questions, [[1]], [""]), { "Deploy now?": "No" });
assert.deepEqual(buildAnswers(questions, [[]], ["typed words"]), { "Deploy now?": { other: "typed words" } });

console.log("mcp-card-check: all passed");
