// Dev harness: the real home page (buildHome, buildHeader and style.css) on fake data, in the states of
// the home mockup: ?s=quiet|warn|full|glfail|two|empty|ao|aofail, &pop=auto|none|claude|codex|bal (a pinned
// popover), &player=0|1 (the footer's 52 px), &lang=ru|en…, &shot=1 (no toolbar, for screenshots).
// Луч is the 2D fallback flame at botPosition. Not part of the bundle.

import "../src/style.css";
import { buildHome } from "../src/views/usage";
import { buildHeader, type ViewActions } from "../src/views/views";
import { Sessions } from "../src/core/sessions";
import { State } from "../src/core/state";
import { AO_ID } from "../src/core/ao";
import aoPayload from "../src-tauri/src/ao/fixtures/payload.json";
import { setLanguage } from "../src/core/i18n";
import { botPosition, islandSize } from "../src/core/layout";
import { homeTileIds, homeTileRows } from "../src/views/home-model";
import { drawFallbackFire } from "../src/luch/fallback";
import { IDLE_FIRE } from "../src/luch/palette";
import { fireLook } from "../src/luch/look";
import type { SessionsSnapshot, Usage } from "../src/core/bridge";

const q = new URLSearchParams(location.search);
const s = q.get("s") ?? "quiet";
const player = q.get("player") === "1";
setLanguage(q.get("lang") ?? "ru");
if (q.get("shot") === "1") document.body.classList.add("shot");

const now = Date.now();
const M = 60_000;
const H = 60 * M;
const D = 24 * H;

// ── Plan limits, as in the mockup ──
// A window of `len` that resets in `left`: the pace follows from the percent and the time gone by.
const claude: NonNullable<Usage["claude"]> = {
  fetchedAtMs: now - 3 * M,
  windows: [
    { id: "5h", percent: 42, resetsAtMs: now + 2 * H + 14 * M },
    { id: "7d", percent: 18, resetsAtMs: now + 4 * D + 6 * H },
  ],
};
const codex: NonNullable<Usage["codex"]> = {
  seenAtMs: now - 12 * M,
  windows: [
    { minutes: 300, percent: 31, resetsAtMs: now + 3 * H + 5 * M },
    { minutes: 10_080, percent: 56, resetsAtMs: now + 4 * D + 1 * H },
  ],
};
if (s === "warn" || s === "full") codex.windows[0] = { minutes: 300, percent: 85, resetsAtMs: now + 2 * H + 10 * M };
if (s === "full") {
  claude.windows[0] = { id: "5h", percent: 100, resetsAtMs: now + 1 * H + 12 * M };
  claude.windows[1] = { id: "7d", percent: 64, resetsAtMs: now + 1 * D + 20 * H };
}
const usage: Usage = {
  claude,
  codex,
  openrouter: {
    fetchedAtMs: now - 4 * M, limit: 20, limitRemaining: 12.4, limitReset: "monthly",
    usage: 7.6, usageDaily: 0.82, usageWeekly: 4.1, usageMonthly: 7.6,
  },
  deepseek: { fetchedAtMs: now - 4 * M, isAvailable: true, balances: [{ currency: "CNY", total: 86.2, granted: 10, toppedUp: 76.2 }] },
  moonshot: q.get("kimi") === "1" ? { fetchedAtMs: now - 2 * M, currency: "USD", available: 49.59, voucher: 46.59, cash: 3 } : null,
};
const snapshot: SessionsSnapshot = {
  rev: 1, now, projects: [], attention: [], counts: { waiting: 0, error: 0, finished: 0, working: 0 },
  hold: false, aggregate: "idle", sources: { claude: "ok", codex: "ok", codexHooks: "not_installed" }, usage,
};
Sessions.accept(snapshot);

// ── Integrations, as in the mockup ──
const fail = s === "glfail";
const iso = (ago: number) => new Date(now - ago).toISOString();
const DATA: Record<string, Record<string, unknown>> = {
  integration_gitlab: {
    host: "gitlab.com", mineTotal: 3, reviewsTotal: 2, issuesTotal: 5,
    ci: fail ? { failed: 1, running: 0, success: 3 } : { failed: 0, running: 1, success: 4 },
    reviews: [{ title: "Миграция настроек на serde v2", reference: "mayak/windows!128", webUrl: "https://gitlab.com/mayak/windows/-/merge_requests/128" }],
    branches: fail ? [{ project: "mayak / windows", branch: "main", status: "failed", webUrl: "https://gitlab.com/mayak/windows/-/pipelines/1" }] : [],
    mine: [
      { title: "Кнопка Connect для GitLab", reference: "mayak/windows!131", pipeline: fail ? "failed" : "success", webUrl: "https://gitlab.com/mayak/windows/-/merge_requests/131" },
      { title: "Иконки трея в тёмной теме", reference: "mayak/windows!127", pipeline: "running", webUrl: "https://gitlab.com/mayak/windows/-/merge_requests/127" },
    ],
    issues: [],
  },
  integration_github: { totalStars: 1200, totalRepos: 24 },
  // Agent Orchestrator: the real daemon's list (anonymised, src-tauri/src/ao/fixtures); «aofail» has a session
  // asking for the user and a failing CI.
  [AO_ID]: s === "aofail"
    ? {
        ...aoPayload,
        sessions: aoPayload.sessions.map((x, i) =>
          i === 4 ? { ...x, status: "needs_input" } : i === 1 ? { ...x, status: "ci_failed", prs: 1, prsOpen: 1, ci: "failing" } : x),
      }
    : aoPayload,
  integration_vercel: {
    deployments: fail
      ? [{ projectName: "landing", state: "ERROR", createdAt: now - 4 * M }, { projectName: "mayak-site", state: "READY", createdAt: now - 2 * H }, { projectName: "mayak-docs", state: "READY", createdAt: now - D }]
      : [{ projectName: "mayak-site", state: "READY", createdAt: now - 12 * M }, { projectName: "mayak-docs", state: "READY", createdAt: now - 2 * H }, { projectName: "landing", state: "ERROR", createdAt: now - D }],
  },
  integration_stripe: {
    balance: 128450, currency: "eur",
    payments: [
      { description: "Подписка Pro", amount: 900, status: "succeeded", createdAt: iso(3 * H) },
      { description: "Годовой план", amount: 9000, status: "succeeded", createdAt: iso(D) },
    ],
  },
};
const active =
  s === "empty" ? []
  : s === "two" ? ["integration_gitlab", "integration_vercel"]
  : s === "ao" || s === "aofail" ? [AO_ID, "integration_github", "integration_stripe"]
  : ["integration_gitlab", "integration_github", "integration_vercel", "integration_stripe"];
