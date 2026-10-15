// core/ao.ts: Agent Orchestrator in the island: reading what Rust sends, the words and tones of AO's
// status codes, which sessions the fold lists, and which session a project of the sessions list belongs to.
// The payload is the file src-tauri/src/ao/model.rs tests its own output against (real AO output, names
// made anonymous), so Rust and this side read the same thing.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  AO_FOLD_KEY, AO_ID, AO_MAX_ROWS, activityWord, aoCounts, aoFor, aoForProject, aoLines, aoRefOf, aoRows, aoSessions,
  byRef, ciColor, ciWord, isInside, needsYou, orderSessions, readAo, roleWord, statusTone, statusWord, tagFor, validAoId,
} from "../../src/core/ao.ts";
import { rowOpens } from "../../src/core/row-open.ts";
import { tileAlert } from "../../src/views/home-model.ts";
import { en } from "../../src/core/i18n/en.ts";

const payload = JSON.parse(readFileSync(new URL("../../src-tauri/src/ao/fixtures/payload.json", import.meta.url), "utf8"));
const real = readAo(payload);

// ── What Rust sends ──
assert.ok(real, "the real payload is read");
assert.equal(real.status, "ok");
assert.equal(real.cli, true);
assert.deepEqual(real.sessions.map((s) => s.id), ["mayak-1", "mayak-2", "mayak-3", "mayak-4", "mayak-5", "mayak-6", "orbit-1", "orbit-2"]);
const [orchestrator, merged] = real.sessions;
assert.deepEqual(orchestrator, {
  id: "mayak-1", project: "mayak", role: "orchestrator", name: null, status: "idle", activity: "idle", harness: "claude-code",
  prs: 0, prsOpen: 0, ci: null, workspace: "c:\\users\\dev\\.ao\\data\\worktrees\\mayak\\orchestrator\\mayak-orchestrator",
});
assert.equal(merged.prs, 2, "two merged pull requests count");
assert.equal(merged.prsOpen, 0);
assert.equal(real.sessions[2].workspace, null, "a session whose folder is unknown has none");

// Anything that is no payload is nothing, never a half-read one.
for (const bad of [null, undefined, "ok", 7, [], {}, { status: "fine" }, { status: 3 }]) assert.equal(readAo(bad), null, JSON.stringify(bad));
for (const status of ["notInstalled", "notRunning", "unreachable", "unsupported"]) {
  const got = readAo({ status, cli: false, sessions: [] });
  assert.deepEqual([got.status, got.cli, got.sessions.length], [status, false, 0]);
}
assert.deepEqual(readAo({ status: "ok" }).sessions, [], "no list is no sessions");
const messy = readAo({
  status: "ok",
  sessions: [
    null, 5, "x", {}, { id: "a" }, { id: "b", role: "worker" }, { id: "", role: "worker", status: "idle" },
    { id: "c", role: "worker", status: "idle", prs: -3, prsOpen: "2", name: 4, ci: "" },
  ],
});
assert.deepEqual(messy.sessions, [{
  id: "c", role: "worker", status: "idle", project: null, name: null, activity: null, harness: null, prs: 0, prsOpen: 0, ci: null, workspace: null,
}]);

// ── Words, tones, colours ──
// Every status code AO's contract lists (ControllersSessionView.status) has a word, and every word exists.
const CODES = [
  "working", "pr_open", "draft", "ci_failed", "review_pending", "changes_requested", "commented", "approved",
  "mergeable", "merged", "needs_input", "exited", "idle", "terminated", "no_signal",
];
for (const code of CODES) {
  const key = statusWord(code);
  assert.ok(key && key in en, `no word for ${code}`);
}
assert.equal(new Set(CODES.map(statusWord)).size, CODES.length, "every code has its own word");
assert.equal(statusWord("brand_new_status"), null, "a code AO adds later shows as itself");
assert.equal(roleWord("worker"), "ao.role.worker");
assert.equal(roleWord("orchestrator"), "ao.role.orchestrator");
assert.equal(roleWord("reviewer"), null);
assert.equal(activityWord("active"), "ao.act.active");
assert.equal(activityWord("idle"), "ao.act.idle");
assert.equal(activityWord("sleeping"), null);
for (const ci of ["failing", "pending", "passing", "unknown"]) assert.ok(ciWord(ci) in en, ci);
assert.equal(ciWord("exploding"), "ao.ci.unknown");
for (const key of ["ao.fold", "ao.more", "ao.prs", "ao.tag", "ao.tagOrchestrator", "ao.tip", "int.aoKind", "int.aoSummary", "int.aoNone",
  "int.aoLooking", "int.aoNotInstalled", "int.aoNotRunning", "int.aoUnreachable", "int.aoUnsupported",
  "set.aoStatus", "set.aoRunning", "set.aoNotRunning", "set.aoNotInstalled", "set.aoUnsupported", "set.aoCli", "set.aoHint"]) {
  assert.ok(key in en, key);
}
assert.deepEqual(
  ["ci_failed", "needs_input", "changes_requested", "working", "merged", "mergeable", "approved", "idle", "pr_open", "x"].map(statusTone),
  ["red", "amber", "amber", "blue", "green", "green", "green", "dim", "dim", "dim"],
);
assert.deepEqual(["failing", "pending", "passing", "unknown", null].map(ciColor), ["#F4505E", "#F5A524", "#22C55E", "#6B7079", "#6B7079"]);

