import assert from "node:assert/strict";
import { botPosition, COMPACT_W, NOTCH_H } from "../../src/core/layout.ts";
import { G, GREET_END, BURSTS, sparkPoints, sparkAt, burstSparks, greetFrame, igniteAt, surgeAt, REST, small, FLINT, IGNITION, CARD, HOLD_T, SOUNDS, COMPLETE_AT } from "../../src/luch/greeting-timeline.ts";

const I = Number.POSITIVE_INFINITY;
assert.equal(GREET_END, G.end);
// about 4.4 s in spec §4; the controller allowed up to 5.5 s for an unhurried ignition (R12)
assert.ok(G.end > 4.2 && G.end <= 5.5, "4.2–5.5 s");

// three strikes at the spec beats, the third catches
assert.deepEqual(BURSTS.map((b) => b.t0), [...G.strikes]);
// flicked by hand: chk … chk-chk — the intervals are uneven, the last pair quick
{
  const [a, b, c] = G.strikes;
  assert.ok(a >= G.open1 && b - a > 2 * (c - b) && c - b >= 0.1, "uneven strikes");
}
assert.ok(BURSTS[2].n > BURSTS[0].n);

// no Луч before the catch: the third strike's sparks fly home to the ignition point first
assert.equal(greetFrame(0.2, I).luch, null);
assert.equal(greetFrame(1.0, I).luch, null);
assert.equal(greetFrame(G.catchAt - 0.01, I).luch, null);
assert.ok(G.catchAt > BURSTS[2].t0, "it catches after the third strike");
const homing = burstSparks(BURSTS[2]).filter((s) => s.home);
assert.ok(homing.length >= 3, "some sparks fly home");
assert.ok(homing.every((s) => s.home.t1 <= G.catchAt + 1e-9) && homing.some((s) => Math.abs(s.home.t1 - G.catchAt) < 1e-9), "the last one lands at the catch");
for (const s of homing) {
  const end = sparkAt(s, s.home.t1 - 1e-6);
  assert.ok(Math.hypot(end.x - IGNITION.x, end.y - IGNITION.y) < 0.5, "a homing spark ends at the ignition point");
  // it decelerates into the point: the last 40 ms cover less ground than the 40 ms before
  const p = (d) => sparkAt(s, s.home.t1 - d);
  const a = p(0.08), b = p(0.04), e = p(1e-6);
  assert.ok(Math.hypot(e.x - b.x, e.y - b.y) < Math.hypot(b.x - a.x, b.y - a.y), "it slows as it lands");
}
// the strikes and the ignition happen where Луч will stand: born in place, no flight from elsewhere
assert.ok(Math.abs(FLINT.x - REST.x) < 1e-9 && Math.abs(FLINT.y - (REST.y + 0.43 * REST.u / 2)) < 1e-9);

