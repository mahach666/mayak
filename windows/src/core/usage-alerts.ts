// 80 % / 100 % plan-limit alerts (home page limits): which windows crossed a threshold since the
// last snapshot. Each window is remembered by provider and window ("claude:5h", "codex:300") with
// its reset time and the highest threshold already told; a window that rolled over (another reset
// time, or a percent lower than the remembered threshold) starts again from nothing. The first
// reading of a window (at start, or a provider that appears) is only remembered: what was crossed
// before Mayak saw it is not announced. Pure: scripts/checks/usage-alerts.mjs runs it in Node.

import type { Usage } from "./bridge";

export type Threshold = 80 | 100;
type Level = 0 | Threshold;

/** Two reset times closer than this are the same window (a re-read may round it differently). */
export const SAME_WINDOW_MS = 15 * 60_000;

export interface WindowReading {
  /** Provider and window: "claude:5h", "claude:7d:opus", "codex:300". */
  key: string;
  percent: number;
  resetsAtMs?: number;
}

interface Remembered {
  resetsAtMs?: number;
  level: Level;
}

export type AlertMemory = ReadonlyMap<string, Remembered>;

export interface Crossing {
  key: string;
  threshold: Threshold;
}

function levelOf(percent: number): Level {
  return percent >= 100 ? 100 : percent >= 80 ? 80 : 0;
}

/** The windows of the home page that alerts watch: Claude's and Codex's, in snapshot order. */
export function usageReadings(usage: Usage | null | undefined): WindowReading[] {
  const out: WindowReading[] = [];
  for (const w of usage?.claude?.windows ?? []) out.push({ key: `claude:${w.id}`, percent: w.percent, resetsAtMs: w.resetsAtMs });
  for (const w of usage?.codex?.windows ?? []) out.push({ key: `codex:${w.minutes}`, percent: w.percent, resetsAtMs: w.resetsAtMs });
  return out;
}

/**
 * One step: the new memory and the crossings to announce, at most one per window (the highest: a
 * jump from 50 to 100 is one 100 alert). A reading whose reset time has passed is old data and
 * changes nothing. Windows missing from `readings` keep their memory.
 */
export function stepAlerts(memory: AlertMemory, readings: WindowReading[], nowMs: number): { memory: AlertMemory; crossings: Crossing[] } {
  const next = new Map(memory);
  const crossings: Crossing[] = [];
  for (const r of readings) {
    if (!Number.isFinite(r.percent)) continue;
    if (r.resetsAtMs !== undefined && r.resetsAtMs <= nowMs) continue;
    const level = levelOf(r.percent);
    const prev = next.get(r.key);
    if (!prev) {
      next.set(r.key, { resetsAtMs: r.resetsAtMs, level });
      continue;
    }
    const otherReset =
      prev.resetsAtMs !== undefined && r.resetsAtMs !== undefined && Math.abs(r.resetsAtMs - prev.resetsAtMs) > SAME_WINDOW_MS;
    const rolled = otherReset || level < prev.level;
    const told = rolled ? 0 : prev.level;
    if (level > told) crossings.push({ key: r.key, threshold: level as Threshold });
    // The same window keeps the reset time it was first seen with, so small moves never add up.
    next.set(r.key, rolled ? { resetsAtMs: r.resetsAtMs, level } : { resetsAtMs: prev.resetsAtMs ?? r.resetsAtMs, level });
  }
  return { memory: next, crossings };
}
