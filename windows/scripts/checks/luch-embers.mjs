// Node check: Луч's ember sprite placement (src/luch/embers.ts). The sprite's glow must land exactly on
// the circle each ember fills, at any size and place, so the pattern reads the gradient where the old
// radial gradient was.
import assert from "node:assert/strict";

const { EMBER_SPRITE_R, EMBER_SPRITE_SIDE, emberPlacement } = await import(new URL("../../src/luch/embers.ts", import.meta.url).href);

// the glow fits in the sprite with a clear pixel round it, centred on a whole pixel
assert.equal(EMBER_SPRITE_SIDE, 2 * EMBER_SPRITE_R + 2);
assert.equal((EMBER_SPRITE_SIDE / 2) % 1, 0);
// large enough that bilinear reads match the gradient (measured: within 7/255 of it)
assert.ok(EMBER_SPRITE_R >= 32);

const close = (a, b, what) => assert.ok(Math.abs(a - b) < 1e-9, `${what}: ${a} vs ${b}`);
for (const [ex, ey, radius] of [[0, 0, 1], [12.3, -40.7, 0.31], [-7.5, 3.25, 2.8], [100, 100, 11]]) {
  const { scale, e, f } = emberPlacement(ex, ey, radius);
  const c = EMBER_SPRITE_SIDE / 2;
  // sprite centre → ember centre
  close(e + c * scale, ex, "centre x");
  close(f + c * scale, ey, "centre y");
  // sprite glow edge → ember circle edge
  close(e + (c + EMBER_SPRITE_R) * scale, ex + radius, "edge x");
  close(f + (c - EMBER_SPRITE_R) * scale, ey - radius, "edge y");
}

console.log(`ember sprite ${EMBER_SPRITE_SIDE} px, glow radius ${EMBER_SPRITE_R}`);
