// views/home-model.ts and core/layout.ts: the home page with rings and integration tiles. The ring
// geometry and values per state (quiet, 85 %, 100 %, a new window, no data), the short balance line,
// which rows each popover shows, where a popover goes (never above the view, clipped with a scroll),
// which integrations become tiles, how they pack and the island height that follows, the empty state.
import assert from "node:assert/strict";
import * as HM from "../../src/views/home-model.ts";
import * as L from "../../src/core/layout.ts";
import { en } from "../../src/core/i18n/en.ts";
import { ru } from "../../src/core/i18n/ru.ts";
import { format } from "../../src/core/i18n-core.ts";

const tr = (k, v) => format(en[k], v);
const trRu = (k, v) => format(ru[k], v);
const M = 60_000;
const H = 60 * M;
const D = 24 * H;
const now = 1_800_000_000_000;

// ── Ring geometry ──
assert.deepEqual([HM.RING_SIZE, HM.RING_STROKE], [26, 2.5]);
{
  const g = HM.ringGeometry(26, 2.5, 42);
  assert.equal(g.c, 13);
  assert.equal(g.r, 11.75);
  assert.ok(Math.abs(g.circumference - 2 * Math.PI * 11.75) < 1e-9);
  assert.equal(g.dash, `${Math.round(g.circumference * 0.42 * 100) / 100} ${Math.round(g.circumference * 100) / 100}`);
  // Nothing to fill at 0 % or without a number (a round cap would still draw a dot).
  for (const p of [0, null, Number.NaN, -5]) assert.equal(HM.ringGeometry(26, 2.5, p).dash, null, String(p));
  // Held at 100 %: the whole circle, never more.
  assert.equal(HM.ringGeometry(26, 2.5, 140).dash, HM.ringGeometry(26, 2.5, 100).dash);
  const full = HM.ringGeometry(26, 2.5, 100).dash.split(" ").map(Number);
  assert.equal(full[0], full[1]);
}

// ── Ring values per state ──
const claude = (five, fiveReset, seven = 18) => ({
  fetchedAtMs: now - 3 * M,
  windows: [
    { id: "5h", percent: five, resetsAtMs: fiveReset },
    { id: "7d", percent: seven, resetsAtMs: now + 4 * D },
    { id: "7d:opus", percent: 99, resetsAtMs: now + 4 * D },
  ],
});
const codex = (five) => ({
  seenAtMs: now - 12 * M,
  windows: [{ minutes: 300, percent: five, resetsAtMs: now + 3 * H }, { minutes: 10_080, percent: 56, resetsAtMs: now + 4 * D }],
});
const cells = (usage) => HM.ringRows(usage, now).map((r) => [r.id, r.title, ...r.cells.map((c) => `${c.text}/${c.tone}`)]);
// Quiet: grey rings; the model's own weekly window (7d Opus, 99 %) is only in the popover.
assert.deepEqual(cells({ claude: claude(42.9, now + 2 * H), codex: codex(31), openrouter: null }), [
  ["claude", "Claude", "42/dim", "18/dim"],
  ["codex", "Codex", "31/dim", "56/dim"],
]);
// Warn and full: amber from 80, red from 95, held at 100 (percentTone).
assert.deepEqual(cells({ claude: claude(100, now + H, 64), codex: codex(85), openrouter: null }), [
  ["claude", "Claude", "100/full", "64/dim"],
  ["codex", "Codex", "85/warn", "56/dim"],
]);
assert.deepEqual(cells({ claude: claude(130, now + H, 95), codex: codex(79.99), openrouter: null })[0].slice(2), ["100/full", "95/full"]);
assert.deepEqual(cells({ claude: null, codex: codex(80), openrouter: null })[1].slice(2), ["80/warn", "56/dim"]);
// A window whose reset time has passed: a new window without data, the empty ring with "—".
for (const at of [now, now - 10 * M]) {
  const [c] = HM.ringRows({ claude: claude(97, at), codex: null, openrouter: null }, now);
  assert.deepEqual(c.cells[0], { percent: null, text: "—", tone: "none" });
  assert.deepEqual(c.cells[0], HM.EMPTY_RING);
}
// No data at all, or no window of that length: empty rings, both rows always there.
assert.deepEqual(cells(undefined), [["claude", "Claude", "—/none", "—/none"], ["codex", "Codex", "—/none", "—/none"]]);
assert.deepEqual(
  cells({ claude: { fetchedAtMs: now, windows: [{ id: "7d", percent: 5 }] }, codex: { seenAtMs: now, windows: [{ minutes: 60, percent: 50 }] }, openrouter: null }),
  [["claude", "Claude", "—/none", "5/dim"], ["codex", "Codex", "—/none", "—/none"]],
  "a window without a reset time still shows; a 1h Codex window is not a column",
);
assert.deepEqual(HM.ringColumns(tr), ["5h", "7d"]);
assert.deepEqual(HM.ringColumns(trRu), ["5 ч", "7 дн."]);
{
  const [c] = HM.ringRows({ claude: claude(42, now - M), codex: null, openrouter: null }, now);
  assert.equal(HM.ringRowLabel(c, tr), "Claude: 5h No data yet, 7d 18%");
}

