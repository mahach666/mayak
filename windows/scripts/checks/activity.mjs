import assert from "node:assert/strict";
import { activityFor } from "../../src/island/activity.ts";
assert.deepEqual(activityFor(null, false), { dance: false, notes: false, prop: null, sleep: false });
assert.deepEqual(activityFor(null, true), { dance: true, notes: true, prop: null, sleep: false });
assert.deepEqual(activityFor("work", false), { dance: false, notes: false, prop: "laptop", sleep: false });
assert.deepEqual(activityFor("short", true), { dance: true, notes: true, prop: "coffee", sleep: false });
assert.deepEqual(activityFor("long", false), { dance: false, notes: false, prop: null, sleep: true });
console.log("activity-check: all passed");
