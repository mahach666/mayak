// core/git-state.ts: the project header's git chip, its tooltip, and which projects are asked again.
import assert from "node:assert/strict";
import * as G from "../../src/core/git-state.ts";
import { format } from "../../src/core/i18n-core.ts";
import { en } from "../../src/core/i18n/en.ts";

const tr = (key, vars) => {
  assert.ok(key in en, `en has no ${key}`);
  return format(en[key], vars);
};
const base = {
  branch: "main", commit: "0123456", upstream: true, ahead: 0, behind: 0,
  changed: 0, untracked: 0, conflicts: 0, insertions: 0, deletions: 0, operation: null,
};
const st = (over) => ({ ...base, ...over });

// Clean, in step: the branch alone, no mark, no arrows.
assert.deepEqual(G.gitChip(base, tr), { ref: "main", mark: "clean", sync: "", op: null });
assert.equal(G.gitTooltip(base, tr), "Branch main\nNo uncommitted changes\nIn step with upstream (as of the last fetch)");

// Dirty, ahead and behind.
const dirty = st({ changed: 3, untracked: 1, insertions: 120, deletions: 34, ahead: 2, behind: 1 });
assert.deepEqual(G.gitChip(dirty, tr), { ref: "main", mark: "dirty", sync: "↑2 ↓1", op: null });
assert.equal(
  G.gitTooltip(dirty, tr),
  "Branch main\nChanged files: 3\nLines: +120 −34\nUntracked files: 1\nCommits to push: 2\nCommits to pull: 1",
);
assert.equal(G.gitChip(st({ ahead: 4 }), tr).sync, "↑4");
assert.equal(G.gitChip(st({ behind: 5 }), tr).sync, "↓5");
// Only untracked files: dirty, no line count.
assert.equal(G.gitChip(st({ untracked: 2 }), tr).mark, "dirty");
assert.ok(!G.gitTooltip(st({ untracked: 2 }), tr).includes("Lines"));

// Conflicts in a merge: red mark and the operation's word.
const merging = st({ conflicts: 2, changed: 1, operation: "merge" });
assert.deepEqual(G.gitChip(merging, tr), { ref: "main", mark: "conflict", sync: "", op: "merge" });
assert.ok(G.gitTooltip(merging, tr).startsWith("Branch main\nmerge in progress\nConflicts: 2\nChanged files: 1"));
// A rebase stopped without a conflict still marks the tree.
assert.equal(G.gitChip(st({ operation: "rebase" }), tr).mark, "dirty");
assert.equal(G.gitChip(st({ operation: "cherryPick" }), tr).op, "cherry-pick");

// Detached HEAD: the short commit, no upstream line.
const detached = st({ branch: null, upstream: false });
assert.equal(G.gitChip(detached, tr).ref, "@0123456");
assert.equal(G.gitTooltip(detached, tr), "Detached HEAD at 0123456\nNo uncommitted changes");

// No upstream, no commits yet.
const fresh = st({ commit: null, upstream: false, untracked: 1 });
assert.equal(G.gitTooltip(fresh, tr), "Branch main\nNo commits yet\nUntracked files: 1\nNo upstream branch");

// Which projects are asked again while the list stays on screen.
const row = (key, kind, since, silent) => ({ key, since, phase: { kind }, ...(silent ? { silent } : {}) });
const before = G.idleMarks([
  { key: "a", sessions: [row("s1", "working", 10), row("s2", "idle", 5)] },
  { key: "b", sessions: [row("s3", "idle", 7)] },
]);
// s1 ends its turn in a; b's row starts a new turn (leaves idle); c appears.
const after = G.idleMarks([
  { key: "a", sessions: [row("s1", "idle", 20), row("s2", "idle", 5)] },
  { key: "b", sessions: [row("s3", "working", 30)] },
  { key: "c", sessions: [row("s4", "working", 1)] },
]);
assert.deepEqual(G.projectsToRefresh(before, after), ["a", "c"]);
// Nothing new: nothing asked.
assert.deepEqual(G.projectsToRefresh(after, after), []);
// A second turn of the same row ends: its since changes, so it is new.
const again = G.idleMarks([{ key: "a", sessions: [row("s1", "idle", 99), row("s2", "idle", 5)] }]);
assert.deepEqual(G.projectsToRefresh(after, again), ["a"]);
// A silent Codex row is no turn.
const silent = G.idleMarks([{ key: "a", sessions: [row("s1", "idle", 20), row("s2", "idle", 5), row("x", "idle", 3, "hooksNotInstalled")] }]);
assert.deepEqual(G.projectsToRefresh(after, silent), []);

console.log("git-state-check: all passed");
