// luch/colour.ts: Луч's colour changes as a fire would: a wave up the flame over about a second, eased
// in and out, the base first and the tongue's tip last; changes in a row never jump; settled, nothing runs.
import assert from "node:assert/strict";
import * as C from "../../src/luch/colour.ts";
import { paletteFor, IDLE_FIRE } from "../../src/luch/palette.ts";

const BLUE = paletteFor("#4a8cff"), GREEN = paletteFor("#34d399"), RED = paletteFor("#ff4a2a");
const DT = 1 / 60;

// Settled: nothing changes, the shader gets 1 (its colour block is skipped), the blend is the palette.
{
  const s = C.colourAt(IDLE_FIRE);
  assert.equal(C.colourSettled(s), true);
  assert.equal(C.colourK(s), 1);
  assert.equal(C.stepColour(s, IDLE_FIRE, DT), s, "same colour: the same state, no work");
  assert.equal(C.stepColour(s, { ...IDLE_FIRE }, DT), s, "palettes compare by value");
  assert.deepEqual(C.colourBlend(s), IDLE_FIRE);
}

// Timing and easing: COLOUR_S (1 s) for a change, eased in and out.
assert.ok(C.COLOUR_S >= 0.8 && C.COLOUR_S <= 1.2);
{
  let s = C.colourAt(IDLE_FIRE), t = 0;
  s = C.stepColour(s, BLUE, DT); t += DT;
  assert.equal(C.colourSettled(s), false);
  assert.ok(C.colourK(s) < 0.002, "the first step is tiny (eased in)");
  assert.deepEqual(C.colourHeading(s), BLUE);
  let prevK = C.colourK(s), maxStep = 0;
  while (!C.colourSettled(s)) {
    s = C.stepColour(s, BLUE, DT); t += DT;
    const k = C.colourK(s);
    assert.ok(k >= prevK, "progress only grows");
    maxStep = Math.max(maxStep, k - prevK); prevK = k;
  }
  assert.ok(Math.abs(t - C.COLOUR_S) <= DT + 1e-9, `settled after ${t} s`);
  assert.ok(maxStep < 1.6 * DT / C.COLOUR_S, "no step bigger than the eased curve's slope");
  assert.deepEqual(s, C.colourAt(BLUE));
  // eased out: just before the end the progress barely moves
  let e = C.colourAt(IDLE_FIRE); for (let i = 0; i < 59; i++) e = C.stepColour(e, BLUE, DT);
  assert.ok(1 - C.colourK(e) < 0.002, "eased out");
  assert.ok(Math.abs(C.colourK({ ...e, level: 0.5 }) - 0.5) < 1e-12, "half way at half time");
  assert.deepEqual(C.colourBlend({ ...e, level: 0 }), IDLE_FIRE);
}

// The wave: at the start of a change all is the old colour, at the end the new; the base catches before
// the body, the body before the tongue's tip; the flare runs along the boundary and is 0 at both ends.
{
  const catchAt = (x, y, n = 0.44) => { for (let k = 0; k <= 1.0001; k += 0.002) if (C.colourWave(k, x, y, n) >= 0.5) return k; return 2; };
  const base = catchAt(0, -0.42), body = catchAt(0.01, -0.11), edge = catchAt(0.42, -0.11), tip = catchAt(-0.09, 0.66);
  assert.ok(base < body && body < tip, `base ${base} < body ${body} < tip ${tip}`);
  assert.ok(body < edge, "the sides a little after the centre");
  assert.ok(base < 0.2 && tip > 0.7, "the base starts early, the tip ends late");
  for (const n of [0, 0.875]) {
    assert.equal(C.colourWave(0, -0.09, -0.44, n), 0);
    assert.equal(C.colourWave(1, 0.9, 1.5, n), 1);
  }
  assert.ok(C.colourFlare(0.5, 0, (0.5 * 1.8 - 0.3 - 0.3 * (0.44 - 0.5)) * 1.1 - 0.44, 0.44) > 0.95, "the flare sits on the boundary");
  assert.equal(C.colourFlare(0.5, 0, -0.44, 0.44), 0, "away from the boundary no flare");
}

