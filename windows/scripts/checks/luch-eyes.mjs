// The eyes' cursor-following travel on Луч's face panel (src/luch/eyes.ts).
//
// Mochi's measure (old code, commit 4b04a8c, windows/src/mochi/engine.ts drawEyes): each eye sits at
//   ex = sin(sd*EYE_SP + yaw) * cos(EYE_P + pitch) * rx        (EYE_SP 0.37, EYE_P -0.12)
//   ey = -sin(EYE_P + pitch) * ry   (canvas y, down)
// with the body half-size rx = 1.14 R, ry = 0.88 R and yaw = 0.62 * lookX, pitch = 0.5 * lookY. The
// pair's centre therefore travels, as a fraction of the body half-size, at full look (|lookX| = 1):
//   x: sin(0.62) cos(0.37) cos(-0.12) = 0.54    y: sin(0.5 - 0.12) - sin(-0.12) = 0.49
// Луч's eyes travel the same fractions of the face panel half-size (0.32 x 0.22); the old Луч moved
// them 0.62 * 0.093 * 0.35 = 0.020 (6 % of the half-width), 8 times less.
import assert from "node:assert/strict";
import { eyeOffset, FACE, FACE_MARGIN } from "../../src/luch/eyes.ts";

const EYE_SP = 0.37, EYE_P = -0.12;
// Mochi's two eyes, straight from its formula (unit = body half-width / half-height)
function mochiPairCentre(yaw, pitch) {
  let x = 0;
  for (const sd of [-1, 1]) x += Math.sin(sd * EYE_SP + yaw) * Math.cos(EYE_P + pitch) / 2;
  return { x, y: Math.sin(EYE_P + pitch) }; // y up
}
const mochiRest = mochiPairCentre(0, 0);

// Луч's big eyes (G4 measures x 1.7) and a mini's (readable floor, s = 16 px): the pair's centre and
// half-extents in p-space. hx = step + 0.8 w, hy = 0.6 h.
const big = { cx: 0.09, cy: -0.095, hx: 0.093 + 0.8 * 0.0374, hy: 0.6 * 0.068 };
const mini = { cx: 0.09, cy: -0.095, hx: 1.52 / 16 + 0.8 * 0.1, hy: 0.6 * 0.1625 };
// Eyes that fit anywhere, to read the unclamped proportion
const tiny = { cx: FACE.cx, cy: FACE.cy, hx: 0.001, hy: 0.001 };

// zero at rest
for (const b of [big, mini, tiny]) {
  const o = eyeOffset(0, 0, b);
  assert.ok(Math.abs(o.x) < 1e-12 && Math.abs(o.y) < 1e-12, "no offset at rest");
}

// the proportion is Mochi's, to the formula (unclamped eyes)
for (const [yaw, pitch] of [[0.62, 0], [-0.62, 0], [0.3, 0], [0, 0.5], [0, -0.5], [0.62, 0.5], [-0.4, -0.25]]) {
  const o = eyeOffset(yaw, pitch, tiny), m = mochiPairCentre(yaw, pitch);
  assert.ok(Math.abs(o.x - (m.x - mochiRest.x) * FACE.hx) < 1e-9, `x ${yaw},${pitch}`);
  assert.ok(Math.abs(o.y - (m.y - mochiRest.y) * FACE.hy) < 1e-9, `y ${yaw},${pitch}`);
}
// the measured ratios themselves (a change of EYE_SP/EYE_P would show here)
assert.ok(Math.abs(eyeOffset(0.62, 0, tiny).x / FACE.hx - 0.54) < 0.01);
assert.ok(Math.abs(eyeOffset(0, 0.5, tiny).y / FACE.hy - 0.49) < 0.01);

// monotonic along each axis (the other held at rest), for the big eyes and the mini's
for (const b of [big, mini]) {
  let px = -Infinity, py = -Infinity;
  for (let i = -70; i <= 70; i++) {
    const x = eyeOffset(i / 100, 0, b).x, y = eyeOffset(0, i / 100, b).y;
    assert.ok(x >= px - 1e-6 && y >= py - 1e-6, "monotonic"); // the clamp is bisected: equal to ~1e-7
    px = x; py = y;
  }
  // and the look moves the eyes the right way: yaw + = right, pitch + = up
  assert.ok(eyeOffset(0.3, 0, b).x > 0 && eyeOffset(-0.3, 0, b).x < 0);
  assert.ok(eyeOffset(0, 0.3, b).y > 0 && eyeOffset(0, -0.3, b).y < 0);
}

// the eyes stay inside the face panel, with the margin, over the whole range the engine can reach
// (yaw ±0.62 scans, pitch -0.55*0.5 … 0.5, past it too), corners included
function rbox(px, py) {
  const dx = Math.abs(px - FACE.cx) - FACE.hx + FACE.r, dy = Math.abs(py - FACE.cy) - FACE.hy + FACE.r;
  return Math.hypot(Math.max(dx, 0), Math.max(dy, 0)) + Math.min(Math.max(dx, dy), 0) - FACE.r;
}
for (const b of [big, mini]) {
  for (let i = -80; i <= 80; i += 2) for (let j = -80; j <= 80; j += 2) {
    const o = eyeOffset(i / 100, j / 100, b);
    for (const sx of [-1, 1]) for (const sy of [-1, 1]) {
      const sd = rbox(b.cx + o.x + sx * b.hx, b.cy + o.y + sy * b.hy);
      assert.ok(sd <= -FACE_MARGIN + 1e-6, `inside the panel at ${i / 100},${j / 100}: ${sd}`);
    }
  }
}

// the big eyes travel a good part of Mochi's way: left (room to spare) the whole 0.54 of the half-width,
// at least 60 % of it to the right (the G4 eyes sit right of the panel's centre) and 0.49 of the half-height
assert.ok(-eyeOffset(-0.62, 0, big).x >= 0.99 * 0.54 * FACE.hx);
assert.ok(eyeOffset(0.62, 0, big).x >= 0.6 * 0.54 * FACE.hx);
assert.ok(eyeOffset(0, 0.5, big).y >= 0.99 * 0.49 * FACE.hy);
assert.ok(eyeOffset(0, -0.5, big).y <= -0.45 * FACE.hy);

// eyes that do not fit even at rest do not move and do not throw
const huge = { cx: 0.09, cy: -0.095, hx: 0.5, hy: 0.5 };
assert.deepEqual(eyeOffset(0.6, 0.4, huge), { x: 0, y: 0 });

console.log("luch-eyes ok");
