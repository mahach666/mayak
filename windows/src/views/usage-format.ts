// Pure texts of the home page's plan limits (usage-home spec §4). No DOM and no runtime imports
// but the pure pace module, so Node runs these functions as they are (scripts/checks/usage-home.mjs).
// The words come from `tr`, which the island passes as t() (core/i18n.ts).

import type { OpenRouterUsage, Usage } from "../core/bridge";
import type { Translate } from "../core/i18n-core";
import type { Key } from "../core/i18n";
import { claudeWindowMs, paceLabel, windowPace } from "./usage-pace.ts";

type Tr = Translate<Key>;

export type PercentTone = "dim" | "warn" | "full";

/** Colour of a percent: grey below 80, "warn" from 80, "full" from 95 (context-fill spec §4, usage-home spec §1). */
export function percentTone(percent: number): PercentTone {
  return percent >= 95 ? "full" : percent >= 80 ? "warn" : "dim";
}

/** A limit's label: the percent rounded down and held within 0-100, for the text, the ring and the colour. */
export function usagePercent(percent: number): { text: string; ring: number; tone: PercentTone } {
  const shown = Math.max(0, Math.min(100, Math.floor(percent)));
  return { text: `${shown}%`, ring: shown, tone: percentTone(shown) };
}

/** Codex window name from `window_minutes`, rounded down: under 60 "Nm", under 1440 "Nh", else "Nd" (300 → "5h", 10080 → "7d"). */
export function codexWindowName(minutes: number, tr: Tr): string {
  if (minutes < 60) return tr("unit.m", { n: minutes });
  if (minutes < 1440) return tr("unit.h", { n: Math.floor(minutes / 60) });
  return tr("unit.d", { n: Math.floor(minutes / 1440) });
}

/** "resets in 2h 14m", "resets in 3d 4h", "resets in 12m", "resets soon" under a minute, "reset" once the time has passed. */
export function resetsText(resetsAtMs: number, nowMs: number, tr: Tr): string {
  const left = resetsAtMs - nowMs;
  if (left <= 0) return tr("usage.reset");
  const minutes = Math.floor(left / 60_000);
  if (minutes < 1) return tr("usage.resetsSoon");
  const hours = Math.floor(minutes / 60);
  const time =
    minutes < 60 ? tr("unit.m", { n: minutes })
    : hours < 24 ? tr("unit.hm", { h: hours, m: minutes % 60 })
    : tr("unit.dh", { d: Math.floor(hours / 24), h: hours % 24 });
  return tr("usage.resetsIn", { time });
}

/** "7h", "12m", "3d" (age rounded down); "" under a minute. */
function ageText(age: number, tr: Tr): string {
  const minutes = Math.floor(age / 60_000);
  if (minutes < 1) return "";
  if (minutes < 60) return tr("unit.m", { n: minutes });
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return tr("unit.h", { n: hours });
  return tr("unit.d", { n: Math.floor(hours / 24) });
}

/** The age of a block's data, always shown: "updated 6h ago", "updated just now" under a minute (or in the future). */
export function updatedText(atMs: number, nowMs: number, tr: Tr): string {
  const time = ageText(nowMs - atMs, tr);
  return time === "" ? tr("usage.updatedJustNow") : tr("usage.updated", { time });
}

/** A Claude window's label from its wire id: "5h" and "7d" as durations, "7d:opus" → "7d Opus"; any other id as it is. */
export function claudeWindowName(id: string, tr: Tr): string {
  const m = /^(\d+)([hd])(?::(.+))?$/.exec(id);
  if (!m) return id;
  const name = tr(m[2] === "h" ? "unit.h" : "unit.d", { n: Number(m[1]) });
  const model = m[3];
  return model ? `${name} ${model.charAt(0).toUpperCase()}${model.slice(1)}` : name;
}

export interface UsageLine {
  name: string;
  /** The label on the left: a percent with its ring, or (`amount`) a sum of money without one. */
  percent: { text: string; ring: number; tone: PercentTone };
  /** True for a balance line (credits in $): the label has no ring. */
  amount?: boolean;
  resets: string;
  /** The pace of a window ("out in ~1h 40m", "reserve 20%"); "" when there is nothing to say. */
  pace: string;
  /** The whole pace for the tooltip ("+12% ahead · runs out in ~1h 40m"). */
  paceTitle: string;
}

/** The providers of the home page, in the order their blocks come. */
export type ProviderId = "claude" | "codex" | "openrouter";

export const PROVIDER_ORDER: readonly ProviderId[] = ["claude", "codex", "openrouter"];

const PROVIDER_TITLE: Record<ProviderId, string> = { claude: "Claude", codex: "Codex", openrouter: "OpenRouter" };

/**
 * Blocks side by side in one row of the home card, and the most it shows. Claude and Codex fill the
 * first row as they always did; a third or fourth provider opens a second row (the island grows by
 * HOME_ROW_H, core/layout.ts). Beyond HOME_MAX_BLOCKS the last providers of PROVIDER_ORDER wait.
 */
export const HOME_PER_ROW = 2;
export const HOME_MAX_BLOCKS = 4;

/** The rows of the home card for `blocks` blocks: 1 for up to HOME_PER_ROW, then 2 at most. */
export function homeRows(blocks: number): number {
  const shown = Math.max(1, Math.min(HOME_MAX_BLOCKS, blocks));
  return Math.ceil(shown / HOME_PER_ROW);
}

/**
 * The providers with a block on the home page, in PROVIDER_ORDER: Claude and Codex always (their
 * empty block says how the data come); any other only once it has data (a saved key that was read).
 */
