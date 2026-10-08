import assert from "node:assert/strict";
import { activityFor } from "../../src/island/activity.ts";
assert.deepEqual(activityFor(null, false), { headphones: false, notes: false, prop: null, sleep: false });
assert.deepEqual(activityFor(null, true), { headphones: true, notes: true, prop: null, sleep: false });
assert.deepEqual(activityFor("work", false), { headphones: false, notes: false, prop: "laptop", sleep: false });
assert.deepEqual(activityFor("short", true), { headphones: true, notes: true, prop: "coffee", sleep: false });
assert.deepEqual(activityFor("long", false), { headphones: false, notes: false, prop: null, sleep: true });
console.log("activity-check: all passed");