// Back where it came from (an emote's tint ending early, idle → working → idle): the same wave runs
// backwards from where it is, so nothing jumps, and it lands in the old colour.
{
  let s = C.colourAt(IDLE_FIRE);
  for (let i = 0; i < 24; i++) s = C.stepColour(s, BLUE, DT);
  const k0 = C.colourK(s);
  const r = C.stepColour(s, IDLE_FIRE, DT);
  assert.equal(r.back, true);
  assert.deepEqual(r.from, s.from); assert.deepEqual(r.to, s.to);
  assert.ok(Math.abs(C.colourK(r) - k0) < 2 * DT, "no jump on the turn");
  assert.deepEqual(C.colourHeading(r), IDLE_FIRE);
  let t = DT, x = r, prev = C.colourK(r);
  while (!C.colourSettled(x)) { x = C.stepColour(x, IDLE_FIRE, DT); t += DT; if (C.colourSettled(x)) break; const k = C.colourK(x); assert.ok(Math.abs(k - prev) < 1.6 * DT); prev = k; }
  assert.ok(prev < 0.002, "eased into the old colour");
  assert.deepEqual(x, C.colourAt(IDLE_FIRE));
  assert.ok(t < 24 * DT + 2 * DT, `back over the same ground: ${t} s`);
  // and forward again
  const f = C.stepColour(C.stepColour(r, IDLE_FIRE, DT), BLUE, DT);
  assert.equal(f.back, false);
}

// A third colour mid-change: the running wave hurries to its end, then the newest colour climbs from
// there. A burst (blue, green, red within a third of a second) ends in red in well under three changes'
// time and never jumps: each point's colour (the palettes of one wave mixed by the wave there) is
// continuous in time. A front sweeping a point turns it quickly, so this is checked on a fine clock: a
// jump would stay as big however small the step, a moving front shrinks with it.
{
  const hex = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  const mixRGB = (a, b, k) => a.map((v, i) => v + (b[i] - v) * k);
  const at = (s, y) => mixRGB(hex(s.from.mid), hex(s.to.mid), C.colourWave(C.colourK(s), 0, y, 0.44));
  const ys = [-0.42, -0.3, -0.11, 0.1, 0.3, 0.5, 0.66];
  const burst = (dt) => {
    let s = C.colourAt(IDLE_FIRE), t = 0, maxJump = 0, prev = ys.map((y) => at(s, y));
    const plan = [[0, BLUE], [0.15, GREEN], [0.3, RED]];
    while (t < 4) {
      const wanted = [...plan].reverse().find(([from]) => t >= from)[1];
      s = C.stepColour(s, wanted, dt); t += dt;
      const now = ys.map((y) => at(s, y));
      for (let i = 0; i < now.length; i++) for (let c = 0; c < 3; c++) maxJump = Math.max(maxJump, Math.abs(now[i][c] - prev[i][c]));
      prev = now;
      if (C.colourSettled(s)) break;
    }
    return { s, t, maxJump };
  };
  const coarse = burst(DT), fine = burst(1 / 6000);
  assert.deepEqual(coarse.s, C.colourAt(RED));
  assert.ok(coarse.t < 2.2, `the burst settles in ${coarse.t.toFixed(2)} s`);
  assert.ok(fine.maxJump < 3, `continuous: largest step ${fine.maxJump.toFixed(2)} of 255 on a fine clock`);
}

// Only the newest waiting colour is kept; a change back to the running wave's target drops it.
{
  let s = C.colourAt(IDLE_FIRE);
  for (let i = 0; i < 10; i++) s = C.stepColour(s, BLUE, DT);
  s = C.stepColour(s, GREEN, DT);
  assert.deepEqual(s.next, GREEN);
  s = C.stepColour(s, RED, DT);
  assert.deepEqual(s.next, RED);
  s = C.stepColour(s, BLUE, DT);
  assert.equal(s.next, null);
  assert.deepEqual(C.colourHeading(s), BLUE);
  assert.deepEqual(C.stepColour(s, BLUE, -1).level, s.level, "no negative step");
}
console.log("luch-colour: all passed");
