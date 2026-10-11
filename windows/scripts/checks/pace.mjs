// Node check: the island loop's frame pacing (src/island/pace.ts) on 60, 120 and 144 Hz panels.
import assert from "node:assert/strict";

const { fireFps, CHARACTER_FPS, tooSoon, fireDelay, cursorNeedsFrame, MiniPace } = await import(new URL("../../src/island/pace.ts", import.meta.url).href);
const { cursorLook, LOOK_SETTLED } = await import(new URL("../../src/luch/eyes.ts", import.meta.url).href);

// spec §2: the fire alone never runs past 30 fps; the compact bar is not faster than the open island
assert.ok(fireFps(false) <= 30 && fireFps(true) <= 30);
assert.ok(fireFps(true) <= fireFps(false));
// Луч dancing to music: the fire runs faster, 30 open and 20 compact, still within the cap; not dancing
// is the plain rate.
assert.equal(fireFps(false, true), 30);
assert.equal(fireFps(true, true), 20);
assert.equal(fireFps(false, false), fireFps(false));
assert.equal(fireFps(true, false), fireFps(true));
assert.deepEqual([fireFps(false), fireFps(true)], [20, 15]);

// A fire-only frame: the timer starts `spent` ms into a frame begun on a vsync, the next frame is the
// first vsync at or after it fires. The interval must land on 1000 / fps where the vsync grid allows it.
const fireInterval = (hz, fps, spent) => {
  const v = 1000 / hz, due = spent + fireDelay(fps, spent);
  return Math.ceil(due / v - 1e-9) * v;
};
for (const fps of [fireFps(true), fireFps(false), fireFps(true, true), fireFps(false, true)]) {
  for (const spent of [0.5, 2, 6]) {
    // 120 Hz: 15 and 20 fps are whole numbers of vsyncs, hit exactly
    assert.ok(Math.abs(fireInterval(120, fps, spent) - 1000 / fps) < 1e-6, `120 Hz ${fps} fps, ${spent} ms spent`);
    for (const hz of [60, 144]) {
      const dt = fireInterval(hz, fps, spent);
      assert.ok(dt >= 1000 / fps - 4 && dt < 1000 / fps + 1000 / hz, `${hz} Hz ${fps} fps: ${dt} ms`);
      assert.ok(1000 / dt <= 30 + 1e-9, "never past 30 fps");
    }
  }
}
assert.equal(fireDelay(20, 80), 0); // a frame that overran its slot: the next one is not delayed further

// Character-only frames: the loop skips the vsyncs that come too soon after the last drawn frame.
const characterFps = (hz) => {
  const v = 1000 / hz; let last = 0, drawn = 0;
  for (let k = 1; k <= hz; k++) { const t = k * v; if (!tooSoon(t - last, CHARACTER_FPS)) { drawn++; last = t; } }
  return drawn;
};
assert.equal(characterFps(60), 60);
assert.equal(characterFps(120), 60);
assert.equal(characterFps(144), 72);
// jitter: a 120 Hz vsync arriving 0.5 ms early or late still draws every other one
assert.ok(!tooSoon(2 * 1000 / 120 - 0.5, CHARACTER_FPS) && tooSoon(1000 / 120 + 0.5, CHARACTER_FPS));

// 90 and 100 Hz panels: every other vsync (the doc's 45 and 50 fps)
assert.equal(characterFps(90), 45);
assert.equal(characterFps(100), 50);

