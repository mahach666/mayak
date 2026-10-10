// luch/mouth.ts: the mouth spring keeps its speed and stability whatever the frame rate. The island
// steps Луч by the real frame time (capped at 0.1 s); a long frame is cut into slices ≤ 0.02 s.
import assert from "node:assert/strict";
import { stepMouth, MOUTH_MAX_SLICE_S } from "../../src/luch/mouth.ts";

const run = (dt, frames, target = 0.42, from = { slot: 0, vel: 0 }) => {
  let s = from;
  for (let i = 0; i < frames; i++) s = stepMouth(s, target, dt);
  return s;
};

// A 60 fps frame is one slice: exactly the old single Euler step.
{
  const dt = 1 / 60, omega = (2 * Math.PI) / 0.25, zeta = 0.6, target = 0.42;
  const acc = omega * omega * target;
  const vel = acc * dt;
  assert.deepEqual(stepMouth({ slot: 0, vel: 0 }, target, dt), { slot: vel * dt, vel });
}

// One 15 fps frame (0.066 s) in slices lands close to four 0.0165 s frames.
{
  const a = stepMouth({ slot: 0, vel: 0 }, 0.42, 0.066);
  const b = run(0.0165, 4);
  assert.ok(Math.abs(a.slot - b.slot) < 1e-9, `slot ${a.slot} vs ${b.slot}`);
  assert.ok(Math.abs(a.vel - b.vel) < 1e-9, `vel ${a.vel} vs ${b.vel}`);
}

// Over 1 s the spring arrives at the same point at 15 fps and at 60 fps.
{
  const slow = run(0.066, Math.round(1 / 0.066));
  const fast = run(0.016, Math.round(1 / 0.016));
  assert.ok(Math.abs(slow.slot - 0.42) < 0.01, `15 fps settles: ${slow.slot}`);
  assert.ok(Math.abs(fast.slot - 0.42) < 0.01, `60 fps settles: ${fast.slot}`);
  assert.ok(Math.abs(slow.slot - fast.slot) < 0.01);
}

// Stable at the island's cap (0.1 s, five slices): it settles, never grows, never goes negative.
{
  let s = { slot: 0, vel: 0 }, maxSlot = 0;
  for (let i = 0; i < 100; i++) {
    s = stepMouth(s, 0.42, 0.1);
    maxSlot = Math.max(maxSlot, s.slot);
    assert.ok(Number.isFinite(s.slot) && s.slot >= 0);
  }
  assert.ok(maxSlot < 0.42 * 1.5, `overshoot bounded: ${maxSlot}`);
  assert.ok(Math.abs(s.slot - 0.42) < 1e-3 && Math.abs(s.vel) < 1e-2, `settled at dt 0.1: ${s.slot}`);
  // closing from open at the cap also settles at 0 without going below it
  let c = { slot: 0.42, vel: 0 };
  for (let i = 0; i < 100; i++) { c = stepMouth(c, 0, 0.1); assert.ok(c.slot >= 0); }
  assert.ok(c.slot < 1e-3 && Math.abs(c.vel) < 1e-2);
}

// Slices never exceed the bound; no or negative time changes nothing.
assert.ok(MOUTH_MAX_SLICE_S <= 0.05 && MOUTH_MAX_SLICE_S > 1 / 60 + 0.002, "≤ 0.05 s; a 60 fps frame is one slice");
{
  const s = { slot: 0.2, vel: 0.5 };
  assert.deepEqual(stepMouth(s, 0.42, 0), s);
  assert.deepEqual(stepMouth(s, 0.42, -1), s);
  // 0.1 s is exactly five 0.02 s slices
  let five = s;
  for (let i = 0; i < 5; i++) five = stepMouth(five, 0.42, 0.02);
  const once = stepMouth(s, 0.42, 0.1);
  assert.ok(Math.abs(once.slot - five.slot) < 1e-12 && Math.abs(once.vel - five.vel) < 1e-12);
}

console.log("mouth spring: same speed at 15 and 60 fps, stable at dt 0.1");