// born in place at full size: ignition starts at the catch and is the only thing that grows
const c0 = greetFrame(G.catchAt, I).luch;
assert.ok(c0 && c0.ignite < 0.01, "ignition starts at the catch");
for (let t = G.catchAt; t <= G.ignite1 + 0.3; t += 0.02) {
  const l = greetFrame(t, I).luch;
  assert.equal(l.x, REST.x); assert.equal(l.u, REST.u);
  assert.ok(Math.abs(l.y - REST.y) < 1e-9, `no rise at ${t}`);
  assert.ok(Math.abs(l.sx - 1) < 0.08 && Math.abs(l.sy - 1) < 0.08, `squash only as a reaction at ${t}`);
}
// the ignition is slow and uneven: it takes over a second, nearly dies once, falls back after surges
assert.ok(G.ignite1 - G.catchAt >= 1.2 && G.ignite1 - G.catchAt <= 1.8, "ignition 1.2–1.8 s");
{
  // a dip: it falls 0.03 below the highest it has been, and later climbs past that again
  let top = 0, dips = 0, inDip = false;
  for (let t = G.catchAt; t <= G.ignite1; t += 0.005) {
    const ig = igniteAt(t);
    assert.ok(ig >= 0 && ig <= 1, `ignite in range (${t})`);
    if (ig > top) { inDip = false; top = ig; } else if (!inDip && ig < top - 0.03) { dips++; inDip = true; }
  }
  assert.ok(dips >= 3, `it falls back at least three times (${dips})`);
}
// it nearly dies once before it takes
assert.ok(igniteAt(G.catchAt + 0.45) < igniteAt(G.catchAt + 0.35) - 0.04, "the struggle");
assert.ok(igniteAt(G.caught) < 0.25, "still small when it takes");
assert.equal(igniteAt(G.ignite1), 1); assert.equal(igniteAt(G.ignite1 + 0.5), 1);
// each surge swells the glow and lifts the flame a little
assert.ok(surgeAt(G.catchAt + 0.72) > 0.5 && greetFrame(G.catchAt + 0.72, I).luch.sy > 1, "a surge");
assert.equal(greetFrame(G.ignite1, I).luch.ignite, 1, "fully alight at G.ignite1");
assert.ok(G.ignite1 < G.eyes, "full ignition before the eyes open");
// the face comes only near the end of the ignition
assert.equal(greetFrame(G.catchAt + 0.3, I).luch.coreA, 0);
assert.ok(greetFrame(G.eyes, I).luch.coreA > 0.99);
// the tongue grows in pulses with the surges and is exactly its height once settled, before the eyes
assert.ok(greetFrame(G.catchAt + 0.72, I).luch.tall > greetFrame(G.catchAt + 0.9, I).luch.tall, "a pulse");
assert.equal(greetFrame(G.eyes, I).luch.tall, 1);
// the eyes wake slowly: half open, shut again, then open
{
  const lid = (d) => greetFrame(G.eyes + d, I).luch.lid;
  assert.ok(lid(0.18) > 0.3 && lid(0.18) < 0.6, "half open");
  assert.ok(lid(0.32) < 0.15, "shut again");
  assert.ok(lid(0.6) > 0.95, "open");
  assert.ok(Math.min(...[0.5, 0.55, 0.6, 0.65].map((d) => greetFrame(G.eyes + d, I).luch.look)) < -0.5, "a look around");
}
// overlapping action: the hands come out while the eyes are still finishing their waking
assert.ok(G.wave0 < G.eyes + 0.5, "the wave starts before the eyes are fully open");

// the second strike's ember glows 0.2 s and dies
assert.ok(greetFrame(BURSTS[1].t0 + 0.2, I).ember, "ember glows");
assert.equal(greetFrame(BURSTS[1].t0 + 0.4, I).ember, null, "ember is gone");

// eyes closed until the fire has fully taken, then they open
assert.equal(greetFrame(G.ignite1, I).luch.eye, "closed");
assert.equal(greetFrame(G.eyes - 0.01, I).luch.eye, "closed");
assert.equal(greetFrame(G.eyes + 0.12, I).luch.eye, "open");

// the wave: hands out, the right hand waving, then tucked back
const w = greetFrame((G.wave0 + G.wave1) / 2, I).luch;
assert.equal(w.handsOut, 1); assert.ok(w.waveT > 0); assert.equal(w.eye, "happy");
assert.equal(greetFrame(G.tuck1 + 0.05, I).luch.handsOut, 0);
// tuck: 0.3 s (spec §4); hands stay out until tuck0, are away at tuck1; the hop flows out of the wave
assert.ok(Math.abs(G.tuck1 - G.tuck0 - 0.3) < 1e-9 && G.tuck0 - G.wave1 < 0.1);
assert.equal(greetFrame(G.tuck0, I).luch.handsOut, 1);
{ const h = greetFrame((G.tuck0 + G.tuck1) / 2, I).luch.handsOut; assert.ok(h > 0 && h < 1); }
assert.equal(greetFrame(G.tuck1, I).luch.handsOut, 0);

