// Node check: the island loop's frame pacing (src/island/pace.ts) on 60, 120 and 144 Hz panels.
import assert from "node:assert/strict";

const { fireFps, CHARACTER_FPS, tooSoon, fireDelay } = await import(new URL("../../src/island/pace.ts", import.meta.url).href);

// spec §2: the fire alone never runs past 30 fps; the compact bar is not faster than the open island
assert.ok(fireFps(false) <= 30 && fireFps(true) <= 30);
assert.ok(fireFps(true) <= fireFps(false));

// A fire-only frame: the timer starts `spent` ms into a frame begun on a vsync, the next frame is the
// first vsync at or after it fires. The interval must land on 1000 / fps where the vsync grid allows it.
const fireInterval = (hz, fps, spent) => {
  const v = 1000 / hz, due = spent + fireDelay(fps, spent);
  return Math.ceil(due / v - 1e-9) * v;
};
for (const fps of [fireFps(true), fireFps(false)]) {
  for (const spent of [0.5, 2, 6]) {
    // 120 Hz: 15 and 20 fps are whole numbers of vsyncs, hit exactly
    assert.ok(Math.abs(fireInterval(120, fps, spent) - 1000 / fps) < 1e-6, `120 Hz ${fps} fps, ${spent} ms spent`);
    for (const hz of [60, 144]) {
      const dt = fireInterval(hz, fps, spent);
      assert.ok(dt >= 1000 / fps - 4 && dt < 1000 / fps + 1000 / hz, `${hz} Hz ${fps} fps: ${dt} ms`);
      assert.ok(1000 / dt <= 30, "never past 30 fps");
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

console.log(`fire ${fireFps(false)}/${fireFps(true)} fps, character ${CHARACTER_FPS}`);
