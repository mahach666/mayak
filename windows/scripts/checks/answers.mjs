// island/answers.ts: question parsing, answers building, the Answer button's rule.
// Run: node --experimental-strip-types answers-check.mjs
import assert from "node:assert/strict";
import {
  parseQuestions, buildAnswers, canAnswer, togglePick, answersOnClick, clickCounts, CLICK_GUARD_MS,
  otherText, questionAnswered, OTHER_MAX_CHARS,
} from "../../src/island/answers.ts";

const opt = (label, description) => (description === undefined ? { label } : { label, description });
const q = (question, labels, extra = {}) => ({ question, header: "H", options: labels.map((l) => opt(l, `${l} desc`)), ...extra });

// ── parseQuestions: accepted shapes ──────────────────────────────────────────
const one = parseQuestions({ questions: [q("Which framework?", ["React", "Vue"])] });
assert.deepEqual(one, [{
  question: "Which framework?", header: "H", multiSelect: false,
  options: [{ label: "React", description: "React desc" }, { label: "Vue", description: "Vue desc" }],
}]);
// header and description are optional; multiSelect true is kept.
assert.deepEqual(parseQuestions({ questions: [{ question: "Pick", options: [opt("a"), opt("b")], multiSelect: true }] }), [{
  question: "Pick", header: "", multiSelect: true,
  options: [{ label: "a", description: "" }, { label: "b", description: "" }],
}]);
// 4 questions of 4 options; extra fields are ignored.
const four = parseQuestions({
  questions: [1, 2, 3, 4].map((i) => ({ ...q(`Q${i}?`, ["a", "b", "c", "d"]), extra: 1 })),
  other: true,
});
assert.equal(four.length, 4);
assert.equal(four[3].options.length, 4);
// A truncated text (the relay's "…" cut) is kept as it came: it is the key the relay maps back.
const long = "x".repeat(2000) + "…";
assert.equal(parseQuestions({ questions: [q(long, ["a", "b"])] })[0].question, long);

// ── parseQuestions: rejected shapes ──────────────────────────────────────────
for (const [why, input] of [
  ["not an object", null],
  ["an array", []],
  ["a string", "questions"],
  ["no questions", {}],
  ["questions not an array", { questions: {} }],
  ["0 questions", { questions: [] }],
  ["5 questions", { questions: [1, 2, 3, 4, 5].map((i) => q(`Q${i}`, ["a", "b"])) }],
  ["a question not an object", { questions: ["Q"] }],
  ["no question text", { questions: [{ header: "H", options: [opt("a"), opt("b")] }] }],
  ["empty question text", { questions: [q("  ", ["a", "b"])] }],
  ["question text not a string", { questions: [{ question: 1, options: [opt("a"), opt("b")] }] }],
  ["1 option", { questions: [q("Q", ["a"])] }],
  ["5 options", { questions: [q("Q", ["a", "b", "c", "d", "e"])] }],
  ["options missing", { questions: [{ question: "Q" }] }],
  ["an option not an object", { questions: [{ question: "Q", options: ["a", "b"] }] }],
  ["an empty label", { questions: [q("Q", ["a", ""])] }],
  ["a label not a string", { questions: [{ question: "Q", options: [{ label: 1 }, opt("b")] }] }],
  ["two equal labels", { questions: [q("Q", ["a", "a"])] }],
  ["two equal question texts", { questions: [q("Q", ["a", "b"]), q("Q", ["c", "d"])] }],
  ["multiSelect not a boolean", { questions: [q("Q", ["a", "b"], { multiSelect: "yes" })] }],
  ["header not a string", { questions: [q("Q", ["a", "b"], { header: 3 })] }],
  ["description not a string", { questions: [{ question: "Q", options: [{ label: "a", description: 2 }, opt("b")] }] }],
]) {
  assert.equal(parseQuestions(input), null, `should reject: ${why}`);
}

// ── answersOnClick: one single-select question answers on the click ─────────
assert.equal(answersOnClick(one), true);
assert.equal(answersOnClick(parseQuestions({ questions: [q("Q", ["a", "b"], { multiSelect: true })] })), false);
const two = parseQuestions({ questions: [q("First?", ["a", "b"]), q("Second?", ["c", "d", "e"], { multiSelect: true })] });
assert.equal(answersOnClick(two), false);

// ── togglePick ───────────────────────────────────────────────────────────────
assert.deepEqual(togglePick(one[0], [], 1), [1]);
assert.deepEqual(togglePick(one[0], [1], 0), [0], "single-select: the new pick replaces the old one");
assert.deepEqual(togglePick(one[0], [0], 0), [0], "single-select: clicking the pick again keeps it");
assert.deepEqual(togglePick(two[1], [], 2), [2]);
assert.deepEqual(togglePick(two[1], [2], 0), [0, 2], "multi: kept in option order");
assert.deepEqual(togglePick(two[1], [0, 2], 2), [0], "multi: a second click unpicks");
assert.deepEqual(togglePick(two[1], [0], 7), [0], "an index outside the options changes nothing");

