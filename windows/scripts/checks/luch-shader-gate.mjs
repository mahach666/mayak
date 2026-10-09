import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// The flare gate (spec §3.5: mini Луч has no flares). smax(a, 0, k) lifts a negative field to 0, which
// paints a translucent square outside the body, so the uniform must gate the merge, not scale its input.
const smax = (a, b, k) => { const h = Math.min(1, Math.max(0, 0.5 + 0.5 * (a - b) / k)); return b + (a - b) * h + k * h * (1 - h); };
const smoothstep = (e0, e1, x) => { const t = Math.min(1, Math.max(0, (x - e0) / (e1 - e0))); return t * t * (3 - 2 * t); };
const mix = (a, b, k) => a * (1 - k) + b * k;
const merge = (fire, fl, k, uFlares, ramp = 1) => mix(fire, smax(fire, fl * 0.95, k), uFlares * ramp); // mirrors the shader line

const outside = -0.8;
assert.equal(merge(outside, 0, 0.06, 0), outside); // flares off: untouched
assert.equal(smoothstep(-0.008, 0.012, merge(outside, 0, 0.06, 0)), 0); // alpha 0 outside the body
assert.ok(smoothstep(-0.008, 0.012, smax(outside, 0, 0.06)) > 0.3); // the old scaled-input form was visible
assert.ok(merge(outside, -0.9, 0.06, 1) <= outside + 0.06 / 4); // flares on, flare field below: smax adds at most k/4
assert.equal(smoothstep(-0.008, 0.012, merge(outside, -0.9, 0.06, 1)), 0); // and alpha stays 0
assert.ok(merge(0.1, 0.5, 0.06, 1) > 0.1); // flares on, a flare merges into the body

// The fade-in ramp (smoothstep(0, 0.12, ph)) gates the merge too. Scaling the flare field by it instead
// sent the field to ~0 at the start of every flare cycle and flashed the whole box (live check, Task 4).
const ramp = smoothstep(0, 0.12, 0.0005);
assert.ok(smoothstep(-0.008, 0.012, smax(outside, -20 * ramp * 0.95, 0.06)) > 0.2); // the scaled-input form floods
assert.equal(smoothstep(-0.008, 0.012, merge(outside, -20, 0.06, 1, ramp)), 0); // the gated form stays clear

// The shader source really uses this gate.
const src = readFileSync(new URL("../../src/luch/fire.frag.glsl", import.meta.url), "utf8");
assert.ok(src.includes("fire = mix(fire, smax(fire, fl2 * 0.95, 0.06 * (1.0 - ph)), uFlares * smoothstep(0.0, 0.12, ph));"), "shader flare gate");
assert.ok(src.includes("float fl2 = drop(qf, 0.045, 0.035, 0.1, 1.2);"), "the flare field is not scaled by the ramp");

// The glow fades out before the canvas edge (spec §3.5, Task 5): the fire box is wider than the canvas,
// so a glow cut by the canvas showed as a faint square. Mirrors the shader's fade and render.ts's mapping
// for an untransformed 22 css px canvas at dpr 1.5, box 1.25 W centred on it.
const viewFade = (vx, vy, w, h, f) => smoothstep(0, f, Math.min(vx, w - vx)) * smoothstep(0, f, Math.min(vy, h - vy));
{
  const W = 22, dpr = 1.5, u = 1.25 * W, half = u / 2, cw = W * dpr, ch = cw;
  const m = { a: dpr, b: 0, c: 0, d: dpr, e: (W / 2) * dpr, f: (W / 2) * dpr }; // after translate(cx, cy)
  const X = [m.a * half, -m.c * half, m.e], Y = [m.b * half, -m.d * half, m.f];
  const at = (px, py) => [X[0] * px + X[1] * py + X[2], Y[0] * px + Y[1] * py + Y[2]];
  const fade = Math.max(2, 0.15 * Math.min(cw, ch));
  const edgeX = (W / 2) / half; // box x of the canvas's right edge
  assert.equal(viewFade(...at(edgeX, 0), cw, ch, fade), 0); // at the canvas edge: no glow left
  assert.equal(viewFade(...at(-edgeX, 0), cw, ch, fade), 0);
  assert.equal(viewFade(...at(0, -edgeX), cw, ch, fade), 0); // bottom edge (box y up, canvas y down)
  assert.equal(viewFade(...at(0, 0), cw, ch, fade), 1); // the centre keeps its glow
  assert.equal(viewFade(...at(0.44, -0.44), cw, ch, fade), 1); // the body's corner too
}
assert.ok(src.includes("glow *= smoothstep(0.0, uViewFade.z, e.x) * smoothstep(0.0, uViewFade.z, e.y);"), "shader glow fade");
assert.ok(src.includes("if (uViewFade.z > 0.0)"), "the fade is off when no canvas is given");
// Mini Луч has no streaks (spec §3.5): the switch gates them.
assert.ok(src.includes("col *= 1.0 + 0.07 * uStreaks * streak * smoothstep(0.05, 0.3, fire);"), "shader streak switch");
// The design copy stays identical to the app's shader.
const design = readFileSync(new URL("../../../design/luch/luch-g4.frag.glsl", import.meta.url), "utf8");
const lf = (t) => t.replace(/\r\n/g, "\n");
assert.equal(lf(design), lf(src), "design/luch/luch-g4.frag.glsl matches fire.frag.glsl");
console.log("luch-shader-gate: all passed");
