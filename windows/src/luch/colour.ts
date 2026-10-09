// Луч's colour changes as a fire would (ideas §33), never as a recolour. Three looks, switchable
// (colourVariant), all drawn by the shader (fire.frag.glsl, the `if (uColorK < 1.0)` block) from the
// "from" palette, the "to" palette and the eased progress this module keeps:
// - "licks": the new colour arrives through the fire's own rising noise: first in the hot core near
//   the base, then in ragged tongues that rise and merge; the rim and the tongue's tip change last;
// - "flare": like powder thrown into the fire: the flame flares (taller, brighter, a whiter heart, a few
//   sparks), the colour swaps inside the flash through the same tongues, then it settles;
// - "core": the new colour is born in the bright heart and spreads out through the body to the rim and
//   up the tongue, the old colour lingering at the edge and the tip like an afterglow.
// colourWave and colourFlash mirror the shader's numbers for the checks. The 2D fallback and the embers
// use colourBlend, the uniform mix at the same progress. Imports only palette.ts (no DOM): the Node
// checks load it as is.
//
// Changes in a row never jump, as a half-done change cannot be baked into one palette:
// - back to where the change came from (an emote's tint ending before its change is done, idle →
//   working → idle): the same change runs backwards from where it is;
// - to a third colour: the running change hurries (COLOUR_HURRY times faster) to its end and the third
//   one starts from there; only the newest such colour is kept, so a burst ends in one more change.

import { mixFire, type FirePalette } from "./palette.ts";

/** How long one change takes (seconds), eased in and out. */
export const COLOUR_S = 1.0;
/**
 * How long an emote's tint takes to come in or go (seconds): an emote lasts 0.8 s (annoyed) to 1.8 s
 * (love), so its tint must be complete well inside it.
 */
export const EMOTE_COLOUR_S = 0.35;

export type ColourVariant = "licks" | "flare" | "core";
export const COLOUR_VARIANTS: readonly ColourVariant[] = ["licks", "flare", "core"];
/** The shader's uColorV for a variant. */
export const variantIndex = (v: ColourVariant): number => COLOUR_VARIANTS.indexOf(v);

let variant: ColourVariant = "flare";
/** The look every Луч's colour change takes (the dev preview switches it). */
export function colourVariant(): ColourVariant { return variant; }
export function setColourVariant(v: ColourVariant) { variant = v; }
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

/**
 * One step on towards `wanted` (the fire's colour for the state and emote now), a change taking
 * `seconds` (COLOUR_S, or EMOTE_COLOUR_S for an emote's tint).
 */
export function stepColour(s: ColourFade, wanted: FirePalette, dt: number, seconds = COLOUR_S): ColourFade {
  const d = Math.max(0, dt) / Math.max(0.05, seconds);
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

/** The shader's flicker noise (nn) lies in 0…NOISE_MAX. */
export const NOISE_MAX = 0.92;

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
const height = (y: number) => clamp01((y + 0.44) / 1.1);
/** licks and core sweep eased out, so the new colour shows at once rather than after a dead start. */
const easeOut = (k: number) => 1 - (1 - clamp01(k)) * (1 - clamp01(k));

/**
 * Mirror of the shader: where the new colour has reached at a box point (x, y; y up, the base at -0.44)
 * for eased progress k: > 0 caught. `n` stands for the shader's rising flicker noise, 0…NOISE_MAX.
 * licks: the height, a little later away from the hot core above the base, strongly torn by the noise
 * (so the boundary is the flame's own tongues, never a line). core: the distance from the heart, a
 * little later higher up, torn by the noise.
 */
export function colourFront(v: ColourVariant, k: number, x: number, y: number, n: number): number {
  if (v === "core") {
    const r = Math.min(Math.hypot((x - 0.03) / 0.4, (y + 0.1) / 0.45), 1.4);
    return -0.3 + 1.66 * easeOut(k) - (0.62 * r + 0.18 * height(y) + 0.4 * (n - 0.45));
  }
  const kk = v === "flare" ? sm(0.04, 0.3, k) : easeOut(k);
  const r = Math.min(Math.hypot((x - 0.02) / 0.45, (y + 0.2) / 0.35), 1.2);
  return -0.32 + 1.4 * kk - (0.45 * height(y) + 0.25 * r + 0.5 * (n - 0.45));
}

/** Mirror of the shader: how much of the new colour a point shows, 0 (from) … 1 (to). */
export function colourWave(v: ColourVariant, k: number, x: number, y: number, n: number): number {
  const f = colourFront(v, k, x, y, n);
  return v === "core" ? sm(-0.1, 0.1, f) : sm(-0.06, 0.06, f);
}

/** Mirror of the shader: the flare's swell (taller, brighter) at eased progress k: fast up, slow down. */
export function colourSwell(v: ColourVariant, k: number): number {
  return v === "flare" ? sm(0, 0.12, k) * (1 - sm(0.12, 1, k)) : 0;
}

/**
 * Mirror of the shader: how much paler a point burns: along the boundary for licks and core, the whole
 * flame in the flare's flash. 0 at the start and the end of a change.
 */
export function colourFlash(v: ColourVariant, k: number, x: number, y: number, n: number): number {
  if (v === "flare") return 0.5 * colourSwell(v, k);
  const f = Math.abs(colourFront(v, k, x, y, n)), s = Math.sin(Math.PI * clamp01(k));
  return v === "core" ? 0.4 * (1 - sm(0, 0.12, f)) * s : 0.5 * (1 - sm(0, 0.08, f)) * s;
}