// ── canAnswer ────────────────────────────────────────────────────────────────
assert.equal(canAnswer(one, [[0]]), true);
assert.equal(canAnswer(one, [[]]), false, "nothing picked");
assert.equal(canAnswer(one, []), false, "no picks for the question");
assert.equal(canAnswer(one, [[0], [1]]), false, "picks for a question that is not there");
assert.equal(canAnswer(one, [[0, 1]]), false, "two picks on a single-select");
assert.equal(canAnswer(one, [[2]]), false, "an index outside the options");
assert.equal(canAnswer(one, [[-1]]), false);
assert.equal(canAnswer(one, [[0.5]]), false);
assert.equal(canAnswer(two, [[1], [0, 2]]), true);
assert.equal(canAnswer(two, [[1], []]), false, "every question needs an answer");
assert.equal(canAnswer(two, [[], [0]]), false);
assert.equal(canAnswer(two, [[1], [0, 0]]), false, "the same option twice");
assert.equal(canAnswer(two, [[1], [0, 1, 2]]), true, "multi: every option");
assert.equal(canAnswer([], []), false, "no question");

// ── buildAnswers ─────────────────────────────────────────────────────────────
assert.deepEqual(buildAnswers(one, [[1]]), { "Which framework?": "Vue" }, "single: the label");
const multi = parseQuestions({ questions: [q("Which?", ["a", "b", "c"], { multiSelect: true })] });
assert.deepEqual(buildAnswers(multi, [[2, 0]]), { "Which?": ["a", "c"] }, "multi: labels in option order");
assert.deepEqual(buildAnswers(multi, [[1]]), { "Which?": ["b"] }, "multi with one pick is still an array");
assert.deepEqual(buildAnswers(two, [[0], [1, 2]]), { "First?": "a", "Second?": ["d", "e"] }, "two questions");
// The key is the question text as received, truncated or not.
assert.deepEqual(buildAnswers(parseQuestions({ questions: [q(long, ["a", "b"])] }), [[0]]), { [long]: "a" });
// Picks canAnswer refuses are a programming error, never a half answer.
assert.throws(() => buildAnswers(two, [[0], []]), RangeError);
assert.throws(() => buildAnswers(one, [[0, 1]]), RangeError);

// ── "Other" (spec §6): the typed text answers its question, and overrides the picks ──
assert.equal(OTHER_MAX_CHARS, 4000);
assert.equal(otherText("  Svelte \n"), "Svelte", "trimmed");
assert.equal(otherText(""), null);
assert.equal(otherText("  \n\t "), null, "only spaces is no answer");
assert.equal(otherText(undefined), null);
assert.equal(otherText("x".repeat(4000)), "x".repeat(4000));
assert.equal(otherText("x".repeat(4001)), null, "over 4000 characters");
// Characters are code points, as Rust counts them: 4000 emoji are 8000 UTF-16 units and still fit.
assert.equal(otherText("😀".repeat(4000)), "😀".repeat(4000));
assert.equal(otherText("😀".repeat(4001)), null);
assert.equal(otherText("  " + "x".repeat(4000) + "  "), "x".repeat(4000), "the limit is on the trimmed text");
assert.equal(questionAnswered(one[0], [], "Svelte"), true, "an Other text with no pick");
assert.equal(questionAnswered(one[0], undefined, "Svelte"), true);
assert.equal(questionAnswered(one[0], [], " "), false);
assert.equal(questionAnswered(one[0], [0], ""), true, "a pick and no text");
assert.equal(questionAnswered(one[0], [0, 1], "Svelte"), true, "the text overrides even bad picks");
assert.equal(canAnswer(one, [[]], ["Svelte"]), true);
assert.equal(canAnswer(one, [[]], [""]), false);
assert.equal(canAnswer(two, [[], []], ["a", ""]), false, "every question needs an answer");
assert.equal(canAnswer(two, [[], [1]], ["mine", ""]), true);
assert.equal(canAnswer(two, [[0], []], ["", "my tests"]), true, "Other on a multi-select");
assert.equal(canAnswer(two, [[0]], ["", "x"]), false, "picks for every question still required");
assert.deepEqual(buildAnswers(one, [[]], ["  Svelte "]), { "Which framework?": { other: "Svelte" } });
assert.deepEqual(buildAnswers(one, [[1]], ["Svelte"]), { "Which framework?": { other: "Svelte" } }, "the text overrides the pick");
assert.deepEqual(buildAnswers(one, [[1]], ["  "]), { "Which framework?": "Vue" }, "blank text: the pick");
assert.deepEqual(
  buildAnswers(two, [[0], [0, 2]], ["", "Smoke only"]),
  { "First?": "a", "Second?": { other: "Smoke only" } },
  "Other on a multi-select is one text, never a list",
);
assert.deepEqual(buildAnswers(two, [[1], [2]], []), { "First?": "b", "Second?": ["e"] }, "no others given: as before");
assert.throws(() => buildAnswers(two, [[], [0]], ["", ""]), RangeError);
assert.throws(() => buildAnswers(one, [[]], ["x".repeat(4001)]), RangeError, "a text too long is no answer");

