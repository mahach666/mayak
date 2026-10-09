// The launch greeting "sparks, as from a lighter" (spec §4) as a pure function of time.
// Canvas 640×150, the same frame as the old greeting; island.ts sizes the island around it.

import { COMPACT_W } from "../core/layout.ts";

export const G = {
  open1: 0.35, strikes: [0.45, 0.85, 1.25] as const, catchAt: 1.3,
  rise0: 1.6, land: 2.2, eyes: 2.3, wave0: 2.45, wave1: 3.55, tuck0: 3.6, tuck1: 3.9,
  leave0: 3.9, end: 4.4, autoLeave: 4.9, COLLAPSE: 0.34,
};
export const GREET_END = G.end;
export const CANVAS = { w: 640, h: 150 };
export const CARD = { x: 10, y: 36, w: 620, h: 104 };
export const FLINT = { x: 320, y: 128 };
export const REST = { x: 320, y: 95, u: 170 };
export const SMALL = { x: 320 - COMPACT_W / 2 + 40, y: 16, u: 44 };

export interface Burst { t0: number; seed: number; n: number; x: number; y: number; power: number }
export const BURSTS: readonly Burst[] = [
  { t0: G.strikes[0], seed: 11, n: 8, x: FLINT.x, y: FLINT.y, power: 0.8 },
  { t0: G.strikes[1], seed: 12, n: 11, x: FLINT.x, y: FLINT.y, power: 0.9 },
  { t0: G.strikes[2], seed: 13, n: 16, x: FLINT.x, y: FLINT.y, power: 1.1 },
];

const clamp01 = (k: number) => Math.min(1, Math.max(0, k));
const seg = (t: number, a: number, b: number) => clamp01((t - a) / (b - a));
const lerp = (a: number, b: number, k: number) => a + (b - a) * k;
const easeOut = (k: number) => 1 - Math.pow(1 - clamp01(k), 3);
const easeInOut = (k: number) => { k = clamp01(k); return k < 0.5 ? 4 * k * k * k : 1 - Math.pow(-2 * k + 2, 3) / 2; };
const hash = (i: number) => { const s = Math.sin(i * 127.1 + 311.7) * 43758.5453; return s - Math.floor(s); };

export interface SparkPoint { x: number; y: number; px: number; py: number; a: number; heat: number }
const GRAV = 520;

export function sparkPoints(b: Burst, t: number): SparkPoint[] {
  const dt = t - b.t0;
  if (dt < 0) return [];
  const out: SparkPoint[] = [];
  for (let i = 0; i < b.n; i++) {
    const h1 = hash(b.seed * 97 + i), h2 = hash(b.seed * 31 + i * 7), h3 = hash(b.seed * 13 + i * 3);
    const ang = -Math.PI / 2 + (h1 - 0.5) * 1.5;
    const v = 260 * b.power * (0.45 + 0.55 * h2);
    const life = 0.55 * (0.6 + 0.4 * h3);
    if (dt > life) continue;
    const pos = (d: number) => [b.x + Math.cos(ang) * v * d, b.y + Math.sin(ang) * v * d + 0.5 * GRAV * d * d];
    const [x, y] = pos(dt), [px, py] = pos(Math.max(0, dt - 0.035));
    out.push({ x, y, px, py, a: 1 - dt / life, heat: 1 - dt / life });
  }
  return out;
}

export interface LuchPose { x: number; y: number; u: number; grow: number; sx: number; sy: number; eye: "closed" | "open" | "happy"; handsOut: number; waveT: number }
export interface GreetFrame {
  card: number; header: number; minis: number;
  luch: LuchPose | null; ember: { x: number; y: number; r: number } | null; bursts: readonly Burst[];
}

