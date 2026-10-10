// views/usage-pace.ts: used % against the share of the window gone by, and when it runs out.
import assert from "node:assert/strict";
import * as P from "../../src/views/usage-pace.ts";
import { en } from "../../src/core/i18n/en.ts";
import { format } from "../../src/core/i18n-core.ts";

const tr = (k, v) => format(en[k], v);
const text = (p) => P.paceLabel(p, tr).text;
const H = 3_600_000;
const W5 = 5 * H;
const now = 1_800_000_000_000;

// The window lengths of Claude's wire ids.
assert.equal(P.claudeWindowMs("5h"), W5);
assert.equal(P.claudeWindowMs("7d"), 7 * 24 * H);
assert.equal(P.claudeWindowMs("7d:opus"), 7 * 24 * H);
assert.equal(P.claudeWindowMs("weird"), 0);

// Half the 5h window gone by (resets in 2h 30m).
const half = now + 2.5 * H;
// 62 % used at 50 %: 12 points ahead; at that rate it is full in 2.5h × 38/62 ≈ 1h 32m, before the reset.
let p = P.windowPace(62, half, W5, now);
assert.equal(p.delta, 12);
assert.ok(p.runsOutInMs !== null && Math.abs(p.runsOutInMs - (2.5 * H * 38) / 62) < 1);
assert.deepEqual(P.paceLabel(p, tr), { text: "out in ~1h 32m", title: "+12% ahead · runs out in ~1h 32m" });
// 30 % used at 50 %: 20 points in reserve, lasts past the reset.
p = P.windowPace(30, half, W5, now);
assert.deepEqual(p, { delta: -20, runsOutInMs: null });
assert.deepEqual(P.paceLabel(p, tr), { text: "reserve 20%", title: "reserve 20%" });
// Ahead but still lasting to the reset: 3h of 5h gone (60 %), 70 % used → full in 3h × 30/70 ≈ 1h 17m, reset in 2h.
p = P.windowPace(70, now + 2 * H, W5, now);
assert.equal(p.delta, 10);
assert.ok(p.runsOutInMs !== null, "1h 17m < 2h: it runs out first");
// 4h of 5h gone (80 %), 85 % used → full in 4h × 15/85 ≈ 42m, reset in 1h: runs out.
assert.equal(text(P.windowPace(85, now + H, W5, now)), "out in ~42m");
// Ahead always runs out before the reset (used > elapsed share): 1h of 5h gone (20 %), 26 % used →
// full in 1h × 74/26 ≈ 2h 51m, reset in 4h.
p = P.windowPace(26, now + 4 * H, W5, now);
assert.equal(p.delta, 6);
assert.equal(text(p), "out in ~2h 51m");
// Behind never runs out: 20 % used at 20 % gone by is exactly on pace.
assert.equal(P.windowPace(20, now + 4 * H, W5, now).runsOutInMs, null);
// A pace ahead without a run-out (not reachable from windowPace, but paceLabel takes any Pace).
assert.deepEqual(P.paceLabel({ delta: 7, runsOutInMs: null }, tr), { text: "+7% ahead", title: "+7% ahead" });
// Within 5 points: on pace, nothing said.
assert.equal(text(P.windowPace(52, half, W5, now)), "");
assert.equal(text(P.windowPace(46, half, W5, now)), "");
// The quiet band ends exactly at PACE_QUIET_POINTS.
assert.equal(text(P.windowPace(45, half, W5, now)), "reserve 5%");

// Nothing to say: no reset time, a passed reset, a full window, too early, no length, a bad percent.
assert.equal(P.windowPace(50, undefined, W5, now), null);
assert.equal(P.windowPace(50, now, W5, now), null);
assert.equal(P.windowPace(50, now - 1, W5, now), null);
assert.equal(P.windowPace(100, half, W5, now), null);
assert.equal(P.windowPace(120, half, W5, now), null);
assert.equal(P.windowPace(10, now + W5 - 0.04 * W5, W5, now), null, "4 % of the window gone by");
assert.ok(P.windowPace(10, now + W5 - 0.05 * W5, W5, now) !== null, "5 % gone by");
assert.equal(P.windowPace(50, half, 0, now), null);
assert.equal(P.windowPace(Number.NaN, half, W5, now), null);
assert.deepEqual(P.paceLabel(null, tr), { text: "", title: "" });
// Nothing used yet: in reserve, never "runs out".
assert.deepEqual(P.windowPace(0, half, W5, now), { delta: -50, runsOutInMs: null });
// A reset further away than the window (clock skew): elapsed is held at 0, so too early.
assert.equal(P.windowPace(50, now + 2 * W5, W5, now), null);

// A 7d window: 3.5 days gone, 80 % used → full in 3.5d × 20/80 = 21h, reset in 3.5d.
p = P.windowPace(80, now + 3.5 * 24 * H, 7 * 24 * H, now);
assert.equal(P.paceLabel(p, tr).title, "+30% ahead · runs out in ~21h 0m");
// Days in the run-out text: 2 of 7 days gone (29 %), 40 % used → full in 2d × 60/40 = 3d, reset in 5d.
p = P.windowPace(40, now + 5 * 24 * H, 7 * 24 * H, now);
assert.equal(text(p), "out in ~3d 0h");
// A minute at least: 99.99 % used, the last bit goes in under a second.
assert.equal(text(P.windowPace(99.99, half, W5, now)), "out in ~1m");

// Every dictionary has the pace texts with their placeholders (i18n-keys checks the rest).
for (const k of ["usage.ahead", "usage.reserve", "usage.runsOut", "usage.outIn"]) assert.ok(k in en, k);
console.log("usage-pace-check: all passed");