// ── clickCounts: a click is bound to the card its button was built for ──────
assert.equal(CLICK_GUARD_MS, 400);
const bound = { requestId: "r1", builtAt: 1000 };
// Past the guard, on the same card: counts.
assert.equal(clickCounts(bound, "r1", 1400), true);
assert.equal(clickCounts(bound, "r1", 60_000), true);
// Within the first 400 ms (the second click of a double click): ignored.
assert.equal(clickCounts(bound, "r1", 1000), false);
assert.equal(clickCounts(bound, "r1", 1399.9), false);
// Another card is pending now, or none at all: ignored.
assert.equal(clickCounts(bound, "r2", 5000), false);
assert.equal(clickCounts(bound, null, 5000), false);
assert.equal(clickCounts(bound, undefined, 5000), false);
// A clock that went back (should not happen with performance.now) never lets a click through early.
assert.equal(clickCounts(bound, "r1", 900), false);

// ── layout.ts: the 560 px panel is for the question card only (spec §6) ──────
const L = await import(new URL("../../src/core/layout.ts", import.meta.url).href);
assert.equal(L.PANEL_W, 720);
assert.equal(L.PANEL_H, 560);
assert.equal(L.CARD_CHROME_H, 52);
assert.equal(L.questionHeight(0), 170, "not measured yet: the layout height");
assert.equal(L.questionHeight(100), 170, "a short card keeps the layout height");
assert.equal(L.questionHeight(300), 352, "content + header and insets");
assert.equal(L.questionHeight(300.2), 353, "a fraction of a pixel is never cut off");
assert.equal(L.questionHeight(508), 560);
assert.equal(L.questionHeight(900), 560, "never past the panel");
assert.equal(L.questionHeight(-5), 170);
assert.deepEqual(L.islandSize("expanded", "question", 0, 0, false, 300), { w: 640, h: 352 });
// Every other view: the height it had before the panel grew (none above 320).
const before = { overview: 160, empty: 160, approval: 160, error: 160, finished: 160, confused: 160, upload: 176,
  uploading: 176, choose: 176, mail: 240, prompt: 240, searching: 160, result: 160, note: 160, settings: 160,
  greeting: 150, sessions: 72, waiting: 160, session: 160, "focus-phase": 160, stats: 260, focus: 260, apps: 200 };
for (const [view, h] of Object.entries(before)) {
  assert.equal(L.islandSize("expanded", view, 0, 0, false, 900).h, h, view);
}
assert.equal(Object.keys(before).length + 1, Object.keys(L.VIEW_LAYOUTS).length, "every view is listed");
assert.equal(L.islandSize("expanded", "prompt", 99).h, 300, "the chat still stops at 300");
assert.equal(L.islandSize("expanded", "sessions", 0, 99).h, 300, "the session list still stops at 300");
assert.equal(L.islandSize("expanded", "overview", 0, 0, true).h, 212, "home with the player row");
assert.deepEqual(L.islandSize("compact", "question"), { w: 288, h: 32 });
assert.deepEqual(L.islandSize("hidden", "question"), { w: 184, h: 0 });

// ── fsm.ts heldOpen: an answer being typed holds the open island (spec §6) ────
globalThis.window ??= globalThis;
const { IslandStateMachine } = await import(new URL("../../src/island/fsm.ts", import.meta.url).href);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
{
  const fsm = new IslandStateMachine();
  const seen = [];
  fsm.onTransition = (from, to) => seen.push(`${from}>${to}`);
  fsm.homeToPetitDelay = 0.05;
  fsm.forceHome();
  fsm.setHeldOpen(true);
  fsm.mouseLeft();
  await sleep(120);
  assert.equal(fsm.state, "home", "held: leaving the island schedules no fold");
  // A fold already scheduled is cancelled by the hold.
  fsm.setHeldOpen(false);
  fsm.mouseLeft();
  fsm.setHeldOpen(true);
  await sleep(120);
  assert.equal(fsm.state, "home", "a hold cancels the fold that was scheduled");
  // Let go: the next leave (or armAutoClose) folds again after the delay.
  fsm.setHeldOpen(false);
  fsm.mouseLeft();
  await sleep(120);
  assert.equal(fsm.state, "petit", "let go: auto-close folds again");
  // The explicit ways still end it while held.
  fsm.forceHome();
  fsm.setHeldOpen(true);
  fsm.forceCollapse();
  assert.equal(fsm.state, "petit", "the collapse button still folds a held island");
  fsm.forceHome();
  fsm.forcePetit();
  assert.equal(fsm.state, "petit", "an explicit close still folds a held island");
  fsm.forceHome();
  fsm.forceHidden();
  assert.equal(fsm.state, "hidden", "pause and the hotkey still hide a held island");
  fsm.cancelTimers();
}

console.log("answers-check: all passed");
