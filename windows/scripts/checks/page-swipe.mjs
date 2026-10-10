// core/page-swipe.ts: the header pages by a horizontal swipe (ideas §40), on synthetic wheel streams.
// The real recordings are replayed in swipe-real.mjs.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { PageSwipe, SWIPE, rubberBand, slide } from "../../src/core/page-swipe.ts";

const W = 640;
const make = () => {
  const s = new PageSwipe();
  s.width = W;
  return s;
};

/** Feeds `deltas` ([dx, dy] or [dx, dy, mode]) every `step` ms from `t0`; returns the steps and the last time. */
function feed(swipe, deltas, t0 = 1000, step = 8) {
  const steps = [];
  let t = t0;
  for (const [dx, dy, mode = 0] of deltas) {
    const end = swipe.quiet(t);
    if (end) steps.push(end);
    steps.push(swipe.wheel({ deltaX: dx, deltaY: dy, deltaMode: mode, timeStamp: t }));
    t += step;
  }
  return { steps, t: t - step };
}
const kinds = (steps) => steps.map((s) => s.kind);
const commits = (steps) => steps.filter((s) => s.kind === "commit");
const repeat = (n, d) => Array.from({ length: n }, () => d);
/** A flick: fast fingers for ~70 ms. */
const flick = (dir) => [4, 14, 24, 40, 50, 55, 45, 30].map((d) => [dir * d, 0]);
/** The momentum after a flick: 3.5 s of decaying deltas every 8 ms, down to ±1 px. */
const momentum = (dir) => Array.from({ length: 440 }, (_, i) => [dir * Math.max(1, Math.round(28 * Math.exp(-i / 60))), 0]);

// A slow stroke: the page follows it 1:1 for as long as it moves, however far, with no switch on
// the way; it is decided when the deltas stop: past RELEASE of the width, the next page.
{
  const s = make();
  const { steps, t } = feed(s, repeat(70, [4, 0]), 1000, 16); // 280 px at 0.25 px/ms
  assert.equal(commits(steps).length, 0, "no switch while the stroke moves");
  const drags = steps.filter((x) => x.kind === "drag").map((x) => x.offset);
  assert.ok(drags.length >= 65);
  for (let i = 1; i < drags.length; i++) assert.equal(drags[i] - drags[i - 1], 4, "1:1");
  assert.equal(drags.at(-1), 280);
  assert.equal(s.quiet(t + SWIPE.QUIET_MS - 1), null, "not quiet yet");
  assert.deepEqual(s.quiet(t + SWIPE.QUIET_MS), { kind: "commit", delta: 1, offset: 280, speed: 0 });
  assert.equal(s.active, false);
  assert.equal(s.quietAt, null);
}

// The same stroke shorter (150 px < 35 % of 640): back. To the left the same, mirrored.
{
  const s = make();
  const { t } = feed(s, repeat(30, [-5, 0]), 1000, 16);
  assert.deepEqual(s.quiet(t + SWIPE.QUIET_MS), { kind: "cancel", offset: -150, speed: 0 });
}

// A slow stroke never became fast: it leaves no momentum to hold. However slowly or unevenly it
// slows down, the page follows it 1:1, and it is decided only once the deltas stop.
{
  const s = make();
  const slow = repeat(40, [6, 0]); // 240 px at 0.375 px/ms
  const stop = [4, 3, 2, 1, 1, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 1].map((d) => [d, 0]);
  const { steps, t } = feed(s, [...slow, ...stop, ...repeat(60, [1, 0])], 1000, 16);
  assert.ok(steps.every((x) => x.kind === "pass" || x.kind === "drag"), kinds(steps).join(","));
  assert.equal(steps.at(-1).offset, 240 + 15 + 60);
  assert.deepEqual(s.quiet(t + SWIPE.QUIET_MS), { kind: "commit", delta: 1, offset: 315, speed: 0 });
}

/**
 * A slow drag, fingers on the touchpad until the end: every event after the first LOCK_PX moves
 * the page by its own delta (1:1, nothing held, nothing decided) and the stroke is decided at quiet.
 * `events` are [t, dx]; returns the quiet step.
 */
