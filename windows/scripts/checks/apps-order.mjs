// views/apps-model.ts: the Apps tab's order after a drag, adding, renaming, removing, hiding and
// showing apps, and the slot a dragged tile targets (with hysteresis).
import assert from "node:assert/strict";
import {
  addApp, cleanName, fullOrder, hideApp, moveItem, newCustomId, removeApp, renameApp, reorder, showApp,
} from "../../src/views/apps-model.ts";

const empty = { appOrder: [], hiddenApps: [], customApps: [] };

// fullOrder: the saved order first, then what is on screen and missing from it, each id once
assert.deepEqual(fullOrder([], ["cursor", "zed"]), ["cursor", "zed"]);
assert.deepEqual(fullOrder(["zed", "gone"], ["cursor", "zed", "zed"]), ["zed", "gone", "cursor"]);

// reorder: the dragged order fills the shown ids' slots, a hidden or uninstalled id keeps its place
assert.deepEqual(reorder([], ["zed", "cursor"]), ["zed", "cursor"]);
assert.deepEqual(reorder(["cursor", "codex", "zed"], ["zed", "cursor"]), ["zed", "codex", "cursor"]);
assert.deepEqual(reorder(["cursor"], ["vscode", "cursor"]), ["vscode", "cursor"], "a new tile moved first");
assert.deepEqual(reorder(["a", "b", "c"], ["a", "b", "c"]), ["a", "b", "c"], "no move, no change");

// moveItem
assert.deepEqual(moveItem(["a", "b", "c", "d"], 0, 2), ["b", "c", "a", "d"]);
assert.deepEqual(moveItem(["a", "b", "c", "d"], 3, 0), ["d", "a", "b", "c"]);
assert.deepEqual(moveItem(["a", "b"], 0, 9), ["b", "a"]);
assert.deepEqual(moveItem(["a", "b"], 5, 0), ["a", "b"]);

// ids
assert.equal(newCustomId([], 36), "user-10");
assert.equal(newCustomId(["user-10"], 36), "user-10-2");
assert.equal(newCustomId(["user-10", "user-10-2"], 36), "user-10-3");

// names
assert.equal(cleanName("  Notepad++  "), "Notepad++");
assert.equal(cleanName("a\n  b"), "a b");
assert.equal(cleanName("   "), null);
assert.equal(cleanName("x".repeat(60)).length, 40);

// add: the new app is the last tile, even with no saved order (it must not jump first)
const added = addApp(empty, ["cursor", "zed"], { name: " Chrome ", path: String.raw`C:\Apps\chrome.exe` }, 1000);
assert.equal(added.id, "user-rs");
assert.deepEqual(added.appOrder, ["cursor", "zed", "user-rs"]);
assert.deepEqual(added.customApps, [{ id: "user-rs", name: "Chrome", path: String.raw`C:\Apps\chrome.exe` }]);
const unnamed = addApp(empty, [], { name: "", path: String.raw`C:\Apps\tool.exe` }, 1000);
assert.equal(unnamed.customApps[0].name, "tool.exe");

// rename, remove
const renamed = renameApp(added, "user-rs", "Browser");
assert.equal(renamed.customApps[0].name, "Browser");
assert.equal(renameApp(added, "user-rs", "  ").customApps[0].name, "Chrome", "an empty name is no rename");
assert.equal(renameApp(added, "user-x", "Other").customApps[0].name, "Chrome");
const removed = removeApp(added, "user-rs");
assert.deepEqual(removed.customApps, []);
assert.deepEqual(removed.appOrder, ["cursor", "zed"]);

// hide, show: built-ins only, the order is kept
const hidden = hideApp({ ...empty, appOrder: ["zed", "cursor"] }, "zed");
assert.deepEqual(hidden.hiddenApps, ["zed"]);
assert.deepEqual(hidden.appOrder, ["zed", "cursor"]);
assert.equal(hideApp(hidden, "zed"), hidden, "hidden once");
assert.deepEqual(hideApp(empty, "user-1").hiddenApps, [], "a user app is removed, not hidden");
assert.deepEqual(showApp(hidden, "zed").hiddenApps, []);

console.log("apps-order: ok");

// slotIndex: nearest slot centre, with hysteresis around the boundary
{
  const { slotIndex, previewSlots } = await import("../../src/views/apps-model.ts");
  // two slots, centres at x = 25 and x = 85: the midpoint is x = 55
  const slots = [{ x: 0, y: 0, w: 50, h: 50 }, { x: 60, y: 0, w: 50, h: 50 }, { x: 0, y: 60, w: 50, h: 50 }];
  let cur = slotIndex(slots, 25, 25, -1);
  assert.equal(cur, 0, "no target yet: the nearest slot");
  const path = [];
  // the pointer wobbles around the midpoint, then crosses it for good, then comes back
  for (const x of [50, 54, 56, 57, 58, 54, 56, 53, 57, 65, 60, 54, 52, 50, 48]) {
    cur = slotIndex(slots, x, 25, cur, 4);
    path.push(cur);
  }
  // margin 4: the target changes only 2 px past the midpoint (x > 57 going right, x < 53 going left)
  assert.deepEqual(path, [0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0], "no flip-flop at the boundary");
  assert.equal(slotIndex(slots, 25, 90, 0, 4), 2, "a slot clearly nearer is taken at once");
  assert.equal(slotIndex([], 0, 0, 0), -1);
  assert.equal(slotIndex(slots, 25, 25, 9), 0, "a stale current is dropped");

  // previewSlots: the held tile takes the target slot, the ones between shift by one
  assert.deepEqual(previewSlots(4, 0, 2), [2, 0, 1, 3]);
  assert.deepEqual(previewSlots(4, 3, 1), [0, 2, 3, 1]);
  assert.deepEqual(previewSlots(3, 1, 1), [0, 1, 2]);
  console.log("apps-order: slots ok");
}
