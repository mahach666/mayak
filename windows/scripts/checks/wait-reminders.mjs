// island/wait-reminders.ts: which waits are reminded of, when, and how often.
// Run: node --experimental-strip-types wait-reminders.mjs
import assert from "node:assert/strict";
import {
  DEFAULT_WAIT_REMINDER, MAX_WAIT_REMINDERS, WAIT_REMINDER_CHOICES,
  episodeId, loudest, nextReminderAt, reminderMinutes, stepReminders, waitingEpisodes,
} from "../../src/island/wait-reminders.ts";

const MIN = 60_000;
const waitP = { kind: "waiting", wait: "permission", label: "permission" };
const waitI = { kind: "waiting", wait: "input", label: "input" };
const row = (key, phase, since, extra = {}) => ({
  key, agent: "claude", host: "vsCode", title: null, phase, since, activity: null, detail: null,
  outcome: null, attention: null, acked: true, ...extra,
});
const snap = (projects) => ({
  rev: 1, now: 0, attention: [], projects,
  counts: { waiting: 0, error: 0, finished: 0, working: 0 }, hold: false, aggregate: "idle",
  sources: { claude: "ok", codex: "ok", codexHooks: "not_installed" }, usage: { claude: null, codex: null },
});

// ── the setting ───────────────────────────────────────────────────────────────
assert.deepEqual([...WAIT_REMINDER_CHOICES], [0, 3, 10, 30]);
assert.equal(DEFAULT_WAIT_REMINDER, 10);
assert.equal(MAX_WAIT_REMINDERS, 3);
for (const n of [0, 3, 10, 30]) assert.equal(reminderMinutes(n), n);
for (const bad of [undefined, null, 5, -3, "10", 10.5, NaN]) assert.equal(reminderMinutes(bad), 10, String(bad));

// ── which rows are waits ──────────────────────────────────────────────────────
{
  const s = snap([
    { key: "p1", name: "mayak", root: "C:/mayak", sessions: [
      row("a", waitP, 100, { attention: "permission", acked: false }),
      row("b", waitI, 200, { attention: "input", acked: false }),
      row("c", waitI, 300, { attention: null, acked: true }), // marked seen
      row("d", { kind: "idle" }, 400, { attention: "finished", acked: false }),
      row("e", waitP, 500, { attention: "permission", acked: false, silent: "hooksNotReaching" }),
    ] },
    { key: "p2", name: "api", root: "C:/api", sessions: [row("f", waitP, 50, { attention: "permission", acked: false })] },
  ]);
  assert.deepEqual(waitingEpisodes(s), [
    { key: "a", kind: "permission", since: 100, project: "mayak" },
    { key: "b", kind: "input", since: 200, project: "mayak" },
    { key: "f", kind: "permission", since: 50, project: "api" },
  ]);
  assert.deepEqual(waitingEpisodes(null), []);
}

// ── stepping: first sight is silent, then N, 2N, 3N, then nothing ─────────────
const e = { key: "a", kind: "permission", since: 0, project: "mayak" };
const N = 3 * MIN;
{
  let r = stepReminders(new Map(), [e], 10, N);
  assert.deepEqual(r.due, [], "a fresh wait does not ring");
  assert.equal(r.memory.get(episodeId(e)), 0);
  assert.equal(nextReminderAt(r.memory, [e], N), N);

  r = stepReminders(r.memory, [e], N - 1, N);
  assert.deepEqual(r.due, [], "not yet");
  r = stepReminders(r.memory, [e], N, N);
  assert.deepEqual(r.due, [e], "first reminder at N");
  assert.equal(nextReminderAt(r.memory, [e], N), 2 * N);
  r = stepReminders(r.memory, [e], N + 5, N);
  assert.deepEqual(r.due, [], "once per step");
  r = stepReminders(r.memory, [e], 2 * N, N);
  assert.deepEqual(r.due, [e], "second at 2N");
  r = stepReminders(r.memory, [e], 3 * N, N);
  assert.deepEqual(r.due, [e], "third at 3N");
  assert.equal(nextReminderAt(r.memory, [e], N), null, "no timer after the last one");
  r = stepReminders(r.memory, [e], 10 * N, N);
  assert.deepEqual(r.due, [], "never a fourth");
}
{
  // A page loaded on an old wait: what it earned is counted, the next step rings.
  let r = stepReminders(new Map(), [e], 2 * N + 1, N);
  assert.deepEqual(r.due, []);
  assert.equal(nextReminderAt(r.memory, [e], N), 3 * N);
  // A sleep across several steps rings once, not once per step.
  r = stepReminders(new Map([[episodeId(e), 0]]), [e], 5 * N, N);
  assert.deepEqual(r.due, [e]);
  assert.equal(r.memory.get(episodeId(e)), MAX_WAIT_REMINDERS);
}
{
  // A new wait of the same session is a new episode; the answered one is forgotten.
  let r = stepReminders(new Map(), [e], 0, N);
  r = stepReminders(r.memory, [e], N, N);
  const again = { ...e, since: N + 10 };
  r = stepReminders(r.memory, [again], N + 20, N);
  assert.deepEqual(r.due, []);
  assert.deepEqual([...r.memory.keys()], [episodeId(again)]);
  assert.equal(nextReminderAt(r.memory, [again], N), again.since + N);
}
{
  // Off (0) forgets everything and sets no timer; no waits, no timer.
  const r = stepReminders(new Map([[episodeId(e), 1]]), [e], 5 * N, 0);
  assert.deepEqual(r.due, []);
  assert.equal(r.memory.size, 0);
  assert.equal(nextReminderAt(new Map(), [e], 0), null);
  assert.equal(nextReminderAt(new Map(), [], N), null);
}
{
  // The timer goes to the earliest due of all waits.
  const b = { key: "b", kind: "input", since: 60_000, project: "api" };
  const r = stepReminders(new Map(), [e, b], 1000, N);
  assert.equal(nextReminderAt(r.memory, [e, b], N), N);
  const later = stepReminders(r.memory, [e, b], N, N);
  assert.deepEqual(later.due, [e]);
  assert.equal(nextReminderAt(later.memory, [e, b], N), b.since + N);
}

// ── the sound: a permission first, then the oldest wait ───────────────────────
{
  const q1 = { key: "q1", kind: "input", since: 10, project: "x" };
  const q0 = { key: "q0", kind: "input", since: 5, project: "x" };
  const p = { key: "p", kind: "permission", since: 50, project: "x" };
  assert.equal(loudest([]), null);
  assert.equal(loudest([q1, q0]), q0);
  assert.equal(loudest([q0, p, q1]), p);
}

console.log("wait-reminders-check: all passed");
