// Pure texts of the home page's plan limits (usage-home spec §4). No DOM and no runtime imports,
// so Node can run these functions as they are (C:\tmp\mayak-spike\usage-format-check.mjs). The
// words come from `tr`, which the island passes as t() (core/i18n.ts).

import type { Usage } from "../core/bridge";
import type { Translate } from "../core/i18n-core";
import type { Key } from "../core/i18n";

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
  percent: { text: string; ring: number; tone: PercentTone };
  resets: string;
}

export interface UsageBlock {
  title: "Claude" | "Codex";
  lines: UsageLine[];
  /** How old the data are, shown under the title ("" when the block is empty). */
  updated: string;
  /** The hint of an empty block ("" otherwise). */
  note: string;
  empty: boolean;
}

const EMPTY_HINT = {
  Claude: "usage.hintClaude",
  Codex: "usage.hintCodex",
} as const;

/** The two blocks of the home page: Claude (dated by its cache) and Codex (dated by the last answer read). */
export function usageBlocks(usage: Usage | undefined, nowMs: number, tr: Tr): UsageBlock[] {
  const claude = usage?.claude ?? null;
  const codex = usage?.codex ?? null;
  return [
    block(
      "Claude",
      claude && claude.windows.map((w) => ({ name: claudeWindowName(w.id, tr), percent: w.percent, resetsAtMs: w.resetsAtMs })),
      claude?.fetchedAtMs ?? 0,
      nowMs,
      tr,
    ),
    block(
      "Codex",
      codex && codex.windows.map((w) => ({ name: codexWindowName(w.minutes, tr), percent: w.percent, resetsAtMs: w.resetsAtMs })),
      codex?.seenAtMs ?? 0,
      nowMs,
      tr,
    ),
  ];
}

function block(
  title: "Claude" | "Codex",
  windows: { name: string; percent: number; resetsAtMs?: number }[] | null,
  atMs: number,
  nowMs: number,
  tr: Tr,
): UsageBlock {
  if (!windows || windows.length === 0) return { title, lines: [], updated: "", note: tr(EMPTY_HINT[title]), empty: true };
  const lines = windows.map((w): UsageLine => {
    // A window whose reset time has passed no longer says how much is used: empty grey ring, "—".
    if (w.resetsAtMs !== undefined && w.resetsAtMs <= nowMs) {
      return { name: w.name, percent: { text: "—", ring: 0, tone: "dim" }, resets: tr("usage.waitingFresh") };
    }
    return {
      name: w.name,
      percent: usagePercent(w.percent),
      resets: w.resetsAtMs === undefined ? "" : resetsText(w.resetsAtMs, nowMs, tr),
    };
  });
  // Both blocks read the same: the age of their data under the title, then the windows.
  return { title, lines, updated: updatedText(atMs, nowMs, tr), note: "", empty: false };
}
