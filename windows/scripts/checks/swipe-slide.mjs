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

const { SwipeSlide } = await import("../../src/island/swipe-slide.ts");
const { HEADER_PAGES } = await import("../../src/core/island-keys.ts");
const { SWIPE } = await import("../../src/core/page-swipe.ts");
const SLIDE_MS = SWIPE.SLIDE_MAX_MS;

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
  const els = new Map(HEADER_PAGES.map((p) => [p, fakeEl()]));
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

/** `n` touchpad events of `dx`, `step` ms apart (a slow stroke by default). */
function drag(slide, n, dx, step = 16) {
  for (let i = 0; i < n; i++) {
    slide.onWheel(wheel(dx));
    advance(now + step);
  }
}
/** A flick and its first momentum, 8 ms apart. */
function flick(slide, dir = 1) {
  for (const dx of [4, 14, 24, 40, 50, 55, 45, 30, 26, 22, 18, 15, 12, 10, 8, 6]) {
    slide.onWheel(wheel(dir * dx));
    advance(now + 8);
  }
}

const styled = (els) => [...els.values()].some((e) => e.style.transform !== "" || e.style.opacity !== "");

// A slow drag: the page follows it 1:1, however far, with no switch while it moves; it ends past
// RELEASE of the width (here 314 px), the page unchanged: the quiet timer switches it.
{
  const { slide, host, els } = setup();
  drag(slide, 50, 5); // 250 px
  assert.equal(els.get("sessions").style.transform, "translateX(-250px)", "1:1");
  assert.equal(els.get("overview").style.transform, "translateX(64px)", "the next page beside it");
  assert.deepEqual(host.switched, [], "no switch before the release");
  advance(now + SWIPE.QUIET_MS + 10);
  assert.deepEqual(host.switched, ["overview"], "released past RELEASE: one page on");
  assert.match(els.get("overview").style.transition, /^transform \d+ms cubic-bezier\(/);
  assert.equal(els.get("overview").style.transform, "translateX(0px)");
  advance(now + SLIDE_MS + 50);
  assert.equal(styled(els), false, "the slide over, the views are back to the stylesheet");
  assert.equal(timers.length, 0, "no timer left once the gesture is over");
}

// The same drag, then a card comes up before the deltas go quiet: no switch, the gesture goes.
{
  const { slide, host, els } = setup();
  drag(slide, 30, 5);
  host.current = null;
  advance(now + SWIPE.QUIET_MS + 10);
  assert.deepEqual(host.switched, [], "a card on screen is never replaced by a header page");
  assert.equal(styled(els), false, "the inline styles go with the gesture");
  assert.equal(timers.length, 0, "no timer left");
}

// The same drag, then a header tab clicked: the click stays, the gesture does not override it.
{
  const { slide, host, els } = setup();
  drag(slide, 30, 5);
  host.current = "stats";
  advance(now + SWIPE.QUIET_MS + 10);
  assert.deepEqual(host.switched, [], "the clicked tab is kept");
  assert.equal(els.get("stats").classList.has("on"), false, "the gesture does not touch the views' classes");
  assert.equal(styled(els), false);
  assert.equal(timers.length, 0);
}

// A short drag (below RELEASE) slides back, then everything is cleared.
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
  assert.deepEqual(host.switched, [HEADER_PAGES.at(-1)], "← wraps to the last page");
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
  flick(slide);
  advance(now + SWIPE.QUIET_MS + SLIDE_MS + 50);
  assert.deepEqual(host.switched, ["overview"]);
  under = fresh;
  for (const dx of [4, 14, 24, 40, 50, 55, 45, 30]) {
    slide.onWheel(wheel(dx, 0, { target: stale }));
    advance(now + 8);
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
  flick(slide);
  advance(now + SWIPE.QUIET_MS + 10);
  assert.deepEqual(host.switched, []);
  assert.equal(styled(els), false);
  assert.equal(timers.length, 0);
}

// The second swipe starts before the first one's momentum has gone quiet: a page each. The flick
// slides on from where it let go, over a slide no longer than SLIDE_MAX_MS.
{
  const { slide, host, els } = setup("sessions");
  flick(slide);
  assert.deepEqual(host.switched, ["overview"], "the flick switches");
  const ms = Number(/transform (\d+)ms/.exec(els.get("overview").style.transition)[1]);
  assert.ok(ms >= SWIPE.SLIDE_MIN_MS && ms <= SWIPE.SLIDE_MAX_MS, String(ms));
  for (let i = 0; i < 60; i++) {
    slide.onWheel(wheel(Math.max(1, Math.round(6 * Math.exp(-i / 20)))));
    advance(now + 8);
  }
  assert.deepEqual(host.switched, ["overview"], "the momentum is held");
  flick(slide);
  assert.deepEqual(host.switched, ["overview", "stats"], "the new stroke switches once more");
  advance(now + SWIPE.QUIET_MS + SLIDE_MS + 50);
  assert.deepEqual(host.switched, ["overview", "stats"]);
  assert.equal(styled(els), false);
  assert.equal(timers.length, 0);
}

// A new gesture whose first event comes before the quiet timer has run: that event settles the
// slow drag before it (a switch), and the new gesture belongs to the page now on screen.
{
  const { slide, host, els } = setup("sessions");
  drag(slide, 50, 5); // 250 px, past RELEASE
  now += SWIPE.QUIET_MS + 10; // the timer is late: not run yet
  drag(slide, 1, 5); // the late timer runs right after this event
  assert.deepEqual(host.switched, ["overview"], "the first drag switched on the new gesture's first event");
  assert.match(els.get("overview").style.transition, /^transform \d+ms/, "its slide goes on, not cut by the late timer");
  drag(slide, 49, 5);
  assert.equal(els.get("overview").style.transform, "translateX(-250px)", "the new gesture drags the page on screen, every event of it");
  advance(now + SWIPE.QUIET_MS + SLIDE_MS + 50);
  assert.deepEqual(host.switched, ["overview", "stats"]);
  assert.equal(styled(els), false);
  assert.equal(timers.length, 0);
}

// The Agents chat is a header page like the others (when the CLI chat is built): a flick on it
// switches, wrapping round to the first page. A code block or a table in an answer that can still
// scroll that way keeps the gesture; at its end, the page goes. A held button (selecting text in the
// log) takes no part.
if (HEADER_PAGES.includes("agentchat")) {
  {
    const { slide, host, els } = setup("agentchat");
    under = fakeEl(els.get("agentchat"));
    flick(slide);
    assert.deepEqual(host.switched, ["sessions"], "→ from the Agents tab wraps to the first page");
    advance(now + SWIPE.QUIET_MS + SLIDE_MS + 50);
    assert.equal(styled(els), false);
    assert.equal(timers.length, 0);
  }
  {
    const { slide, host, els } = setup("agentchat");
    flick(slide, -1);
    assert.deepEqual(host.switched, ["apps"], "← from the Agents tab is Apps");
    advance(now + SWIPE.QUIET_MS + SLIDE_MS + 50);
  }
  {
    const { slide, host, els } = setup("agentchat");
    under = fakeEl(sideScroller(els.get("agentchat")));
    flick(slide);
    advance(now + SWIPE.QUIET_MS + SLIDE_MS + 50);
    assert.deepEqual(host.switched, [], "a code block that can still scroll sideways keeps the gesture");
    assert.equal(styled(els), false);
  }
  {
    const { slide, host, els } = setup("agentchat");
    under = fakeEl(els.get("agentchat"), { overflowX: "auto", scrollWidth: 900, scrollLeft: 600 });
    flick(slide);
    assert.deepEqual(host.switched, ["sessions"], "a table scrolled to its end lets the page go");
    advance(now + SWIPE.QUIET_MS + SLIDE_MS + 50);
  }
  {
    const { slide, host, els } = setup("agentchat");
    under = fakeEl(els.get("agentchat"));
    for (let i = 0; i < 12; i++) {
      slide.onWheel(wheel(30, 0, { buttons: 1 }));
      advance(now + 8);
    }
    advance(now + SWIPE.QUIET_MS + SLIDE_MS + 50);
    assert.deepEqual(host.switched, [], "selecting text in the log never pages");
    assert.equal(styled(els), false);
  }
}

console.log("swipe-slide: ok");
