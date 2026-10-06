// Pure texts of the home page's plan limits (usage-home spec §4). No DOM and no runtime imports,
// so Node can run these functions as they are (C:\tmp\mayak-spike\usage-format-check.mjs).

import type { Usage } from "../core/bridge";

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
export function codexWindowName(minutes: number): string {
  if (minutes < 60) return `${minutes}m`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)}h`;
  return `${Math.floor(minutes / 1440)}d`;
}

/** "resets in 2h 14m", "resets in 3d 4h", "resets in 12m", "resets soon" under a minute, "reset" once the time has passed. */
export function resetsText(resetsAtMs: number, nowMs: number): string {
  const left = resetsAtMs - nowMs;
  if (left <= 0) return "reset";
  const minutes = Math.floor(left / 60_000);
  if (minutes < 1) return "resets soon";
  if (minutes < 60) return `resets in ${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `resets in ${hours}h ${minutes % 60}m`;
  return `resets in ${Math.floor(hours / 24)}d ${hours % 24}h`;
}

/** "7h ago", "12m ago", "3d ago" (age rounded down); "" under a minute. */
function agoText(age: number): string {
  const minutes = Math.floor(age / 60_000);
  if (minutes < 1) return "";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

/** The age of a block's data, always shown: "updated 6h ago", "updated just now" under a minute (or in the future). */
export function updatedText(atMs: number, nowMs: number): string {
  const ago = agoText(nowMs - atMs);
  return ago === "" ? "updated just now" : `updated ${ago}`;
}

/** A Claude window's label from its wire id: "5h" and "7d" as they are, "7d:opus" → "7d Opus". */
export function claudeWindowName(id: string): string {
  if (!id.startsWith("7d:")) return id;
  const model = id.slice(3);
  return `7d ${model.charAt(0).toUpperCase()}${model.slice(1)}`;
}

/** The right column of a window whose reset time has passed: its percent is of the old window. */
export const WAITING_TEXT = "reset — waiting for fresh data";

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
  Claude: "Claude Code updates it while it runs",
  Codex: "Codex reports it with each answer",
} as const;

/** The two blocks of the home page: Claude (dated by its cache) and Codex (dated by the last answer read). */
export function usageBlocks(usage: Usage | undefined, nowMs: number): UsageBlock[] {
  const claude = usage?.claude ?? null;
  const codex = usage?.codex ?? null;
  return [
    block(
      "Claude",
      claude && claude.windows.map((w) => ({ name: claudeWindowName(w.id), percent: w.percent, resetsAtMs: w.resetsAtMs })),
      claude?.fetchedAtMs ?? 0,
      nowMs,
    ),
    block(
      "Codex",
      codex && codex.windows.map((w) => ({ name: codexWindowName(w.minutes), percent: w.percent, resetsAtMs: w.resetsAtMs })),
      codex?.seenAtMs ?? 0,
      nowMs,
    ),
  ];
}

function block(
  title: "Claude" | "Codex",
  windows: { name: string; percent: number; resetsAtMs?: number }[] | null,
  atMs: number,
  nowMs: number,
): UsageBlock {
  if (!windows || windows.length === 0) return { title, lines: [], updated: "", note: EMPTY_HINT[title], empty: true };
  const lines = windows.map((w): UsageLine => {
    // A window whose reset time has passed no longer says how much is used: empty grey ring, "—".
    if (w.resetsAtMs !== undefined && w.resetsAtMs <= nowMs) {
      return { name: w.name, percent: { text: "—", ring: 0, tone: "dim" }, resets: WAITING_TEXT };
    }
    return {
      name: w.name,
      percent: usagePercent(w.percent),
      resets: w.resetsAtMs === undefined ? "" : resetsText(w.resetsAtMs, nowMs),
    };
  });
  // Both blocks read the same: the age of their data under the title, then the windows.
  return { title, lines, updated: updatedText(atMs, nowMs), note: "", empty: false };
}
