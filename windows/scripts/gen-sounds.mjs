// Renders Луч's sounds (scripts/sounds/sfx.mjs) to sounds/<name>.wav: 48 kHz, 16-bit PCM, stereo.
// The output is deterministic, so a rerun with an unchanged sfx.mjs leaves git clean, and
// scripts/checks/sounds.mjs fails when a committed WAV no longer matches the script.
//
//   npm run sounds

import { mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { extname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { NAMES, SR, render, toWav } from "./sounds/sfx.mjs";

const OUT = fileURLToPath(new URL("../sounds/", import.meta.url));
mkdirSync(OUT, { recursive: true });

// A sound removed from the set must not linger in the folder (Vite ships every .wav in it).
for (const f of readdirSync(OUT)) {
  if (extname(f) === ".wav" && !NAMES.includes(f.slice(0, -4))) rmSync(join(OUT, f));
}
for (const name of NAMES) writeFileSync(join(OUT, `${name}.wav`), toWav(render(name, SR), SR));
console.log(`wrote ${NAMES.length} sounds to sounds/`);
