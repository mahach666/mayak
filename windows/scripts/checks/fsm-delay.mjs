// Node check: a new home -> petit delay restarts a collapse already scheduled.
let timers = new Map(), id = 0, now = 0;
globalThis.window = {
  setTimeout: (fn, ms) => { timers.set(++id, { fn, at: now + ms }); return id; },
  clearTimeout: (t) => timers.delete(t),
};
const advance = (ms) => {
  now += ms;
  for (const [k, t] of [...timers]) if (t.at <= now) { timers.delete(k); t.fn(); }
};
const { IslandStateMachine } = await import(new URL("../../src/island/fsm.ts", import.meta.url).href);
const assert = (c, m) => { if (!c) { console.error("FAIL", m); process.exit(1); } };
const f = new IslandStateMachine();
f.state = "home"; f.homeToPetitDelay = 15;
f["scheduleHomeCollapse"]();
advance(5000);
f.setHomeToPetitDelay(30);
advance(20000);
assert(f.state === "home", "still home 25 s in: the 30 s delay restarted at 5 s");
advance(10001);
assert(f.state === "petit", "folds 30 s after the change");
const g = new IslandStateMachine();
g.state = "home";
g.setHomeToPetitDelay(5);
advance(60000);
assert(g.state === "home", "no countdown running: changing the delay schedules nothing");
console.log("fsm-delay: ok");
const k = new IslandStateMachine();
k.state = "home"; k.homeToPetitDelay = 15;
k["scheduleHomeCollapse"]();
advance(10000);
k.setHomeToPetitDelay(15);
advance(5001);
assert(k.state === "petit", "the same delay does not restart the countdown");
console.log("fsm-delay: same delay ok");
