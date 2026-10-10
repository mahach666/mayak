// island/swipe-slide.ts: the swipe's DOM side (ideas §40), on a fake host, fake views and a fake
// clock. The page leaving under a gesture abandons it, the timers end with the gesture, the inline
// styles go, and reduced motion switches without a slide.
import assert from "node:assert/strict";

// A clock and timers that only move when the check says so.
let now = 0;
let timers = [];
let nextId = 1;
let reduced = false;
globalThis.window = {
  setTimeout(fn, ms) {
    const id = nextId++;
    timers.push({ id, at: now + ms, fn });
    return id;
  },
  clearTimeout(id) {
    timers = timers.filter((t) => t.id !== id);
  },
  matchMedia: () => ({ matches: reduced }),
};
Object.defineProperty(globalThis, "performance", { value: { now: () => now }, configurable: true });
globalThis.Element = class {};
globalThis.getComputedStyle = () => ({ overflowX: "visible" });

/** Moves the clock to `to`, running every timer due on the way, in order. */
function advance(to) {
  for (;;) {
    timers.sort((a, b) => a.at - b.at);
    const t = timers[0];
    if (!t || t.at > to) break;
    timers.shift();
    now = t.at;
    t.fn();
  }
  now = to;
}

const { SwipeSlide, SLIDE_MS } = await import("../../src/island/swipe-slide.ts");
const { SWIPE } = await import("../../src/core/page-swipe.ts");

function fakeEl() {
  const classes = new Set();
  return {
    style: { transition: "", opacity: "", transform: "" },
    classList: { add: (c) => classes.add(c), remove: (c) => classes.delete(c), has: (c) => classes.has(c) },
    clientWidth: 300,
    offsetWidth: 300,
  };
}

function setup(start = "sessions") {
  now = 1000;
  timers = [];
  const els = new Map(["sessions", "overview", "stats", "focus", "apps"].map((p) => [p, fakeEl()]));
  const host = {
    current: start,
    switched: [],
    page: () => host.current,
    viewEl: (p) => els.get(p) ?? null,
    prepare: () => {},
    switchTo: (p) => {
      host.switched.push(p);
      host.current = p;
    },
  };
  const slide = new SwipeSlide(fakeEl(), host);
  return { slide, host, els };
}

const wheel = (dx, dy = 0, extra = {}) => ({
  deltaX: dx,
  deltaY: dy,
  deltaMode: 0,
  timeStamp: now,
  ctrlKey: false,
  shiftKey: false,
  buttons: 0,
  target: null,
  preventDefault() {},
  ...extra,
});

/** `n` touchpad events of `dx`, 16 ms apart. */
function drag(slide, n, dx) {
  for (let i = 0; i < n; i++) {
    slide.onWheel(wheel(dx));
    advance(now + 16);
  }
}

const styled = (els) => [...els.values()].some((e) => e.style.transform !== "" || e.style.opacity !== "");

// A drag between RELEASE_PX and COMMIT_PX, the page unchanged: the quiet timer switches it.
{
  const { slide, host, els } = setup();
  drag(slide, 12, 5); // 60 px
  assert.ok(styled(els), "the page follows the drag");
  assert.deepEqual(host.switched, [], "no switch before the release");
  advance(now + SWIPE.QUIET_MS + 10);
  assert.deepEqual(host.switched, ["overview"], "released past RELEASE_PX: one page on");
  advance(now + SLIDE_MS + 50);
  assert.equal(styled(els), false, "the slide over, the views are back to the stylesheet");
  assert.equal(timers.length, 0, "no timer left once the gesture is over");
}

// The same drag, then a card comes up before the deltas go quiet: no switch, the gesture goes.
{
  const { slide, host, els } = setup();
  drag(slide, 12, 5);
  host.current = null;
  advance(now + SWIPE.QUIET_MS + 10);
  assert.deepEqual(host.switched, [], "a card on screen is never replaced by a header page");
  assert.equal(styled(els), false, "the inline styles go with the gesture");
  assert.equal(timers.length, 0, "no timer left");
}

// The same drag, then a header tab clicked: the click stays, the gesture does not override it.
{
  const { slide, host, els } = setup();
  drag(slide, 12, 5);
  host.current = "stats";
  advance(now + SWIPE.QUIET_MS + 10);
  assert.deepEqual(host.switched, [], "the clicked tab is kept");
  assert.equal(els.get("stats").classList.has("on"), false, "the gesture does not touch the views' classes");
  assert.equal(styled(els), false);
  assert.equal(timers.length, 0);
}

// A short drag (below RELEASE_PX) slides back, then everything is cleared.
{
  const { slide, host, els } = setup();
  drag(slide, 5, 4); // 20 px
  advance(now + SWIPE.QUIET_MS + 10);
  assert.deepEqual(host.switched, []);
  advance(now + SLIDE_MS + 50);
  assert.equal(styled(els), false);
  assert.equal(timers.length, 0);
}

// A wheel tilted left, reduced motion: the switch is instant, no inline style is left.
{
  reduced = true;
  const { slide, host, els } = setup("sessions");
  slide.onWheel(wheel(-1, 0, { deltaMode: 1 }));
  assert.deepEqual(host.switched, ["apps"], "← wraps to the last page");
  assert.equal(styled(els), false, "no slide under reduced motion");
  advance(now + SWIPE.QUIET_MS + 10);
  assert.equal(timers.length, 0);
  reduced = false;
}

// A held button (a tile drag, the seek bar) takes no part, and abandons a gesture under way.
{
  const { slide, host, els } = setup();
  drag(slide, 6, 5);
  slide.onWheel(wheel(5, 0, { buttons: 1 }));
  assert.equal(styled(els), false);
  assert.equal(timers.length, 0);
  advance(now + SWIPE.QUIET_MS + 10);
  assert.deepEqual(host.switched, []);
}

console.log("swipe-slide: ok");
