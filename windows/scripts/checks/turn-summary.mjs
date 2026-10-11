// views/turn-summary.ts (ideas §3 «Итог хода»): the row's "+N −M", the card's files line and the
// project-relative file names.
import assert from "node:assert/strict";
import { diffText, fileLabel, rowDiff, turnLine } from "../../src/views/turn-summary.ts";
import { format } from "../../src/core/i18n-core.ts";
import { en } from "../../src/core/i18n/en.ts";
import { ru } from "../../src/core/i18n/ru.ts";

const tr = (dict) => (key, vars) => format(dict[key], vars);
const elapsed = (ms) => `${Math.floor(ms / 60_000)}m`;
const file = (path, added, removed, line = 1) => ({ path, added, removed, line });
const turn = (files, extra = {}) => ({
  files,
  fileCount: files.length,
  added: files.reduce((n, f) => n + f.added, 0),
  removed: files.reduce((n, f) => n + f.removed, 0),
  partial: false,
  ...extra,
});

// the diff uses a real minus sign; a lower bound is marked
assert.equal(diffText(120, 34), "+120 \u221234");
assert.equal(diffText(0, 0, true), "\u2265 +0 \u22120");

// the row: only when files changed
const four = turn([file("C:\\p\\a.rs", 100, 30), file("C:\\p\\b.rs", 20, 4), file("C:\\p\\c.rs", 0, 0), file("C:\\p\\d.md", 0, 0)], { ms: 360_000 });
assert.equal(rowDiff(four), "+120 \u221234");
assert.equal(rowDiff(undefined), null);
assert.equal(rowDiff(turn([], { ms: 5_000 })), null, "a Codex turn with only its length");
assert.equal(rowDiff({ ...four, partial: true }), "\u2265 +120 \u221234");

// the card line, in English and Russian
assert.equal(turnLine(four, tr(en), elapsed), "Files: 4 · +120 \u221234 · 6m");
assert.equal(turnLine(four, tr(ru), elapsed), "Файлов: 4 · +120 \u221234 · 6m");
assert.equal(turnLine({ ...four, ms: undefined }, tr(en), elapsed), "Files: 4 · +120 \u221234", "length unknown");
assert.equal(turnLine(turn([], { ms: 120_000 }), tr(en), elapsed), "Turn 2m");
assert.equal(turnLine(turn([]), tr(en), elapsed), "", "nothing to say");
assert.equal(turnLine(undefined, tr(en), elapsed), "");
// fileCount is the real number even when fewer files were sent
assert.equal(turnLine({ ...turn([file("C:\\p\\a.rs", 1, 0)]), fileCount: 60, added: 70 }, tr(en), elapsed), "Files: 60 · +70 \u22120");

// file names: relative inside the root (case-insensitive, either separator), full path outside
assert.deepEqual(fileLabel("C:\\Users\\dev\\App\\src\\lib.rs", "C:\\Users\\dev\\App"), { name: "lib.rs", relative: "src\\lib.rs" });
assert.deepEqual(fileLabel("c:\\users\\dev\\app\\src\\lib.rs", "C:\\Users\\dev\\App\\"), { name: "lib.rs", relative: "src\\lib.rs" });
assert.deepEqual(fileLabel("C:/Users/dev/App/README.md", "C:/Users/dev/App"), { name: "README.md", relative: "README.md" });
assert.deepEqual(fileLabel("C:\\a.rs", "C:\\"), { name: "a.rs", relative: "a.rs" });
assert.deepEqual(
  fileLabel("C:\\Users\\dev\\AppOther\\x.rs", "C:\\Users\\dev\\App"),
  { name: "x.rs", relative: "C:\\Users\\dev\\AppOther\\x.rs" },
  "a sibling folder sharing the prefix is outside",
);
assert.deepEqual(fileLabel("D:\\notes\\plan.md", "C:\\Users\\dev\\App"), { name: "plan.md", relative: "D:\\notes\\plan.md" });
assert.deepEqual(fileLabel("C:\\Users\\dev\\App", "C:\\Users\\dev\\App"), { name: "App", relative: "C:\\Users\\dev\\App" });
assert.deepEqual(fileLabel("C:\\x.rs", ""), { name: "x.rs", relative: "C:\\x.rs" });

console.log("turn-summary: ok");
