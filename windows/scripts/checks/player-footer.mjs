// core/layout.ts: which views show the player footer, and the island height it adds (night spec §B).
import assert from "node:assert/strict";
import * as L from "../../src/core/layout.ts";
import { OPEN_PAGES } from "../../src/core/open-view.ts";

// The footer is on the main tabs, the header's pages, and nowhere else.
assert.deepEqual([...L.PLAYER_VIEWS].sort(), [...OPEN_PAGES].sort());
for (const v of ["sessions", "overview", "stats", "focus", "apps"]) assert.equal(L.showsPlayer(v), true, v);
const cards = [
  "empty", "approval", "question", "error", "finished", "confused", "upload", "uploading", "choose", "mail",
  "prompt", "searching", "result", "note", "settings", "greeting", "waiting", "session", "focus-phase", "agentchat",
];
for (const v of cards) assert.equal(L.showsPlayer(v), false, v);
// Every view is one or the other.
assert.equal(cards.length + L.PLAYER_VIEWS.size, Object.keys(L.VIEW_LAYOUTS).length);

// Something playing: a main tab grows by the row, its card keeps its height.
assert.equal(L.PLAYER_ROW_H, 52);
for (const v of ["overview", "stats", "focus", "apps"]) {
  assert.equal(L.islandSize("expanded", v, 0, 0, true).h, L.VIEW_LAYOUTS[v].height + L.PLAYER_ROW_H, v);
  assert.equal(L.islandSize("expanded", v, 0, 0, false).h, L.VIEW_LAYOUTS[v].height, v);
}
assert.equal(L.islandSize("expanded", "overview", 0, 0, true).h, 212);
// The sessions list keeps its own height (and its 300 px cap), the footer comes on top of it.
assert.equal(L.islandSize("expanded", "sessions", 0, 3, true).h, L.sessionsHeight(3) + 52);
assert.equal(L.islandSize("expanded", "sessions", 0, 99, true).h, 352);
// Cards, the chat and the greeting never grow for it.
for (const v of cards) {
  assert.equal(L.islandSize("expanded", v, 2, 3, true, 120).h, L.islandSize("expanded", v, 2, 3, false, 120).h, v);
}
// Compact and hidden are the same with or without it.
for (const v of ["overview", "sessions", "approval"]) {
  assert.deepEqual(L.islandSize("compact", v, 0, 0, true), { w: L.COMPACT_W, h: L.NOTCH_H });
  assert.deepEqual(L.islandSize("hidden", v, 0, 0, true), { w: L.NOTCH_W, h: 0 });
}
// The tallest main tab with the footer still fits in the window.
for (const v of L.PLAYER_VIEWS) assert.ok(L.islandSize("expanded", v, 0, 99, true).h <= L.PANEL_H, v);

// Луч on the home page sits where it does without the player: the island hands botPosition the
// height above the footer.
assert.deepEqual(
  L.botPosition("expanded", "overview", L.islandSize("expanded", "overview", 0, 0, true).h - L.PLAYER_ROW_H),
  L.botPosition("expanded", "overview", L.islandSize("expanded", "overview", 0, 0, false).h),
);
console.log("player-footer-check: all passed");
