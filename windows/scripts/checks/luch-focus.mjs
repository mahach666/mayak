// luch/focus.ts: Луч's pomodoro looks (work, short break, long break) ease in and out, wrap without a
// jump, breathe slowly in the short break, and hand over to one another without a pop.
import assert from "node:assert/strict";
import * as F from "../../src/luch/focus.ts";

const near = (a, b, e = 1e-9) => Math.abs(a - b) <= e;

// The ramp: FOCUS_RAMP_S (1.5 s) both ways, eased, so a phase never starts or ends with a jolt.
assert.equal(F.FOCUS_RAMP_S, 1.5);
let lv = F.NO_FOCUS, t = 0;
while (lv.work < 1) { lv = F.stepFocus(lv, "focus", 1 / 20); t += 1 / 20; }
assert.ok(Math.abs(t - 1.5) < 1 / 20 + 1e-9, `in over ${t} s`);
assert.deepEqual(lv, { work: 1, rest: 0, ember: 0 });
t = 0;
while (lv.work > 0) { lv = F.stepFocus(lv, null, 1 / 20); t += 1 / 20; }
assert.ok(Math.abs(t - 1.5) < 1 / 20 + 1e-9, `out over ${t} s`);
assert.ok(F.focusAmounts(F.stepFocus(F.NO_FOCUS, "rest", 1 / 20)).rest < 0.01, "the first step in is tiny");
assert.ok(1 - F.focusAmounts(F.stepFocus({ work: 0, rest: 0, ember: 1 }, null, 1 / 20)).ember < 0.01, "the first step out is tiny");
assert.deepEqual(F.stepFocus({ work: 0.5, rest: 0, ember: 0 }, "focus", -1), { work: 0.5, rest: 0, ember: 0 }, "no negative step");

// A hand-over (work → short break): one goes as the other comes, at no step does anything jump.
{
  let l = { work: 1, rest: 0, ember: 0 }, prev = F.focusAmounts(l);
  for (let i = 0; i < 40; i++) {
    l = F.stepFocus(l, "rest", 1 / 20);
    const a = F.focusAmounts(l);
    for (const k of ["work", "rest", "ember"]) assert.ok(Math.abs(a[k] - prev[k]) < 0.06, `smooth ${k} at step ${i}`);
    prev = a;
  }
  assert.deepEqual(l, { work: 0, rest: 1, ember: 0 });
}

// At no look nothing moves.
for (const T of [0, 1.3, 50, 99.9]) {
  assert.deepEqual(F.focusMove(T, F.NO_FOCUS, 0.09, -0.095), { x: 0.09, y: -0.095 });
  assert.deepEqual(F.focusStretch(T, F.NO_FOCUS), { sx: 1, sy: 1 });
}
assert.equal(F.focusActive(F.NO_FOCUS), false);
assert.equal(F.focusActive({ work: 0, rest: 1e-6, ember: 0 }), true);

// The looks: work taller and narrower, the short break lower, the embers a low wide mound; the base stays.
const full = (k) => ({ work: 0, rest: 0, ember: 0, [k]: 1 });
{
  const w = F.focusStretch(0, full("work")), e = F.focusStretch(0, full("ember"));
  assert.ok(w.sy > 1.05 && w.sx < 1, "work: taller, narrower");
  assert.ok(e.sy < 0.9 && e.sx > 1, "embers: low and wide");
  for (let T = 0; T < 10; T += 0.05) assert.ok(F.focusStretch(T, full("rest")).sy < 0.96, "the short break stays lower");
  for (const k of ["work", "rest", "ember"]) assert.deepEqual(F.focusMove(3.3, full(k), 0, -0.44), { x: 0, y: -0.44 }, `${k}: the base stays put`);
  // the face stays inside its panel: the eyes move at most a few hundredths of the box
  for (const k of ["work", "rest", "ember"]) for (let T = 0; T < 10; T += 0.1) {
    const m = F.focusMove(T, full(k), 0.09, -0.095);
    assert.ok(Math.abs(m.x - 0.09) < 0.01 && Math.abs(m.y + 0.095) < 0.05, `${k} eyes at ${T}: ${m.x}, ${m.y}`);
  }
}

// The short break breathes slowly and visibly: 0.22 Hz (about 4.5 s a breath), rising and settling.
assert.ok(F.REST_BREATH_HZ >= 0.2 && F.REST_BREATH_HZ <= 0.25);
{
  let lo = Infinity, hi = -Infinity;
  for (let T = 0; T < 5; T += 0.01) { const sy = F.focusStretch(T, full("rest")).sy; lo = Math.min(lo, sy); hi = Math.max(hi, sy); }
  assert.ok(hi - lo > 0.05, `the breath shows: ${lo}…${hi}`);
  assert.ok(near(F.restBreath(0.3), F.restBreath(0.3 + 1 / F.REST_BREATH_HZ), 1e-9), "one breath per period");
}

// The clock wraps every 100 s without a jump: every component makes whole cycles in it.
for (const hz of [F.REST_BREATH_HZ, F.EMBER_GLOW_HZ, 0.25, 0.3]) assert.ok(near((hz * F.FOCUS_PERIOD_S) % 1, 0, 1e-9) || near((hz * F.FOCUS_PERIOD_S) % 1, 1, 1e-9), `${hz} Hz wraps whole`);
const before = F.FOCUS_PERIOD_S - 1e-6, after = F.focusClock(F.FOCUS_PERIOD_S + 1e-6);
assert.ok(Math.abs(F.restBreath(before) - F.restBreath(after)) < 1e-4);
assert.ok(Math.abs(F.emberGlow(before) - F.emberGlow(after)) < 1e-4);
assert.equal(F.focusClock(F.FOCUS_PERIOD_S + 2.5), 2.5);
for (let T = 0; T < 20; T += 0.07) { const g = F.emberGlow(T); assert.ok(g >= 0 && g <= 1); }
console.log("luch-focus: all passed");
