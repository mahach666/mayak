// views/usage-format.ts and core/layout.ts: the providers of the home page, their order, their blocks
// and rows (the shared "balances" block past four), the island height for them, the OpenRouter,
// DeepSeek and Kimi blocks and the label of a window that has just reset.
import assert from "node:assert/strict";
import * as F from "../../src/views/usage-format.ts";
import * as L from "../../src/core/layout.ts";
import { en } from "../../src/core/i18n/en.ts";
import { format } from "../../src/core/i18n-core.ts";

const tr = (k, v) => format(en[k], v);
const M = 60_000;
const H = 60 * M;
const now = 1_800_000_000_000;

const claude = { fetchedAtMs: now - 6 * M, windows: [{ id: "5h", percent: 62, resetsAtMs: now + 2.5 * H }, { id: "7d", percent: 41 }] };
const codex = { seenAtMs: now - 2 * H, windows: [{ minutes: 300, percent: 30, resetsAtMs: now + 2.5 * H }] };
const or = {
  fetchedAtMs: now - 3 * M, limit: 20, limitRemaining: 7.5, limitReset: "monthly",
  usage: 31.2, usageDaily: 1.2, usageWeekly: 4.8, usageMonthly: 12.5,
};

// ── Which providers, in which order ──
assert.deepEqual(F.PROVIDER_ORDER, ["claude", "codex", "openrouter", "deepseek", "moonshot"]);
// Claude and Codex always (their empty blocks say how data come), as they always were.
assert.deepEqual(F.homeProviders(undefined), ["claude", "codex"]);
assert.deepEqual(F.homeProviders({ claude: null, codex: null, openrouter: null }), ["claude", "codex"]);
// A provider with a balance only with data, after them.
assert.deepEqual(F.homeProviders({ claude: null, codex, openrouter: or }), ["claude", "codex", "openrouter"]);
assert.deepEqual(F.usageBlocks({ claude, codex, openrouter: or }, now, tr).map((b) => b.title), ["Claude", "Codex", "OpenRouter"]);
// An older snapshot without the field reads as no OpenRouter.
assert.deepEqual(F.homeProviders({ claude, codex }), ["claude", "codex"]);

// ── Rows ──
assert.equal(F.HOME_PER_ROW, 2);
assert.equal(F.HOME_MAX_BLOCKS, 4);
assert.deepEqual([0, 1, 2, 3, 4, 5, 9].map(F.homeRows), [1, 1, 1, 2, 2, 2, 2]);
// The island: one row is the home page as it was, two grow it by HOME_ROW_H, never more.
assert.equal(L.islandSize("expanded", "overview").h, 160);
assert.equal(L.islandSize("expanded", "overview", 0, 0, false, 0, 1).h, 160);
assert.equal(L.islandSize("expanded", "overview", 0, 0, false, 0, 2).h, 160 + L.HOME_ROW_H);
assert.equal(L.islandSize("expanded", "overview", 0, 0, false, 0, 5).h, 160 + L.HOME_ROW_H);
assert.equal(L.islandSize("expanded", "overview", 0, 0, true, 0, 2).h, 160 + L.HOME_ROW_H + L.PLAYER_ROW_H, "the player footer adds on top");
// Other views never take the home rows.
for (const v of ["sessions", "stats", "apps", "approval", "question"]) {
  assert.equal(L.islandSize("expanded", v, 0, 3, true, 120, 2).h, L.islandSize("expanded", v, 0, 3, true, 120, 1).h, v);
}
assert.deepEqual(L.islandSize("compact", "overview", 0, 0, false, 0, 2), { w: L.COMPACT_W, h: L.NOTCH_H });
// Two rows with the player still fit in the window.
assert.ok(L.islandSize("expanded", "overview", 0, 0, true, 0, 2).h <= L.PANEL_H);

// ── Claude and Codex blocks keep what they showed ──
{
  const [c, x] = F.usageBlocks({ claude, codex, openrouter: null }, now, tr);
  assert.equal(c.updated, "updated 6m ago");
  assert.deepEqual(c.lines.map((l) => [l.name, l.percent.text, l.resets]), [["5h", "62%", "resets in 2h 30m"], ["7d", "41%", ""]]);
  assert.equal(c.lines[0].pace, "out in ~1h 32m", "62 % at half the window");
  assert.equal(c.lines[1].pace, "", "no reset time, no pace");
  assert.ok(c.lines.every((l) => !l.amount));
  assert.deepEqual([x.lines[0].pace, x.lines[0].paceTitle], ["reserve 20%", "reserve 20%"]);
}
// Empty blocks: the hints as before.
{
  const [c, x] = F.usageBlocks(undefined, now, tr);
  assert.deepEqual([c.empty, c.note, x.empty, x.note], [true, en["usage.hintClaude"], true, en["usage.hintCodex"]]);
}

