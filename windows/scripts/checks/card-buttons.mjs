// core/card-buttons.ts: the buttons of the session cards. A permission or question the island let go
// leaves the session on the waiting card, which must offer Open as the finished and error cards do.
import assert from "node:assert/strict";
import { cardButtons } from "../../src/core/card-buttons.ts";

// The alert cards: Open first (primary), then OK.
for (const view of ["waiting", "finished", "error"]) {
  assert.deepEqual([...cardButtons(view)], ["open", "ok"], view);
}
// A session opened from its row has no alert to acknowledge: Open only.
assert.deepEqual([...cardButtons("session")], ["open"]);
// Every session card can reach the session's window.
for (const view of ["waiting", "finished", "error", "session"]) {
  assert.equal(cardButtons(view)[0], "open", view);
}
console.log("card-buttons-check: all passed");