// A cursor move: what answers it now draws a frame at once, anything else waits for the fire frame.
const still = { crossed: false, hover: false, dragging: false, eyeLag: 0 };
assert.equal(cursorNeedsFrame(still), false);
for (const k of ["crossed", "hover", "dragging"]) assert.equal(cursorNeedsFrame({ ...still, [k]: true }), true, k);
assert.equal(cursorNeedsFrame({ ...still, eyeLag: LOOK_SETTLED * 1.01 }), true);
assert.equal(cursorNeedsFrame({ ...still, eyeLag: LOOK_SETTLED * 0.99 }), false);
// The island's lookX / lookY are tanh of the distance (260 px in x, 200 in y). Eyes settled on a cursor
// far away do not need a frame for a move further out; a move of the same size near Луч does.
const lag = (from, to, stateLook = null) => {
  const [y0, p0] = cursorLook(Math.tanh(from[0] / 260), -Math.tanh(from[1] / 200), stateLook);
  const [y1, p1] = cursorLook(Math.tanh(to[0] / 260), -Math.tanh(to[1] / 200), stateLook);
  return Math.max(Math.abs(y1 - y0), Math.abs(p1 - p0));
};
assert.ok(lag([1200, 300], [1240, 330]) < LOOK_SETTLED, "a far move (40 px) keeps the fire's pace");
assert.ok(lag([-900, 500], [-1000, 560]) < LOOK_SETTLED, "a far move (100 px) keeps the fire's pace");
assert.ok(lag([20, 10], [40, 10]) > LOOK_SETTLED, "a 20 px move by Луч draws at once");
// a state with its own gaze (thinking) follows the cursor 0.35 as much
assert.ok(lag([20, 10], [40, 10], [0.55, 0.55]) < lag([20, 10], [40, 10]));
assert.deepEqual(cursorLook(0.5, -0.4, null), [0.5 * 0.62, -0.4 * 0.5]);

// The minis (MiniPace): the fire-only loop's paced frames draw them, as before (`now`); a loop at the
// panel's rate for the big Луч draws them at the fire's rate, not every vsync.
const fireRates = [fireFps(true), fireFps(false), fireFps(true, true), fireFps(false, true)];
// The panel's rate: every vsync at 60 Hz, every other at 120 Hz, 72 fps at 144 Hz (characterFps above);
// jittered by up to ±0.4 ms. Over 10 s the minis draw at the fire's rate; a gap is under a period by
// less than one frame of the loop (the carried remainder, so 144 Hz still averages out) and over it
// by less than one.
const jitter = (k) => (((k * 7919) % 9) - 4) * 0.1;
for (const fps of fireRates) {
  for (const loopFps of [60, 72]) {
    const pace = new MiniPace();
    let drawn = 0, sinceDraw = 0, minGap = Infinity, maxGap = 0;
    const frames = loopFps * 10;
    for (let k = 0; k < frames; k++) {
      const dt = (1000 / loopFps + jitter(k)) / 1000;
      sinceDraw += dt;
      if (pace.due(dt, fps)) {
        if (drawn > 0) { minGap = Math.min(minGap, sinceDraw); maxGap = Math.max(maxGap, sinceDraw); }
        drawn++; sinceDraw = 0;
      }
    }
    assert.ok(Math.abs(drawn - fps * 10) <= 1, `${loopFps} fps loop, fire ${fps}: minis at ${drawn / 10} fps`); // ±1 at the ends
    const frame = 1 / loopFps + 0.0005;
    assert.ok(minGap > 1 / fps - frame, `${loopFps} fps loop, fire ${fps}: a ${minGap * 1000} ms gap`);
    assert.ok(maxGap < 1 / fps + frame, `${loopFps} fps loop, fire ${fps}: a ${maxGap * 1000} ms gap`);
  }
}
{
  const pace = new MiniPace();
  // a fire-only loop's paced frame always draws them, even one that came a few ms early
  for (let k = 0; k < 10; k++) assert.equal(pace.due(1 / 20 - 0.004, 20, true), true);
  // a state change draws them at once and starts the next period from there
  assert.equal(pace.due(0.001, 20, true), true);
  assert.equal(pace.due(1 / 60, 20), false);
  assert.equal(pace.due(1 / 60, 20), false);
  assert.equal(pace.due(1 / 60 + 0.0001, 20), true);
  // after a stall (a 0.1 s frame) at most one period carries: one catch-up frame, then the pace again
  assert.equal(pace.due(0.1, 20), true);
  assert.equal(pace.due(1 / 60, 20), true);
  assert.equal(pace.due(1 / 60, 20), false);
}

console.log(`fire ${fireFps(false)}/${fireFps(true)} fps (dancing ${fireFps(false, true)}/${fireFps(true, true)}), character ${CHARACTER_FPS}`);
