// island/approval-queue.ts: requests that wait behind the card on screen (island/hooks.ts).
// Run: node --experimental-strip-types approval-queue.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  ApprovalQueue, KeptOffChat, QUEUE_MAX, firstStillAsking, nextInTime, remainingMs,
} from "../../src/island/approval-queue.ts";

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

// ── nextInTime: the oldest request still in time, the ones out of time or held back dropped ──
{
  const line = new ApprovalQueue();
  line.push(card("old"), 0, 1);
  line.push(card("muted"), 100, 1);
  line.push(card("ok"), 200, 1);
  line.push(card("later"), 300, 1);
  const out = nextInTime(line, 590_050, 590_000, (c) => c.requestId !== "muted");
  assert.deepEqual(out.dropped, [{ card: card("old"), why: "limit" }, { card: card("muted"), why: "held back" }]);
  assert.deepEqual(out.next, { card: card("ok"), leftMs: 150 });
  assert.equal(line.size, 1, "the one behind keeps its place");
  assert.deepEqual(nextInTime(new ApprovalQueue(), 0, 590_000, () => true), { next: null, dropped: [] });
}

// ── Review fix 2: a card's own time limit lets go of that card only ──
// A comes at t=0 (on screen), B at t=10 s, C at t=300 s. A is answered at t=580 s: B comes up with
// what is left of its own 590 s. B's limit then ends B only (the line is kept), and C comes up
// with about 290 s left instead of being declined.
{
  const LIMIT = 590_000;
  const line = new ApprovalQueue();
  assert.equal(line.push(card("B"), 10_000, 1), true);
  assert.equal(line.push(card("C"), 300_000, 1), true);
  // A answered at 580 s.
  let out = nextInTime(line, 580_000, LIMIT, () => true);
  assert.deepEqual(out, { next: { card: card("B"), leftMs: 20_000 }, dropped: [] });
  // B's limit fires at 600 s: the line is kept, so the next one in time comes up.
  out = nextInTime(line, 600_000, LIMIT, () => true);
  assert.deepEqual(out, { next: { card: card("C"), leftMs: 290_000 }, dropped: [] });
}

// ── Review fix 1: only the chat requests a card kept off come up again, and only once ──
{
  const pendingInChat = new Set(["R", "S"]);
  const isPending = async (id) => pendingInChat.has(id);
  // What resumeChatRequest does with the records (island/agent-chat.ts).
  const resume = async (kept) => {
    const { found, gone } = await firstStillAsking(kept.entries(), isPending);
    for (const id of gone) kept.forget(id);
    if (found) kept.clear();
    return found?.requestId ?? null;
  };

  // R opens the island when it comes (no card up): not kept. The user folds the island; a later
  // hook card answered does not bring R up again.
  const kept = new KeptOffChat();
  assert.equal(kept.arrive("R", 1, false), false);
  assert.equal(kept.size, 0);
  assert.equal(await resume(kept), null);

  // S comes while a card is up: kept, and brought up once after the cards are answered.
  assert.equal(kept.arrive("S", 2, true), true);
  assert.equal(await resume(kept), "S");
  assert.equal(await resume(kept), null, "not again after the next card");

  // Kept off, then the cards are let go (fold, other view, pause): it stays in its chat.
  kept.arrive("S", 2, true);
  kept.clear();
  assert.equal(await resume(kept), null);

  // One answered in the chat meanwhile is forgotten, the next pending one comes up.
  kept.arrive("X", 3, true);
  kept.arrive("R", 1, true);
  assert.equal(await resume(kept), "R");
  assert.equal(kept.size, 0);
}

// ── The wiring, read from the sources: which path keeps the line and which drops the records ──
{
  const here = new URL("../../", import.meta.url);
  const hooks = readFileSync(new URL("src/island/hooks.ts", here), "utf8");
  const chat = readFileSync(new URL("src/island/agent-chat.ts", here), "utf8");
  const main = readFileSync(new URL("src/main.ts", here), "utf8");
  assert.ok(hooks.includes("releaseOpenCard(island, true)"), "the card's own limit keeps the line");
  assert.equal(hooks.match(/releaseOpenCard\(island, true\)/g).length, 1, "and nothing else does");
  assert.ok(/releaseOpenCard\(island\);/.test(main), "pause lets go of the whole line");
  assert.ok(hooks.includes("if (!keepLine) releaseWaiting();"), "a let-go hands the line back");
  assert.ok(hooks.includes("dropKeptOffChat()"), "a let-go drops the kept-off chat requests");
  assert.ok(hooks.includes("if (answered) void resumeChatRequest(island);"), "only an answer resumes the chat");
  assert.ok(chat.includes("keptOff.arrive(request.requestId, request.conv, State.pendingApproval != null)"),
    "a chat request is kept only when a card is up");
  assert.ok(chat.includes("keptOff.clear();"), "resuming forgets what it brought up");
  assert.ok(chat.includes('State.view === "focus-phase"'), "a focus phase card comes first");
}

console.log("approval-queue: ok");