// ── A window whose reset time has passed: new window, no data yet ──
for (const resetsAtMs of [now, now - 10 * M]) {
  const passed = { fetchedAtMs: now - H, windows: [{ id: "5h", percent: 97, resetsAtMs }] };
  const xpassed = { seenAtMs: now - H, windows: [{ minutes: 300, percent: 97, resetsAtMs }] };
  for (const b of F.usageBlocks({ claude: passed, codex: xpassed, openrouter: null }, now, tr)) {
    assert.deepEqual(
      [b.lines[0].percent, b.lines[0].resets, b.lines[0].pace],
      [{ text: "—", ring: 0, tone: "dim" }, "new window — no data yet", ""],
      b.title,
    );
  }
}
assert.ok(!("usage.waitingFresh" in en), "the old label is gone");

// ── OpenRouter ──
{
  const b = F.usageBlocks({ claude: null, codex: null, openrouter: or }, now, tr)[2];
  assert.deepEqual([b.id, b.title, b.updated, b.empty], ["openrouter", "OpenRouter", "updated 3m ago", false]);
  assert.deepEqual(
    b.lines.map((l) => [l.percent.text, !!l.amount, l.name, l.resets]),
    [
      ["62%", false, "$7.50 left of $20.00", "resets monthly"],
      ["$1.20", true, "today", "week $4.80 · month $12.50"],
    ],
  );
  assert.equal(b.lines[0].percent.ring, 62);
  assert.equal(b.lines[0].percent.tone, "dim");
}
{
  // No limit on the key.
  const b = F.usageBlocks({ claude: null, codex: null, openrouter: { ...or, limit: null, limitRemaining: null, limitReset: null } }, now, tr)[2];
  assert.deepEqual(b.lines[0], { name: "spent in total", percent: { text: "$31.20", ring: 0, tone: "dim" }, amount: true, resets: "no key limit", pace: "", paceTitle: "" });
}
{
  // A limit without what remains, a reset OpenRouter may add later, a spent limit.
  const b = F.usageBlocks({ claude: null, codex: null, openrouter: { ...or, limitRemaining: null, limitReset: "yearly" } }, now, tr)[2];
  assert.deepEqual([b.lines[0].name, b.lines[0].percent.text, b.lines[0].amount, b.lines[0].resets], ["limit $20.00", "$31.20", true, ""]);
  const spent = F.usageBlocks({ claude: null, codex: null, openrouter: { ...or, limitRemaining: 0, limitReset: "daily" } }, now, tr)[2];
  assert.deepEqual([spent.lines[0].percent.text, spent.lines[0].percent.tone, spent.lines[0].resets], ["100%", "full", "resets daily"]);
  const zero = F.usageBlocks({ claude: null, codex: null, openrouter: { ...or, limit: 0, limitRemaining: 0 } }, now, tr)[2];
  assert.equal(zero.lines[0].name, "spent in total", "a zero limit is no limit to divide by");
}
assert.deepEqual([F.money(7.5), F.money(0.044), F.money(1234.56), F.money(Number.NaN)], ["$7.50", "$0.04", "$1235", "$0.00"]);

// ── The Settings line under the OpenRouter key ──
assert.deepEqual(F.openRouterNotice("rejected"), { tone: "err", key: "set.openrouterRejected" });
assert.deepEqual(F.openRouterNotice("unreachable"), { tone: "warn", key: "set.openrouterUnreachable" });
for (const none of [null, undefined]) assert.equal(F.openRouterNotice(none), null);
assert.equal(en["set.openrouterRejected"], "Key rejected by OpenRouter");
assert.equal(en["set.openrouterUnreachable"], "Couldn't reach OpenRouter");

// ── DeepSeek and Kimi ──
const ds = {
  fetchedAtMs: now - 4 * M, isAvailable: true,
  balances: [{ currency: "CNY", total: 110, granted: 10, toppedUp: 100 }, { currency: "USD", total: 5.25, granted: 0, toppedUp: 5.25 }],
};
const ms = { fetchedAtMs: now - 2 * M, currency: "USD", available: 49.58894, voucher: 46.58893, cash: 3.00001 };
{
  const b = F.usageBlocks({ claude: null, codex: null, openrouter: null, deepseek: ds }, now, tr)[2];
  assert.deepEqual([b.id, b.title, b.updated, b.note, b.empty], ["deepseek", "DeepSeek", "updated 4m ago", "", false]);
  assert.deepEqual(
    b.lines.map((l) => [l.percent.text, !!l.amount, l.name, l.resets]),
    [["¥110.00", true, "balance", "granted ¥10.00"], ["$5.25", true, "balance", ""]],
    "a line per currency, in DeepSeek's order",
  );
  const low = F.usageBlocks({ claude: null, codex: null, openrouter: null, deepseek: { ...ds, isAvailable: false } }, now, tr)[2];
  assert.equal(low.note, "too low for API calls", "is_available false");
  const none = F.usageBlocks({ claude: null, codex: null, openrouter: null, deepseek: { ...ds, balances: [] } }, now, tr)[2];
  assert.deepEqual(none.lines.map((l) => l.percent.text), ["—"], "no currency yet");
}
{
  const b = F.usageBlocks({ claude: null, codex: null, openrouter: null, moonshot: ms }, now, tr)[2];
  assert.deepEqual([b.id, b.title, b.updated, b.note], ["moonshot", "Kimi", "updated 2m ago", ""]);
  assert.deepEqual(b.lines.map((l) => [l.percent.text, l.name, l.resets]), [["$49.59", "balance", "voucher $46.59 · cash $3.00"]]);
  const owed = F.usageBlocks({ claude: null, codex: null, openrouter: null, moonshot: { ...ms, currency: "CNY", available: 0, voucher: 1.5, cash: -1.5 } }, now, tr)[2];
  assert.deepEqual([owed.lines[0].percent.text, owed.lines[0].resets, owed.note], ["¥0.00", "voucher ¥1.50 · cash -¥1.50", "too low for API calls"]);
}
assert.deepEqual(
  [F.moneyIn(7.5, "USD"), F.moneyIn(110, "CNY"), F.moneyIn(-1.5, "USD"), F.moneyIn(-0.001, "USD"), F.moneyIn(1234.5, "CNY"), F.moneyIn(12, "EUR"), F.moneyIn(Number.NaN, "USD")],
  ["$7.50", "¥110.00", "-$1.50", "$0.00", "¥1235", "12.00 EUR", "$0.00"],
);