// it leaves for the compact island
const end = greetFrame(G.end, I).luch;
assert.ok(Math.abs(end.x - small().x) < 1 && Math.abs(end.y - small().y) < 1 && Math.abs(end.u - small().u) < 1);
assert.ok(greetFrame(G.end, I).minis > 0.99);
// the colours turn to the compact island's on the way there, and only then
assert.equal(greetFrame(G.leave0, I).toCompact, 0);
{ const k = greetFrame((G.leave0 + G.end) / 2, I).toCompact; assert.ok(k > 0 && k < 1); }
assert.equal(greetFrame(G.end, I).toCompact, 1);
assert.equal(greetFrame(G.end + 1, I).toCompact, 1);
// SMALL is where the island draws its compact Луч (botPosition("compact"): a canvas of diameter / 0.6,
// u = LUCH_BOX 1.5 × canvas, flame at cy + 0.06 R + 0.05 u), so the island takes over from that frame
{
  const p = botPosition("compact", "sessions", NOTCH_H), W = Math.round(p.diameter / 0.6), u = W * 1.5;
  assert.ok(Math.abs(small().x - (320 - COMPACT_W / 2 + p.cx)) < 1e-9);
  assert.ok(Math.abs(small().y - (p.cy + 0.06 * W * 0.3 + 0.05 * u)) < 1e-9 && Math.abs(small().u - u) < 1e-9);
}

// interrupted at any moment: collapses from where it is to SMALL within G.COLLAPSE, no hands, no sparks after
for (const tc of [0.5, 1.3, 1.7, 2.4, 3.0, 4.0]) {
  const f = greetFrame(tc + G.COLLAPSE + 0.01, tc);
  assert.equal(f.card, 0); assert.equal(f.header, 0);
  if (f.luch) {
    assert.ok(Math.abs(f.luch.x - small().x) < 1 && Math.abs(f.luch.u - small().u) < 1, `tc ${tc}`);
    assert.equal(f.luch.handsOut, 0);
    assert.equal(f.luch.ignite, 1, `fully alight after the collapse (tc ${tc})`);
  }
  assert.equal(f.bursts.length, 0, `no strikes after the collapse (tc ${tc})`);
  assert.equal(f.sparks.length, 0, `no sparks after the collapse (tc ${tc})`);
  assert.equal(f.ember, null, `no ember after the collapse (tc ${tc})`);
}
// an interruption mid-ignition leaves a whole Луч: alight, full tongue, face on
for (const tc of [1.55, 1.9, 2.5, 2.95]) {
  const f = greetFrame(tc + G.COLLAPSE + 0.01, tc);
  assert.ok(f.luch.ignite === 1 && f.luch.tall === 1 && f.luch.coreA === 1, `tc ${tc}`);
}
// the minis never drop across a collapse and are in by its end
{
  const tc = G.end - 0.1;
  let prev = greetFrame(tc, tc).minis;
  for (let t = tc; t <= tc + G.COLLAPSE + 1e-9; t += 0.01) {
    const m = greetFrame(t, tc).minis;
    assert.ok(m >= prev - 1e-12, `minis dropped at ${t}`);
    prev = m;
  }
  assert.ok(greetFrame(tc + G.COLLAPSE, tc).minis >= 0.99);
}
// strike times live once: bursts follow G.strikes
assert.deepEqual(BURSTS.map((b) => b.t0), [...G.strikes]);

