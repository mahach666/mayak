// Луч's colour changes as a fire would (ideas §33): the new colour catches at the flame's base and
// climbs up it as a wave, the tongue's tip last, with a short bright flare along the moving boundary.
// The shader (fire.frag.glsl, the `if (uColorK < 1.0)` block) draws the wave from the "from" palette,
// the "to" palette and the eased progress this module keeps; colourWave and colourFlare mirror its
// numbers for the checks. The 2D fallback and the embers use colourBlend, the uniform mix at the same
// progress. Imports only palette.ts (no DOM): the Node checks load it as is.
//
// Changes in a row never jump, as a wave cannot be baked into one palette:
// - back to where the change came from (an emote's tint ending before its wave is done, idle → working
//   → idle): the same wave runs backwards from where it is;
// - to a third colour: the running wave hurries (COLOUR_HURRY times faster) to its end and the third
//   one starts from there; only the newest such colour is kept, so a burst ends in one more wave.

import { mixFire, type FirePalette } from "./palette.ts";

/** How long one change takes (seconds), eased in and out. */
export const COLOUR_S = 1.0;
/** How much faster a running change ends while a newer colour waits. */
export const COLOUR_HURRY = 2.5;

/**
 * A colour change: the wave from `from` to `to`, `level` its linear progress 0…1; `back` while it runs
 * back towards `from`; `next` the newest colour waiting for it to end. Settled: `from` and `to` alike,
 * level 1, nothing back or waiting.
 */
export interface ColourFade {
  readonly from: FirePalette;
  readonly to: FirePalette;
  readonly level: number;
  readonly back: boolean;
  readonly next: FirePalette | null;
}

export function samePalette(a: FirePalette, b: FirePalette): boolean {
  return a.deep === b.deep && a.mid === b.mid && a.light === b.light && a.core === b.core;
}

/** A fire settled in `p`. */
export function colourAt(p: FirePalette): ColourFade {
  return { from: p, to: p, level: 1, back: false, next: null };
}

export function colourSettled(s: ColourFade): boolean {
  return !s.back && s.next == null && s.level >= 1;
}

/** The colour the fire is heading for. */
export function colourHeading(s: ColourFade): FirePalette {
  return s.next ?? (s.back ? s.from : s.to);
}

/** One step on towards `wanted` (the fire's colour for the state and emote now). */
export function stepColour(s: ColourFade, wanted: FirePalette, dt: number): ColourFade {
  const d = Math.max(0, dt) / COLOUR_S;
  if (colourSettled(s)) {
    if (samePalette(wanted, s.to)) return s;
    return advance({ from: s.to, to: wanted, level: 0, back: false, next: null }, d);
  }
  const target = s.back ? s.from : s.to, origin = s.back ? s.to : s.from;
  let { back, next } = s;
  if (samePalette(wanted, target)) next = null;
  else if (samePalette(wanted, origin)) { back = !back; next = null; }
  else next = wanted;
  return advance({ ...s, back, next }, next ? d * COLOUR_HURRY : d);
}

function advance(s: ColourFade, d: number): ColourFade {
  const level = s.level + (s.back ? -d : d);
  if (!s.back && level >= 1) {
    return s.next ? { from: s.to, to: s.next, level: 0, back: false, next: null } : colourAt(s.to);
  }
  if (s.back && level <= 0) {
    return s.next ? { from: s.from, to: s.next, level: 0, back: false, next: null } : colourAt(s.from);
  }
  return { ...s, level };
}

/** The eased progress the shader gets (uColorK): 1 when settled, the fire exactly as before. */
export function colourK(s: ColourFade): number {
  if (colourSettled(s)) return 1;
  const k = Math.min(1, Math.max(0, s.level));
  return k * k * (3 - 2 * k);
}

/** The uniform mix at the eased progress: the 2D fallback's fire and the embers' colour. */
export function colourBlend(s: ColourFade): FirePalette {
  return mixFire(s.from, s.to, colourK(s));
}

const sm = (e0: number, e1: number, x: number) => { const t = Math.min(1, Math.max(0, (x - e0) / (e1 - e0))); return t * t * (3 - 2 * t); };

/**
 * Mirror of the shader: how far a box point (x, y; y up, the base at -0.44) has caught the new colour
 * at eased progress k, 0 (from) … 1 (to). `n` stands for the shader's flicker noise, 0…0.875. The height
 * up the flame plus a little for the sides: the centre catches first, the tongue's tip last.
 */
export function colourFront(k: number, x: number, y: number, n: number): number {
  const hc = Math.min(1, Math.max(0, (y + 0.44) / 1.1)) + 0.22 * Math.min(Math.abs(x), 1) + 0.3 * (n - 0.5);
  return -0.3 + 1.8 * Math.min(1, Math.max(0, k)) - hc;
}

export function colourWave(k: number, x: number, y: number, n: number): number {
  return sm(-0.1, 0.1, colourFront(k, x, y, n));
}

/** Mirror of the shader: the bright flare along the boundary, 0 at the start and the end of a change. */
export function colourFlare(k: number, x: number, y: number, n: number): number {
  return (1 - sm(0, 0.14, Math.abs(colourFront(k, x, y, n)))) * Math.sin(Math.PI * Math.min(1, Math.max(0, k)));
}
