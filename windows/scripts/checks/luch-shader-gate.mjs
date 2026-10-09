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
// (flareK is uFlares, scaled down by the pomodoro looks: still a gate on the merge)
assert.ok(src.includes("fire = mix(fire, smax(fire, fl2 * 0.95, 0.06 * (1.0 - ph)), flareK * smoothstep(0.0, 0.12, ph));"), "shader flare gate");
assert.ok(src.includes("float warpK = 1.0, erK = 1.0, flareK = uFlares, glowK = 1.0, emb = 0.0;"), "flareK starts as uFlares");
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
// The dance (music, luch/dance.ts): the field itself moves. Mirrors the shader block; at uDance = 0 it
// must leave the fire exactly as before.
{
  const TAU = 6.2831853;
  const clamp01 = (v) => Math.min(1, Math.max(0, v));
  // returns the sampled point and the brightness pulse for the box point (px, py)
  const dance = (px, py, uDance, T) => {
    let pulse = 1;
    if (uDance > 0) {
      const bph = T * 2 - Math.floor(T * 2);
      const env = (1 - Math.exp(-bph * 22)) * Math.exp(-bph * 4);
      px = 0 + (px - 0) / (1 - 0.03 * uDance * env);
      py = -0.44 + (py + 0.44) / (1 + 0.08 * uDance * env);
      const hd = clamp01((py + 0.44) / 1.1);
      px -= uDance * Math.pow(hd, 1.5) * (0.06 * Math.sin(TAU * T - 1.8 * hd) + 0.025 * Math.sin(TAU * 0.37 * T - 1.2 * hd + 1.3));
      pulse = 1 + 0.08 * uDance * env;
    }
    return [px, py, pulse];
  };
  for (let px = -1; px <= 1; px += 0.1) for (let py = -1; py <= 1; py += 0.1) for (const T of [0, 0.13, 0.5, 7.77, 99.9]) {
    assert.deepEqual(dance(px, py, 0, T), [px, py, 1], `untouched at uDance 0 (${px}, ${py}, ${T})`);
  }
  // luch/dance.ts says where a flame point is drawn: the shader samples that point back to it, so the
  // eyes the engine moves with danceMove stay on the face the shader draws.
  const D = await import(new URL("../../src/luch/dance.ts", import.meta.url).href);
  for (const T of [0.02, 0.08, 0.31, 0.77, 3.3]) for (const amt of [0.4, 1]) for (const [cx, cy] of [[0.09, -0.095], [-0.09, 0.66], [0.3, 0.1], [0, -0.44]]) {
    const v = D.danceMove(T, amt, cx, cy);
    const [sx, sy, pulse] = dance(v.x, v.y, amt, T);
    assert.ok(Math.abs(sx - cx) < 1e-6 && Math.abs(sy - cy) < 1e-6, `danceMove is the shader's inverse at (${cx}, ${cy}), T ${T}`);
    assert.ok(Math.abs(pulse - D.dancePulse(T, amt)) < 1e-6, "same pulse");
  }
  // the base stays put, the tip swings furthest, and the tip follows the base (it peaks later)
  const swayAt = (hd, T) => D.danceSway(T, hd, 1);
  const peak = (hd) => { let best = 0, at = 0; for (let T = 0; T < 1; T += 0.001) { const s = swayAt(hd, T + 10); if (s > best) { best = s; at = T; } } return [best, at]; };
  assert.equal(swayAt(0, 1.234), 0);
  assert.ok(peak(1)[0] > 2 * peak(0.4)[0], "the tip swings further than the body");
  assert.ok(peak(1)[1] > peak(0.4)[1], "the tip peaks after the body");
}
assert.ok(src.includes("uniform float uDance, uDanceT;") && src.includes("float pulse = 1.0;") && src.includes("if (uDance > 0.0) {"), "the dance block is skipped at uDance = 0");
assert.ok(src.includes("float bph = fract(uDanceT * 2.0);") && src.includes("float env = (1.0 - exp(-bph * 22.0)) * exp(-bph * 4.0);"), "shader dance beat");
assert.ok(src.includes("p = base + (p - base) / vec2(1.0 - 0.03 * uDance * env, 1.0 + 0.08 * uDance * env);"), "shader dance stretch");
assert.ok(src.includes("p.x -= uDance * pow(hd, 1.5) * (0.06 * sin(6.2831853 * uDanceT - 1.8 * hd) + 0.025 * sin(6.2831853 * 0.37 * uDanceT - 1.2 * hd + 1.3));"), "shader dance sway");
assert.ok(src.includes("pulse = 1.0 + 0.08 * uDance * env;") && src.includes("col *= uBright * pulse;") && src.includes("* uBright * pulse * edge * glowK;"), "shader dance pulse");

