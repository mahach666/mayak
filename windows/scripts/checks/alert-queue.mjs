// island/alert-queue.ts: when a session card is stale, and which alert comes next (spec §2.6).
// Run: node --experimental-strip-types alert-queue-check.mjs
import assert from "node:assert/strict";
import {
  episodeStart, staleCard, nextAlert, ATTENTION_RANK,
} from "../../src/island/alert-queue.ts";

const working = { kind: "working" };
const idle = { kind: "idle" };
const waitP = { kind: "waiting", wait: "permission", label: "permission" };
const waitI = { kind: "waiting", wait: "input", label: "input" };
const unknown = { kind: "unknown", label: "x" };
const fin = (at) => ({ kind: "finished", summary: "done", at });
const err = (at) => ({ kind: "error", summary: "boom", at });
const intr = (at) => ({ kind: "interrupted", summary: "", at });

const row = (key, phase, since, extra = {}) => ({
  key, agent: "claude", host: "vsCode", title: null, phase, since, activity: null, detail: null,
  outcome: null, attention: null, acked: true, ...extra,
});
const snap = (rows, attention = rows.filter((r) => r.attention).map((r) => r.key)) => ({
  rev: 1, now: 0, attention,
  projects: [{ key: "p", name: "p", root: "C:/p", sessions: rows }],
  counts: { waiting: 0, error: 0, finished: 0, working: 0 }, hold: false, aggregate: "idle",
  sources: { claude: "ok", codex: "ok", codexHooks: "not_installed" }, usage: { claude: null, codex: null },
});

// ── ranks: the same scale as Rust's attention_rank / Alert.rank ──────────────
assert.deepEqual(ATTENTION_RANK, { permission: 0, input: 1, error: 2, finished: 3 });

// ── episodeStart: Waiting → since; Idle + finished/error → outcome.at; else null ──
assert.equal(episodeStart(row("a", waitP, 100)), 100);
assert.equal(episodeStart(row("a", idle, 100, { outcome: fin(150) })), 150);
assert.equal(episodeStart(row("a", idle, 100, { outcome: err(160) })), 160);
assert.equal(episodeStart(row("a", idle, 100, { outcome: intr(170) })), null);
assert.equal(episodeStart(row("a", idle, 100)), null);
assert.equal(episodeStart(row("a", working, 100, { outcome: fin(50) })), null);
assert.equal(episodeStart(row("a", unknown, 100)), null);

// ── staleCard: waiting card ─────────────────────────────────────────────────
const wcard = { key: "a", kind: "waiting", since: 100 };
// Same wait, acked or not: not stale (the ack of a clicked row changes nothing).
assert.equal(staleCard(wcard, snap([row("a", waitP, 100)])), false);
assert.equal(staleCard(wcard, snap([row("a", waitP, 100, { attention: "permission", acked: false })])), false);
// Answered in the agent's window: working again, or the turn ended.
assert.equal(staleCard(wcard, snap([row("a", working, 120)])), true);
assert.equal(staleCard(wcard, snap([row("a", idle, 130, { outcome: fin(140) })])), true);
assert.equal(staleCard(wcard, snap([row("a", idle, 130, { outcome: intr(140) })])), true);
// A newer wait (the first one was answered there, another came): stale.
assert.equal(staleCard(wcard, snap([row("a", waitI, 200)])), true);
// Unknown phase or a session the snapshot no longer has: not decided, the card stays.
assert.equal(staleCard(wcard, snap([row("a", unknown, 300)])), false);
assert.equal(staleCard(wcard, snap([row("b", working, 300)])), false);

