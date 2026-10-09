// The player's progress bar (ideas §35): where the track is now, from what Rust last reported
// (media::Timeline), and how its times read. Pure, so scripts/checks/player-time.mjs runs it.

import type { MediaRepeat, MediaTimeline } from "./bridge";

/** How often the bar and the elapsed time are redrawn while the player is on screen and playing. */
export const PLAYER_TICK_MS = 500;

/**
 * The position (ms) at `nowMs`: run on from `updatedAt` while playing (like upstream's
 * spotifyPosition), frozen while paused, never before 0 nor past the end.
 */
export function mediaPosition(t: MediaTimeline, playing: boolean, nowMs: number): number {
  const elapsed = playing ? Math.max(0, nowMs - t.updatedAt) : 0;
  return Math.min(Math.max(0, t.positionMs + elapsed), Math.max(0, t.durationMs));
}

/** The share of the track played, 0…1. */
export function mediaProgress(t: MediaTimeline, playing: boolean, nowMs: number): number {
  return t.durationMs > 0 ? mediaPosition(t, playing, nowMs) / t.durationMs : 0;
}

/** m:ss, or h:mm:ss from an hour on; whole seconds, rounded down. */
export function formatTime(ms: number): string {
  const s = Math.max(0, Math.floor(Number.isFinite(ms) ? ms / 1000 : 0));
  const pad = (n: number) => String(n).padStart(2, "0");
  return s >= 3600
    ? `${Math.floor(s / 3600)}:${pad(Math.floor((s % 3600) / 60))}:${pad(s % 60)}`
    : `${Math.floor(s / 60)}:${pad(s % 60)}`;
}

/** Where a press at `x` on a bar spanning `left`…`left + width` asks to go, in ms. */
export function seekTarget(x: number, left: number, width: number, durationMs: number): number {
  if (!(width > 0) || !(durationMs > 0)) return 0;
  const share = Math.min(1, Math.max(0, (x - left) / width));
  return Math.round(share * durationMs);
}

// ── The hold after a seek ──
//
// A player answers a seek with a report or two that still show the old place before the new one:
// drawn as they come, the bar would jump back on release, then forward. After a seek the bar holds
// the target (running on from it while playing) until a report lands near it, or SEEK_HOLD_MS pass.

/** The longest the bar holds a seek's target without the player confirming it. */
export const SEEK_HOLD_MS = 3000;
/** A report this close to the held position confirms the seek. */
export const SEEK_CONFIRM_MS = 1500;

export interface SeekHold {
  targetMs: number;
  /** When the seek was asked for (Date.now()). */
  at: number;
  /** The track it was asked on (trackKey). */
  track: string;
}

/** One track of one app: a hold does not outlive it. */
export function trackKey(s: { title: string; artist: string; app: string }): string {
  return `${s.title}\u001f${s.artist}\u001f${s.app}`;
}

export function startHold(targetMs: number, nowMs: number, track: string): SeekHold {
  return { targetMs, at: nowMs, track };
}

/** Where the held seek is now: the target, run on since it was asked while playing, capped at the end. */
export function holdPosition(h: SeekHold, t: MediaTimeline, playing: boolean, nowMs: number): number {
  const elapsed = playing ? Math.max(0, nowMs - h.at) : 0;
  return Math.min(Math.max(0, h.targetMs + elapsed), Math.max(0, t.durationMs));
}

/**
 * The hold after a report (`t` on track `track`): kept while the report still shows somewhere
 * else; dropped once it lands within SEEK_CONFIRM_MS of the held position, when SEEK_HOLD_MS have
 * passed, when the track changed or when there is no timeline any more.
 */
export function settleHold(
  h: SeekHold | null, track: string, t: MediaTimeline | null, playing: boolean, nowMs: number,
): SeekHold | null {
  if (!h || !t || h.track !== track || nowMs - h.at >= SEEK_HOLD_MS) return null;
  const confirmed = Math.abs(mediaPosition(t, playing, nowMs) - holdPosition(h, t, playing, nowMs)) <= SEEK_CONFIRM_MS;
  return confirmed ? null : h;
}

/** The position the bar shows: the held seek while it lasts, else the player's. */
export function shownPosition(h: SeekHold | null, t: MediaTimeline, playing: boolean, nowMs: number): number {
  return h && nowMs - h.at < SEEK_HOLD_MS ? holdPosition(h, t, playing, nowMs) : mediaPosition(t, playing, nowMs);
}

/** The repeat button's next mode: off → all → one → off. */
export function nextRepeat(mode: MediaRepeat): MediaRepeat {
  return mode === "off" ? "all" : mode === "all" ? "one" : "off";
}

/** The redraw timer runs only while the row is on screen, playing, with a timeline. */
export function needsTick(onScreen: boolean, playing: boolean, timeline: MediaTimeline | null): boolean {
  return onScreen && playing && timeline != null && timeline.durationMs > 0;
}