// ── Counts and order ──
assert.deepEqual(aoCounts(real.sessions), { attention: 0, working: 3, prsOpen: 0, ciFailing: 0 });
const s = (id, status, extra = {}) => ({
  id, status, role: "worker", project: "p", name: null, activity: null, harness: null, prs: 0, prsOpen: 0, ci: null, workspace: null, ...extra,
});
const mixed = [s("a", "idle"), s("b", "working"), s("c", "needs_input"), s("d", "merged"), s("e", "ci_failed", { prsOpen: 1, ci: "failing" }), s("f", "working"), s("g", "changes_requested")];
assert.deepEqual(aoCounts(mixed), { attention: 3, working: 2, prsOpen: 1, ciFailing: 1 });
assert.deepEqual(mixed.filter(needsYou).map((x) => x.id), ["c", "e", "g"]);
assert.deepEqual(orderSessions(mixed).map((x) => x.id), ["c", "e", "g", "b", "f", "a", "d"], "those that ask for someone, those working, the rest; AO's order inside");
assert.deepEqual(mixed.map((x) => x.id), ["a", "b", "c", "d", "e", "f", "g"], "the input is not reordered");

// ── The fold ──
const many = Array.from({ length: AO_MAX_ROWS + 5 }, (_, i) => s(`s-${i}`, "idle"));
const shown = aoRows(many);
assert.equal(shown.rows.length, AO_MAX_ROWS);
assert.equal(shown.more, 5);
assert.deepEqual(aoRows([]), { rows: [], more: 0 });
assert.equal(aoLines(0, true), 0, "no sessions, no fold");
assert.equal(aoLines(0, false), 0);
assert.equal(aoLines(8, false), 1, "closed: its line");
assert.equal(aoLines(8, true), 1 + 8, "open: its line and a row each");
assert.equal(aoLines(AO_MAX_ROWS, true), 1 + AO_MAX_ROWS);
assert.equal(aoLines(AO_MAX_ROWS + 5, true), 1 + AO_MAX_ROWS + 1, "past the cap: one «+N more» line");
assert.equal(rowOpens(AO_FOLD_KEY), false, "the fold opens and closes, it opens no session");
assert.equal(rowOpens("claude:1"), true);

// ── What State hands out ──
const info = { data: payload };
const on = [AO_ID, "integration_github"];
const got = aoSessions(info, on);
assert.equal(got.list.length, 8);
assert.equal(aoSessions(info, on).rev, got.rev, "the same read is not read again");
assert.equal(aoSessions(info, on).list, got.list);
const off = aoSessions(info, ["integration_github"]);
assert.deepEqual(off.list, [], "a switched off pill shows nothing of AO, whatever was read before");
assert.notEqual(off.rev, got.rev);
for (const status of ["notInstalled", "notRunning", "unreachable", "unsupported"]) {
  assert.deepEqual(aoSessions({ data: { status, cli: true, sessions: [] } }, on).list, [], status);
}
assert.deepEqual(aoSessions(undefined, on).list, []);
assert.deepEqual(aoSessions({ data: { status: "ok", sessions: "no" } }, on).list, []);
assert.deepEqual(aoSessions({ data: 5 }, on).list, []);

// ── Which session a project belongs to ──
const W = "c:\\users\\dev\\.ao\\data\\worktrees\\mayak\\mayak-6";
assert.equal(isInside(W, W), true, "the folder itself");
assert.equal(isInside(`${W}\\windows`, W), true, "a folder inside it");
assert.equal(isInside("/home/dev/.ao/data/worktrees/mayak/mayak-6/src", "/home/dev/.ao/data/worktrees/mayak/mayak-6"), true);
assert.equal(isInside(W.replace("mayak-6", "mayak-60"), W), false, "a sibling with the same beginning is not inside");
assert.equal(isInside("c:\\users\\dev\\.ao\\data\\worktrees\\mayak", W), false, "a folder above it is not");
assert.equal(isInside("", ""), false);
assert.equal(isInside(W, ""), false);
assert.equal(isInside("", W), false);

