// The "auto-compaction soon" notice (ideas §6): a session whose context crosses CONTEXT_ALERT_PERCENT
// of its window is told once, quietly. Each session is remembered by key with whether it is at or
// above the threshold; it is told again only after its context fell below (a compaction, /clear).
// The first reading of a session (at start, or a session that appears) is only remembered: what
// was crossed before Mayak saw it is not announced. Only sessions whose agent writes the window
// count; without one there is no percent. Pure: scripts/checks/context-alerts.mjs runs it in Node.

import type { ContextFill, SessionsSnapshot } from "./bridge";

/** Claude Code auto-compacts a 1M window at about 967k (96-97 %): 85 % leaves a few turns. */
export const CONTEXT_ALERT_PERCENT = 85;

export interface ContextReading {
  key: string;
  percent: number;
}

/** key → at or above the threshold at the last reading. */
export type ContextMemory = ReadonlyMap<string, boolean>;

/** floor(used / window × 100) held within 0-100, computed on integers; null without a window. */
export function contextPercent(c: ContextFill | undefined): number | null {
  if (!c || c.window === undefined || !(c.window > 0)) return null;
  return Math.min(100, Math.floor((c.used * 100) / c.window));
}

/** The context is at or above the notice threshold. */
export function nearAutoCompact(c: ContextFill | undefined): boolean {
  const p = contextPercent(c);
  return p !== null && p >= CONTEXT_ALERT_PERCENT;
}

/** The sessions of a snapshot that have a percent, in snapshot order; silent rows are no sessions. */
export function contextReadings(s: SessionsSnapshot | null | undefined): { readings: ContextReading[]; keys: Set<string> } {
  const readings: ContextReading[] = [];
  const keys = new Set<string>();
  for (const project of s?.projects ?? []) {
    for (const session of project.sessions) {
      if (session.silent) continue;
      keys.add(session.key);
      const percent = contextPercent(session.context);
      if (percent !== null) readings.push({ key: session.key, percent });
    }
  }
  return { readings, keys };
}

/**
 * One step: the new memory and the keys to tell. A session gone from the snapshot (`present`) is
 * forgotten; one present without a reading keeps its memory.
 */
export function stepContextAlerts(
  memory: ContextMemory,
  readings: ContextReading[],
  present: ReadonlySet<string>,
): { memory: ContextMemory; crossed: string[] } {
  const next = new Map<string, boolean>();
  for (const [key, above] of memory) if (present.has(key)) next.set(key, above);
  const crossed: string[] = [];
  for (const r of readings) {
    if (!Number.isFinite(r.percent)) continue;
    const above = r.percent >= CONTEXT_ALERT_PERCENT;
    const prev = next.get(r.key);
    if (prev === false && above) crossed.push(r.key);
    next.set(r.key, above);
  }
  return { memory: next, crossed };
}

/** Local "HH:MM" of a time in ms, for the "compacted" mark. */
export function clockTime(ms: number, locale?: string): string {
  return new Date(ms).toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" });
}
