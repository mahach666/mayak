// core/next-waiting.ts: which session the «next waiting session» shortcut goes to (ideas §2).
import assert from "node:assert/strict";
import { nextWaiting } from "../../src/core/next-waiting.ts";

// Episodes as Rust gives them: a waiting session's since, a finished one's at.
const episodes = { perm: 100, ask: 200, done: 50 };
const episodeOf = (key) => episodes[key] ?? null;
const order = ["perm", "ask", "done"];
const none = new Map();

// Nothing waits: no session.
assert.equal(nextWaiting([], episodeOf, none), null);
assert.equal(nextWaiting(["gone"], episodeOf, none), null, "a key with no episode is not waiting");

// The first press goes to the first session in Rust's order (the permission waiting longest).
let r = nextWaiting(order, episodeOf, none);
assert.equal(r.key, "perm");
assert.deepEqual([...r.visited], [["perm", 100]]);

// Each next press goes on to the next one…
r = nextWaiting(order, episodeOf, r.visited);
assert.equal(r.key, "ask");
r = nextWaiting(order, episodeOf, r.visited);
assert.equal(r.key, "done");
// …and after the last the round starts again at the first.
r = nextWaiting(order, episodeOf, r.visited);
assert.equal(r.key, "perm");
assert.deepEqual([...r.visited], [["perm", 100]]);

// A session answered meanwhile leaves the list: the next press skips nothing it should not.
r = nextWaiting(["ask", "done"], episodeOf, new Map([["perm", 100]]));
assert.equal(r.key, "ask");
assert.deepEqual([...r.visited], [["ask", 200]], "a visit of a session no longer waiting is dropped");

// The snapshot has not caught up with the press yet (the open is acknowledged a moment later):
// the same session is not opened twice.
r = nextWaiting(order, episodeOf, new Map([["perm", 100]]));
assert.equal(r.key, "ask");

// The same session waiting again (a new episode) is new, even though it was visited before.
r = nextWaiting(["perm", "ask"], (k) => (k === "perm" ? 900 : episodeOf(k)), new Map([["perm", 100], ["ask", 200]]));
assert.equal(r.key, "perm");
assert.deepEqual([...r.visited].sort(), [["ask", 200], ["perm", 900]]);

// The input map is never changed.
const mine = new Map([["perm", 100]]);
nextWaiting(order, episodeOf, mine);
assert.deepEqual([...mine], [["perm", 100]]);

console.log("next-waiting-check: all passed");
