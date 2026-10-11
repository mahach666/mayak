// core/context-alerts.ts: the 85 % "auto-compaction soon" notice, once per session until its context
// falls below again; the first reading is only remembered; sessions without a window never count.
import assert from "node:assert/strict";
import * as C from "../../src/core/context-alerts.ts";

assert.equal(C.CONTEXT_ALERT_PERCENT, 85);

// Percent: integers, floor, at most 100; none without a window.
assert.equal(C.contextPercent({ used: 870_000, window: 1_000_000 }), 87);
assert.equal(C.contextPercent({ used: 849_999, window: 1_000_000 }), 84);
assert.equal(C.contextPercent({ used: 29, window: 100 }), 29, "0.29 * 100 would be 28.999… in floating point");
assert.equal(C.contextPercent({ used: 1_200_000, window: 1_000_000 }), 100);
assert.equal(C.contextPercent({ used: 612_000 }), null, "Claude without a window: no percent");
assert.equal(C.contextPercent({ used: 5, window: 0 }), null);
assert.equal(C.contextPercent(undefined), null);
assert.equal(C.nearAutoCompact({ used: 850_000, window: 1_000_000 }), true);
assert.equal(C.nearAutoCompact({ used: 849_000, window: 1_000_000 }), false);
assert.equal(C.nearAutoCompact({ used: 990_000 }), false);

/** Runs readings ([key, percent][]) through stepContextAlerts; `present` defaults to the keys read. */
function run(steps, memory = new Map()) {
  const out = [];
  let mem = memory;
  for (const step of steps) {
    const readings = step.readings.map(([key, percent]) => ({ key, percent }));
    const present = new Set(step.present ?? step.readings.map(([key]) => key));
    const r = C.stepContextAlerts(mem, readings, present);
    mem = r.memory;
    out.push(r.crossed);
  }
  return { out, memory: mem };
}
const at = (...readings) => ({ readings });

// First reading only remembered, even above the threshold (no notice at start).
assert.deepEqual(run([at(["a", 92])]).out, [[]]);
// 40 → 84 → 85 → 90 → 97: once, at 85.
assert.deepEqual(run([at(["a", 40]), at(["a", 84]), at(["a", 85]), at(["a", 90]), at(["a", 97])]).out, [[], [], ["a"], [], []]);
// A compaction drops it below: told again on the next crossing.
assert.deepEqual(run([at(["a", 50]), at(["a", 88]), at(["a", 3]), at(["a", 60]), at(["a", 86])]).out, [[], ["a"], [], [], ["a"]]);
// Seen at start above it: told only after it fell below and came back.
assert.deepEqual(run([at(["a", 90]), at(["a", 95]), at(["a", 10]), at(["a", 85])]).out, [[], [], [], ["a"]]);
// Several sessions, each on its own, in snapshot order.
assert.deepEqual(run([at(["a", 10], ["b", 20]), at(["a", 86], ["b", 90])]).out, [[], ["a", "b"]]);
// A present session without a reading keeps its memory; a session gone is forgotten.
assert.deepEqual(
  run([at(["a", 50]), { readings: [], present: ["a"] }, at(["a", 90])]).out,
  [[], [], ["a"]],
);
assert.deepEqual(run([at(["a", 50]), { readings: [], present: [] }, at(["a", 90])]).out, [[], [], []], "back after it was gone: a first reading");
// A non-finite percent changes nothing.
assert.deepEqual(run([at(["a", 50]), at(["a", NaN]), at(["a", 90])]).out, [[], [], ["a"]]);

// Readings from a snapshot: only sessions with a window, silent rows are no sessions.
const session = (key, context, silent) => ({ key, context, silent });
const snap = {
  projects: [
    { key: "p1", sessions: [session("claude:1", { used: 612_000 }), session("codex:1", { used: 230_000, window: 258_400 })] },
    { key: "p2", sessions: [session("silent:x", { used: 10, window: 100 }, "hooksNotReaching"), session("codex:2", undefined)] },
  ],
};
const r = C.contextReadings(snap);
assert.deepEqual(r.readings, [{ key: "codex:1", percent: 89 }]);
assert.deepEqual([...r.keys], ["claude:1", "codex:1", "codex:2"]);
assert.deepEqual(C.contextReadings(null), { readings: [], keys: new Set() });

// The compaction mark's time: "HH:MM".
assert.match(C.clockTime(Date.UTC(2026, 9, 1, 14, 2, 10), "en-GB"), /^\d{2}:\d{2}$/);

console.log("context alerts ok");
