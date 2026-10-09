// Fire palettes for Луч (spec §1, §3.1). Pure: no DOM, no WebGL.

export type Hex = string;
export interface FirePalette { deep: Hex; mid: Hex; light: Hex; core: Hex }

/** The idle fire, read off the reference (spec §1). */
export const IDLE_FIRE: FirePalette = { deep: "#e0380a", mid: "#ff7410", light: "#ffbc34", core: "#fff0c2" };

const toRgb = (h: Hex): [number, number, number] => {
  const v = parseInt(h.replace("#", ""), 16);
  return [(v >> 16) & 255, (v >> 8) & 255, v & 255];
};
const toHex = (c: readonly number[]): Hex =>
  "#" + c.map((v) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, "0")).join("");

export function mixHex(a: Hex, b: Hex, k: number): Hex {
  const A = toRgb(a), B = toRgb(b);
  return toHex(A.map((v, i) => v + (B[i] - v) * k));
}

/** Each colour of a fire mixed from `a` towards `b` by k (0 = a, 1 = b). */
export function mixFire(a: FirePalette, b: FirePalette, k: number): FirePalette {
  if (k <= 0) return a;
  if (k >= 1) return b;
  return { deep: mixHex(a.deep, b.deep, k), mid: mixHex(a.mid, b.mid, k), light: mixHex(a.light, b.light, k), core: mixHex(a.core, b.core, k) };
}

/** Components 0…1, as `BotEngine` keeps its colours. */
export function rgbToHex(c: readonly [number, number, number]): Hex {
  return toHex(c.map((v) => v * 255));
}

/** The idle mid colour as 0…1 — the engine's idle state colour, so its colour tween never passes Mochi's grey. */
export const IDLE_FIRE_RGB: readonly [number, number, number] = [0xff / 255, 0x74 / 255, 0x10 / 255];

/** Spec §3.1: a state or pill colour → its fire; `null` → the idle fire. */
export function paletteFor(c: Hex | null): FirePalette {
  if (c == null) return IDLE_FIRE;
  const m = c.toLowerCase();
  return { deep: mixHex(m, "#000000", 0.25), mid: m, light: mixHex(m, "#ffffff", 0.45), core: mixHex("#fffbf2", m, 0.08) };
}

/** The engine's animated colour → its fire. Within 1/255 of the idle colour it is the idle fire. */
export function paletteForRGB(c: readonly [number, number, number]): FirePalette {
  const near = c.every((v, i) => Math.abs(v - IDLE_FIRE_RGB[i]) < 1 / 255);
  return near ? IDLE_FIRE : paletteFor(rgbToHex(c));
}