function followsToTheEnd(events, label) {
  const s = make();
  let x = 0;
  let locked = false;
  for (const [t, dx] of events) {
    assert.equal(s.quiet(t), null, `${label}: no gesture end at ${t}`);
    const step = s.wheel({ deltaX: dx, deltaY: 0, deltaMode: 0, timeStamp: t });
    x += dx;
    if (step.kind === "pass" && !locked) continue;
    locked = true;
    assert.equal(step.kind, "drag", `${label}: ${step.kind} at ${t} ms, ${x} px in`);
    assert.equal(step.offset, rubberBand(x, W), `${label}: 1:1 at ${t} ms`);
  }
  const end = s.quiet(events.at(-1)[0] + SWIPE.QUIET_MS);
  assert.ok(end && end.offset === rubberBand(x, W), `${label}: decided at quiet from where the page is`);
  return end;
}

// A slow drag with a pause of 60–120 ms, the fingers kept down (no event, shorter than QUIET_MS).
for (const pause of [60, 90, 120]) {
  const ev = [];
  let t = 1000;
  for (let i = 0; i < 160; i++) {
    t += i === 70 ? pause : 8;
    ev.push([t, 2 + (i % 2)]);
  }
  assert.equal(followsToTheEnd(ev, `pause ${pause}`).kind, "commit");
}

// A slow drag that starts faster than it goes on (one 3 px event, then 1–2 px every 8–24 ms).
{
  const ev = [[1000, 3]];
  let t = 1000;
  const gaps = [8, 16, 24, 8, 16];
  for (let i = 0; i < 220; i++) {
    t += gaps[i % gaps.length];
    ev.push([t, 1 + (i % 3 === 0 ? 1 : 0)]);
  }
  assert.equal(followsToTheEnd(ev, "slowing start").kind, "commit");
}

// 1 px ± 1 every 8 ms: short of RELEASE, back, but only once the fingers stop.
{
  const ev = Array.from({ length: 120 }, (_, i) => [1000 + 8 * i, [1, 0, 2, 1, 1, 2, 0][i % 7]]);
  assert.equal(followsToTheEnd(ev, "1 ± 1").kind, "cancel");
}

// The real slow short swipe (touchpad-labeled, series C, 12.0–12.7 s) made twice as slowly: it
// still follows the fingers to the end and goes back.
{
  const rec = JSON.parse(readFileSync(new URL("./fixtures/touchpad-labeled.json", import.meta.url), "utf8"));
  const t0 = rec[0][0];
  const c = rec.filter(([t]) => t - t0 >= 11_900 && t - t0 <= 12_800);
  const stretched = c.map(([t, dx]) => [1000 + 2 * (t - c[0][0]), dx]);
  assert.equal(followsToTheEnd(stretched, "series C x2").kind, "cancel");
}

// A slow stroke whose fingers turn back: it ends there, sliding back.
{
  const s = make();
  const { steps } = feed(s, [...repeat(30, [4, 0]), ...repeat(10, [-3, 0])], 1000, 16);
  const end = steps.findIndex((x) => x.kind === "cancel");
  assert.ok(end > 30, kinds(steps).join(","));
  assert.ok(steps.slice(0, end).every((x) => x.kind === "pass" || x.kind === "drag"));
}

// Far past the width: the page goes on, slower and slower, never more than OVERSHOOT_PX further.
{
  assert.equal(rubberBand(300, W), 300);
  assert.equal(rubberBand(-W, W), -W);
  assert.ok(rubberBand(W + 10, W) > W && rubberBand(W + 10, W) < W + 10);
  assert.ok(rubberBand(-5000, W) > -W - SWIPE.OVERSHOOT_PX);
  const s = make();
  const { steps } = feed(s, repeat(220, [4, 0]), 1000, 16);
  const drags = steps.filter((x) => x.kind === "drag").map((x) => x.offset);
  for (let i = 1; i < drags.length; i++) assert.ok(drags[i] > drags[i - 1]);
  assert.ok(drags.at(-1) > W && drags.at(-1) < W + SWIPE.OVERSHOOT_PX);
}

// The finger turns back: the page follows it back, and the stroke ends there, short or not.
{
  const s = make();
  const { steps } = feed(s, [...repeat(60, [5, 0]), ...repeat(20, [-5, 0])], 1000, 16);
  const drags = steps.filter((x) => x.kind === "drag");
  assert.equal(Math.max(...drags.map((x) => x.offset)), 300);
  assert.equal(commits(steps).length, 0, kinds(steps).join(","));
  const cancel = steps.find((x) => x.kind === "cancel");
  assert.ok(cancel && cancel.offset > 0 && cancel.offset < 300);
}

