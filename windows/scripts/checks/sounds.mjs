// windows/sounds: one WAV per SOUND_NAMES entry, nothing else, and each file is exactly what
// scripts/sounds/sfx.mjs renders today, so the committed set never drifts from the script
// (`npm run sounds` writes it again).
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { SOUND_NAMES } from "../../src/core/sound-names.ts";
import { NAMES, SR, TARGET_RMS, render, toWav } from "../sounds/sfx.mjs";

const dir = fileURLToPath(new URL("../../sounds/", import.meta.url));
const sorted = (xs) => [...xs].sort();

assert.deepEqual(sorted(NAMES), sorted(SOUND_NAMES), "sfx.mjs and SOUND_NAMES name different sounds");
assert.deepEqual(sorted(Object.keys(TARGET_RMS)), sorted(SOUND_NAMES), "TARGET_RMS and SOUND_NAMES differ");
assert.deepEqual(
  sorted(readdirSync(dir)),
  sorted(SOUND_NAMES.map((n) => `${n}.wav`)),
  "windows/sounds must hold exactly one <name>.wav per SOUND_NAMES entry",
);

for (const name of SOUND_NAMES) {
  const committed = readFileSync(join(dir, `${name}.wav`));
  const fresh = toWav(render(name, SR), SR);
  assert.ok(committed.equals(fresh), `${name}.wav differs from sfx.mjs: run npm run sounds`);
}

console.log(`sounds-check: all passed (${SOUND_NAMES.length} sounds)`);
