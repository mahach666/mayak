// Runs every check in this folder, each in its own Node process: the front end's pure modules
// (state machine, alert queue, answers, pomodoro, i18n…) imported straight from src/ as
// TypeScript. Node 22.6+ strips the types; nothing is built first. Exit code 1 if any fails.
//
//   npm run check            all of them
//   npm run check -- i18n    only the checks whose name contains "i18n"

import { spawnSync } from "node:child_process";
import { readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const dir = fileURLToPath(new URL(".", import.meta.url));
const only = process.argv[2] ?? "";
const checks = readdirSync(dir)
  .filter((f) => f.endsWith(".mjs") && f !== "run.mjs" && f.includes(only))
  .sort();

if (checks.length === 0) {
  console.error(`no check matches "${only}"`);
  process.exit(1);
}

const failed = [];
for (const file of checks) {
  const run = spawnSync(
    process.execPath,
    ["--experimental-strip-types", "--disable-warning=ExperimentalWarning", join(dir, file)],
    { encoding: "utf8" },
  );
  const out = `${run.stdout}${run.stderr}`.trim();
  if (run.status === 0) {
    console.log(`ok    ${file}${out ? `  ${out.split("\n").at(-1)}` : ""}`);
  } else {
    failed.push(file);
    console.log(`FAIL  ${file}\n${out}`);
  }
}

console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
process.exit(failed.length ? 1 : 0);
