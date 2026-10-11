// Node check: an integration poll that re-sends the same data does not redraw the island
// (src/island/integration-entry.ts, used by handle() in src/island/integrations.ts).
import assert from "node:assert/strict";

const { nextIntegration, sameIntegration, sameJson, shouldStore } = await import(new URL("../../src/island/integration-entry.ts", import.meta.url).href);

// sameJson: structural, key order ignored, arrays ordered, types strict
assert.ok(sameJson({ a: 1, b: [1, { c: "x" }] }, { b: [1, { c: "x" }], a: 1 }));
assert.ok(!sameJson({ a: 1 }, { a: 1, b: 2 }));
assert.ok(!sameJson({ a: 1, b: undefined }, { a: 1, c: undefined }));
assert.ok(!sameJson([1, 2], [2, 1]));
assert.ok(!sameJson([], {}));
assert.ok(!sameJson({ a: null }, { a: {} }));
assert.ok(!sameJson(1, "1"));
assert.ok(sameJson(null, null));

// handle()'s gate (shouldStore, the same call handle() makes), on a fake store: write + notify unless unchanged and no event
const store = {};
let notifies = 0;
const handle = (update) => {
  const previous = store[update.id];
  const next = nextIntegration(previous, update);
  if (!shouldStore(previous, next, update.event)) return;
  store[update.id] = next;
  notifies++;
};
const id = "integration_calcom";
const poll = (data, extra = {}) => ({ id, data, error: null, event: null, ...extra });

handle(poll({ bookings: [{ title: "a", start: 1 }] }));
assert.equal(notifies, 1, "first update is new");
handle(poll({ bookings: [{ start: 1, title: "a" }] }));
assert.equal(notifies, 1, "same data again: no notify");
handle(poll({ bookings: [{ title: "b", start: 1 }] }));
assert.equal(notifies, 2, "changed data notifies");
const event = { success: true, label: "New booking", detail: null };
handle(poll({ bookings: [{ title: "b", start: 1 }] }, { event }));
handle(poll({ bookings: [{ title: "b", start: 1 }] }, { event }));
assert.equal(notifies, 4, "an event always passes");

// errors: the first notifies and keeps the last good data; the same error again does not
handle(poll({}, { error: "HTTP 500" }));
assert.equal(notifies, 5);
assert.deepEqual(store[id].data, { bookings: [{ title: "b", start: 1 }] });
assert.equal(store[id].loaded, true);
handle(poll({}, { error: "HTTP 500" }));
assert.equal(notifies, 5, "same error again: no notify");
handle(poll({ bookings: [{ title: "b", start: 1 }] }));
assert.equal(notifies, 6, "recovering from the error notifies");
assert.equal(store[id].error, null);

// an error before any data: not loaded, and still a change from nothing
handle({ id: "integration_stripe", data: {}, error: "bad key", event: null });
assert.equal(notifies, 7);
assert.equal(store.integration_stripe.loaded, false);

// configured comes from the stored entry and counts in the comparison
store.integration_vercel = { data: { x: 1 }, error: null, loaded: true, configured: false };
const kept = nextIntegration(store.integration_vercel, { id: "integration_vercel", data: { x: 1 }, error: null, event: null });
assert.equal(kept.configured, false);
assert.ok(sameIntegration(store.integration_vercel, kept));
assert.ok(!sameIntegration({ ...kept, configured: true }, kept));
assert.ok(!sameIntegration(undefined, kept));

console.log("integration updates: unchanged polls skipped, changes and events pass");
