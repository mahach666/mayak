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
// Ignition (the launch greeting, Task 7): the fire catches inside the full-size silhouette, nothing is
// scaled. Mirrors the shader block; at uIgnite = 1 it must leave the fire exactly as before.
{
  // `f` is the uneroded silhouette field (the fire before erosion); `t` drifts the front sideways;
  // (px, py) stand for the shader's warped q, which is within 0.1 of p
  const ignite = (fire, calm, px, py, fk, seed, uIgnite, f = fire, t = 0) => {
    const igK = 1 - Math.min(1, Math.max(0, uIgnite));
    if (!(igK > 0)) return [fire, calm, 0];
    const ig = 1 - igK;
    const reach = (1.05 * Math.max(0, ig - 0.12)) / 0.88;
    const ss = smoothstep(0, 0.2, reach);
    const ox = 0.12 * Math.sin(t * 1.9) * igK;
    const dx = px - ox, dy = py + 0.4;
    const dist = Math.hypot(dx * (1 + 0.9 * Math.max(dy, 0)), dy * 0.7);
    const front = (reach - dist + igK * ss * (0.25 * Math.min(1, Math.max(0, f)) + (fk - 0.5) * 0.7)) * 1.6;
    fire = Math.min(fire, front); calm = Math.min(calm, front);
    const sk = 1 - smoothstep(0.1, 0.3, ig);
    return [mix(fire, smax(fire, seed, 0.04), sk), mix(calm, Math.max(calm, seed), sk), sk];
  };

  for (const [fire, calm] of [[-0.8, -0.8], [0.004, 0.01], [0.6, 0.6], [1.25, 1.25]]) {
    for (const ig of [1, 1.3]) {
      const [f, c, sk] = ignite(fire, calm, 0.1, -0.2, 0.7, 0.3, ig);
      assert.equal(f, fire); assert.equal(c, calm); assert.equal(sk, 0); // untouched at ignite >= 1
    }
  }
  // at ignite 0 (and while the lighter tongue burns alone, up to 0.12) nothing of the body burns
  // beyond a hair around the base point; the front reaches the tongue's tip only at the end
  for (const ig of [0, 0.12]) for (let px = -1; px <= 1; px += 0.05) for (let py = -1; py <= 1; py += 0.05) for (const fk of [0, 1]) {
    if (Math.hypot(px, (py + 0.4) * 0.8) < 0.08) continue;
    const [f] = ignite(1.25, 1.25, px, py, fk, -1, ig);
    assert.equal(smoothstep(-0.008, 0.012, f), 0, `body hidden at (${px}, ${py}), ignite ${ig}`);
  }
  for (const fk of [0, 1]) assert.ok(ignite(0.9, 0.9, -0.09, 0.66, fk, -1, 0.999)[0] > 0.012, "the whole tongue burns just before 1");
  assert.ok(smoothstep(-0.008, 0.012, ignite(-0.5, -0.5, 0, -0.4, 0.5, 0.2, 0)[0]) > 0.9, "the seed tongue burns at ignite 0");
  // the burning region spreads from the core out and up: the body's centre catches before its top and edge
  const catchAt = (fire, px, py) => { for (let ig = 0; ig <= 1; ig += 0.01) if (ignite(fire, fire, px, py, 0.5, -1, ig)[0] > 0.012) return ig; return 1; };
  assert.ok(catchAt(1.25, 0.01, -0.11) < catchAt(0.9, -0.05, 0.3), "core before the tongue");
  assert.ok(catchAt(0.9, -0.05, 0.3) < catchAt(0.2, -0.09, 0.6), "tongue before its tip");
  assert.ok(catchAt(1.25, 0.01, -0.11) < catchAt(0.15, 0.4, -0.11), "core before the edge");
  assert.ok(catchAt(0.2, -0.09, 0.6) > catchAt(0.15, 0.4, -0.11) && catchAt(0.2, -0.09, 0.6) > catchAt(0.15, -0.4, -0.11), "the tongue's tip catches last");
}
assert.ok(src.includes("if (igK > 0.0) {"), "the ignition block is skipped at uIgnite = 1");
assert.ok(src.includes("float front = (reach - dist + igK * ss * (0.25 * clamp(f, 0.0, 1.0) + (fk - 0.5) * 0.7)) * 1.6;") && src.includes("float dist = length(vec2(dq.x * (1.0 + 0.9 * max(dq.y, 0.0)), dq.y * 0.7));"), "shader ignition front");
assert.ok(src.includes("vec2 o = vec2(0.12 * sin(t * 1.9) * igK, -0.4);") && src.includes("float reach = 1.05 * max(0.0, ig - 0.12) / 0.88;"), "shader ignition reach and drift");
assert.ok(src.includes("fire = mix(fire, smax(fire, seed, 0.04), sk);"), "the seed merge is gated, not scaled");
assert.ok(src.includes("vec2 p = base + (p0 - base) / g;") && !/float g = .*uIgnite/.test(src), "ignition does not scale the coordinates");
// The design copy stays identical to the app's shader.
const design = readFileSync(new URL("../../../design/luch/luch-g4.frag.glsl", import.meta.url), "utf8");
const lf = (t) => t.replace(/\r\n/g, "\n");
assert.equal(lf(design), lf(src), "design/luch/luch-g4.frag.glsl matches fire.frag.glsl");
console.log("luch-shader-gate: all passed");
