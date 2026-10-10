// Node check: Open on hover (ideas §34; Coucou's IslandHoverTests.swift cases), the strip dwell
// and the fold time the countdown reads (homeCollapseDueAt).
import assert from "node:assert/strict";

let timers = new Map(), id = 0, now = 0;
globalThis.window = {
  setTimeout: (fn, ms) => { timers.set(++id, { fn, at: now + ms }); return id; },
  clearTimeout: (t) => timers.delete(t),
};
const seconds = (s) => {
  const end = now + Math.round(s * 1000);
  for (;;) {
    const next = [...timers].filter(([, t]) => t.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
    if (!next) break;
    timers.delete(next[0]);
    now = next[1].at;
    next[1].fn();
  }
  now = end;
};
const { IslandStateMachine } = await import(new URL("../../src/island/fsm.ts", import.meta.url).href);

let fsm;
const fresh = (state = "petit") => {
  timers = new Map();
  fsm = new IslandStateMachine();
  fsm.now = () => now;
  fsm.state = state;
  return fsm;
};
const test = (name, fn) => {
  fn();
  console.log(`fsm-hover: ${name} ok`);
};

test("open on hover off: hovering only peeks", () => {
  fresh();
  fsm.mouseEntered();
  seconds(1);
  assert.equal(fsm.state, "petit");
  assert.equal(fsm.openedByHover, false);
  fresh("hidden");
  fsm.mouseEntered();
  seconds(1);
  assert.equal(fsm.state, "petit");
});

test("hovering opens, leaving folds after the short grace period", () => {
  fresh();
  fsm.openOnHover = true;
  fsm.mouseEntered();
  seconds(0.15);
  assert.equal(fsm.state, "home");
  assert.equal(fsm.openedByHover, true);
  fsm.mouseLeft();
  assert.equal(fsm.homeCollapseDueAt, now + 600, "the countdown reads the 0.6 s fold");
  seconds(0.5);
  assert.equal(fsm.state, "home");
  seconds(0.1);
  assert.equal(fsm.state, "petit");
  assert.equal(fsm.openedByHover, false, "an expired fold clears the hover flag");
  assert.equal(fsm.homeCollapseDueAt, null);
  // From the compact island it opens again.
  fsm.mouseEntered();
  seconds(0.15);
  assert.equal(fsm.state, "home");
});

test("coming back before the grace period keeps it open", () => {
  fresh();
  fsm.openOnHover = true;
  fsm.mouseEntered();
  seconds(0.15);
  fsm.mouseLeft();
  seconds(0.3);
  fsm.mouseEntered();
  assert.equal(fsm.homeCollapseDueAt, null);
  seconds(5);
  assert.equal(fsm.state, "home");
});

test("a click inside makes it an ordinary open island", () => {
  fresh();
  fsm.openOnHover = true;
  fsm.homeToPetitDelay = 3;
  fsm.mouseEntered();
  seconds(0.15);
  fsm.mouseLeft();
  fsm.userInteracted(); // the short countdown becomes the normal one
  assert.equal(fsm.openedByHover, false);
  assert.equal(fsm.homeCollapseDueAt, now + 3000, "the countdown reads the normal fold");
  seconds(1);
  assert.equal(fsm.state, "home");
  seconds(2);
  assert.equal(fsm.state, "petit");
});

test("a waiting card holds the island, hover neither opens nor folds it", () => {
  fresh();
  fsm.openOnHover = true;
  fsm.pinned = true;
  fsm.mouseEntered();
  seconds(1);
  assert.equal(fsm.state, "petit");
  assert.equal(fsm.openedByHover, false);
  fsm.forceHome();
  fsm.mouseLeft();
  assert.equal(fsm.homeCollapseDueAt, null);
  seconds(5);
  assert.equal(fsm.state, "home");
});

test("an island opened by an alert keeps the normal delay", () => {
  fresh();
  fsm.openOnHover = true;
  fsm.homeToPetitDelay = 3;
  fsm.forceHome();
  fsm.mouseEntered();
  fsm.mouseLeft();
  assert.equal(fsm.homeCollapseDueAt, now + 3000);
  seconds(1);
  assert.equal(fsm.state, "home");
  seconds(2);
  assert.equal(fsm.state, "petit");
});

test("forceHome, forcePetit, forceHidden and the collapse button clear the hover flag", () => {
  for (const force of ["forceHome", "forcePetit", "forceHidden", "forceCollapse"]) {
    fresh();
    fsm.openOnHover = true;
    fsm.homeToPetitDelay = 3;
    fsm.mouseEntered();
    seconds(0.15);
    assert.equal(fsm.openedByHover, true);
    fsm[force]();
    assert.equal(fsm.openedByHover, false, force);
    if (fsm.state === "home") {
      fsm.mouseLeft();
      seconds(1);
      assert.equal(fsm.state, "home", `${force}: the normal delay, not the hover one`);
    }
  }
});

test("strip: a touch wakes the compact island, a pass along the top does not open it", () => {
  fresh("hidden");
  fsm.openOnHover = true;
  fsm.mouseEntered(); // strip touch
  assert.equal(fsm.state, "petit", "the touch itself never opens it all the way");
  seconds(0.1);
  fsm.mouseLeft(); // the pointer moved on before the dwell
  seconds(1);
  assert.equal(fsm.state, "petit");
  assert.equal(fsm.openedByHover, false);
});

test("strip: the pointer resting after the touch opens it", () => {
  fresh("hidden");
  fsm.openOnHover = true;
  fsm.mouseEntered();
  seconds(0.14);
  assert.equal(fsm.state, "petit");
  seconds(0.01);
  assert.equal(fsm.state, "home");
  assert.equal(fsm.openedByHover, true);
});

test("strip: a pointer the island reports outside on the wake cancels the dwell", () => {
  fresh("hidden");
  fsm.openOnHover = true;
  // As Island.wireFsm: on entering petit with the pointer off the island, it tells mouseLeft.
  let inIsland = false;
  fsm.onTransition = (_from, to) => {
    if (to === "petit" && !inIsland) fsm.mouseLeft();
  };
  fsm.mouseEntered();
  seconds(1);
  assert.equal(fsm.state, "petit");
  // Then the pointer reaches the compact island and rests there.
  inIsland = true;
  fsm.mouseEntered();
  seconds(0.15);
  assert.equal(fsm.state, "home");
});

test("a click on the compact island during the dwell is an ordinary open", () => {
  fresh();
  fsm.openOnHover = true;
  fsm.homeToPetitDelay = 3;
  fsm.mouseEntered();
  seconds(0.05);
  fsm.click();
  assert.equal(fsm.state, "home");
  assert.equal(fsm.openedByHover, false);
  seconds(1);
  fsm.mouseLeft();
  seconds(1);
  assert.equal(fsm.state, "home", "the normal delay");
});

test("homeCollapseDueAt follows the scheduled fold and a delay change", () => {
  fresh("home");
  fsm.homeToPetitDelay = 15;
  assert.equal(fsm.homeCollapseDueAt, null);
  fsm.mouseLeft();
  assert.equal(fsm.homeCollapseDueAt, now + 15000);
  seconds(5);
  fsm.setHomeToPetitDelay(30);
  assert.equal(fsm.homeCollapseDueAt, now + 30000);
  fsm.mouseEntered();
  assert.equal(fsm.homeCollapseDueAt, null);
});

console.log("fsm-hover: all passed");
