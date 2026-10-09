// core/player-time.ts: the player's progress bar (ideas §35) — the position run on from the last
// report, its times, the seek target, the repeat cycle and when the redraw timer may run.
import assert from "node:assert/strict";
import * as P from "../../src/core/player-time.ts";

const tl = (positionMs, durationMs, updatedAt = 1_000_000, seekable = true) => ({ positionMs, durationMs, updatedAt, seekable });

// Playing: runs on from updatedAt; paused: frozen where it was reported.
const t = tl(60_000, 200_000);
assert.equal(P.mediaPosition(t, true, 1_000_000), 60_000);
assert.equal(P.mediaPosition(t, true, 1_002_500), 62_500);
assert.equal(P.mediaPosition(t, false, 1_002_500), 60_000);
assert.equal(P.mediaPosition(t, false, 9_999_999), 60_000);
// Capped at the end, never below 0; a clock before the report does not run it back.
assert.equal(P.mediaPosition(t, true, 1_000_000 + 500_000), 200_000);
assert.equal(P.mediaPosition(t, true, 999_000), 60_000);
assert.equal(P.mediaPosition(tl(-5, 1_000), false, 0), 0);
assert.equal(P.mediaPosition(tl(5_000, 1_000), false, 0), 1_000);
// Progress.
assert.equal(P.mediaProgress(t, false, 0), 0.3);
assert.equal(P.mediaProgress(tl(10, 0), true, 5), 0);
assert.equal(P.mediaProgress(t, true, 1_000_000 + 10_000_000), 1);

// m:ss, then h:mm:ss from an hour on; rounded down to the second.
assert.equal(P.formatTime(0), "0:00");
assert.equal(P.formatTime(999), "0:00");
assert.equal(P.formatTime(1_000), "0:01");
assert.equal(P.formatTime(59_999), "0:59");
assert.equal(P.formatTime(61_000), "1:01");
assert.equal(P.formatTime(600_000), "10:00");
assert.equal(P.formatTime(3_599_999), "59:59");
assert.equal(P.formatTime(3_600_000), "1:00:00");
assert.equal(P.formatTime(3_725_000), "1:02:05");
assert.equal(P.formatTime(36_000_000), "10:00:00");
assert.equal(P.formatTime(-1), "0:00");
assert.equal(P.formatTime(NaN), "0:00");
assert.equal(P.formatTime(Infinity), "0:00");

// A press on the bar: its share of the track, clamped to the bar.
assert.equal(P.seekTarget(150, 100, 200, 200_000), 50_000);
assert.equal(P.seekTarget(100, 100, 200, 200_000), 0);
assert.equal(P.seekTarget(50, 100, 200, 200_000), 0);
assert.equal(P.seekTarget(400, 100, 200, 200_000), 200_000);
assert.equal(P.seekTarget(150, 100, 0, 200_000), 0, "a bar with no width");
assert.equal(P.seekTarget(150, 100, 200, 0), 0, "no length");

// The repeat button walks off → all → one → off.
assert.equal(P.nextRepeat("off"), "all");
assert.equal(P.nextRepeat("all"), "one");
assert.equal(P.nextRepeat("one"), "off");

// The redraw timer: only on screen, playing, with a timeline; at most twice a second.
assert.equal(P.needsTick(true, true, t), true);
assert.equal(P.needsTick(false, true, t), false, "hidden island: 0 % CPU");
assert.equal(P.needsTick(true, false, t), false, "paused: frozen");
assert.equal(P.needsTick(true, true, null), false, "no timeline: no bar");
assert.equal(P.needsTick(true, true, tl(0, 0)), false);
assert.ok(P.PLAYER_TICK_MS >= 500);

console.log("player-time-check: all passed");
