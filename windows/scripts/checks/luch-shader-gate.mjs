import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// The flare gate (spec §3.5: mini Луч has no flares). smax(a, 0, k) lifts a negative field to 0, which
// paints a translucent square outside the body, so the uniform must gate the merge, not scale its input.
const smax = (a, b, k) => { const h = Math.min(1, Math.max(0, 0.5 + 0.5 * (a - b) / k)); return b + (a - b) * h + k * h * (1 - h); };
const smoothstep = (e0, e1, x) => { const t = Math.min(1, Math.max(0, (x - e0) / (e1 - e0))); return t * t * (3 - 2 * t); };
const mix = (a, b, k) => a * (1 - k) + b * k;
const merge = (fire, fl, k, uFlares) => mix(fire, smax(fire, fl * 0.95, k), uFlares); // mirrors the shader line

const outside = -0.8;
assert.equal(merge(outside, 0, 0.06, 0), outside); // flares off: untouched
assert.equal(smoothstep(-0.008, 0.012, merge(outside, 0, 0.06, 0)), 0); // alpha 0 outside the body
assert.ok(smoothstep(-0.008, 0.012, smax(outside, 0, 0.06)) > 0.3); // the old scaled-input form was visible
assert.ok(merge(outside, -0.9, 0.06, 1) <= outside + 0.06 / 4); // flares on, flare field below: smax adds at most k/4
assert.equal(smoothstep(-0.008, 0.012, merge(outside, -0.9, 0.06, 1)), 0); // and alpha stays 0
assert.ok(merge(0.1, 0.5, 0.06, 1) > 0.1); // flares on, a flare merges into the body

// The shader source really uses this gate.
const src = readFileSync(new URL("../../src/luch/fire.frag.glsl", import.meta.url), "utf8");
assert.ok(src.includes("fire = mix(fire, smax(fire, fl2 * 0.95, 0.06 * (1.0 - ph)), uFlares);"), "shader flare gate");
console.log("luch-shader-gate: all passed");
