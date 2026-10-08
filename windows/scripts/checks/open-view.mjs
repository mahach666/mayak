// core/open-view.ts: which page a user-initiated open shows (quick-wins spec §2).
// Run: node --experimental-strip-types open-view-check.mjs
import assert from "node:assert/strict";
import { viewOnOpen, isOpenPage, OPEN_PAGES, OPEN_ON_CLICK_VALUES } from "../../src/core/open-view.ts";

// The default: sessions, or the overview while there is nothing to list (as defaultView() always did).
assert.equal(viewOnOpen("sessions", null, true), "sessions");
assert.equal(viewOnOpen("sessions", null, false), "overview");
// A fixed page opens as chosen, sessions or not.
for (const p of ["overview", "stats", "focus", "apps"]) {
  assert.equal(viewOnOpen(p, null, true), p);
  assert.equal(viewOnOpen(p, "apps", false), p);
}
// Last: the last tab, the sessions while none was opened.
assert.equal(viewOnOpen("last", null, true), "sessions");
assert.equal(viewOnOpen("last", null, false), "overview");
for (const p of OPEN_PAGES) assert.equal(viewOnOpen("last", p, true), p);
assert.equal(viewOnOpen("last", "stats", false), "stats");
assert.equal(viewOnOpen("last", "sessions", false), "overview");
// Unknown, empty or wrongly cased values mean sessions.
for (const bad of ["", "Sessions", "approval", "upload", "settings", "somewhere", "null", "undefined"]) {
  assert.equal(viewOnOpen(bad, "stats", true), "sessions", bad);
  assert.equal(viewOnOpen(bad, null, false), "overview", bad);
}
// The select lists every page and Last.
assert.deepEqual([...OPEN_ON_CLICK_VALUES], ["sessions", "overview", "stats", "focus", "apps", "last"]);
assert.ok(OPEN_PAGES.every(isOpenPage));
for (const v of ["last", "prompt", "upload", "settings", "empty", "waiting", "session", ""]) assert.equal(isOpenPage(v), false, v);
console.log("open-view-check: all passed");
