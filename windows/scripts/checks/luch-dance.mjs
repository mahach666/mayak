// luch/dance.ts: Луч's dance to music eases in and out, wraps without a jump, and is not a metronome.
import assert from "node:assert/strict";
import * as D from "../../src/luch/dance.ts";

// The amount ramps over DANCE_RAMP_S (0.6 s) both ways, eased: no pop when music starts or stops.
assert.equal(D.DANCE_RAMP_S, 0.6);
let level = 0, t = 0;
while (level < 1) { level = D.stepDance(level, true, 1 / 30); t += 1 / 30; }
assert.ok(Math.abs(t - 0.6) < 1 / 30 + 1e-9, `in over ${t} s`);
t = 0;
while (level > 0) { level = D.stepDance(level, false, 1 / 30); t += 1 / 30; }
assert.ok(Math.abs(t - 0.6) < 1 / 30 + 1e-9, `out over ${t} s`);
assert.equal(D.danceAmount(0), 0);
assert.equal(D.danceAmount(1), 1);
// the eased amount barely moves at the ends: a 1/30 s step in is a tiny fraction of the full dance
assert.ok(D.danceAmount(D.stepDance(0, true, 1 / 30)) < 0.01);
assert.ok(1 - D.danceAmount(D.stepDance(1, false, 1 / 30)) < 0.01);
assert.equal(D.stepDance(0.5, true, -1), 0.5, "no negative step");

// At amount 0 nothing moves.
for (const T of [0, 0.2, 0.7, 50]) {
  assert.deepEqual(D.danceMove(T, 0, 0.09, -0.095), { x: 0.09, y: -0.095 });
  assert.equal(D.dancePulse(T, 0), 1);
}

// The clock wraps every 100 s without a jump: every component makes whole cycles in it.
const before = D.DANCE_PERIOD_S - 1e-6;
for (const hd of [0.3, 1]) assert.ok(Math.abs(D.danceSway(before, hd, 1) - D.danceSway(D.danceClock(D.DANCE_PERIOD_S + 1e-6), hd, 1)) < 1e-4, `wrap at ${hd}`);
assert.ok(Math.abs(D.danceEnvelope(before) - D.danceEnvelope(1e-6)) < 1e-3 || D.danceEnvelope(1e-6) < 0.01, "the beat restarts at the wrap");
assert.equal(D.danceClock(D.DANCE_PERIOD_S + 2.5), 2.5);

// The beat: a quick rise after each beat and a slow settle, at 2 Hz.
const env = (T) => D.danceEnvelope(T);
assert.ok(env(0) < 0.01 && env(0.1) > 0.4 && env(0.45) < env(0.1));
assert.ok(Math.abs(env(0.1) - env(0.6)) < 1e-9, "2 Hz");

// Not a metronome: the tip's sway on one beat differs from the same beat a cycle later.
const swings = [0, 1, 2, 3].map((k) => D.danceSway(0.25 + k, 1, 1));
assert.ok(Math.max(...swings) - Math.min(...swings) > 0.02, `the slow drift changes each swing: ${swings}`);
// Gentle: the tip moves at most about a tenth of the box's half-width.
let most = 0;
for (let T = 0; T < 20; T += 0.01) most = Math.max(most, Math.abs(D.danceSway(T, 1, 1)));
assert.ok(most <= 0.086, `the tip's reach ${most}`);
console.log("luch-dance: all passed");