// ── The balance line ──
const or = {
  fetchedAtMs: now - 4 * M, limit: 20, limitRemaining: 12.4, limitReset: "monthly",
  usage: 7.6, usageDaily: 0.82, usageWeekly: 4.1, usageMonthly: 7.6,
};
const ds = { fetchedAtMs: now - 4 * M, isAvailable: true, balances: [{ currency: "CNY", total: 86.2, granted: 10, toppedUp: 76.2 }] };
const ms = { fetchedAtMs: now - 2 * M, currency: "USD", available: 49.59, voucher: 46.59, cash: 3 };
const chips = (u) => HM.balanceChips(u).map((c) => [c.id, c.text, c.ring, c.tone]);
assert.deepEqual(chips(undefined), []);
assert.deepEqual(chips({ claude: null, codex: null, openrouter: null }), [], "no balance read: no line");
// A ring only where the key has a limit: the share of it spent.
assert.deepEqual(chips({ claude: null, codex: null, openrouter: or, deepseek: ds }), [
  ["openrouter", "$12.40", 38, "dim"],
  ["deepseek", "¥86.20", null, "dim"],
]);
assert.deepEqual(chips({ claude: null, codex: null, openrouter: { ...or, limit: null, limitRemaining: null } }), [["openrouter", "$7.60", null, "dim"]]);
assert.deepEqual(chips({ claude: null, codex: null, openrouter: { ...or, limitRemaining: 0.5 } })[0].slice(2), [97, "full"]);
// A balance too low for API calls reads red.
assert.deepEqual(chips({ claude: null, codex: null, openrouter: null, deepseek: { ...ds, isAvailable: false }, moonshot: { ...ms, available: 0 } }), [
  ["deepseek", "¥86.20", null, "full"],
  ["moonshot", "$0.00", null, "full"],
]);
// Three balances: the compact form, whole units.
assert.equal(HM.BALANCE_COMPACT_FROM, 3);
assert.deepEqual(chips({ claude: null, codex: null, openrouter: or, deepseek: ds, moonshot: ms }).map((c) => c[1]), ["$12", "¥86", "$50"]);
assert.deepEqual(
  chips({ claude: null, codex: null, openrouter: or, deepseek: { ...ds, balances: [] }, moonshot: { ...ms, currency: "EUR", available: -2.4 } }).map((c) => c[1]),
  ["$12", "—", "-2 EUR"],
);

