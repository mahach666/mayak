// core/session-menu.ts: the session row menu's keys, its highlight and its place (idea #8).
import assert from "node:assert/strict";
import { menuKeyAction, menuPlace, stepItem } from "../../src/core/session-menu.ts";

const press = (key, extra = {}) => ({ key, code: "", ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...extra });
const shift = (key) => press(key, { shiftKey: true });

// Closed: only the menu key and Shift+F10 open it; everything else is the island's.
assert.deepEqual(menuKeyAction(press("ContextMenu"), false), { kind: "open" });
assert.deepEqual(menuKeyAction(shift("F10"), false), { kind: "open" });
for (const k of ["ArrowDown", "ArrowUp", "Enter", " ", "Escape", "Tab", "F10", "a"]) {
  assert.equal(menuKeyAction(press(k), false), null, `closed: ${k}`);
}
assert.equal(menuKeyAction(press("ContextMenu", { ctrlKey: true }), false), null);
assert.equal(menuKeyAction(press("F10", { shiftKey: true, altKey: true }), false), null);
assert.equal(menuKeyAction(shift("ContextMenu"), false), null);

// Open: ↑ ↓ move, Enter / Space run, Esc / Tab / the menu key close.
assert.deepEqual(menuKeyAction(press("ArrowDown"), true), { kind: "move", delta: 1 });
assert.deepEqual(menuKeyAction(press("ArrowUp"), true), { kind: "move", delta: -1 });
assert.deepEqual(menuKeyAction(press("Enter"), true), { kind: "run" });
assert.deepEqual(menuKeyAction(press(" "), true), { kind: "run" });
assert.deepEqual(menuKeyAction(press("Escape"), true), { kind: "close" });
assert.deepEqual(menuKeyAction(press("Tab"), true), { kind: "close" });
assert.deepEqual(menuKeyAction(shift("Tab"), true), { kind: "close" });
assert.deepEqual(menuKeyAction(press("ContextMenu"), true), { kind: "close" });
assert.deepEqual(menuKeyAction(shift("F10"), true), { kind: "close" });
// Other keys (← → switch pages, which closes the menu with the view) and modified keys pass.
for (const k of ["ArrowLeft", "ArrowRight", "a", "Home"]) assert.equal(menuKeyAction(press(k), true), null, `open: ${k}`);
assert.equal(menuKeyAction(press("Enter", { ctrlKey: true }), true), null);
assert.equal(menuKeyAction(shift("ArrowDown"), true), null);

// The highlight wraps; from none, ↓ starts at the top and ↑ at the bottom.
assert.equal(stepItem(-1, 4, 1), 0);
assert.equal(stepItem(-1, 4, -1), 3);
assert.equal(stepItem(0, 4, 1), 1);
assert.equal(stepItem(3, 4, 1), 0);
assert.equal(stepItem(0, 4, -1), 3);
assert.equal(stepItem(7, 4, 1), 0, "an index past the end restarts");
assert.equal(stepItem(0, 0, 1), -1, "no items");

// Placed at the point, kept 4 px inside the card.
const menu = { w: 180, h: 110 };
const area = { w: 600, h: 200 };
assert.deepEqual(menuPlace({ x: 50, y: 40 }, menu, area), { left: 50, top: 40 });
assert.deepEqual(menuPlace({ x: 590, y: 190 }, menu, area), { left: 416, top: 86 });
assert.deepEqual(menuPlace({ x: -20, y: 1 }, menu, area), { left: 4, top: 4 });
assert.deepEqual(menuPlace({ x: 10, y: 10 }, menu, { w: 100, h: 60 }), { left: 4, top: 4 }, "a card smaller than the menu");

console.log("session-menu-check: all passed");
