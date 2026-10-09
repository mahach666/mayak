import assert from "node:assert/strict";
import { G, GREET_END, BURSTS, sparkPoints, greetFrame, REST, SMALL, FLINT } from "../../src/luch/greeting-timeline.ts";

const I = Number.POSITIVE_INFINITY;
assert.equal(GREET_END, G.end);
assert.ok(Math.abs(G.end - 4.4) < 0.25, "about 4.4 s (spec §4)");

// three strikes at the spec beats, the third catches
assert.deepEqual(BURSTS.map((b) => b.t0), [0.45, 0.85, 1.25]);
assert.ok(BURSTS[2].n > BURSTS[0].n);

// no Луч before the catch; a growing ember right after it
assert.equal(greetFrame(0.2, I).luch, null);
assert.equal(greetFrame(1.0, I).luch, null);
const c = greetFrame(1.4, I);
assert.ok(c.luch && c.luch.grow > 0 && c.luch.grow < 0.5, "a small flame after the third strike");
assert.ok(c.luch.y > REST.y, "it starts low, near the flint");

// it rises, then lands at REST with a squash
const land = greetFrame(G.land + 0.04, I).luch;
assert.ok(Math.abs(land.y - REST.y) < 4 && land.grow > 0.95);
assert.ok(land.sy < 1 && land.sx > 1, "squash on landing");
assert.equal(greetFrame(G.land + 0.6, I).luch.sy.toFixed(2), "1.00");

// eyes closed until they open after landing
assert.equal(greetFrame(G.land - 0.1, I).luch.eye, "closed");
assert.notEqual(greetFrame(G.land + 0.3, I).luch.eye, "closed");

// the wave: hands out, the right hand waving, then tucked back
const w = greetFrame((G.wave0 + G.wave1) / 2, I).luch;
assert.equal(w.handsOut, 1); assert.ok(w.waveT > 0); assert.equal(w.eye, "happy");
assert.equal(greetFrame(G.tuck1 + 0.05, I).luch.handsOut, 0);

// it leaves for the compact island
const end = greetFrame(G.end, I).luch;
assert.ok(Math.abs(end.x - SMALL.x) < 1 && Math.abs(end.y - SMALL.y) < 1 && Math.abs(end.u - SMALL.u) < 1);
assert.ok(greetFrame(G.end, I).minis > 0.99);

// interrupted at any moment: collapses from where it is to SMALL within G.COLLAPSE, no hands, no sparks after
for (const tc of [0.5, 1.3, 3.0]) {
  const f = greetFrame(tc + G.COLLAPSE + 0.01, tc);
  assert.equal(f.card, 0); assert.equal(f.header, 0);
  if (f.luch) {
    assert.ok(Math.abs(f.luch.x - SMALL.x) < 1 && Math.abs(f.luch.u - SMALL.u) < 1, `tc ${tc}`);
    assert.equal(f.luch.handsOut, 0);
  }
  assert.equal(f.bursts.length, 0, `no sparks after the collapse (tc ${tc})`);
}

// sparks: start at the flint, rise, fall under gravity, fade out
const pts0 = sparkPoints(BURSTS[0], BURSTS[0].t0 + 0.02);
assert.ok(pts0.length >= 6);
for (const p of pts0) assert.ok(Math.hypot(p.x - FLINT.x, p.y - FLINT.y) < 20);
const later = sparkPoints(BURSTS[0], BURSTS[0].t0 + 0.2);
assert.ok(later.some((p) => p.y < FLINT.y - 10), "they fly up");
assert.equal(sparkPoints(BURSTS[0], BURSTS[0].t0 + 1.2).length, 0, "all gone");
for (const p of later) assert.ok(p.a >= 0 && p.a <= 1 && p.heat >= 0 && p.heat <= 1);
console.log("luch-greeting: all passed");
