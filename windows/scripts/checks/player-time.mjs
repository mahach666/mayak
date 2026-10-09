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

// ── The hold after a seek ──
const T0 = 5_000_000;
const song = P.trackKey({ title: "Song", artist: "Band", app: "Chrome" });
const other = P.trackKey({ title: "Next", artist: "Band", app: "Chrome" });
assert.notEqual(song, other);
// Playing at 0:10 of 3:25; the user seeks to 2:00 at T0.
const before = tl(10_000, 205_000, T0 - 1_000);
let hold = P.startHold(120_000, T0, song);
// Held: the target, running on from the release while playing; frozen while paused.
assert.equal(P.shownPosition(hold, before, true, T0), 120_000);
assert.equal(P.shownPosition(hold, before, true, T0 + 800), 120_800);
assert.equal(P.shownPosition(hold, before, false, T0 + 800), 120_000);
// A report that still shows the old place is ignored for display: the hold stays.
const stale = tl(11_000, 205_000, T0 + 200);
hold = P.settleHold(hold, song, stale, true, T0 + 200);
assert.ok(hold, "old position: still held");
assert.equal(P.shownPosition(hold, stale, true, T0 + 200), 120_200, "no jump back");
// Confirm: a report near the expected place ends the hold; the player's own position takes over.
const landed = tl(120_300, 205_000, T0 + 500);
assert.equal(P.settleHold(hold, song, landed, true, T0 + 500), null);
assert.equal(P.shownPosition(null, landed, true, T0 + 500), 120_300);
// Within SEEK_CONFIRM_MS either way confirms; beyond it does not.
const h2 = P.startHold(60_000, T0, song);
assert.equal(P.settleHold(h2, song, tl(61_500, 205_000, T0), false, T0), null);
assert.equal(P.settleHold(h2, song, tl(58_500, 205_000, T0), false, T0), null);
assert.equal(P.settleHold(h2, song, tl(61_501, 205_000, T0), false, T0), h2);
// Timeout: after SEEK_HOLD_MS the player's report is shown whatever it says.
const h3 = P.startHold(120_000, T0, song);
assert.equal(P.settleHold(h3, song, stale, true, T0 + P.SEEK_HOLD_MS - 1), h3);
assert.equal(P.settleHold(h3, song, stale, true, T0 + P.SEEK_HOLD_MS), null);
assert.equal(P.shownPosition(h3, stale, true, T0 + P.SEEK_HOLD_MS), P.mediaPosition(stale, true, T0 + P.SEEK_HOLD_MS));
assert.equal(P.SEEK_HOLD_MS, 3000);
assert.equal(P.SEEK_CONFIRM_MS, 1500);
// A new seek during a hold replaces it: its own target and its own clock.
let h4 = P.startHold(120_000, T0, song);
h4 = P.startHold(30_000, T0 + 1_000, song);
assert.equal(P.shownPosition(h4, stale, true, T0 + 1_000), 30_000);
assert.ok(P.settleHold(h4, song, tl(120_500, 205_000, T0 + 1_200), true, T0 + 1_200), "the first target does not confirm the second");
assert.equal(P.settleHold(h4, song, tl(30_400, 205_000, T0 + 1_300), true, T0 + 1_300), null);
assert.equal(P.settleHold(h4, song, stale, true, T0 + 3_500), h4, "the timeout counts from the second seek");
// A track change during a hold clears it, as does a timeline that went away.
const h5 = P.startHold(120_000, T0, song);
assert.equal(P.settleHold(h5, other, tl(0, 180_000, T0 + 300), true, T0 + 300), null);
assert.equal(P.settleHold(h5, song, null, true, T0 + 300), null);
assert.equal(P.settleHold(null, song, stale, true, T0), null);
// The held position stays inside the track.
assert.equal(P.shownPosition(P.startHold(204_000, T0, song), before, true, T0 + 2_000), 205_000);

console.log("player-time-check: all passed");
