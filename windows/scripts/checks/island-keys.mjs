// core/island-keys.ts: the keys of the open island (ideas §36) and the session-list highlight.
import assert from "node:assert/strict";
import { cyclePage, islandKeyAction, keepSelection, moveSelection } from "../../src/core/island-keys.ts";

const press = (key, extra = {}) => ({
  key,
  code: extra.code ?? "",
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...extra,
});
const ctrl = (key, code = "") => press(key, { ctrlKey: true, code });
const at = (view, extra = {}) => ({ view, inTextField: false, onControl: false, ...extra });
const list = at("sessions");

// The session list: ↑ ↓ and Ctrl+↑ Ctrl+↓ move, Enter and Ctrl+O open.
assert.deepEqual(islandKeyAction(press("ArrowDown"), list), { kind: "list", delta: 1 });
assert.deepEqual(islandKeyAction(press("ArrowUp"), list), { kind: "list", delta: -1 });
assert.deepEqual(islandKeyAction(ctrl("ArrowDown"), list), { kind: "list", delta: 1 });
assert.deepEqual(islandKeyAction(ctrl("ArrowUp"), list), { kind: "list", delta: -1 });
assert.deepEqual(islandKeyAction(press("Enter"), list), { kind: "open" });
assert.deepEqual(islandKeyAction(ctrl("o", "KeyO"), list), { kind: "open" });
assert.deepEqual(islandKeyAction(ctrl("щ", "KeyO"), list), { kind: "open" }, "another layout");
assert.deepEqual(islandKeyAction(ctrl("O"), list), { kind: "open" });
// Ctrl+Enter, a plain O and other letters do nothing.
assert.equal(islandKeyAction(ctrl("Enter"), list), null);
assert.equal(islandKeyAction(press("o", { code: "KeyO" }), list), null);
assert.equal(islandKeyAction(ctrl("p", "KeyP"), list), null);
// Enter on a focused button or link is the button's own.
assert.equal(islandKeyAction(press("Enter"), at("sessions", { onControl: true })), null);
assert.deepEqual(islandKeyAction(press("ArrowDown"), at("sessions", { onControl: true })), { kind: "list", delta: 1 });
// A text field keeps its arrows and Enter.
for (const k of [press("ArrowDown"), press("ArrowUp"), press("ArrowLeft"), press("Enter"), ctrl("ArrowDown"), ctrl("o", "KeyO")]) {
  assert.equal(islandKeyAction(k, at("sessions", { inTextField: true })), null, k.key);
  assert.equal(islandKeyAction(k, at("prompt", { inTextField: true })), null, k.key);
}
// Alt, Shift and Win leave every key alone.
for (const mod of ["altKey", "shiftKey", "metaKey"]) {
  for (const key of ["ArrowDown", "ArrowRight", "Enter"]) assert.equal(islandKeyAction(press(key, { [mod]: true }), list), null);
}
// Escape is island.ts's own.
assert.equal(islandKeyAction(press("Escape"), list), null);

// Cards never answer these keys: no move, no open, no page.
for (const card of ["approval", "question", "waiting", "session", "finished", "error", "prompt", "upload", "settings", "note"]) {
  for (const k of [press("ArrowDown"), press("ArrowUp"), press("Enter"), ctrl("o", "KeyO"), press("ArrowRight"), press("ArrowLeft")]) {
    assert.equal(islandKeyAction(k, at(card)), null, `${card} ${k.key}`);
  }
}
// The other header pages: ← → switch, ↑ ↓ Enter do nothing.
for (const page of ["sessions", "overview", "stats", "focus", "apps"]) {
  assert.deepEqual(islandKeyAction(press("ArrowRight"), at(page)), { kind: "page", delta: 1 });
  assert.deepEqual(islandKeyAction(ctrl("ArrowLeft"), at(page)), { kind: "page", delta: -1 });
}
for (const page of ["overview", "stats", "focus", "apps"]) {
  assert.equal(islandKeyAction(press("ArrowDown"), at(page)), null);
  assert.equal(islandKeyAction(press("Enter"), at(page)), null);
}

// Moving: clamped at both ends, ↓ from nothing starts at the top, ↑ at the bottom.
const keys = ["a", "b", "c"];
assert.equal(moveSelection(keys, null, 1), "a");
assert.equal(moveSelection(keys, null, -1), "c");
assert.equal(moveSelection(keys, "a", 1), "b");
assert.equal(moveSelection(keys, "c", 1), "c");
assert.equal(moveSelection(keys, "a", -1), "a");
assert.equal(moveSelection(keys, "b", -1), "a");
assert.equal(moveSelection(keys, "gone", 1), "a");
assert.equal(moveSelection(keys, "gone", -1), "c");
assert.equal(moveSelection([], null, 1), null);
assert.equal(moveSelection([], "a", -1), null);

// The list changes: the same session stays, a gone one leaves no highlight.
assert.equal(keepSelection(["x", "b", "y"], "b"), "b");
assert.equal(keepSelection(["x", "y"], "b"), null);
assert.equal(keepSelection(keys, null), null);
assert.equal(keepSelection([], "a"), null);

// The pages, in the header's order, wrapping around.
assert.equal(cyclePage("sessions", 1), "overview");
assert.equal(cyclePage("overview", 1), "stats");
assert.equal(cyclePage("apps", 1), "sessions");
assert.equal(cyclePage("sessions", -1), "apps");
assert.equal(cyclePage("stats", -1), "overview");

console.log("island-keys-check: all passed");