assert.equal(tagFor(real.sessions, W)?.id, "mayak-6");
assert.equal(tagFor(real.sessions, `${W}\\hook`)?.id, "mayak-6");
assert.equal(tagFor(real.sessions, "c:\\users\\dev\\.ao\\data\\worktrees\\mayak\\orchestrator\\mayak-orchestrator")?.id, "mayak-1");
assert.equal(tagFor(real.sessions, "c:\\users\\dev\\.ao\\data\\worktrees\\orbit\\orchestrator\\orbit-orchestrator")?.id, "orbit-1");
assert.equal(tagFor(real.sessions, "c:\\users\\dev\\source\\repos\\mayak"), null, "the user's own checkout is no AO session");
assert.equal(tagFor(real.sessions, ""), null);
assert.equal(tagFor([], W), null);
assert.equal(tagFor(real.sessions, "c:\\users\\dev\\.ao\\data\\worktrees\\mayak\\mayak-3"), null, "mayak-3's folder is unknown to the payload");
const nested = [s("outer", "idle", { workspace: "c:\\a" }), s("inner", "idle", { workspace: "c:\\a\\b" })];
assert.equal(tagFor(nested, "c:\\a\\b\\c")?.id, "inner", "the deepest folder wins");
assert.equal(tagFor(nested.reverse(), "c:\\a\\b\\c")?.id, "inner", "whatever the order");

// ── The home tile ──
assert.equal(tileAlert(AO_ID, { state: "idle" }, payload), false);
assert.equal(tileAlert(AO_ID, { state: "idle" }, { ...payload, sessions: [s("x", "ci_failed", { prsOpen: 1, ci: "failing" })] }), true, "a failing CI tints the tile");
assert.equal(tileAlert(AO_ID, { state: "idle" }, undefined), false);
assert.equal(tileAlert(AO_ID, { state: "idle" }, { status: "unreachable" }), false);

// ── …or by the ids an agent's own environment named (the relay's AO_CONTEXT) ──
// The same rule as Rust's valid_id (src-tauri/src/ao/model.rs).
for (const ok of ["mayak-6", "a", "x_1.2", "ABC-9", "x".repeat(64)]) assert.equal(validAoId(ok), true, ok);
for (const bad of ["", ".", "..", "a/b", "a\\b", "a b", "a?b", "é", "<script>", "x".repeat(65)]) assert.equal(validAoId(bad), false, bad);
assert.deepEqual(aoRefOf("mayak-6", "mayak"), { session: "mayak-6", project: "mayak" });
assert.deepEqual(aoRefOf(" mayak-6 ", "bad id"), { session: "mayak-6", project: null }, "a bad project id is dropped, the session kept");
for (const [session, project] of [[undefined, "mayak"], [null, null], [42, "mayak"], ["../x", "mayak"], ["", ""]]) {
  assert.equal(aoRefOf(session, project), null, String(session));
}

const me = real.sessions.find((x) => x.id === "mayak-6");
assert.equal(byRef(real.sessions, { session: "mayak-6", project: "mayak" }), me);
assert.equal(byRef(real.sessions, { session: "mayak-6", project: null }), me, "no project said: the id is enough");
assert.equal(byRef(real.sessions, { session: "mayak-6", project: "orbit" }), null, "another project's session of that id is not this one");
assert.equal(byRef(real.sessions, { session: "mayak-99", project: "mayak" }), null, "a session AO does not list shows nothing");
assert.equal(byRef(real.sessions, null), null);
assert.equal(byRef([], { session: "mayak-6", project: null }), null, "AO off or stopped: nothing");

// The folder comes first; the ids only fill in for a project outside every AO folder.
const outside = "c:\\users\\dev\\source\\repos\\mayak";
assert.equal(aoFor(real.sessions, W, { session: "orbit-2", project: "orbit" })?.id, "mayak-6", "the folder wins over the ids");
assert.equal(aoFor(real.sessions, outside, { session: "orbit-2", project: "orbit" })?.id, "orbit-2");
assert.equal(aoFor(real.sessions, outside, null), null);
assert.equal(aoFor(real.sessions, null, { session: "mayak-1", project: "mayak" })?.id, "mayak-1", "a card without a project key uses the ids");
assert.equal(aoFor(real.sessions, null, null), null);

assert.equal(aoForProject(real.sessions, { key: W, sessions: [] })?.id, "mayak-6");
assert.equal(aoForProject(real.sessions, { key: outside, sessions: [{}, { ao: { session: "nope", project: null } }, { ao: { session: "orbit-2", project: "orbit" } }] })?.id, "orbit-2",
  "the first row whose ids AO lists");
assert.equal(aoForProject(real.sessions, { key: outside, sessions: [{}, { ao: null }] }), null);
assert.equal(aoForProject([], { key: W, sessions: [{ ao: { session: "mayak-6", project: null } }] }), null, "nothing without AO's list");

console.log("ao-check: all passed");
