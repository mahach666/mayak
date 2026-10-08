import assert from "node:assert/strict";
import { resolveLanguage, format } from "../../src/core/i18n-core.ts";
assert.equal(resolveLanguage("system", "ru-RU"), "ru");
assert.equal(resolveLanguage("system", "zh-CN"), "zh");
assert.equal(resolveLanguage("system", "pt-BR"), "en");
assert.equal(resolveLanguage("de", "ru-RU"), "de");
assert.equal(resolveLanguage("xx", "ja-JP"), "ja");
assert.equal(resolveLanguage("", ""), "en");
assert.equal(format("{n} finished", { n: 3 }), "3 finished");
assert.equal(format("Couldn't open {name}", { name: "Cursor" }), "Couldn't open Cursor");
assert.equal(format("{a}{b}{a}", { a: 1, b: "x" }), "1x1");
assert.equal(format("no vars {x}", {}), "no vars {x}");

// The English defaults of the pure modules (kept for the Node checks) say what en.ts says.
import { en } from "../../src/core/i18n/en.ts";
import { phaseLabel, phaseCardTitle } from "../../src/island/pomodoro.ts";
import { focusCountsLine, focusProjectsLine } from "../../src/island/focus.ts";
const tEn = (key, vars) => {
  assert.ok(key in en, `en.ts has no ${key}`);
  return format(en[key], vars);
};
const cfg = { work: 25, short: 5, long: 15, cycles: 4 };
for (const [phase, cycle] of [["work", 2], ["short", 1], ["long", 4]]) {
  assert.equal(phaseLabel(phase, cycle, cfg, tEn), phaseLabel(phase, cycle, cfg));
  assert.equal(phaseCardTitle(phase, cycle, cfg, tEn), phaseCardTitle(phase, cycle, cfg));
}
for (const c of [{ finished: 3, error: 1, waited: 1 }, { finished: 0, error: 2, waited: 0 }]) {
  assert.equal(focusCountsLine(c, tEn), focusCountsLine(c));
}
const projects = [{ name: "mayak", kinds: ["finished", "waited"] }, { name: "api", kinds: ["error"] }];
assert.equal(focusProjectsLine(projects), "mayak: finished, waited · api: error");
assert.equal(focusProjectsLine(projects, tEn), focusProjectsLine(projects));
// A translator reaches every label.
const tUp = (key, vars) => format(en[key], vars).toUpperCase();
assert.equal(phaseLabel("work", 2, cfg, tUp), "WORK · CYCLE 2 OF 4");
assert.equal(focusCountsLine({ finished: 3, error: 1, waited: 0 }, tUp), "3 FINISHED · 1 ERROR");
console.log("i18n-check: all passed");
