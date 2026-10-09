import assert from "node:assert/strict";
import { activityFor } from "../../src/island/activity.ts";
// No props drawn over the fire any more (spec §A): each phase is a look of the fire itself.
assert.deepEqual(activityFor(null, false), { dance: false, notes: false, focus: null });
assert.deepEqual(activityFor(null, true), { dance: true, notes: true, focus: null });
assert.deepEqual(activityFor("work", false), { dance: false, notes: false, focus: "focus" });
assert.deepEqual(activityFor("short", true), { dance: true, notes: true, focus: "rest" });
assert.deepEqual(activityFor("long", false), { dance: false, notes: false, focus: "embers" });
assert.deepEqual(activityFor("long", true), { dance: true, notes: true, focus: "embers" });
console.log("activity-check: all passed");
