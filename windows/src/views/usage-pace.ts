// Pace of a plan-limit window (home page): how far the percent used is ahead of or behind the share
// of the window that has gone by, and when the current rate would use it all up. Pure: no DOM and
// no runtime imports, so scripts/checks/usage-pace.mjs runs it in Node.

import type { Translate } from "../core/i18n-core";
import type { Key } from "../core/i18n";

type Tr = Translate<Key>;

/** Too early to tell: under this share of the window gone by, no pace is shown. */
export const MIN_ELAPSED_SHARE = 0.05;
/** Within this many points of the elapsed share the pace is "on pace" and says nothing. */
export const PACE_QUIET_POINTS = 5;

export interface Pace {
  /** Percent used minus percent of the window gone by, rounded: > 0 ahead, < 0 in reserve. */
  delta: number;
  /** At the current rate the window is used up this long from now, before it resets; else null. */
  runsOutInMs: number | null;
}

/**
 * The pace of a window of `windowMs` that resets at `resetsAtMs`, `percent` used at `nowMs`. Null
 * without a reset time, once it has passed, for a full window, or before MIN_ELAPSED_SHARE of it.
 */
export function windowPace(percent: number, resetsAtMs: number | undefined, windowMs: number, nowMs: number): Pace | null {
  if (resetsAtMs === undefined || resetsAtMs <= nowMs || !(windowMs > 0) || !Number.isFinite(percent)) return null;
  const used = Math.max(0, Math.min(100, percent)) / 100;
  if (used >= 1) return null;
  const elapsedMs = Math.max(0, Math.min(windowMs, nowMs - (resetsAtMs - windowMs)));
  const elapsed = elapsedMs / windowMs;
  if (elapsed < MIN_ELAPSED_SHARE) return null;
  const delta = Math.round((used - elapsed) * 100);
  let runsOutInMs: number | null = null;
  if (used > 0) {
    const toFull = ((1 - used) * elapsedMs) / used;
    if (nowMs + toFull < resetsAtMs) runsOutInMs = toFull;
  }
  return { delta, runsOutInMs };
}

/** A window's length from its Claude wire id ("5h", "7d", "7d:opus"); 0 for an id it cannot read. */
export function claudeWindowMs(id: string): number {
  const m = /^(\d+)([hd])(?::.+)?$/.exec(id);
  if (!m) return 0;
  return Number(m[1]) * (m[2] === "h" ? 3_600_000 : 86_400_000);
}

/** "~1h 40m", "~12m", "~2d 3h": an estimate, to the nearest minute and at least one. */
function aboutText(ms: number, tr: Tr): string {
  const minutes = Math.max(1, Math.round(ms / 60_000));
  const hours = Math.floor(minutes / 60);
  const time =
    minutes < 60 ? tr("unit.m", { n: minutes })
    : hours < 24 ? tr("unit.hm", { h: hours, m: minutes % 60 })
    : tr("unit.dh", { d: Math.floor(hours / 24), h: hours % 24 });
  return `~${time}`;
}

/** What the row shows of a pace: a short `text`, and the whole of it for the tooltip (`title`). */
export interface PaceLabel {
  text: string;
  title: string;
}

/**
 * The muted label of a pace. Ahead of the elapsed share by PACE_QUIET_POINTS or more, the current
 * rate always uses the window up before it resets (used > elapsed share), so the row says when:
 * "out in ~1h 40m", the tooltip "+12% ahead · runs out in ~1h 40m". In reserve by as much:
 * "reserve 20%". Within the band (on pace), or without a pace: nothing ("").
 */
export function paceLabel(p: Pace | null, tr: Tr): PaceLabel {
  if (!p) return { text: "", title: "" };
  if (p.delta >= PACE_QUIET_POINTS) {
    const ahead = tr("usage.ahead", { n: p.delta });
    if (p.runsOutInMs === null) return { text: ahead, title: ahead };
    const time = aboutText(p.runsOutInMs, tr);
    return { text: tr("usage.outIn", { time }), title: `${ahead} · ${tr("usage.runsOut", { time })}` };
  }
  if (p.delta <= -PACE_QUIET_POINTS) {
    const reserve = tr("usage.reserve", { n: -p.delta });
    return { text: reserve, title: reserve };
  }
  return { text: "", title: "" };
}
