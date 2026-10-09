import assert from "node:assert/strict";
import { IDLE_FIRE, IDLE_FIRE_RGB, mixHex, rgbToHex, paletteFor, paletteForRGB } from "../../src/luch/palette.ts";
import { fireLook, brightAt } from "../../src/luch/look.ts";

// mixHex / rgbToHex
assert.equal(mixHex("#000000", "#ffffff", 0.5), "#808080");
assert.equal(mixHex("#ff7410", "#ff7410", 0.3), "#ff7410");
assert.equal(rgbToHex([1, 0.5, 0]), "#ff8000");

// idle palette (spec §1) and the derived palette (spec §3.1)
assert.deepEqual(paletteFor(null), IDLE_FIRE);
assert.deepEqual(IDLE_FIRE, { deep: "#e0380a", mid: "#ff7410", light: "#ffbc34", core: "#fff0c2" });
const blue = paletteFor("#3b9eff");
assert.equal(blue.mid, "#3b9eff");
assert.equal(blue.deep, mixHex("#3b9eff", "#000000", 0.25));
assert.equal(blue.light, mixHex("#3b9eff", "#ffffff", 0.45));
assert.equal(blue.core, mixHex("#fffbf2", "#3b9eff", 0.08));

// RGB entry: the idle sentinel gives the idle palette; anything else is derived
assert.deepEqual(paletteForRGB(IDLE_FIRE_RGB), IDLE_FIRE);
const mid = paletteForRGB([0.6, 0.3, 0.2]);
assert.equal(mid.mid, rgbToHex([0.6, 0.3, 0.2]));
// a colour half way from idle to red is derived, never Mochi's grey
assert.notEqual(paletteForRGB([1, 0.38, 0.2]).mid, "#e6e9ee");

// look table (spec §3.1–3.2)
assert.equal(fireLook("idle", null, false).live, 1);
assert.equal(fireLook("working", null, false).live, 1.5);
assert.equal(fireLook("approval", null, false).flicker, "flash");
assert.equal(fireLook("thinking", null, false).flicker, "pulse");
const err = fireLook("error", null, false);
assert.ok(err.tall < 1 && err.smoke && err.flicker === "sputter");
const sleep = fireLook("sleeping", null, false);
assert.ok(sleep.live <= 0.35 && sleep.tall <= 0.5 && sleep.bright < 0.6);
assert.deepEqual(fireLook("idle", null, true), sleep, "the long break sleeps");
assert.ok(fireLook("ratelimit", null, false).tall < 0.7);
assert.equal(fireLook("idle", "love", false).tint, "#ff6fa0");
assert.equal(fireLook("idle", "annoyed", false).tint, "#ff4a2a");
assert.ok(fireLook("idle", "annoyed", false).smoke);
assert.ok(fireLook("idle", "proud", false).tall > 1.1);
assert.ok(fireLook("idle", "yawn", false).tall < 0.8);
assert.equal(fireLook("working", null, false).tint, null);

// brightness stays in range for every flicker
for (const s of ["idle", "approval", "thinking", "error", "sleeping"]) {
  for (let t = 0; t < 5; t += 0.137) {
    const b = brightAt(fireLook(s, null, false), t);
    assert.ok(b > 0.3 && b <= 1.0001, `${s} ${t} ${b}`);
  }
}
console.log("luch-look: all passed");
