// Reminders for sessions that wait too long (a permission or a question nobody answered): the pure
// part, with no DOM, no Tauri and no State, so it can be checked on its own. The island keeps one
// one-shot timer at the next reminder due, and only while such a wait exists.

import type { SessionsSnapshot } from "../core/bridge";
import { shouldOpenForAlert } from "./focus.ts";

/** What Settings offers, in minutes; 0 is off. */
export const WAIT_REMINDER_CHOICES = [0, 3, 10, 30] as const;
export const DEFAULT_WAIT_REMINDER = 10;
/** Reminders per wait: after N, 2N and 3N minutes, then the wait is left to the tray's number. */
export const MAX_WAIT_REMINDERS = 3;

/** A setting outside WAIT_REMINDER_CHOICES (a hand-edited file) acts as the default. */
export function reminderMinutes(value: unknown): number {
  return (WAIT_REMINDER_CHOICES as readonly unknown[]).includes(value) ? (value as number) : DEFAULT_WAIT_REMINDER;
}

/** One unanswered wait of the snapshot. */
export interface WaitEpisode {
  key: string;
  kind: "permission" | "input";
  /** The wait's start (SessionView.since); a new wait of the same session has a later one. */
  since: number;
  /** The project's display name, for the pomodoro summary. */
  project: string;
}

/**
 * The waits a reminder is for: rows waiting for a permission or an answer, the rows the tray
 * counts. A wait stays one while the session waits, also once its card was dismissed or its
 * window opened (marked seen): a dismissed card is the case reminders are for. A wait the agent
 * reports as neither a permission nor a question counts as a question. A silent row never is one.
 */
export function waitingEpisodes(snapshot: SessionsSnapshot | null): WaitEpisode[] {
  const out: WaitEpisode[] = [];
  for (const project of snapshot?.projects ?? []) {
    for (const s of project.sessions) {
      if (s.silent || s.phase.kind !== "waiting") continue;
      const kind = s.phase.wait === "permission" ? "permission" : "input";
      out.push({ key: s.key, kind, since: s.since, project: project.name });
    }
  }
  return out;
}

/** Reminders already counted per wait, by episodeId. */
export type ReminderMemory = ReadonlyMap<string, number>;

/**
 * A wait is known by its session and start. When the agent reports the same wait again with a
 * later start (the store keeps the latest), it is a new episode: its reminders count from there.
 */
export function episodeId(e: WaitEpisode): string {
  return `${e.key}@${e.since}`;
}

/** How many reminders a wait has earned by `now`, at most `max`. */
function earned(e: WaitEpisode, now: number, intervalMs: number, max: number): number {
  return Math.max(0, Math.min(max, Math.floor((now - e.since) / intervalMs)));
}

/**
 * One look at the waits at `now`. A wait seen for the first time counts what it has already
 * earned without a reminder (a page loaded on an old wait does not ring at once); a wait that
 * earned more since the last look is due, once, however many it earned meanwhile. Waits that are
 * gone are forgotten. `intervalMs` <= 0 (off) forgets everything.
 */
export function stepReminders(
  memory: ReminderMemory,
  episodes: readonly WaitEpisode[],
  now: number,
  intervalMs: number,
  max = MAX_WAIT_REMINDERS,
): { memory: Map<string, number>; due: WaitEpisode[] } {
  const next = new Map<string, number>();
  const due: WaitEpisode[] = [];
  if (intervalMs <= 0) return { memory: next, due };
  for (const e of episodes) {
    const id = episodeId(e);
    const count = earned(e, now, intervalMs, max);
    const before = memory.get(id);
    if (before !== undefined && count > before) due.push(e);
    next.set(id, Math.max(count, before ?? 0));
  }
  return { memory: next, due };
}

/**
 * When the next reminder falls due: the earliest `since + (counted + 1) × interval` of the waits
 * that have reminders left. Null when none has (no timer at all).
 */
export function nextReminderAt(
  memory: ReminderMemory,
  episodes: readonly WaitEpisode[],
  intervalMs: number,
  max = MAX_WAIT_REMINDERS,
): number | null {
  if (intervalMs <= 0) return null;
  let at: number | null = null;
  for (const e of episodes) {
    const counted = memory.get(episodeId(e)) ?? 0;
    if (counted >= max) continue;
    const due = e.since + (counted + 1) * intervalMs;
    if (at === null || due < at) at = due;
  }
  return at;
}

/** The most pressing of the due waits for the sound: a permission before a question, then the oldest. */
export function loudest(due: readonly WaitEpisode[]): WaitEpisode | null {
  let best: WaitEpisode | null = null;
  for (const e of due) {
    if (
      best === null
      || (e.kind === "permission" && best.kind !== "permission")
      || (e.kind === best.kind && e.since < best.since)
    ) best = e;
  }
  return best;
}

/** What the island knows when reminders fall due. */
export interface ReminderGate {
  paused: boolean;
  /** The session of the permission or question card on screen, which already stands for its wait. */
  cardKey: string | null;
  /** A pomodoro runs. */
  inPomodoro: boolean;
  /** A work interval with Mute on. */
  muting: boolean;
  /** Settings: waits get through a muted interval. */
  letWaits: boolean;
  /** The user answers or types in the expanded island. */
  viewInUse: boolean;
}

/**
 * What due reminders do. Paused: nothing (the reminders are spent). The card's own session is
 * left out. In a muted pomodoro interval, the waits that may not get through go into the phase
 * summary (`hold`). The sound is for the most pressing wait that may get through, unless the user
 * is typing in the island; `fresh` is how many due waits were left after the card's.
 */
export function planReminder(
  due: readonly WaitEpisode[],
  g: ReminderGate,
): { hold: WaitEpisode[]; sound: WaitEpisode | null; fresh: number } {
  if (g.paused) return { hold: [], sound: null, fresh: 0 };
  const fresh = due.filter((e) => e.key !== g.cardKey);
  const opens = (e: WaitEpisode, muting: boolean) => shouldOpenForAlert(e.kind, { muting, paused: false, letWaits: g.letWaits });
  const hold = g.inPomodoro && g.muting ? fresh.filter((e) => !opens(e, true)) : [];
  const top = loudest(fresh.filter((e) => opens(e, g.muting)));
  return { hold, sound: top && !g.viewInUse ? top : null, fresh: fresh.length };
}
