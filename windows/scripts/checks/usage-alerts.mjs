// core/usage-alerts.ts: 80 % / 100 % crossings, once per window and threshold, again after a rollover.
import assert from "node:assert/strict";
import * as A from "../../src/core/usage-alerts.ts";

const M = 60_000;
const H = 60 * M;
const now = 1_800_000_000_000;
const R1 = now + 3 * H;
const R2 = R1 + 5 * H;

/** Runs the readings through stepAlerts one after the other; returns the crossings of each step. */
function run(steps, memory = new Map()) {
  const out = [];
  let mem = memory;
  for (const [readings, at = now] of steps) {
    const r = A.stepAlerts(mem, readings, at);
    mem = r.memory;
    out.push(r.crossings.map((c) => `${c.key}@${c.threshold}`));
  }
  return { out, memory: mem };
}
const five = (percent, resetsAtMs = R1) => ({ key: "claude:5h", percent, resetsAtMs });

// The first reading of a window is only remembered, even above a threshold (no alert at start).
assert.deepEqual(run([[[five(92)]]]).out, [[]]);
// 50 → 79 → 80 → 85 → 99 → 100 → 100: once at 80, once at 100.
assert.deepEqual(
  run([[[five(50)]], [[five(79)]], [[five(80)]], [[five(85)]], [[five(99)]], [[five(100)]], [[five(100)]]]).out,
  [[], [], ["claude:5h@80"], [], [], ["claude:5h@100"], []],
);
// A jump over both thresholds is one alert, the highest.
assert.deepEqual(run([[[five(40)]], [[five(100)]]]).out, [[], ["claude:5h@100"]]);
// Seen at start at 85: 80 is not told later, 100 is.
assert.deepEqual(run([[[five(85)]], [[five(90)]], [[five(100)]]]).out, [[], [], ["claude:5h@100"]]);

// Rollover: a new reset time starts the window again.
assert.deepEqual(
  run([[[five(50)]], [[five(85)]], [[five(10, R2)]], [[five(81, R2)]]]).out,
  [[], ["claude:5h@80"], [], ["claude:5h@80"]],
);
// A new reset time already above 80: told for the new window.
assert.deepEqual(run([[[five(50)]], [[five(85)]], [[five(88, R2)]]]).out, [[], ["claude:5h@80"], ["claude:5h@80"]]);
// A reset time that moved by less than SAME_WINDOW_MS is the same window (a re-read rounds it).
assert.equal(A.SAME_WINDOW_MS, 15 * M);
assert.deepEqual(run([[[five(50)]], [[five(85)]], [[five(86, R1 + 14 * M)]], [[five(87, R1 - 15 * M)]]]).out, [[], ["claude:5h@80"], [], []]);
// A percent that falls under the told threshold is a new window too (no reset time known).
const week = (percent) => ({ key: "codex:10080", percent });
assert.deepEqual(run([[[week(50)]], [[week(82)]], [[week(83)]], [[week(5)]], [[week(80)]]]).out, [[], ["codex:10080@80"], [], [], ["codex:10080@80"]]);

// A reading whose reset time has passed is old data: nothing told, nothing remembered.
{
  const { out, memory } = run([[[five(50)]], [[five(95)], now + 4 * H]]);
  assert.deepEqual(out, [[], []]);
  assert.equal(memory.get("claude:5h").level, 0);
}
// Windows are told apart by provider and window; a window missing from a step keeps its memory.
assert.deepEqual(
  run([
    [[five(10), week(10)]],
    [[five(81)]],
    [[week(81)]],
    [[five(82), week(82)]],
  ]).out,
  [[], ["claude:5h@80"], ["codex:10080@80"], []],
);
// A bad percent is skipped.
assert.deepEqual(run([[[five(10)]], [[five(Number.NaN)]], [[five(80)]]]).out, [[], [], ["claude:5h@80"]]);
// stepAlerts leaves the memory it is given as it was.
{
  const before = new Map([["claude:5h", { resetsAtMs: R1, level: 0 }]]);
  A.stepAlerts(before, [five(90)], now);
  assert.deepEqual([...before], [["claude:5h", { resetsAtMs: R1, level: 0 }]]);
}

// The readings of a snapshot: Claude's and Codex's windows; OpenRouter has none.
assert.deepEqual(
  A.usageReadings({
    claude: { fetchedAtMs: 1, windows: [{ id: "5h", percent: 12, resetsAtMs: R1 }, { id: "7d:opus", percent: 3 }] },
    codex: { seenAtMs: 1, windows: [{ minutes: 300, percent: 40, resetsAtMs: R2 }] },
    openrouter: { fetchedAtMs: 1, limit: 10, limitRemaining: 0, limitReset: null, usage: 10, usageDaily: 0, usageWeekly: 0, usageMonthly: 0 },
  }),
  [
    { key: "claude:5h", percent: 12, resetsAtMs: R1 },
    { key: "claude:7d:opus", percent: 3, resetsAtMs: undefined },
    { key: "codex:300", percent: 40, resetsAtMs: R2 },
  ],
);
assert.deepEqual(A.usageReadings(undefined), []);
assert.deepEqual(A.usageReadings({ claude: null, codex: null, openrouter: null }), []);
console.log("usage-alerts-check: all passed");