// The pomodoro looks (luch/focus.ts): the field itself changes, as for the dance. Mirrors the shader's
// deformation; with every amount at 0 the block is skipped and every factor it would change keeps the
// value that leaves the old expression exactly as it was (x * 1.0 is x, uTall stays uTall).
{
  const TAU = 6.2831853;
  // returns the sampled point and the factors for the box point (px, py) after the dance
  const focus = (px, py, w, r, e, T) => {
    let tall = 1, warpK = 1, erK = 1, flareK = 1, glowK = 1, pulse = 1;
    if (w + r + e > 0) {
      const br = Math.sin(TAU * 0.22 * T), emb = 0.5 + 0.5 * Math.sin(TAU * 0.18 * T);
      const fx = (1 - 0.06 * w) * (1 + 0.03 * r) * (1 + 0.05 * e);
      const fy = (1 + 0.08 * w) * (1 - 0.08 * r + 0.03 * r * br) * (1 - 0.12 * e);
      px = 0 + (px - 0) / fx; py = -0.44 + (py + 0.44) / fy;
      tall *= (1 + 0.18 * w) * (1 - 0.25 * r + 0.14 * r * br) * (1 - 0.7 * e);
      warpK = (1 - 0.5 * w) * (1 - 0.35 * r) * (1 - 0.5 * e);
      erK = (1 - 0.3 * w) * (1 - 0.2 * r);
      flareK *= (1 - 0.6 * w) * (1 - 0.5 * r) * (1 - e);
      pulse *= (1 + 0.06 * w) * (1 - 0.08 * r + 0.06 * r * br) * (1 - 0.15 * e + 0.1 * e * emb);
      glowK = (1 + 0.3 * r * br) * (1 - 0.3 * e + 0.3 * e * emb);
    }
    return { px, py, tall, warpK, erK, flareK, glowK, pulse };
  };
  for (let px = -1; px <= 1; px += 0.1) for (let py = -1; py <= 1; py += 0.1) for (const T of [0, 2.2, 50, 99.9]) {
    assert.deepEqual(focus(px, py, 0, 0, 0, T), { px, py, tall: 1, warpK: 1, erK: 1, flareK: 1, glowK: 1, pulse: 1 }, `untouched at no look (${px}, ${py}, ${T})`);
  }
  // luch/focus.ts says where a flame point is drawn: the shader samples that point back to it, so the
  // eyes the engine moves with focusMove stay on the face the shader draws.
  const F = await import(new URL("../../src/luch/focus.ts", import.meta.url).href);
  for (const T of [0.4, 2.2, 7.7, 63.1]) for (const [w, r, e] of [[1, 0, 0], [0, 1, 0], [0, 0, 1], [0.3, 0.7, 0], [0, 0.4, 0.6]]) for (const [cx, cy] of [[0.09, -0.095], [-0.09, 0.66], [0, -0.44]]) {
    const v = F.focusMove(T, { work: w, rest: r, ember: e }, cx, cy);
    const s = focus(v.x, v.y, w, r, e, T);
    assert.ok(Math.abs(s.px - cx) < 1e-6 && Math.abs(s.py - cy) < 1e-6, `focusMove is the shader's inverse at (${cx}, ${cy}), T ${T}`);
  }
  // the embers leave no flares and only a stub of the tongue; the factors stay positive (no flipped field)
  const em = focus(0, 0, 0, 0, 1, 1);
  assert.equal(em.flareK, 0);
  assert.ok(em.tall > 0.1 && em.tall < 0.35, `embers tongue ${em.tall}`);
  for (const [w, r, e] of [[1, 0, 0], [0, 1, 0], [0, 0, 1], [1, 1, 1]]) for (let T = 0; T < 10; T += 0.1) {
    const k = focus(0, 0, w, r, e, T);
    for (const n of ["tall", "warpK", "erK", "glowK", "pulse"]) assert.ok(k[n] > 0, `${n} > 0`);
  }
}
const phasedBlock = src.slice(src.indexOf("if (phased) {"), src.indexOf("// a slow, broad drift"));
const phasedPaint = src.slice(src.lastIndexOf("if (phased) {"), src.indexOf("// the face: a big cream panel"));
assert.ok(src.includes("uniform float uWork, uRest, uEmber, uFocusT;") && src.includes("bool phased = uWork + uRest + uEmber > 0.0;"), "the phase blocks are skipped at no look");
assert.ok(src.includes("float tall = uTall, lean = -0.16, curl = -1.6, iLean = -0.18, iCurl = -1.5;"), "the tongue's old constants are the defaults");
assert.ok(src.includes("float ft = lick(q, vec2(0.02, 0.06), lean, 0.34, 0.64 * tall, curl, t, 0.0);") && src.includes("float inner = lick(q, vec2(-0.02, -0.02), iLean, 0.25, 0.56 * tall, iCurl, t + 0.25, 0.4);"), "the tongue reads the phase's lean, curl and height");
assert.ok(src.includes("vec2 q = p + warp * (0.015 + 0.16 * h * h) * warpK;") && src.includes("float fire = f - (er - 0.45) * (0.02 + 0.25 * h * h) * erK;"), "the drift and erosion factors multiply the old expressions");
assert.ok(src.includes("vec2 fs = vec2((1.0 - 0.06 * uWork) * (1.0 + 0.03 * uRest) * (1.0 + 0.05 * uEmber),") && src.includes("p = base + (p - base) / fs;"), "shader phase stretch");
assert.ok(src.includes("tall *= (1.0 + 0.18 * uWork) * (1.0 - 0.25 * uRest + 0.14 * uRest * br) * (1.0 - 0.7 * uEmber);") && src.includes("flareK *= (1.0 - 0.6 * uWork) * (1.0 - 0.5 * uRest) * (1.0 - uEmber);"), "the mirror's tongue and flare factors are the shader's");
assert.ok(src.includes("(1.0 + 0.08 * uWork) * (1.0 - 0.08 * uRest + 0.03 * uRest * br) * (1.0 - 0.12 * uEmber));") && src.includes("float br = sin(6.2831853 * 0.22 * uFocusT);") && src.includes("emb = 0.5 + 0.5 * sin(6.2831853 * 0.18 * uFocusT);"), "the mirror's breath and stretch are the shader's");
assert.ok(src.includes("vec3 glowCol = mix(cMid, cDeep, 0.4);") && src.includes("gl_FragColor = vec4(col * a + glowCol * glow, a + glow * 0.85);"), "the glow colour defaults to the old one");
// Hidden effects never lift the field outside the silhouette (a square once showed from smax(fire, 0)):
// the phase blocks never raise fire or calm, the coal is a mix of colours, the sparks only add a small
// disc of alpha gated by the embers' amount.
for (const block of [phasedBlock, phasedPaint]) {
  assert.ok(!/fire\s*=|calm\s*=|max\(fire|smax\(fire/.test(block), "the phase blocks leave the field alone");
}
assert.ok(phasedPaint.includes("col = mix(col, coal, uEmber);"), "the coal is a colour mix");
assert.ok(phasedPaint.includes("float s = on * uEmber * smoothstep(0.0, 0.1, ph) * (1.0 - smoothstep(0.5, 1.0, ph)) * smoothstep(0.02, 0.006, length(p - c));") && phasedPaint.includes("a = max(a, s);"), "the sparks are small discs gated by uEmber");
{
  // the spark's alpha is 0 at uEmber = 0 and off its small disc
  const sm = (e0, e1, x) => { const k = Math.min(1, Math.max(0, (x - e0) / (e1 - e0))); return k * k * (3 - 2 * k); };
  const spark = (on, e, ph, d) => on * e * sm(0, 0.1, ph) * (1 - sm(0.5, 1, ph)) * sm(0.02, 0.006, d);
  for (const ph of [0, 0.2, 0.5, 0.9]) for (const d of [0, 0.01, 0.5]) assert.equal(spark(1, 0, ph, d), 0);
  for (const ph of [0.2, 0.4]) { assert.equal(spark(1, 1, ph, 0.03), 0); assert.ok(spark(1, 1, ph, 0) > 0.9); }
}

// The colour change (luch/colour.ts): three looks (licks, flare, core). With no change (uColorK = 1)
// every colour block is skipped and the palette main() paints with is the uniforms as before: every use
// of uDeep, uMid, uLight and uCore in main() now reads cDeep… which start as exactly those.
{
  const main = src.slice(src.indexOf("void main() {"));
  assert.ok(src.includes("uniform vec3 uFromDeep, uFromMid, uFromLight, uFromCore;") && src.includes("uniform float uColorK, uColorV;"), "colour uniforms");
  assert.ok(main.includes("vec3 cDeep = uDeep, cMid = uMid, cLight = uLight, cCore = uCore;") && main.includes("float flash = 0.0, swell = 0.0;"), "the palette starts as the uniforms");
  const block = main.slice(main.indexOf("if (uColorK < 1.0) {"), main.indexOf("// the dance: the field itself moves"));
  assert.ok(block.length > 0 && block.includes("float ck = clamp(uColorK, 0.0, 1.0);"), "the colour block is skipped at uColorK = 1");
  // outside the colour block main() reads the palette only through cDeep…
  const rest = main.replace(block, "").replace("vec3 cDeep = uDeep, cMid = uMid, cLight = uLight, cCore = uCore;", "");
  assert.ok(!/\bu(Deep|Mid|Light|Core|FromDeep|FromMid|FromLight|FromCore)\b/.test(rest), "main() paints with the c palette only");
  // the old expressions, renamed: the paint is the same at no change
  for (const line of [
    "vec3 col = mix(cDeep, cMid, smoothstep(0.0, 0.05, fire));",
    "col = mix(col, cLight, lightZone * smoothstep(0.03, 0.1, fire));",
    "col = mix(col, mix(cLight, cCore, 0.5), smoothstep(0.5, 0.8, max(fb, inner * 0.8)));",
    "col = mix(col, mix(cCore, mix(cCore, cLight, 0.4), smoothstep(-0.09, 0.0, sd)), core);",
  ]) assert.ok(main.includes(line), line);
  // every other colour line is behind the same switch; the pale burn is a colour mix inside the flame,
  // the sparks small discs gated by the swell (0 unless a flare change runs); the block only warps the
  // coordinates (the flare's swell), never the field or the alpha
  assert.ok(main.includes("  if (uColorK < 1.0) pulse *= 1.0 + 0.3 * swell;"), "the swell's brightness is gated");
  const paint = main.slice(main.indexOf("// the colour change's paler burn"), main.indexOf("// the face: a big cream panel"));
  assert.ok(paint.includes("col = mix(col, mix(cLight, cCore, 0.6), flash * smoothstep(0.02, 0.1, fire));"), "the pale burn line");
  assert.ok(paint.includes("float s = swell * smoothstep(0.024, 0.008, length(p - c));") && paint.includes("a = max(a, s);"), "the flare's sparks are discs gated by the swell");
  assert.ok(!/fire\s*=|calm\s*=|max\(fire|smax\(fire/.test(paint), "the paint leaves the field alone");
  assert.ok(!/fire\s*=|calm\s*=|\ba\s*=|max\(fire|smax\(fire/.test(block), "the colour block leaves the field alone");
  assert.ok(block.includes("p = base + (p - base) / vec2(1.0 - 0.03 * swell, 1.0 + 0.12 * swell);"), "the swell stretches the field about its base");
  // mirror of the block, transliterated: colour.ts gives the same wave, flash and swell
  const C = await import(new URL("../../src/luch/colour.ts", import.meta.url).href);
  for (const line of [
    "float nn = 0.65 * fbm3(vec2(p.x * 5.0, p.y * 2.0 - t * 2.2)) + 0.35 * noise(vec2(p.x * 10.0, p.y * 4.0 - t * 4.5));",
    "float ko = 1.0 - (1.0 - ck) * (1.0 - ck);",
    "float ch = clamp((p.y + 0.44) / 1.1, 0.0, 1.0);",
    "if (uColorV > 1.5) {",
    "float cr = min(length(vec2((p.x - 0.03) / 0.4, (p.y + 0.1) / 0.45)), 1.4);",
    "cf = -0.3 + 1.66 * ko - (0.62 * cr + 0.18 * ch + 0.4 * (nn - 0.45));",
    "cw = smoothstep(-0.1, 0.1, cf);",
    "flash = 0.4 * (1.0 - smoothstep(0.0, 0.12, abs(cf))) * sin(3.14159265 * ck);",
    "float kk = uColorV > 0.5 ? smoothstep(0.04, 0.3, ck) : ko;",
    "float cr = min(length(vec2((p.x - 0.02) / 0.45, (p.y + 0.2) / 0.35)), 1.2);",
    "cf = -0.32 + 1.4 * kk - (0.45 * ch + 0.25 * cr + 0.5 * (nn - 0.45));",
    "cw = smoothstep(-0.06, 0.06, cf);",
    "flash = 0.5 * (1.0 - smoothstep(0.0, 0.08, abs(cf))) * sin(3.14159265 * ck);",
    "swell = smoothstep(0.0, 0.12, ck) * (1.0 - smoothstep(0.12, 1.0, ck));",
    "flash = 0.5 * swell;",
    "cDeep = mix(uFromDeep, uDeep, cw); cMid = mix(uFromMid, uMid, cw);",
    "cLight = mix(uFromLight, uLight, cw); cCore = mix(uFromCore, uCore, cw);",
  ]) assert.ok(block.includes(line), `shader colour: ${line}`);
  const shader = (V, k, x, y, nn) => {
    const ch = Math.min(1, Math.max(0, (y + 0.44) / 1.1)), ko = 1 - (1 - k) * (1 - k);
    let cf, cw, flash, swell = 0;
    if (V > 1.5) {
      const cr = Math.min(Math.hypot((x - 0.03) / 0.4, (y + 0.1) / 0.45), 1.4);
      cf = -0.3 + 1.66 * ko - (0.62 * cr + 0.18 * ch + 0.4 * (nn - 0.45));
      cw = smoothstep(-0.1, 0.1, cf);
      flash = 0.4 * (1 - smoothstep(0, 0.12, Math.abs(cf))) * Math.sin(3.14159265 * k);
    } else {
      const kk = V > 0.5 ? smoothstep(0.04, 0.3, k) : ko;
      const cr = Math.min(Math.hypot((x - 0.02) / 0.45, (y + 0.2) / 0.35), 1.2);
      cf = -0.32 + 1.4 * kk - (0.45 * ch + 0.25 * cr + 0.5 * (nn - 0.45));
      cw = smoothstep(-0.06, 0.06, cf);
      flash = 0.5 * (1 - smoothstep(0, 0.08, Math.abs(cf))) * Math.sin(3.14159265 * k);
      if (V > 0.5) { swell = smoothstep(0, 0.12, k) * (1 - smoothstep(0.12, 1, k)); flash = 0.5 * swell; }
    }
    return { cw, flash, swell };
  };
  for (const v of C.COLOUR_VARIANTS) for (const k of [0, 0.05, 0.2, 0.5, 0.77, 1]) for (let x = -3; x <= 3; x += 0.25) for (let y = -3; y <= 3; y += 0.25) for (const n of [0, 0.4, C.NOISE_MAX]) {
    const m = shader(C.variantIndex(v), k, x, y, n);
    assert.ok(Math.abs(m.cw - C.colourWave(v, k, x, y, n)) < 1e-9, `${v} wave mirror at k ${k} (${x}, ${y})`);
    assert.ok(Math.abs(m.flash - C.colourFlash(v, k, x, y, n)) < 1e-6, `${v} flash mirror at k ${k} (${x}, ${y})`);
    assert.ok(Math.abs(m.swell - C.colourSwell(v, k)) < 1e-12, `${v} swell mirror at k ${k}`);
    // the whole box (and the glow past it, at any grow) is the old palette at the start and the new one at the end
    if (k === 0) { assert.equal(m.cw, 0); assert.equal(m.flash, 0); assert.equal(m.swell, 0); }
    if (k === 1) { assert.equal(m.cw, 1); assert.ok(Math.abs(m.flash) < 1e-6); assert.equal(m.swell, 0); }
  }
  // the noise's bound (colour.ts NOISE_MAX): 0.65 × fbm3 (at most 0.875) + 0.35 × noise (at most 1)
  assert.ok(0.65 * 0.875 + 0.35 <= C.NOISE_MAX + 1e-9);
}

// The design copy stays identical to the app's shader.
const design = readFileSync(new URL("../../../design/luch/luch-g4.frag.glsl", import.meta.url), "utf8");
const lf = (t) => t.replace(/\r\n/g, "\n");
assert.equal(lf(design), lf(src), "design/luch/luch-g4.frag.glsl matches fire.frag.glsl");
console.log("luch-shader-gate: all passed");