// sparks: start at the flint, rise, fall under gravity, fade out
const pts0 = sparkPoints(BURSTS[0], BURSTS[0].t0 + 0.02);
assert.ok(pts0.length >= 6);
for (const p of pts0) assert.ok(Math.hypot(p.x - FLINT.x, p.y - FLINT.y) < 20);
const later = sparkPoints(BURSTS[0], BURSTS[0].t0 + 0.2);
assert.ok(later.some((p) => p.y < FLINT.y - 10), "they fly up");
assert.equal(sparkPoints(BURSTS[0], BURSTS[0].t0 + 1.2).length, 0, "all gone");
for (const p of later) assert.ok(p.a >= 0 && p.a <= 1 && p.heat >= 0 && p.heat <= 1);
// the cursor holds the greeting (spec §4): it rests on the card after the hop instead of leaving,
// alive (the fire's clock runs on), until the mouse leaves; then it collapses from there
{
  const rest = (f) => f.luch && Math.abs(f.luch.x - REST.x) < 1e-9 && Math.abs(f.luch.u - REST.u) < 1e-9 && Math.abs(f.luch.y - REST.y) < 1e-9;
  const h = 3.0;
  assert.ok(HOLD_T > G.tuck1 && HOLD_T > G.leave0);
  for (const t of [h, 4.0, 5.0, 6, 9]) {
    const f = greetFrame(t, I, h);
    assert.ok(f.card > 0.99 && Math.abs(f.cardRect.w - CARD.w) < 1 && f.minis === 0, `held: the card stays at ${t}`);
    assert.equal(f.clock, t, "the fire's clock runs on");
  }
  for (const t of [5.0, 6, 9]) {
    const f = greetFrame(t, I, h);
    assert.ok(rest(f), `held: resting on the card, not leaving, at ${t}`);
    assert.equal(f.luch.ignite, 1); assert.equal(f.luch.handsOut, 0); assert.equal(f.luch.coreA, 1);
  }
  // the pose rests (only the blink, breath and glance move) while the clock runs
  { const a = greetFrame(6, I, h).luch, b = greetFrame(9, I, h).luch; assert.ok(a.x === b.x && a.y === b.y && a.u === b.u); }
  // before the hold it is the plain run, and the held pose matches it until the leave would start
  for (const t of [1.0, 2.5, 3.5, 4.2, 4.8]) {
    const a = greetFrame(t, I, h), b = greetFrame(t, I);
    assert.deepEqual(a.luch, b.luch, `held = plain before the leave (${t})`);
  }
  // still alive while held: it blinks and breathes
  const lids = [], sys = [];
  for (let t = 6; t < 10; t += 0.02) { const l = greetFrame(t, I, h).luch; lids.push(l.lid); sys.push(l.sy); }
  assert.ok(Math.min(...lids) < 0.3 && Math.max(...lids) === 1, "it blinks while held");
  assert.ok(Math.max(...sys) - Math.min(...sys) > 0.01, "it breathes while held");
  // held while already leaving: it comes back to rest on the card
  const late = greetFrame(G.leave0 + 0.3 + 0.5, I, G.leave0 + 0.3);
  assert.ok(rest(late) && late.card > 0.99, "held mid-leave: back on the card");
  assert.ok(!rest(greetFrame(G.leave0 + 0.35, I, G.leave0 + 0.3)), "it flies back, no jump");
  // the mouse leaves at 8 s: it collapses from the held pose to SMALL, card gone, fully alight
  const c = greetFrame(8 + G.COLLAPSE + 0.01, 8, h);
  assert.ok(Math.abs(c.luch.x - small().x) < 1 && Math.abs(c.luch.u - small().u) < 1 && c.card === 0 && c.minis > 0.99);
  assert.equal(greetFrame(7, I, h).toCompact, 0, "held on the card: still the idle fire");
  assert.equal(c.toCompact, 1, "collapsed: in the compact island's colours");
  assert.ok(rest(greetFrame(8, 8, h)), "the collapse starts from the held pose");
}

// the sounds (spec §4): a tick per strike, pop when the flame takes, greet with the wave; then done
assert.deepEqual(SOUNDS.map((s) => [s.t, s.name]), [...G.strikes.map((t) => [t, "tick"]), [G.caught, "pop"], [G.wave0, "greet"]]);
assert.ok(SOUNDS.every((s, i) => i === 0 || s.t >= SOUNDS[i - 1].t), "in order");
assert.equal(COMPLETE_AT, G.end + 0.05);
console.log("luch-greeting: all passed");
