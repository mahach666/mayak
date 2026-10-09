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

/** The repeat button's next mode: off → all → one → off. */
export function nextRepeat(mode: MediaRepeat): MediaRepeat {
  return mode === "off" ? "all" : mode === "all" ? "one" : "off";
}

/** The redraw timer runs only while the row is on screen, playing, with a timeline. */
export function needsTick(onScreen: boolean, playing: boolean, timeline: MediaTimeline | null): boolean {
  return onScreen && playing && timeline != null && timeline.durationMs > 0;
}