// ── Which rows a popover shows ──
const usage = { claude: claude(62, now + 2.5 * H), codex: codex(31), openrouter: or, deepseek: ds };
const kinds = (rows) => rows.map((r) => r.kind);
{
  const blocks = HM.popoverBlocks("claude", usage, now, tr);
  assert.deepEqual(blocks.map((b) => b.id), ["claude"]);
  const rows = HM.popoverRows(blocks, tr);
  // The title and its age, each window with its reset, the whole pace under a window that has one.
  assert.deepEqual(kinds(rows), ["head", "line", "pace", "line", "pace", "line", "pace"]);
  assert.deepEqual(rows[0], { kind: "head", title: "Claude", updated: "updated 3m ago" });
  assert.deepEqual(rows[1], { kind: "line", label: "62%", ring: 62, tone: "dim", name: "5h", resets: "resets in 2h 30m" });
  assert.deepEqual(rows[2], { kind: "pace", text: "+12% ahead · runs out in ~1h 32m" });
  assert.deepEqual(rows.filter((r) => r.kind === "line").slice(1).map((r) => [r.name, r.label, r.tone]), [["7d", "18%", "dim"], ["7d Opus", "99%", "full"]]);
  assert.equal(rows[4].text, "reserve 25%");
  // A window without a reset time has no pace row.
  const bare = HM.popoverRows(HM.popoverBlocks("claude", { ...usage, claude: { fetchedAtMs: now, windows: [{ id: "7d", percent: 5 }] } }, now, tr), tr);
  assert.deepEqual(kinds(bare), ["head", "line"]);
}
{
  // A new window: "—", the empty ring and the "new window — no data yet" text, no pace.
  const rows = HM.popoverRows(HM.popoverBlocks("codex", { ...usage, codex: { ...codex(97), windows: [{ minutes: 300, percent: 97, resetsAtMs: now - M }] } }, now, tr), tr);
  assert.deepEqual(kinds(rows), ["head", "line"]);
  assert.deepEqual(rows[1], { kind: "line", label: "—", ring: 0, tone: "dim", name: "5h", resets: "new window — no data yet" });
}
{
  // No data: where the data come from.
  const claudeRows = HM.popoverRows(HM.popoverBlocks("claude", undefined, now, tr), tr);
  assert.deepEqual(claudeRows, [
    { kind: "head", title: "Claude", updated: "" },
    { kind: "empty", text: "No data yet" },
    { kind: "note", text: en["usage.hintClaude"] },
  ]);
  const codexRows = HM.popoverRows(HM.popoverBlocks("codex", undefined, now, tr), tr);
  assert.equal(codexRows.at(-1).text, en["usage.hintCodex"]);
}
{
  // Balances: each provider's block, a separator between; a sum has no ring.
  const rows = HM.popoverRows(HM.popoverBlocks("balances", usage, now, tr), tr);
  assert.deepEqual(kinds(rows), ["head", "line", "line", "sep", "head", "line"]);
  assert.deepEqual(rows.filter((r) => r.kind === "head").map((r) => r.title), ["OpenRouter", "DeepSeek"]);
  assert.deepEqual([rows[1].ring, rows[2].ring, rows[5].ring], [38, null, null]);
  // Three: the shared "balances" block, a line each.
  const three = HM.popoverRows(HM.popoverBlocks("balances", { ...usage, moonshot: ms }, now, tr), tr);
  assert.deepEqual(kinds(three), ["head", "line", "line", "line"]);
  assert.equal(three[0].title, "Balances");
  // A low balance keeps its note.
  const low = HM.popoverRows(HM.popoverBlocks("balances", { ...usage, openrouter: null, deepseek: { ...ds, isAvailable: false } }, now, tr), tr);
  assert.deepEqual(low.at(-1), { kind: "note", text: "too low for API calls" });
  assert.deepEqual(HM.popoverBlocks("balances", { claude: null, codex: null, openrouter: null }, now, tr), []);
}

// ── Where a popover goes ──
// The view of a 160 px island: 620 × 108 under the header; the rings column on the right.
const area = { width: 620, height: 108 };
const claudeRow = { left: 506, top: 40, width: 106, height: 30 };
{
  const at = HM.placePopover(claudeRow, { width: 332, height: 74 }, area);
  assert.equal(at.left, 506 - HM.POP_GAP - 332, "left of the ring row");
  assert.equal(at.top, 40 + 15 - 37, "centred on it");
  assert.equal(at.maxH, 108 + HM.POP_BOTTOM_SLACK);
}
{
  // Taller than the room: it never goes above the view (the header and its top-centre stay clear)
  // nor past the slack under it, and gets a max-height to scroll.
  const at = HM.placePopover({ ...claudeRow, top: 2 }, { width: 332, height: 150 }, area);
  assert.equal(at.top, 0);
  assert.equal(at.maxH, 112);
  assert.ok(at.top + Math.min(150, at.maxH) <= area.height + HM.POP_BOTTOM_SLACK);
  // Near the bottom: pushed up to fit.
  const low = HM.placePopover({ ...claudeRow, top: 90 }, { width: 332, height: 60 }, area);
  assert.equal(low.top, 112 - 60);
  // At the top: held at the view's top.
  assert.equal(HM.placePopover({ ...claudeRow, top: 0, height: 10 }, { width: 332, height: 100 }, area).top, 0);
}
{
  // Two rows of tiles (256): more room, the same rule.
  const at = HM.placePopover({ ...claudeRow, top: 80 }, { width: 332, height: 124 }, { width: 620, height: 204 });
  assert.deepEqual([at.top, at.maxH], [95 - 62, 208]);
  // No room on the left: on the right, within the view.
  const right = HM.placePopover({ left: 20, top: 40, width: 60, height: 30 }, { width: 332, height: 60 }, area);
  assert.equal(right.left, 20 + 60 + HM.POP_GAP);
  assert.ok(right.left + 332 <= area.width - HM.POP_EDGE);
}
// The 160 px island leaves the view 108 px, the popover at most 112 (the "~114 px" of the design).
assert.equal(L.islandSize("expanded", "overview").h - L.CARD_CHROME_H, 108);