// A flick: the page follows, then switches once SWITCH_PX is reached, sliding on at the flick's
// speed; its 3.5 s of momentum is held, every event of it.
for (const dir of [1, -1]) {
  const s = make();
  const { steps, t } = feed(s, [...flick(dir), ...momentum(dir)]);
  assert.deepEqual(kinds(steps.slice(0, 1)), ["pass"], "undecided below LOCK_PX goes on to the page");
  assert.ok(steps.some((x) => x.kind === "drag"), "the page follows the flick");
  const c = commits(steps);
  assert.equal(c.length, 1, "one stroke, one page");
  assert.equal(c[0].delta, dir);
  assert.ok(c[0].offset * dir >= SWIPE.SWITCH_PX && c[0].offset * dir < 200);
  assert.ok(c[0].speed > SWIPE.VMIN, "the slide starts at the flick's speed");
  const after = steps.slice(steps.indexOf(c[0]) + 1);
  assert.ok(after.length > 400 && after.every((x) => x.kind === "hold"), "momentum held");
  assert.equal(s.quiet(t + SWIPE.QUIET_MS), null, "a gesture already decided ends with no step");
  assert.equal(s.active, false);
}

// Momentum whose events come coalesced (two or three frames in one) or stall: still held.
{
  const s = make();
  const a = feed(s, flick(1));
  assert.equal(commits(a.steps).length, 1);
  let t = a.t;
  for (const [dx, dt] of [[30, 8], [52, 16], [22, 8], [18, 8], [32, 24], [10, 8], [0, 16], [6, 8], [2, 8], [4, 16], [1, 120], [1, 8]]) {
    t += dt;
    assert.equal(s.wheel({ deltaX: dx, deltaY: 0, deltaMode: 0, timeStamp: t }).kind, "hold", `${dx}/${dt}`);
  }
}

// A slow stroke that turns into a flick: one page, not two.
{
  const s = make();
  const { steps } = feed(s, [...repeat(10, [3, 0]), ...[8, 16, 30, 45, 50, 50, 40, 30].map((d) => [d, 0]), ...momentum(1)]);
  assert.equal(commits(steps).length, 1, kinds(steps).join(","));
}

// Fingers back on the touchpad while the momentum still comes (no QUIET_MS gap, a second or two
// in): a new stroke, a second page, in either direction.
for (const dir of [1, -1]) {
  for (const at of [60, 150, 300]) {
    const s = make();
    const a = feed(s, [...flick(1), ...momentum(1).slice(0, at)]);
    assert.equal(commits(a.steps).length, 1);
    const again = feed(s, [...flick(dir), ...momentum(dir).slice(0, 100)], a.t + 8);
    const c = commits(again.steps);
    assert.equal(c.length, 1, `${dir} at ${at}: ${kinds(again.steps).join(",")}`);
    assert.equal(c[0].delta, dir);
    assert.ok(again.steps.some((x) => x.kind === "drag"), "the page follows the new stroke");
  }
}

// Mouse wheel tilted: discrete clicks (WebView2 reports pixels, ~100 per click). Each click
// switches one page at once; clicks of a held tilt closer than REFRACT_MS are the same stroke.
{
  const s = make();
  assert.deepEqual(s.wheel({ deltaX: 100, deltaY: 0, deltaMode: 0, timeStamp: 0 }), { kind: "commit", delta: 1, offset: 100, speed: 0 });
  assert.deepEqual(s.wheel({ deltaX: 100, deltaY: 0, deltaMode: 0, timeStamp: 90 }), { kind: "hold" }, "held tilt, one page");
  assert.deepEqual(s.wheel({ deltaX: 100, deltaY: 0, deltaMode: 0, timeStamp: 180 }), { kind: "hold" });
  assert.equal(s.quiet(380), null);
  assert.equal(s.wheel({ deltaX: -100, deltaY: 0, deltaMode: 0, timeStamp: 600 }).delta, -1);
  // Clicks 100 ms apart without a pause: each one REFRACT_MS after the last switch switches.
  const held = make();
  const switched = [];
  for (let t = 0; t <= 900; t += 100) {
    if (held.wheel({ deltaX: 100, deltaY: 0, deltaMode: 0, timeStamp: t }).kind === "commit") switched.push(t);
  }
  assert.deepEqual(switched, [0, 300, 600, 900]);
  // A click in lines (deltaMode 1), even a small one, is a wheel.
  const lines = make();
  assert.deepEqual(lines.wheel({ deltaX: -1, deltaY: 0, deltaMode: 1, timeStamp: 0 }), { kind: "commit", delta: -1, offset: -SWIPE.LINE_PX, speed: 0 });
  // A new gesture starts after QUIET_MS even when no quiet() call ended the old one.
  const late = make();
  late.wheel({ deltaX: 100, deltaY: 0, deltaMode: 0, timeStamp: 0 });
  assert.equal(late.wheel({ deltaX: 100, deltaY: 0, deltaMode: 0, timeStamp: SWIPE.QUIET_MS }).kind, "commit");
}

