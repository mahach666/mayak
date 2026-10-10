// Dev harness: the home page's provider blocks built with the real buildHome() and styles, on fake
// snapshots: Claude and Codex as they ship, with the pace, a Claude window that has just reset, empty
// blocks, a third provider (OpenRouter, with and without a key limit) on a second row, four blocks
// (OpenRouter and DeepSeek), and three balance providers sharing the "balances" block. The stage is
// as tall as the island's card for that many rows (core/layout.ts). Not part of the bundle.

import "../src/style.css";
import { buildHome } from "../src/views/usage";
import { Sessions } from "../src/core/sessions";
import { setLanguage } from "../src/core/i18n";
import { CARD_CHROME_H, islandSize, botPosition } from "../src/core/layout";
import { homeBlocks, homeRows } from "../src/views/usage-format";
import type { DeepSeekBalance, MoonshotBalance, OpenRouterUsage, SessionsSnapshot, Usage } from "../src/core/bridge";

const params = new URLSearchParams(location.search);
setLanguage(params.get("lang") ?? "en");

const now = Date.now();
const M = 60_000;
const H = 60 * M;
const D = 24 * H;

function snapshot(usage: Usage, rev: number): SessionsSnapshot {
  return {
    rev, now, projects: [], attention: [], counts: { waiting: 0, error: 0, finished: 0, working: 0 },
    hold: false, aggregate: "idle", sources: { claude: "ok", codex: "ok", codexHooks: "not_installed" }, usage,
  };
}

const claude = (fivePercent: number, fiveResets: number): Usage["claude"] => ({
  fetchedAtMs: now - 6 * M,
  windows: [
    { id: "5h", percent: fivePercent, resetsAtMs: fiveResets },
    { id: "7d", percent: 41, resetsAtMs: now + 3 * D + 4 * H },
    { id: "7d:opus", percent: 12, resetsAtMs: now + 3 * D + 4 * H },
  ],
});
const codex: Usage["codex"] = {
  seenAtMs: now - 2 * H,
  windows: [
    { minutes: 300, percent: 85, resetsAtMs: now + 50 * M },
    { minutes: 10_080, percent: 30, resetsAtMs: now + 4 * D },
  ],
};
const orLimited: OpenRouterUsage = {
  fetchedAtMs: now - 3 * M, limit: 20, limitRemaining: 7.5, limitReset: "monthly",
  usage: 31.2, usageDaily: 1.2, usageWeekly: 4.8, usageMonthly: 12.5,
};
const orUnlimited: OpenRouterUsage = { ...orLimited, limit: null, limitRemaining: null, limitReset: null };
const deepseek: DeepSeekBalance = {
  fetchedAtMs: now - 4 * M, isAvailable: true,
  balances: [{ currency: "CNY", total: 110, granted: 10, toppedUp: 100 }, { currency: "USD", total: 5.25, granted: 0, toppedUp: 5.25 }],
};
const moonshot: MoonshotBalance = { fetchedAtMs: now - 2 * M, currency: "USD", available: 49.58894, voucher: 46.58893, cash: 3.00001 };

const root = document.getElementById("panels")!;
let rev = 0;

function panel(id: string, title: string, usage: Usage) {
  const rows = homeRows(homeBlocks(usage).length);
  const islandH = islandSize("expanded", "overview", 0, 0, false, 0, rows).h;
  const wrap = document.createElement("div");
  wrap.className = "panel";
  wrap.id = id;
  const h2 = document.createElement("h2");
  h2.textContent = `${title} — island ${islandH}px`;
  const stage = document.createElement("div");
  stage.className = "stage";
  stage.style.height = `${islandH - CARD_CHROME_H}px`;
  // Луч's place on the island (botPosition), less the 8 + 34 px above the card and #content's 10 px.
  const bot = botPosition("expanded", "overview", islandH);
  const luch = document.createElement("div");
  luch.className = "luch";
  luch.style.cssText = `left:${bot.cx - 10 - bot.diameter / 2}px;top:${bot.cy - 42 - bot.diameter / 2}px;width:${bot.diameter}px;height:${bot.diameter}px`;
  wrap.append(h2, stage);
  root.append(wrap);
  Sessions.accept(snapshot(usage, ++rev));
  const host = buildHome();
  host.el.classList.add("on");
  stage.append(host.el, luch);
  host.sync();
  host.setActive?.(true);
  host.setActive?.(false);
}

panel("today", "Claude and Codex as they ship (5h ahead, Codex 5h runs out)", { claude: claude(62, now + 2.5 * H), codex, openrouter: null });
panel("calm", "On pace and in reserve", {
  claude: claude(48, now + 2.5 * H),
  codex: { ...codex, windows: [{ minutes: 300, percent: 10, resetsAtMs: now + 2 * H }, codex.windows[1]] },
  openrouter: null,
});
panel("new-window", "Claude 5h reset passed, no fresh number yet", { claude: claude(97, now - 10 * M), codex, openrouter: null });
panel("empty", "No data from either agent", { claude: null, codex: null, openrouter: null });
panel("openrouter", "Three providers: OpenRouter with a key limit opens a second row", {
  claude: claude(62, now + 2.5 * H), codex, openrouter: orLimited,
});
panel("openrouter-unlimited", "OpenRouter key without a limit", { claude: claude(62, now + 2.5 * H), codex, openrouter: orUnlimited });
panel("four", "Four blocks: OpenRouter and DeepSeek, each in its own", {
  claude: claude(62, now + 2.5 * H), codex, openrouter: orLimited, deepseek,
});
panel("balances", "Five providers: OpenRouter, DeepSeek and Kimi share the balances block", {
  claude: claude(62, now + 2.5 * H), codex, openrouter: orLimited, deepseek, moonshot,
});
document.body.dataset.ready = "1";
