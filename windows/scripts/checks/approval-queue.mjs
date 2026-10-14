// island/approval-queue.ts: requests that wait behind the card on screen (island/hooks.ts).
// Run: node --experimental-strip-types approval-queue.mjs
import assert from "node:assert/strict";
import { ApprovalQueue, QUEUE_MAX, firstStillAsking, remainingMs } from "../../src/island/approval-queue.ts";

const card = (id) => ({ requestId: id });

assert.equal(QUEUE_MAX, 5);

// ── order: the oldest request comes up first ──
const q = new ApprovalQueue();
assert.equal(q.size, 0);
assert.equal(q.shift(), null);
assert.equal(q.push(card("a"), 100, 1), true);
assert.equal(q.push(card("b"), 200, 1), true);
assert.equal(q.size, 2);
assert.equal(q.has("a"), true);
assert.equal(q.has("z"), false);
assert.deepEqual(q.shift(), { card: card("a"), arrivedAt: 100 });
assert.deepEqual(q.shift(), { card: card("b"), arrivedAt: 200 });
assert.equal(q.shift(), null);

// ── a request already in line is not added twice ──
assert.equal(q.push(card("a"), 1, 1), true);
assert.equal(q.push(card("a"), 2, 1), false);
assert.equal(q.size, 1);
q.drain();

// ── the cap counts the card on screen: 1 shown + 4 waiting, the fifth waiter is refused ──
for (let i = 0; i < QUEUE_MAX - 1; i++) assert.equal(q.push(card(`w${i}`), i, 1), true, `w${i}`);
assert.equal(q.size, 4);
assert.equal(q.push(card("late"), 9, 1), false);
assert.equal(q.has("late"), false);
// With nothing on screen the same line holds one more.
assert.equal(q.push(card("late"), 9, 0), true);
assert.equal(q.size, 5);
assert.equal(q.push(card("later"), 10, 0), false);

// ── remove: a withdrawn request leaves the line, the others keep their place ──
assert.equal(q.remove("w1"), true);
assert.equal(q.remove("w1"), false);
assert.equal(q.remove("nope"), false);
assert.deepEqual(q.drain().map((c) => c.requestId), ["w0", "w2", "w3", "late"]);

// ── drain empties the line ──
assert.equal(q.size, 0);
assert.deepEqual(q.drain(), []);

// ── remainingMs: the agent's wait started when the request came ──
assert.equal(remainingMs(1000, 1000, 590_000), 590_000);
assert.equal(remainingMs(1000, 11_000, 590_000), 580_000);
assert.equal(remainingMs(1000, 591_000, 590_000), 0);
assert.equal(remainingMs(1000, 999_999, 590_000), 0);
// A clock that stepped back never gives more than the limit.
assert.equal(remainingMs(5000, 1000, 590_000), 590_000);

// ── firstStillAsking: the chat's oldest request still pending, the answered ones to forget ──
{
  const pending = new Set(["r3", "r4"]);
  const calls = [];
  const isPending = async (id, conv) => { calls.push(`${id}@${conv}`); return pending.has(id); };
  const asked = new Map([["r1", 1], ["r2", 2], ["r3", 1], ["r4", 3]]);
  const out = await firstStillAsking(asked, isPending);
  assert.deepEqual(out, { found: { requestId: "r3", conv: 1 }, gone: ["r1", "r2"] });
  // It stops at the first pending one: r4 is not asked about.
  assert.deepEqual(calls, ["r1@1", "r2@2", "r3@1"]);

  pending.clear();
  assert.deepEqual(await firstStillAsking(asked, isPending), { found: null, gone: ["r1", "r2", "r3", "r4"] });
  assert.deepEqual(await firstStillAsking(new Map(), isPending), { found: null, gone: [] });
}

console.log("approval-queue: ok");
