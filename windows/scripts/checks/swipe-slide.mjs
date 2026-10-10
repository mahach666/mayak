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
globalThis.getComputedStyle = (el) => ({ overflowX: el.overflowX ?? "visible" });
/** What the page has under the cursor (document.elementFromPoint), set by each check. */
let under = null;
globalThis.document = { elementFromPoint: () => under };

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

function fakeEl(parentElement = null, extra = {}) {
  const classes = new Set();
  const el = {
    style: { transition: "", opacity: "", transform: "" },
    classList: { add: (c) => classes.add(c), remove: (c) => classes.delete(c), has: (c) => classes.has(c) },
    parentElement,
    clientWidth: 300,
    offsetWidth: 300,
    scrollWidth: 300,
    scrollLeft: 0,
    contains(other) {
      for (let n = other; n; n = n.parentElement) if (n === el) return true;
      return false;
    },
    ...extra,
  };
  return el;
}

/** A strip inside `view` that can still scroll sideways both ways. */
const sideScroller = (view) => fakeEl(view, { overflowX: "auto", scrollWidth: 900, scrollLeft: 200 });

function setup(start = "sessions") {
  now = 1000;
  timers = [];
  under = null;
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

// Two swipes in a row with the cursor still. Chromium hands the second gesture's events to the
// element the first one started on (here a sideways scroller in the page that slid away): what
// decides is what is under the cursor in the page on screen, so both gestures switch.
{
  const { slide, host, els } = setup("sessions");
  const stale = sideScroller(els.get("sessions"));
  const fresh = fakeEl(els.get("overview"));
  under = fakeEl(els.get("sessions"));
  drag(slide, 10, 15);
  advance(now + SWIPE.QUIET_MS + SLIDE_MS + 50);
  assert.deepEqual(host.switched, ["overview"]);
  under = fresh;
  for (let i = 0; i < 10; i++) {
    slide.onWheel(wheel(15, 0, { target: stale }));
    advance(now + 16);
  }
  advance(now + SWIPE.QUIET_MS + SLIDE_MS + 50);
  assert.deepEqual(host.switched, ["overview", "stats"], "the second gesture switches too");
  assert.equal(styled(els), false);
  assert.equal(timers.length, 0);
}

// A sideways scroller under the cursor, in the page on screen, keeps its gesture.
{
  const { slide, host, els } = setup("sessions");
  under = fakeEl(sideScroller(els.get("sessions")));
  drag(slide, 10, 15);
  advance(now + SWIPE.QUIET_MS + 10);
  assert.deepEqual(host.switched, []);
  assert.equal(styled(els), false);
  assert.equal(timers.length, 0);
}

// The second swipe starts before the first one's momentum has gone quiet: a page each.
{
  const { slide, host, els } = setup("sessions");
  drag(slide, 10, 15); // switches at 120 px
  for (const dx of [12, 9, 6, 4, 2, 1]) {
    slide.onWheel(wheel(dx));
    advance(now + 16);
  }
  assert.deepEqual(host.switched, ["overview"], "the momentum is held");
  for (const dx of [2, 6, 12, 18, 22, 24, 24, 24]) {
    slide.onWheel(wheel(dx));
    advance(now + 16);
  }
  assert.deepEqual(host.switched, ["overview", "stats"], "the new stroke switches once more");
  advance(now + SWIPE.QUIET_MS + SLIDE_MS + 50);
  assert.deepEqual(host.switched, ["overview", "stats"]);
  assert.equal(styled(els), false);
  assert.equal(timers.length, 0);
}

console.log("swipe-slide: ok");
