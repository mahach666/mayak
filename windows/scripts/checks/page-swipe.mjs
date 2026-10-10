// core/page-swipe.ts: the header pages by a horizontal swipe (ideas §40), on synthetic wheel streams.
import assert from "node:assert/strict";
import { PageSwipe, SWIPE } from "../../src/core/page-swipe.ts";

/** Feeds `deltas` ([dx, dy] or [dx, dy, mode]) every `step` ms from `t0`; returns the steps and the last time. */
function feed(swipe, deltas, t0 = 1000, step = 16) {
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

// A touchpad swipe to the left (deltaX > 0): the page follows, then switches to the next one once,
// and the momentum after it is held until the deltas go quiet.
{
  const s = new PageSwipe();
  const swipe = [[1, 0], [2, 0.3], [4, 0], [6, 0.5], [9, 0], [12, 0], [14, 0], [16, 0], [18, 0], [20, 0], [22, 0]];
  const momentum = [[18, 0], [14, 0], [10, 0], [7, 0], [5, 0], [3, 0], [2, 0], [1, 0], [1, 0]];
  const { steps, t } = feed(s, [...swipe, ...momentum]);
  assert.deepEqual(kinds(steps.slice(0, 2)), ["pass", "pass"], "undecided below LOCK_PX goes on to the page");
  const drags = steps.filter((x) => x.kind === "drag").map((x) => x.offset);
  assert.ok(drags.length >= 4, "the page follows the finger");
  for (let i = 1; i < drags.length; i++) assert.ok(drags[i] > drags[i - 1], "offset grows with the finger");
  assert.ok(drags.every((o) => o > 0 && o < SWIPE.COMMIT_PX));
  const c = commits(steps);
  assert.equal(c.length, 1, "one gesture, one page");
  assert.equal(c[0].delta, 1);
  assert.ok(c[0].offset >= SWIPE.COMMIT_PX);
  const after = steps.slice(steps.indexOf(c[0]) + 1);
  assert.ok(after.length > 0 && after.every((x) => x.kind === "hold"), "momentum tail held");
  assert.equal(s.active, true);
  assert.equal(s.quietAt, t + SWIPE.QUIET_MS);
  assert.equal(s.quiet(t + SWIPE.QUIET_MS - 1), null, "not quiet yet");
  assert.equal(s.quiet(t + SWIPE.QUIET_MS), null, "a gesture that already switched ends with no step");
  assert.equal(s.active, false);
  assert.equal(s.quietAt, null);
  // The next gesture, after the quiet: to the right this time, the previous page.
  const back = feed(s, repeat(12, [-14, 0]), t + 400);
  assert.deepEqual(commits(back.steps).map((x) => x.delta), [-1]);
}

// A momentum tail with short gaps (< QUIET_MS) after a switch stays held: still one page.
{
  const s = new PageSwipe();
  const a = feed(s, repeat(10, [15, 0]));
  assert.equal(commits(a.steps).length, 1);
  const b = feed(s, repeat(6, [8, 0]), a.t + SWIPE.QUIET_MS - 20, SWIPE.QUIET_MS - 20);
  assert.deepEqual(kinds(b.steps), repeat(6, "hold"));
}

// A short swipe that lifts past RELEASE_PX switches when it goes quiet; shorter slides back.
{
  const s = new PageSwipe();
  const { steps, t } = feed(s, [[4, 0], [8, 0], [10, 0], [10, 0], [12, 0]]);
  assert.equal(commits(steps).length, 0);
  assert.equal(steps.at(-1).kind, "drag");
  assert.deepEqual(s.quiet(t + SWIPE.QUIET_MS), { kind: "commit", delta: 1, offset: 44 });

  const short = new PageSwipe();
  const r = feed(short, [[-3, 0], [-6, 0], [-8, 0], [-6, 0]]);
  assert.deepEqual(r.steps.at(-1), { kind: "drag", offset: -23 });
  assert.deepEqual(short.quiet(r.t + SWIPE.QUIET_MS), { kind: "cancel", offset: -23 });
  assert.equal(short.active, false);
}

// The finger turns back before lifting: the offset follows it and the gesture slides back.
{
  const s = new PageSwipe();
  const { steps, t } = feed(s, [...repeat(5, [10, 0]), ...repeat(5, [-9, 0])]);
  assert.equal(commits(steps).length, 0);
  assert.deepEqual(steps.at(-1), { kind: "drag", offset: 5 });
  assert.deepEqual(s.quiet(t + 500), { kind: "cancel", offset: 5 });
}

// Mouse wheel tilted: discrete clicks (WebView2 reports pixels, ~100 per click). The first click
// switches at once; clicks closer than QUIET_MS are the same gesture; a click after a pause is a new one.
{
  const s = new PageSwipe();
  const first = s.wheel({ deltaX: 100, deltaY: 0, deltaMode: 0, timeStamp: 0 });
  assert.deepEqual(first, { kind: "commit", delta: 1, offset: 100 });
  assert.deepEqual(s.wheel({ deltaX: 100, deltaY: 0, deltaMode: 0, timeStamp: 90 }), { kind: "hold" }, "held tilt, one page");
  assert.equal(s.quiet(290), null);
  const left = s.wheel({ deltaX: -100, deltaY: 0, deltaMode: 0, timeStamp: 600 });
  assert.deepEqual(left, { kind: "commit", delta: -1, offset: -100 });
  // A click in lines (deltaMode 1), even a small one, is a wheel.
  const lines = new PageSwipe();
  assert.deepEqual(lines.wheel({ deltaX: -1, deltaY: 0, deltaMode: 1, timeStamp: 0 }), { kind: "commit", delta: -1, offset: -SWIPE.LINE_PX });
  // A new gesture starts after QUIET_MS even when no quiet() call ended the old one.
  const late = new PageSwipe();
  late.wheel({ deltaX: 100, deltaY: 0, deltaMode: 0, timeStamp: 0 });
  assert.equal(late.wheel({ deltaX: 100, deltaY: 0, deltaMode: 0, timeStamp: SWIPE.QUIET_MS }).kind, "commit");
}

// Vertical scrolling of a list with a little sideways drift never swipes, for the whole gesture,
// even when the drift later adds up; its events all go on to the list.
{
  const s = new PageSwipe();
  const scroll = repeat(40, [0.6, 9]).concat(repeat(10, [3, 2]));
  const { steps } = feed(s, scroll);
  assert.ok(steps.every((x) => x.kind === "pass"), kinds(steps).join(","));
  // A wheel notch down (deltaY 100) and Shift-less wheel clicks: a scroll.
  const notch = new PageSwipe();
  assert.deepEqual(notch.wheel({ deltaX: 0, deltaY: 100, deltaMode: 0, timeStamp: 0 }), { kind: "pass" });
  assert.deepEqual(notch.wheel({ deltaX: 0, deltaY: 100, deltaMode: 0, timeStamp: 50 }), { kind: "pass" });
  assert.equal(notch.quiet(1000), null, "a scroll ends with no step");
}

// A diagonal gesture (x not clearly dominant) is a scroll; a mostly sideways one is a swipe.
{
  const diagonal = new PageSwipe();
  const d = feed(diagonal, repeat(20, [6, 5]));
  assert.ok(d.steps.every((x) => x.kind === "pass"));
  const steep = new PageSwipe();
  const e = feed(steep, repeat(20, [6, 3.5]));
  assert.ok(e.steps.every((x) => x.kind === "pass"), "1.7 : 1 is not clearly horizontal");
  const flat = new PageSwipe();
  const f = feed(flat, repeat(20, [-8, 2]));
  assert.equal(commits(f.steps).length, 1);
  assert.equal(commits(f.steps)[0].delta, -1);
}

// No movement at all decides nothing; reset() forgets a drag without a step.
{
  const s = new PageSwipe();
  const z = feed(s, repeat(5, [0, 0]));
  assert.ok(z.steps.every((x) => x.kind === "pass"));
  feed(s, repeat(4, [6, 0]), z.t + 16);
  assert.equal(s.active, true);
  s.reset();
  assert.equal(s.active, false);
  assert.equal(s.quiet(1e9), null);
}

console.log("page-swipe-check: all passed");
