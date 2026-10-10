// settings/pages.ts: every section of the Settings window sits on exactly one page, and every page
// title is a key of the dictionaries.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { PAGES, FIRST_PAGE, pageOrFirst, stepPage } from "../../src/settings/pages.ts";
import { en } from "../../src/core/i18n/en.ts";

// The sections main.ts builds: the keys of its `builders` record (typed Record<SectionId, …>, so
// tsc already refuses a section without a builder or a builder for an unknown section).
const main = readFileSync(new URL("../../src/settings/main.ts", import.meta.url), "utf8");
const record = main.match(/const builders: Record<SectionId,[^\n]*= \{([\s\S]*?)\r?\n  \};/);
assert.ok(record, "main.ts has no builders record");
const built = [...record[1].matchAll(/^\s{4}(\w+):/gm)].map((m) => m[1]).sort();
assert.ok(built.length > 0, "the builders record is empty");

// Each section on exactly one page.
const placed = PAGES.flatMap((p) => p.sections);
assert.equal(new Set(placed).size, placed.length, `a section is on two pages: ${placed}`);
assert.deepEqual([...placed].sort(), built, "the pages' sections differ from main.ts's builders");

// Page ids unique; every title translated (i18n-keys checks the other dictionaries match en).
assert.equal(new Set(PAGES.map((p) => p.id)).size, PAGES.length, "two pages share an id");
for (const p of PAGES) {
  assert.ok(p.title in en, `en.ts has no ${p.title}`);
  assert.ok(p.sections.length > 0, `page ${p.id} is empty`);
}
for (const k of ["set.tabs", "set.limits", "set.pages", "set.noTelemetryShort"]) assert.ok(k in en, `en.ts has no ${k}`);

// The order of the sidebar (ideas §37).
assert.deepEqual(PAGES.map((p) => p.id), ["island", "sound", "alerts", "agents", "limits", "integrations", "system"]);

// The session's last page: a known id is kept, anything else opens the first page.
assert.equal(FIRST_PAGE, "island");
assert.equal(pageOrFirst("limits"), "limits");
assert.equal(pageOrFirst(null), "island");
assert.equal(pageOrFirst("general"), "island");
assert.equal(pageOrFirst(""), "island");

// ↑ / ↓ step through the pages and stop at either end.
assert.equal(stepPage("island", 1), "sound");
assert.equal(stepPage("sound", -1), "island");
assert.equal(stepPage("island", -1), "island");
assert.equal(stepPage("system", 1), "system");

console.log(`settings-pages-check: all passed (${PAGES.length} pages, ${placed.length} sections)`);