// Vertical scrolling of a list with a little sideways drift never swipes, for the whole gesture,
// even when the drift later adds up; its events all go on to the list.
{
  const s = make();
  const { steps } = feed(s, repeat(40, [0.6, 9]).concat(repeat(10, [3, 2])), 1000, 16);
  assert.ok(steps.every((x) => x.kind === "pass"), kinds(steps).join(","));
  const notch = make();
  assert.deepEqual(notch.wheel({ deltaX: 0, deltaY: 100, deltaMode: 0, timeStamp: 0 }), { kind: "pass" });
  assert.deepEqual(notch.wheel({ deltaX: 0, deltaY: 100, deltaMode: 0, timeStamp: 50 }), { kind: "pass" });
  assert.equal(notch.quiet(1000), null, "a scroll ends with no step");
  // A sideways flick during the scroll's momentum is a stroke of its own.
  const turn = make();
  const sc = feed(turn, [...[10, 30, 50, 40].map((d) => [0, d]), ...Array.from({ length: 80 }, (_, i) => [0, Math.max(1, Math.round(30 * Math.exp(-i / 40)))])]);
  assert.ok(sc.steps.every((x) => x.kind === "pass"));
  const sw = feed(turn, flick(-1), sc.t + 8);
  assert.deepEqual(commits(sw.steps).map((x) => x.delta), [-1]);
}

// A scroll made during a swipe's momentum goes on to the page; the momentum stays held.
{
  const s = make();
  const a = feed(s, [...flick(1), ...momentum(1).slice(0, 50)]);
  assert.equal(commits(a.steps).length, 1);
  const b = feed(s, [[1, 12], [2, 0], [0, 15]], a.t + 8);
  assert.deepEqual(kinds(b.steps), ["pass", "hold", "pass"]);
}

// A diagonal gesture (x not clearly dominant) is a scroll; a mostly sideways one is a swipe.
{
  const diagonal = make();
  assert.ok(feed(diagonal, repeat(20, [6, 5])).steps.every((x) => x.kind === "pass"));
  const steep = make();
  assert.ok(feed(steep, repeat(20, [6, 3.5])).steps.every((x) => x.kind === "pass"), "1.7 : 1 is not clearly horizontal");
  const flat = make();
  const f = feed(flat, repeat(20, [-24, 2]));
  assert.deepEqual(commits(f.steps).map((x) => x.delta), [-1]);
}

// No movement at all decides nothing; reset() forgets a drag without a step.
{
  const s = make();
  const z = feed(s, repeat(5, [0, 0]));
  assert.ok(z.steps.every((x) => x.kind === "pass"));
  feed(s, repeat(4, [6, 0]), z.t + 16, 16);
  assert.equal(s.active, true);
  s.reset();
  assert.equal(s.active, false);
  assert.equal(s.quiet(1e9), null);
}

// The slide on: it starts at the stroke's speed (the curve's first slope × distance / length),
// never shorter than SLIDE_MIN_MS nor longer than SLIDE_MAX_MS.
{
  const start = ({ ms, ease }, d) => (Number(ease.split(",")[1]) / 0.3) * (d / ms);
  for (const [d, v] of [[540, 2.5], [540, 4], [300, 3], [450, 6]]) {
    const s = slide(d, v);
    assert.ok(s.ms >= SWIPE.SLIDE_MIN_MS && s.ms <= SWIPE.SLIDE_MAX_MS, `${d}/${v}: ${s.ms}`);
    assert.ok(Math.abs(start(s, d) - v) < 0.05, `${d}/${v}: starts at ${start(s, d)}`);
    assert.match(s.ease, /^cubic-bezier\(0\.3, [\d.]+, 0\.3, 1\)$/);
  }
  assert.equal(slide(20, 0).ms, SWIPE.SLIDE_MIN_MS, "a short way back is quick");
  assert.equal(slide(600, 0).ms, SWIPE.SLIDE_MAX_MS);
  assert.ok(slide(540, 8).ms < slide(540, 3).ms, "a faster flick, a shorter slide");
}

console.log("page-swipe-check: all passed");
