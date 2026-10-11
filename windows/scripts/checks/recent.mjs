// core/recent.ts: the «Recent» fold of the sessions list (ideas §7) and the cards' «Copy resume».
// Run: node --experimental-strip-types recent.mjs
import assert from "node:assert/strict";
import {
  acceptRecent, closedRecent, copyCommand, findRecent, noteAfterResume, offersResumeCopy, recentDetail, recentEndWord, recentKey, recentLines,
  resumeCommand,
} from "../../src/core/recent.ts";
import { SessionsStore } from "../../src/core/sessions.ts";

const row = (over = {}) => ({
  agent: "claude", id: "3235e946-0d64-4356-98c9-f7b8c8949416", cwd: "C:\\work\\app", project: "app",
  title: null, last: null, end: null, at: 0, ...over,
});

// Lines: the fold line alone while closed; open, "…" while reading, "none" when empty, else one per row.
assert.equal(recentLines(closedRecent()), 1);
assert.equal(recentLines({ ...closedRecent(), open: true }), 2);
assert.equal(recentLines({ ...closedRecent(), open: true, list: [] }), 2);
assert.equal(recentLines({ ...closedRecent(), open: true, list: [row(), row({ id: "b" }), row({ id: "c" })] }), 4);

// The list's height counts them after the projects (or after the empty-state line).
const store = new SessionsStore();
assert.equal(store.rowCount(), 2);
store.recent = { ...closedRecent(), open: true, list: [row(), row({ id: "b" })] };
assert.equal(store.rowCount(), 4);
store.accept({
  rev: 0, now: 0, attention: [], counts: { waiting: 0, error: 0, finished: 0, working: 0 }, hold: false,
  aggregate: "idle", sources: {}, usage: {},
  projects: [{ key: "k", name: "app", root: "C:\\work\\app", sessions: [{ key: "claude:1:2", phase: { kind: "working" }, acked: false }] }],
});
assert.equal(store.rowCount(), 2 + 3);

// Keys: never a session key, and found back in the list.
assert.equal(recentKey(row()), "recent:claude:3235e946-0d64-4356-98c9-f7b8c8949416");
assert.equal(recentKey(row({ agent: "codex", id: "x" })), "recent:codex:x");
for (const prefix of ["claude:", "codex:", "antigravity:", "silent:"]) assert.ok(!recentKey(row()).startsWith(prefix));
const list = [row(), row({ agent: "codex", id: "x" })];
assert.equal(findRecent(list, "recent:codex:x"), list[1]);
assert.equal(findRecent(list, "recent:claude:x"), null);
assert.equal(findRecent(null, "recent:codex:x"), null);

// Text: title, else last phrase, else the folder's name.
assert.equal(recentDetail(row({ title: "T", last: "L" })), "T");
assert.equal(recentDetail(row({ last: "L" })), "L");
assert.equal(recentDetail(row()), "app");

// Only the sessions that stopped short have a phase word.
assert.deepEqual(recentEndWord("error"), { key: "phase.error", tone: "red" });
assert.deepEqual(recentEndWord("unfinished"), { key: "recent.unfinished", tone: "amber" });
assert.deepEqual(recentEndWord("interrupted"), { key: "phase.interrupted", tone: "dim" });
assert.equal(recentEndWord("finished"), null);
assert.equal(recentEndWord(null), null);

// The tooltip shows the command resume.rs runs.
assert.equal(resumeCommand("claude", "abc"), "claude --resume abc");
assert.equal(resumeCommand("codex", "abc"), "codex resume abc");

// After Resume: started folds the island (no note); copied means the CLI was missing; else failed.
assert.equal(noteAfterResume("started"), null);
assert.equal(noteAfterResume("copied"), "noCli");
assert.equal(noteAfterResume("failed"), "failed");
assert.equal(noteAfterResume(null), "failed");

// «Copy resume» on the cards: Claude and Codex sessions whose turn ended on an error or an interrupt.
assert.ok(offersResumeCopy("claude", "error"));
assert.ok(offersResumeCopy("codex", "interrupted"));
assert.ok(!offersResumeCopy("claude", "finished"));
assert.ok(!offersResumeCopy("claude", null));
assert.ok(!offersResumeCopy("antigravity", "error"));

// The scan's answer is taken only by the fold that asked for it: open, and with the same rev.
const asked = { ...closedRecent(), open: true, rev: 3 };
assert.deepEqual(acceptRecent(asked, 3, [row()]), { ...asked, list: [row()], rev: 4 });
assert.deepEqual(acceptRecent(asked, 3, null)?.list, []);
assert.equal(acceptRecent({ ...closedRecent(), rev: 4 }, 3, [row()]), null); // closed meanwhile
assert.equal(acceptRecent({ ...asked, rev: 5 }, 3, [row()]), null); // closed and reopened
assert.equal(acceptRecent({ ...asked, rev: 3, open: false }, 3, [row()]), null);

// The Copy tooltip shows the PowerShell text resume.rs copy_text puts on the clipboard.
assert.equal(
  copyCommand("claude", "3235e946-0d64-4356-98c9-f7b8c8949416", "C:\work\it's [app]"),
  "Set-Location -LiteralPath 'C:\work\it''s [app]'; claude --resume 3235e946-0d64-4356-98c9-f7b8c8949416",
);
assert.equal(copyCommand("codex", "abc", "C:\w"), "Set-Location -LiteralPath 'C:\w'; codex resume abc");
// PowerShell ends a single-quoted string on ’ ‘ ‚ ‛ too: each is doubled, as in resume.rs copy_text.
assert.equal(
  copyCommand("codex", "abc", "C:\\x\u2019; calc; \u2018y\u201A\u201B"),
  "Set-Location -LiteralPath 'C:\\x\u2019\u2019; calc; \u2018\u2018y\u201A\u201A\u201B\u201B'; codex resume abc",
);

console.log("recent-check: all passed");