// ── The layout rule: at most four blocks, two rows ──
// Up to two balance providers: a block each, as OpenRouter has today.
assert.deepEqual(F.homeBlocks({ claude, codex, openrouter: or }), ["claude", "codex", "openrouter"]);
assert.deepEqual(F.homeBlocks({ claude, codex, openrouter: null, deepseek: ds, moonshot: ms }), ["claude", "codex", "deepseek", "moonshot"]);
assert.deepEqual(F.homeBlocks({ claude, codex, openrouter: or, deepseek: ds }), ["claude", "codex", "openrouter", "deepseek"]);
assert.deepEqual(F.homeBlocks({ claude: null, codex: null, openrouter: null, moonshot: ms }), ["claude", "codex", "moonshot"]);
// Three: they share one "balances" block, a line each in PROVIDER_ORDER.
const all = { claude, codex, openrouter: or, deepseek: ds, moonshot: ms };
assert.deepEqual(F.homeProviders(all), ["claude", "codex", "openrouter", "deepseek", "moonshot"]);
assert.deepEqual(F.homeBlocks(all), ["claude", "codex", "balances"]);
for (const usage of [undefined, { claude, codex, openrouter: null }, { claude, codex, openrouter: or }, { claude, codex, openrouter: or, deepseek: ds }, all]) {
  const blocks = F.homeBlocks(usage);
  assert.ok(blocks.length <= F.HOME_MAX_BLOCKS, JSON.stringify(blocks));
  assert.deepEqual(F.usageBlocks(usage, now, tr).map((b) => b.id), blocks);
  assert.ok(F.homeRows(blocks.length) <= 2);
}
assert.equal(F.homeRows(F.homeBlocks({ claude, codex, openrouter: null }).length), 1, "Claude and Codex alone: one row, as today");
{
  const b = F.usageBlocks(all, now, tr)[2];
  assert.deepEqual([b.id, b.title, b.updated, b.note, b.empty], ["balances", "Balances", "updated 4m ago", "", false], "dated by the oldest read");
  assert.deepEqual(
    b.lines.map((l) => [l.percent.text, !!l.amount, l.name, l.resets]),
    [
      ["$7.50", true, "OpenRouter", "limit $20.00"],
      ["¥110.00", true, "DeepSeek", "$5.25"],
      ["$49.59", true, "Kimi", ""],
    ],
  );
  const lean = F.usageBlocks({
    ...all,
    openrouter: { ...or, limit: null, limitRemaining: null, limitReset: null },
    deepseek: { ...ds, isAvailable: false, balances: [] },
    moonshot: { ...ms, available: -2 },
  }, now, tr)[2];
  assert.deepEqual(
    lean.lines.map((l) => [l.percent.text, l.resets]),
    [["$31.20", "spent in total"], ["—", "too low for API calls"], ["-$2.00", "too low for API calls"]],
  );
}

// ── The Settings lines under the DeepSeek and Moonshot keys ──
for (const p of ["openrouter", "deepseek", "moonshot"]) {
  const rejected = F.keyNotice(p, "rejected");
  const unreachable = F.keyNotice(p, "unreachable");
  assert.deepEqual([rejected.tone, unreachable.tone], ["err", "warn"], p);
  assert.ok(en[rejected.key] && en[unreachable.key], p);
  for (const none of [null, undefined]) assert.equal(F.keyNotice(p, none), null);
}
assert.equal(en["set.deepseekRejected"], "Key rejected by DeepSeek");
assert.equal(en["set.moonshotUnreachable"], "Couldn't reach Moonshot");
console.log("usage-home-check: all passed");