// ── Which integrations are tiles ──
const tasks = ["integration_resend", "integration_vercel", "integration_github", "integration_gitlab", "integration_stripe"].map((id) => ({ id }));
const conf = (...ids) => Object.fromEntries(ids.map((id) => [id, { configured: true }]));
// Active and with a key, in the tasks' order, at most four.
assert.deepEqual(HM.homeTileIds(tasks, conf("integration_gitlab", "integration_vercel")), ["integration_vercel", "integration_gitlab"]);
assert.deepEqual(HM.homeTileIds(tasks, { ...conf("integration_gitlab"), integration_vercel: { configured: false } }), ["integration_gitlab"]);
assert.deepEqual(HM.homeTileIds(tasks, conf("integration_notion")), [], "a key for an integration that is not active");
assert.equal(HM.homeTileIds(tasks, conf(...tasks.map((t) => t.id))).length, HM.HOME_MAX_TILES);
assert.deepEqual(HM.homeTileIds([], conf("integration_gitlab")), [], "the empty state: no tile, the hint");
assert.equal(HM.HOME_MAX_TILES, 4);
assert.ok(en["home.connectHint"] && ru["home.connectHint"] === "Подключите сервисы в настройках");
// The hint waits for the first key check: before it, no tile and no hint (no startup flash for
// someone whose integrations are about to show); after it, the hint only when no tile came.
assert.equal(HM.homeHintShown(false, 0), false, "before the key check: the area stays empty");
assert.equal(HM.homeHintShown(false, 2), false);
assert.equal(HM.homeHintShown(true, 0), true, "checked, nothing connected: the hint");
assert.equal(HM.homeHintShown(true, 1), false, "checked, a tile: no hint");
assert.equal(HM.homeHintShown(true, HM.HOME_MAX_TILES), false);

// ── Packing and the island's height ──
assert.deepEqual([0, 1, 2, 3, 4, 5].map(HM.homeTileRows), [1, 1, 1, 2, 2, 2]);
assert.deepEqual(HM.tileWide(0), []);
assert.deepEqual(HM.tileWide(1), [true], "one tile takes the whole width");
assert.deepEqual(HM.tileWide(2), [false, false]);
assert.deepEqual(HM.tileWide(3), [false, false, true], "two, then one wide");
assert.deepEqual(HM.tileWide(4), [false, false, false, false]);
const islandH = (tiles, player = false) => L.islandSize("expanded", "overview", 0, 0, player, 0, HM.homeTileRows(tiles)).h;
assert.deepEqual([0, 1, 2, 3, 4].map((n) => islandH(n)), [160, 160, 160, 256, 256]);
assert.equal(L.HOME_ROW_H, 96);
assert.deepEqual([0, 4].map((n) => islandH(n, true)), [212, 308], "the player footer adds its 52 px");
assert.equal(L.islandSize("expanded", "overview").h, 160, "without rows given: one row");
assert.equal(L.islandSize("expanded", "overview", 0, 0, false, 0, 5).h, 256, "never more than two rows");
// Other views never take the home rows.
for (const v of ["sessions", "stats", "apps", "approval", "question"]) {
  assert.equal(L.islandSize("expanded", v, 0, 3, true, 120, 2).h, L.islandSize("expanded", v, 0, 3, true, 120, 1).h, v);
}
assert.deepEqual(L.islandSize("compact", "overview", 0, 0, false, 0, 2), { w: L.COMPACT_W, h: L.NOTCH_H });
assert.ok(islandH(4, true) <= L.PANEL_H);
// Луч in its 90 px slot on the card's left (the card starts 10 px in).
assert.deepEqual([L.VIEW_LAYOUTS.overview.botX, L.VIEW_LAYOUTS.overview.botDiameter], [55, 54]);
assert.equal(L.VIEW_LAYOUTS.overview.botX, 10 + 90 / 2);

// ── Badges and alerts ──
assert.equal(HM.tileBadge({ pillBadge: "error", state: "idle" }), "error");
assert.equal(HM.tileBadge({ pillBadge: null, state: "finished" }), "finished");
assert.equal(HM.tileBadge({ pillBadge: undefined, state: "error" }), "error");
assert.equal(HM.tileBadge({ pillBadge: null, state: "idle" }), null);
assert.equal(HM.tileBadge({ pillBadge: "approval", state: "finished" }), "approval", "the pill badge first");
assert.equal(HM.tileAlert("integration_gitlab", { state: "idle" }, { ci: { failed: 1 } }), true);
assert.equal(HM.tileAlert("integration_gitlab", { state: "idle" }, { ci: { failed: 0, running: 2 } }), false);
assert.equal(HM.tileAlert("integration_vercel", { state: "idle" }, { deployments: [{ state: "ERROR" }, { state: "READY" }] }), true);
assert.equal(HM.tileAlert("integration_vercel", { state: "idle" }, { deployments: [{ state: "READY" }, { state: "ERROR" }] }), false);
assert.equal(HM.tileAlert("integration_github", { state: "error" }, {}), true);
assert.equal(HM.tileAlert("integration_stripe", { state: "idle" }, undefined), false);

console.log("home-rings-check: all passed");