function play(t: number): GreetFrame {
  const card = easeOut(seg(t, 0, G.open1));
  const bursts = BURSTS.filter((b) => t >= b.t0 && t < b.t0 + 0.7);
  // the second strike leaves an ember that glows 0.2 s (spec §4) and dies
  const e2 = BURSTS[1].t0;
  const ember = t > e2 + 0.15 && t < e2 + 0.35 ? { x: FLINT.x + 6, y: FLINT.y, r: 3 * (1 - seg(t, e2 + 0.25, e2 + 0.35)) } : null;
  if (t < G.catchAt) return { card, header: card, minis: 0, luch: null, ember, bursts };

  // caught: a small flame breathes in at the flint, then rises to REST and lands with a squash
  const breathe = easeOut(seg(t, G.catchAt, G.rise0));
  const rise = easeInOut(seg(t, G.rise0, G.land));
  // the flame fades in over 0.12 s so it does not pop from nothing at the catch
  const grow = lerp((0.18 + 0.2 * breathe) * easeOut(seg(t, G.catchAt, G.catchAt + 0.12)), 1, rise);
  let x = REST.x, y = lerp(FLINT.y - 8, REST.y, rise), u = REST.u, sx = 1, sy = 1;
  if (t > G.rise0 && t < G.land) { const k = Math.sin(Math.PI * seg(t, G.rise0, G.land)); sx = 1 - 0.08 * k; sy = 1 + 0.12 * k; }
  if (t >= G.land && t < G.land + 0.5) {
    const k = seg(t, G.land, G.land + 0.5);
    const d = Math.exp(-6 * k) * Math.cos(k * Math.PI * 3);
    sx = 1 + 0.1 * d; sy = 1 - 0.12 * d;
  }
  // the happy hop after the wave
  if (t >= G.tuck0 && t < G.tuck1) y -= 10 * Math.sin(Math.PI * seg(t, G.tuck0, G.tuck1));
  // leave for the compact island
  const lv = easeInOut(seg(t, G.leave0, G.end));
  x = lerp(x, SMALL.x, lv); y = lerp(y, SMALL.y, lv); u = lerp(u, SMALL.u, lv);

  const eye = t < G.eyes ? "closed" : t >= G.wave0 - 0.1 && t < G.tuck1 ? "happy" : "open";
  const handsOut = t < G.wave0 ? 0 : t < G.wave0 + 0.28 ? easeOut((t - G.wave0) / 0.28) : t < G.tuck0 ? 1 : 1 - easeOut((t - G.tuck0) / (G.tuck1 - G.tuck0));
  const waveT = t >= G.wave0 + 0.15 && t < G.wave1 ? t - G.wave0 - 0.15 : -1;
  return {
    card: card * (1 - seg(t, G.leave0, G.leave0 + 0.2)), header: card * (1 - seg(t, G.leave0, G.leave0 + 0.1)),
    minis: easeOut(seg(t, G.end - 0.2, G.end)),
    luch: { x, y, u, grow, sx, sy, eye, handsOut: t >= G.tuck1 ? 0 : handsOut, waveT }, ember: null, bursts,
  };
}

/** The frame at t; from `tc` on it collapses from where it was to the compact pose in G.COLLAPSE. */
export function greetFrame(t: number, tc: number): GreetFrame {
  if (t < tc) return play(Math.min(t, G.end + 10));
  const a = play(tc);
  const e = easeInOut(seg(t, tc, tc + G.COLLAPSE));
  const luch = a.luch && {
    ...a.luch, grow: lerp(a.luch.grow, 1, e), x: lerp(a.luch.x, SMALL.x, e), y: lerp(a.luch.y, SMALL.y, e), u: lerp(a.luch.u, SMALL.u, e),
    sx: lerp(a.luch.sx, 1, e), sy: lerp(a.luch.sy, 1, e), handsOut: a.luch.handsOut * (1 - seg(t, tc, tc + 0.15)),
    waveT: -1, eye: "open" as const,
  };
  return {
    card: a.card * (1 - seg(t, tc, tc + 0.18)), header: a.header * (1 - seg(t, tc, tc + 0.1)),
    minis: lerp(a.minis, 1, easeOut(seg(t, tc, tc + G.COLLAPSE))), luch, ember: null,
    bursts: t > tc + G.COLLAPSE ? [] : a.bursts,
  };
}