export function homeProviders(usage: Usage | null | undefined): ProviderId[] {
  return PROVIDER_ORDER
    .filter((id) => id === "claude" || id === "codex" || usage?.[id] != null)
    .slice(0, HOME_MAX_BLOCKS);
}

export interface UsageBlock {
  id: ProviderId;
  title: string;
  lines: UsageLine[];
  /** How old the data are, shown under the title ("" when the block is empty). */
  updated: string;
  /** The hint of an empty block ("" otherwise). */
  note: string;
  empty: boolean;
}

const EMPTY_HINT = {
  claude: "usage.hintClaude",
  codex: "usage.hintCodex",
} as const;

/**
 * The blocks of the home page (homeProviders): Claude (dated by its cache), Codex (dated by the last
 * answer read), then the providers with a balance, such as OpenRouter (dated by its last read).
 */
export function usageBlocks(usage: Usage | undefined, nowMs: number, tr: Tr): UsageBlock[] {
  return homeProviders(usage).map((id): UsageBlock => {
    switch (id) {
      case "claude": {
        const claude = usage?.claude ?? null;
        return windowBlock(
          id,
          claude && claude.windows.map((w) => ({
            name: claudeWindowName(w.id, tr), percent: w.percent, resetsAtMs: w.resetsAtMs, windowMs: claudeWindowMs(w.id),
          })),
          claude?.fetchedAtMs ?? 0,
          nowMs,
          tr,
        );
      }
      case "codex": {
        const codex = usage?.codex ?? null;
        return windowBlock(
          id,
          codex && codex.windows.map((w) => ({
            name: codexWindowName(w.minutes, tr), percent: w.percent, resetsAtMs: w.resetsAtMs, windowMs: w.minutes * 60_000,
          })),
          codex?.seenAtMs ?? 0,
          nowMs,
          tr,
        );
      }
      case "openrouter":
        // homeProviders lists it only with data.
        return openRouterBlock(usage!.openrouter!, nowMs, tr);
    }
  });
}

function windowBlock(
  id: "claude" | "codex",
  windows: { name: string; percent: number; resetsAtMs?: number; windowMs: number }[] | null,
  atMs: number,
  nowMs: number,
  tr: Tr,
): UsageBlock {
  const title = PROVIDER_TITLE[id];
  if (!windows || windows.length === 0) return { id, title, lines: [], updated: "", note: tr(EMPTY_HINT[id]), empty: true };
  const lines = windows.map((w): UsageLine => {
    // A window whose reset time has passed is a new window nobody has read yet: empty grey ring, "—".
    if (w.resetsAtMs !== undefined && w.resetsAtMs <= nowMs) {
      return { name: w.name, percent: { text: "—", ring: 0, tone: "dim" }, resets: tr("usage.newWindow"), pace: "", paceTitle: "" };
    }
    return {
      name: w.name,
      percent: usagePercent(w.percent),
      resets: w.resetsAtMs === undefined ? "" : resetsText(w.resetsAtMs, nowMs, tr),
      ...paceOf(windowPace(w.percent, w.resetsAtMs, w.windowMs, nowMs), tr),
    };
  });
  // Both blocks read the same: the age of their data under the title, then the windows.
  return { id, title, lines, updated: updatedText(atMs, nowMs, tr), note: "", empty: false };
}

function paceOf(p: ReturnType<typeof windowPace>, tr: Tr): { pace: string; paceTitle: string } {
  const label = paceLabel(p, tr);
  return { pace: label.text, paceTitle: label.title };
}

/** "$7.50", "$0.04", "$1234" (whole dollars from 1000 on). */
export function money(usd: number): string {
  const n = Number.isFinite(usd) ? usd : 0;
  return Math.abs(n) >= 1000 ? `$${Math.round(n)}` : `$${n.toFixed(2)}`;
}

const RESET_KEY = { daily: "usage.orDaily", weekly: "usage.orWeekly", monthly: "usage.orMonthly" } as const;

/**
 * The OpenRouter block: with a key limit, the share of it spent (ring) and what is left; without
 * one, the total spent and "no key limit". Then what was spent today, this week and this month.
 */
function openRouterBlock(or: OpenRouterUsage, nowMs: number, tr: Tr): UsageBlock {
  const lines: UsageLine[] = [];
  const reset = or.limitReset !== null && Object.hasOwn(RESET_KEY, or.limitReset)
    ? tr(RESET_KEY[or.limitReset as keyof typeof RESET_KEY]) : "";
  const amount = (usd: number) => ({ text: money(usd), ring: 0, tone: "dim" as const });
  if (or.limit !== null && or.limit > 0 && or.limitRemaining !== null) {
    lines.push({
      name: tr("usage.orLeft", { left: money(or.limitRemaining), limit: money(or.limit) }),
      percent: usagePercent((1 - or.limitRemaining / or.limit) * 100),
      resets: reset,
      pace: "",
      paceTitle: "",
    });
  } else if (or.limit !== null && or.limit > 0) {
    lines.push({ name: tr("usage.orLimit", { limit: money(or.limit) }), percent: amount(or.usage), amount: true, resets: reset, pace: "", paceTitle: "" });
  } else {
    lines.push({ name: tr("usage.orTotal"), percent: amount(or.usage), amount: true, resets: tr("usage.orNoLimit"), pace: "", paceTitle: "" });
  }
  lines.push({
    name: tr("usage.orToday"),
    percent: amount(or.usageDaily),
    amount: true,
    resets: tr("usage.orWeekMonth", { week: money(or.usageWeekly), month: money(or.usageMonthly) }),
    pace: "",
    paceTitle: "",
  });
  return { id: "openrouter", title: PROVIDER_TITLE.openrouter, lines, updated: updatedText(or.fetchedAtMs, nowMs, tr), note: "", empty: false };
}