// ── staleCard: finished / error cards ───────────────────────────────────────
for (const kind of ["finished", "error"]) {
  const card = { key: "a", kind, since: 150 };
  const outcome = kind === "finished" ? fin(150) : err(150);
  assert.equal(staleCard(card, snap([row("a", idle, 100, { outcome })])), false, kind);
  assert.equal(staleCard(card, snap([row("a", idle, 100, { outcome, acked: true })])), false, kind);
  // A new turn after a user message.
  assert.equal(staleCard(card, snap([row("a", working, 200)])), true, kind);
  assert.equal(staleCard(card, snap([row("a", waitP, 200)])), true, kind);
  // A newer turn that already ended (busy → idle between two reads).
  assert.equal(staleCard(card, snap([row("a", idle, 200, { outcome: fin(210) })])), true, kind);
  assert.equal(staleCard(card, snap([row("a", idle, 200, { outcome: intr(210) })])), true, kind);
  // Idle with the outcome still being evaluated, unknown phase, gone: not decided.
  assert.equal(staleCard(card, snap([row("a", idle, 200)])), false, kind);
  assert.equal(staleCard(card, snap([row("a", unknown, 200)])), false, kind);
  assert.equal(staleCard(card, snap([])), false, kind);
}

// ── Permission and question cards are never stale ───────────────────────────
// CardEpisode has no "answer" kind any more: island.ts cardEpisode returns null on the approval
// and question views, so no snapshot moves them on (spec §2.6). They end only by a click,
// "In terminal", the island leaving them, pause, a language reload or the 590 s limit.

// ── nextAlert ────────────────────────────────────────────────────────────────
const all = () => true;
const none = { key: "", since: 0 };
assert.equal(nextAlert(snap([]), none, all), null);
assert.equal(nextAlert(snap([row("a", working, 1), row("b", idle, 1)]), none, all), null);

const pa = row("pa", waitP, 50, { attention: "permission", acked: false });
const ib = row("ib", waitI, 40, { attention: "input", acked: false });
const ec = row("ec", idle, 30, { outcome: err(35), attention: "error", acked: false });
const fd = row("fd", idle, 20, { outcome: fin(25), attention: "finished", acked: false });
const ackedFin = row("af", idle, 20, { outcome: fin(26) });
// Most important first by rank, whatever the order of the attention list.
assert.deepEqual(nextAlert(snap([fd, ec, ib, pa, ackedFin], ["fd", "ec", "ib", "pa"]), none, all), { key: "pa", kind: "permission", since: 50 });
assert.deepEqual(nextAlert(snap([fd, ec, ib]), none, all), { key: "ib", kind: "input", since: 40 });
assert.deepEqual(nextAlert(snap([fd, ec]), none, all), { key: "ec", kind: "error", since: 35 });
assert.deepEqual(nextAlert(snap([fd, ackedFin]), none, all), { key: "fd", kind: "finished", since: 25 });
// Acknowledged rows are not alerts.
assert.equal(nextAlert(snap([ackedFin]), none, all), null);
// Equal ranks keep Rust's attention order (cmp_sessions tie-breaks).
const pa2 = row("pa2", waitP, 10, { attention: "permission", acked: false });
assert.equal(nextAlert(snap([pa, pa2], ["pa2", "pa"]), none, all).key, "pa2");
assert.equal(nextAlert(snap([pa, pa2], ["pa", "pa2"]), none, all).key, "pa");
// A key in the attention list that is missing from the projects, or with no attention: skipped.
assert.deepEqual(nextAlert(snap([fd], ["ghost", "fd"]), none, all), { key: "fd", kind: "finished", since: 25 });
assert.equal(nextAlert(snap([row("x", working, 1)], ["x"]), none, all), null);
// Silent rows never count.
assert.equal(nextAlert(snap([row("s", idle, 1, { attention: "finished", acked: false, outcome: fin(2), silent: "hooksNotReaching" })]), none, all), null);
// The card's own episode is skipped; a newer episode of the same session is not.
assert.deepEqual(nextAlert(snap([pa, fd]), { key: "pa", since: 50 }, all), { key: "fd", kind: "finished", since: 25 });
assert.deepEqual(nextAlert(snap([pa, fd]), { key: "pa", since: 49 }, all), { key: "pa", kind: "permission", since: 50 });
// Kinds the island may not open for are skipped.
assert.deepEqual(nextAlert(snap([pa, ib, ec, fd]), none, (k) => k === "finished" || k === "error"), { key: "ec", kind: "error", since: 35 });
assert.equal(nextAlert(snap([pa, ib, ec, fd]), none, () => false), null);

console.log("alert-queue-check: all assertions passed");