State.settings.activeIntegrations = active;
State.loadIntegrationTasks();
for (const id of active) State.integrations[id] = { data: DATA[id], error: null, loaded: true, configured: true };
// The key check has answered (island/integrations.ts refreshConfigured): "empty" shows the hint.
State.integrationsChecked = true;
// An event on Vercel while it is not focused: the pill badge.
if (fail) State.tasks.find((t) => t.id === "integration_vercel")!.pillBadge = "error";
State.view = "overview";
State.mode = "expanded";

// ── The island ──
const noop = () => {};
const actions = new Proxy({}, { get: () => noop }) as unknown as ViewActions;
const header = buildHeader(actions);
const home = buildHome(actions);
home.el.classList.add("on");
const views = document.createElement("div");
views.id = "views";
views.append(home.el);
const footer = document.createElement("div");
footer.id = "player-footer";
if (player) {
  footer.style.height = "52px";
  footer.innerHTML = `<div class="player"><span class="player-cover none">♪</span><div class="player-text"><div class="player-title">Paper Planes</div><div class="player-meta">M.I.A. · Chrome</div></div><span class="player-buttons"></span></div>`;
}
const content = document.createElement("div");
content.id = "content";
content.append(header.el, views, footer);

const rows = homeTileRows(homeTileIds(State.tasks, State.integrations).length);
const islandH = islandSize("expanded", "overview", 0, 0, player, 0, rows).h;
const island = document.createElement("div");
island.className = "island";
island.style.height = `${islandH}px`;
island.append(content);

// Луч: the fallback flame where the island draws it (botPosition takes the height above the footer).
const bot = botPosition("expanded", "overview", islandH - (player ? 52 : 0));
const box = Math.round(bot.diameter * 1.3);
const luch = document.createElement("canvas");
luch.className = "luch";
const dpr = 2;
luch.width = box * dpr;
luch.height = box * dpr;
luch.style.cssText = `width:${box}px;height:${box}px;left:${bot.cx - box / 2}px;top:${bot.cy - box / 2}px`;
const x = luch.getContext("2d")!;
x.scale(dpr, dpr);
x.translate(box / 2, box / 2);
const u = bot.diameter * 1.2;
drawFallbackFire(x, u, 0, IDLE_FIRE, fireLook("idle", null), 1);
// The eyes on the face panel (the engine draws them in the app).
x.fillStyle = "#2a1408";
for (const ex of [-0.07, 0.13]) {
  x.beginPath();
  x.ellipse(ex * u, 0.05 * u, 0.028 * u, 0.042 * u, 0, 0, Math.PI * 2);
  x.fill();
}
island.append(luch);

const stage = document.getElementById("stage")!;
const ruler = document.createElement("div");
ruler.className = "ruler";
ruler.textContent = `${islandH} px`;
stage.append(island, ruler);

header.sync();
home.sync();
home.setActive?.(true);

// The popover of the state, pinned (as the mockup's screenshots).
const AUTO: Record<string, string> = { quiet: "claude", warn: "codex", full: "claude", glfail: "none", two: "bal", empty: "none", ao: "none", aofail: "none" };
const popKey = (q.get("pop") ?? "auto") === "auto" ? AUTO[s] : q.get("pop")!;
const group = popKey === "bal" ? "balances" : popKey;
const anchor = home.el.querySelector<HTMLButtonElement>(`[data-group="${group}"]`);
if (anchor) requestAnimationFrame(() => anchor.click());

// Toolbar.
const bar = document.getElementById("bar")!;
for (const st of ["quiet", "warn", "full", "glfail", "two", "empty", "ao", "aofail"]) {
  for (const p of ["0", "1"]) {
    const a = document.createElement("a");
    const params = new URLSearchParams(q);
    params.set("s", st);
    params.set("player", p);
    a.href = `?${params}`;
    a.textContent = `${st}${p === "1" ? " + player" : ""}`;
    if (st === s && (p === "1") === player) a.className = "on";
    bar.append(a);
  }
}
document.body.dataset.ready = "1";
