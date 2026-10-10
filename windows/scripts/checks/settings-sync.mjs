// settings/sync.ts: a settings-changed updates every registered control but the one being edited.
import assert from "node:assert/strict";
import { createSyncRegistry } from "../../src/settings/sync.ts";

const reg = createSyncRegistry();
const seen = [];
const slider = {}, input = {}, select = {};
reg.add(slider, (s) => seen.push(["slider", s.v]));
reg.add(input, (s) => seen.push(["input", s.v]));
reg.add(select, (s) => seen.push(["select", s.v]));
reg.add(null, (s) => seen.push(["switch", s.v]));

// Nothing focused: all four update, in registration order.
reg.run({ v: 1 }, null);
assert.deepEqual(seen.splice(0), [["slider", 1], ["input", 1], ["select", 1], ["switch", 1]]);

// A dragged slider (focused) keeps its value; the rest follow.
reg.run({ v: 2 }, slider);
assert.deepEqual(seen.splice(0), [["input", 2], ["select", 2], ["switch", 2]]);

// A focused element that is no registered control exempts nothing.
reg.run({ v: 3 }, {});
assert.equal(seen.splice(0).length, 4);

// A switch (el null) is never exempt, even when nothing is focused (active null).
reg.run({ v: 4 }, null);
assert.equal(seen.pop()[0], "switch");
// Leaving a skipped field redraws it from the latest settings, once.
reg.run({ v: 5 }, slider);
seen.length = 0;
reg.settle(slider, { v: 6 });
assert.deepEqual(seen.splice(0), [["slider", 6]]);
reg.settle(slider, { v: 7 });
assert.equal(seen.length, 0);
// A field that was not skipped is left alone, and a later full run clears a pending skip.
reg.settle(input, { v: 8 });
assert.equal(seen.length, 0);
reg.run({ v: 9 }, slider);
reg.run({ v: 10 }, null);
seen.length = 0;
reg.settle(slider, { v: 11 });
assert.equal(seen.length, 0);

console.log("settings-sync: all passed");
