// The launch greeting "sparks, as from a lighter" (spec §4) as a pure function of time.
// Canvas 640×150, the same frame as the old greeting; island.ts sizes the island around it.
// Луч is not scaled into being: the strikes happen where he will stand, the third strike's sparks
// curl back into that point, and the fire catches there inside his full-size silhouette (uIgnite).
// Every move is eased: anticipation before it, overshoot or a squash after it, nothing linear.

import { COMPACT_W, NOTCH_H, NOTCH_W } from "../core/layout.ts";

export const G = {
  open1: 0.35,
  /** Flicked like a lighter by hand: chk … chk-chk (uneven on purpose). */
  strikes: [0.5, 1.04, 1.2] as const,
  /** The last of the third strike's sparks lands on the base: a coal starts to glow there. */
  catchAt: 1.45,
  /** After a struggle (it nearly dies once) the little flame really takes. */
  caught: 2.1,
  /** Fully alight (the tongue's tip catches last): the fire is the normal one from here on. */
  ignite1: 3.1,
  /** The face has come in over the settled fire; the eyes start a slow, sleepy opening. */
  eyes: 3.35,
  wave0: 3.72, wave1: 4.5, tuck0: 4.55, tuck1: 4.85,
  leave0: 4.85, end: 5.4, autoLeave: 5.9, COLLAPSE: 0.34,
};
export const GREET_END = G.end;
export const CANVAS = { w: 640, h: 150 };
export const CARD = { x: 10, y: 36, w: 620, h: 104 };
/** The notch strip the card unfolds from, and the compact island it folds back into. */
const STRIP = { x: 320 - NOTCH_W / 2, y: 0, w: NOTCH_W, h: NOTCH_H };
const COMPACT = { x: 320 - COMPACT_W / 2, y: 0, w: COMPACT_W, h: NOTCH_H };
export const REST = { x: 320, y: 95, u: 170 };
/** The flame's base (p.y = −0.44 of the box) sits 0.22 u below the box centre. */
export const BASE_K = 0.22;
/**
 * The strikes happen where Луч will stand, at the foot of the shader's lighter tongue (box p.y ≈ −0.43):
 * a lighter's spark and its flame share one point, so the fire is seen to catch from the sparks.
 */
export const FLINT = { x: REST.x, y: REST.y + 0.43 * (REST.u / 2) };
/**
 * The compact island's Луч, as the engine draws it there: botPosition("compact") is cx 40, cy 16,
 * diameter 20 → a 33 px canvas, u = 33 × LUCH_BOX, the flame centred at cy + 0.06 R + 0.05 u.
 */
export const SMALL = { x: 320 - COMPACT_W / 2 + 40, y: 16 + 0.06 * 9.9 + 0.05 * 49.5, u: 49.5 };

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
const easeIn = (k: number) => Math.pow(clamp01(k), 3);
const easeInOut = (k: number) => { k = clamp01(k); return k < 0.5 ? 4 * k * k * k : 1 - Math.pow(-2 * k + 2, 3) / 2; };
/** Ends past 1 and comes back (overshoot `c`). */
const backOut = (k: number, c = 1.7) => { k = clamp01(k) - 1; return 1 + (c + 1) * k * k * k + c * k * k; };
/** Starts below 0 (anticipation) and ends past 1 (overshoot). */
const backInOut = (k: number, c = 1.2) => {
  k = clamp01(k); const c2 = c * 1.525;
  return k < 0.5 ? (Math.pow(2 * k, 2) * ((c2 + 1) * 2 * k - c2)) / 2 : (Math.pow(2 * k - 2, 2) * ((c2 + 1) * (k * 2 - 2) + c2) + 2) / 2;
};
/** A damped spring's offset after an impulse at k = 0: 1 → wobbles → 0. */
const settle = (k: number, freq = 3.2, damp = 5.5) => (k <= 0 || k >= 1 ? 0 : Math.exp(-damp * k) * Math.cos(k * Math.PI * freq) * (1 - k));
/** 0 → 1 → 0 over k, eased at both ends. */
const pulse = (k: number) => (k <= 0 || k >= 1 ? 0 : Math.sin(Math.PI * k));
const hash = (i: number) => { const s = Math.sin(i * 127.1 + 311.7) * 43758.5453; return s - Math.floor(s); };

// ── Sparks: launched with a velocity, slowed by the air, pulled down by gravity ─────────────────

/** A spark drawn back into the ignition point: a curve that slows down as it arrives at t1. */
export interface SparkHome { t1: number; c1x: number; c1y: number; c2x: number; c2y: number; x: number; y: number }
/** One spark: born at t0 at (x, y) with velocity (vx, vy) px/s, dies after `life` s. */
export interface Spark {
  i: number; t0: number; x: number; y: number; vx: number; vy: number; life: number; drag: number; grav: number;
  /** Stays bright to its end (the ember's spark lands still glowing). */ keep?: boolean;
  /** Flies home along a curve instead of the free flight (the sparks that light Луч). */ home?: SparkHome;
}
export interface SparkPoint { i: number; x: number; y: number; px: number; py: number; a: number; heat: number }

/** Where the spark is `d` s after its birth (closed form of dv/dt = g − k v, or its curve home). */
function sparkPos(s: Spark, d: number): [number, number] {
  if (s.home) {
    const h = s.home, k = 1 - Math.pow(1 - clamp01(d / (h.t1 - s.t0)), 2.2), m = 1 - k;
    return [
      m * m * m * s.x + 3 * m * m * k * h.c1x + 3 * m * k * k * h.c2x + k * k * k * h.x,
      m * m * m * s.y + 3 * m * m * k * h.c1y + 3 * m * k * k * h.c2y + k * k * k * h.y,
    ];
  }
  const k = s.drag;
  if (k < 1e-3) return [s.x + s.vx * d, s.y + s.vy * d + 0.5 * s.grav * d * d];
  const e = 1 - Math.exp(-k * d), gk = s.grav / k;
  return [s.x + (s.vx * e) / k, s.y + gk * d + ((s.vy - gk) * e) / k];
}

/** The spark at t: position, the point 35 ms before it, alpha and heat (1 white-hot → 0 cold). */
export function sparkAt(s: Spark, t: number): SparkPoint | null {
  const d = t - s.t0;
  if (d < 0 || d > s.life) return null;
  const [x, y] = sparkPos(s, d), [px, py] = sparkPos(s, Math.max(0, d - 0.035));
  const k = d / s.life;
  // bright at once, then it cools and fades the last third of its life; homing sparks stay hot
  const a = clamp01(d / 0.02) * (s.keep || s.home ? 1 : 1 - easeIn(seg(k, 0.35, 1)));
  return { i: s.i, x, y, px, py, a, heat: s.home ? 1 - 0.3 * Math.sin(Math.PI * k) : 1 - k };
}

/** The second strike's spark that falls back by the flint and smoulders there (spec §4). */
const EMBER_HOP = 0.15;
const EMBER_SPARK: Spark = { i: 99, t0: G.strikes[1], x: FLINT.x, y: FLINT.y, vx: 40, vy: -0.5 * 900 * EMBER_HOP, life: EMBER_HOP, drag: 0, grav: 900, keep: true };
/** The ignition point: a hair above the base, inside the lighter tongue. */
export const IGNITION = { x: FLINT.x, y: FLINT.y - 3 };
/** Which of the third strike's sparks fly home, and when each arrives (the last one at the catch). */
const HOMING = [2, 5, 8, 11, 14];

const burstCache = new Map<Burst, Spark[]>();
/** A strike's fan of sparks, upwards around the vertical; the stronger the strike, the faster. */
export function burstSparks(b: Burst): Spark[] {
  let out = burstCache.get(b);
  if (out) return out;
  out = [];
  for (let i = 0; i < b.n; i++) {
    const h1 = hash(b.seed * 97 + i), h2 = hash(b.seed * 31 + i * 7), h3 = hash(b.seed * 13 + i * 3);
    const ang = -Math.PI / 2 + (h1 - 0.5) * 1.6;
    // flung hard and braked by the air: a quick spray that hangs at the top and drops
    const v = 640 * b.power * (0.55 + 0.45 * h2);
    const s: Spark = { i, t0: b.t0, x: b.x, y: b.y, vx: Math.cos(ang) * v, vy: Math.sin(ang) * v, life: 0.42 * (0.8 + 0.35 * h3) * (0.8 + 0.2 * b.power), drag: 4.5, grav: 900 };
    const j = b === BURSTS[2] ? HOMING.indexOf(i) : -1;
    if (j >= 0) {
      // out along its launch line, a wide turn, then down into the ignition point, slowing as it lands
      const t1 = G.catchAt - (HOMING.length - 1 - j) * 0.03, side = Math.cos(ang) < 0 ? -1 : 1;
      s.home = {
        t1, x: IGNITION.x, y: IGNITION.y,
        c1x: b.x + Math.cos(ang) * 110, c1y: b.y + Math.sin(ang) * 110,
        c2x: IGNITION.x + side * (34 + 20 * h2), c2y: IGNITION.y - 70 - 25 * h3,
      };
      s.life = t1 - b.t0;
    }
    out.push(s);
  }
  if (b === BURSTS[1]) out.push(EMBER_SPARK);
  burstCache.set(b, out);
  return out;
}

export function sparkPoints(b: Burst, t: number): SparkPoint[] {
  const out: SparkPoint[] = [];
  for (const s of burstSparks(b)) { const p = sparkAt(s, t); if (p) out.push(p); }
  return out;
}

// ── Луч ───────────────────────────────────────────────────────────────────────────────────────

export interface LuchPose {
  x: number; y: number; u: number; sx: number; sy: number;
  /** 0 → 1: the fire catching inside the full-size silhouette (the shader's uIgnite); not monotonic. */ ignite: number;
  /** 0…1: how hard the fire is surging (glow and tongue swell with it). */ surge: number;
  /** The main tongue's height (1 = normal); it shoots up past it once as the fire takes. */ tall: number;
  /** The face panel, faded in only once the fire has taken. */ coreA: number;
  eye: "closed" | "open" | "happy"; /** Eyelid, 1 open → 0 shut (blinks). */ lid: number;
  /** Where the eyes look, −1 left … 1 right. */ look: number;
  handsOut: number; waveT: number; /** How far the right hand is in its wave pose, 0…1. */ waveK: number;
}
export interface GreetFrame {
  /** The fire's own clock (s): it keeps running while a held pose rests. */
  clock: number;
  card: number; header: number; minis: number;
  /**
   * 0…1: how far the flame's colours have turned from the idle fire to the compact island's (the
   * state the island will show there). It turns on the way to the compact pose, so the hand-off to
   * the island's own Луч and minis does not jump in colour.
   */
  toCompact: number;
  /** The card's rectangle: it unfolds from the notch strip and folds into the compact island. */
  cardRect: { x: number; y: number; w: number; h: number; r: number };
  luch: LuchPose | null; ember: { x: number; y: number; r: number } | null; bursts: readonly Burst[];
  /** Every spark that may be in flight now (the strikes' and those the catching fire throws up). */
  sparks: readonly Spark[];
}

const REST_BASE = REST.y + BASE_K * REST.u;

/** Smooth steps through hand-set keys (time, value): each key is a little plateau or a turn. */
function keyed(keys: readonly (readonly [number, number])[], t: number): number {
  if (t <= keys[0][0]) return keys[0][1];
  for (let i = 1; i < keys.length; i++) {
    if (t < keys[i][0]) {
      const k = (t - keys[i - 1][0]) / (keys[i][0] - keys[i - 1][0]);
      return lerp(keys[i - 1][1], keys[i][1], k * k * (3 - 2 * k));
    }
  }
  return keys[keys.length - 1][1];
}

/**
 * The ignition by hand, in seconds after G.catchAt: a coal glows, a tiny flame flickers on and nearly
 * dies, then it takes and climbs in three uneven surges, each falling back a little, the tip last.
 */
const IGNITE_KEYS: readonly (readonly [number, number])[] = [
  [0, 0], [0.12, 0.03], [0.25, 0.08], [0.35, 0.1], [0.45, 0.03], [0.55, 0.05],
  [G.caught - G.catchAt, 0.16], [0.8, 0.34], [0.9, 0.3], [1.0, 0.33],
  [1.12, 0.58], [1.22, 0.54], [1.32, 0.6], [1.45, 0.86], [1.55, 0.83], [G.ignite1 - G.catchAt, 1],
];
const igniteCurve = (t: number) => keyed(IGNITE_KEYS, t - G.catchAt);

/** The ignition at t: the hand-set curve with a small flutter while it burns unevenly; exactly 1 from G.ignite1. */
export function igniteAt(t: number): number {
  if (t >= G.ignite1) return 1;
  if (t <= G.catchAt) return 0;
  const v = igniteCurve(t);
  const flutter = 0.012 * (Math.sin(t * 37) + Math.sin(t * 23 + 1.3)) * Math.min(v, 1 - v) * 4;
  return clamp01(v + flutter);
}

/** 0…1: how hard the fire is surging now (the curve's rise); it swells glow, tongue and squash. */
export function surgeAt(t: number): number {
  if (t <= G.catchAt || t >= G.ignite1 + 0.25) return 0;
  return clamp01((igniteCurve(t + 0.03) - igniteCurve(t - 0.03)) / 0.06 / 2.2);
}

function hopBase(t: number): number {
  const k = seg(t, G.tuck0 + 0.08, G.tuck1 - 0.04);
  return REST_BASE - 13 * 4 * k * (1 - k);
}

/** Stretch along a vertical speed of `v` px/s (volume kept). */
function stretch(v: number, per: number, max: number): [number, number] {
  const sy = 1 + max * Math.min(1, Math.abs(v) / per);
  return [1 / Math.sqrt(sy), sy];
}

/** Lid keys after G.eyes: a slow, sleepy half-open, shut again, then open with a little widening. */
const LID_KEYS: readonly (readonly [number, number])[] = [[0, 0], [0.14, 0.45], [0.24, 0.42], [0.32, 0.04], [0.4, 0.1], [0.5, 1.08], [0.56, 1]];
/** A little look around once awake (−1 left … 1 right), overlapping the hands coming out. */
const LOOK_KEYS: readonly (readonly [number, number])[] = [[0.42, 0], [0.48, -0.8], [0.55, -0.75], [0.61, 0.6], [0.67, 0.55], [0.72, 0]];

/** `hold`: the cursor holds the greeting, so it never leaves on its own (spec §4). */
function luchAt(t: number, hold = false): LuchPose {
  let x = REST.x, base = REST_BASE, u = REST.u, sx = 1, sy = 1;
  const ignite = igniteAt(t), surge = surgeAt(t);
  // the tongue grows in pulses with the surges and is exactly its height once the fire has settled
  const tall = t >= G.ignite1 + 0.25 ? 1 : 0.5 + 0.5 * ignite + 0.16 * surge;
  const coreA = easeInOut(seg(t, G.ignite1, G.eyes - 0.02));
  // each surge lifts the flame a little; while it is catching it breathes
  sy += 0.06 * surge; sx -= 0.03 * surge;
  const br = 0.012 * Math.sin(t * 2.3) * seg(t, G.catchAt, G.caught);
  sy += br; sx -= br * 0.5;
  // the happy hop flows out of the wave: crouch, jump (stretched), land (squashed)
  if (t >= G.tuck0 && t < G.tuck0 + 0.08) {
    const c = easeOut(seg(t, G.tuck0, G.tuck0 + 0.08)); sx *= 1 + 0.08 * c; sy *= 1 - 0.13 * c;
  } else if (t >= G.tuck0 + 0.08 && t < G.tuck1 - 0.04) {
    base = hopBase(t);
    const [fx, fy] = stretch((hopBase(t + 0.01) - hopBase(t - 0.01)) / 0.02, 180, 0.14);
    const r = easeOut(seg(t, G.tuck0 + 0.08, G.tuck0 + 0.13));
    sx *= lerp(1.08, fx, r); sy *= lerp(0.87, fy, r);
  } else if (t >= G.tuck1 - 0.04) {
    const dd = settle(seg(t, G.tuck1 - 0.04, G.tuck1 + 0.3), 2.6, 6);
    sx *= 1 + 0.09 * dd; sy *= 1 - 0.12 * dd;
  }
  // while it waves, the whole flame bobs with the hand
  const waveT = t >= G.wave0 + 0.15 && t < G.tuck0 + 0.2 ? t - G.wave0 - 0.15 : -1;
  const waveK = waveT < 0 ? 0 : easeOut(waveT / 0.2) * (1 - easeInOut(seg(t, G.wave1, G.tuck0 + 0.2)));
  if (waveT >= 0) base += 1.4 * Math.sin(13 * waveT + 0.8) * waveK;

  let y = base - BASE_K * u;
  // leave for the compact island: a crouch (squash), a wind-up to the right, an arc up and left
  // stretched along the flight, a slight overshoot in size on arrival
  if (t >= G.leave0 && !hold) {
    const c = pulse(seg(t, G.leave0 - 0.02, G.leave0 + 0.12));
    sx *= 1 + 0.1 * c; sy *= 1 - 0.15 * c;
    const k = seg(t, G.leave0 + 0.06, G.end);
    x = lerp(x, SMALL.x, backInOut(k, 0.9));
    // it shrinks a little ahead of the climb so the tongue stays inside the canvas
    y = lerp(y, SMALL.y, easeInOut(k));
    u = lerp(u, SMALL.u, easeInOut(Math.pow(k, 0.8))) * (1 - 0.07 * Math.sin(Math.PI * seg(k, 0.7, 1)));
    const sp = Math.sin(Math.PI * k);
    sx *= 1 - 0.1 * sp; sy *= 1 + 0.18 * sp;
  }

  const happy0 = G.eyes + 0.72;
  const eye = t < G.eyes ? "closed" : t >= happy0 && t < G.tuck1 ? "happy" : "open";
  let lid = 1, look = 0;
  if (eye === "open") {
    lid = keyed(LID_KEYS, t - G.eyes);
    look = keyed(LOOK_KEYS, t - G.eyes);
    // they squeeze into the happy ones
    if (t < G.tuck1) lid *= 1 - 0.6 * easeIn(seg(t, happy0 - 0.06, happy0));
    const b2 = hold ? 0 : seg(t, G.leave0 + 0.16, G.leave0 + 0.28);
    if (b2 > 0 && b2 < 1) lid = 1 - 0.92 * Math.sin(Math.PI * b2);
  }
  // hands pop out (overshoot) while the eyes are still waking, and are pulled back in after a tiny wind-up
  let handsOut = 0;
  if (t >= G.wave0 && t < G.tuck0) handsOut = backOut(seg(t, G.wave0, G.wave0 + 0.3), 1.6);
  else if (t >= G.tuck0 && t < G.tuck1) handsOut = 1 - windIn(seg(t, G.tuck0, G.tuck1));
  return { x, y, u, sx, sy, ignite, surge, tall, coreA, eye, lid, look, handsOut, waveT, waveK };
}
/** An ease-in that first dips a little below 0 (a wind-up, never under −0.03). */
function windIn(k: number) { const c = 0.8; return (c + 1) * k * k * k - c * k * k; }

/** Each surge throws a few sparks up off the fire's front; they rise on the heat and fade. */
const SURGE_SPARKS: Spark[] = [G.caught + 0.12, G.catchAt + 1.08, G.catchAt + 1.4, G.ignite1 - 0.04].flatMap((tb, s) =>
  Array.from({ length: 4 }, (_, i) => {
    const h1 = hash(s * 17 + i + 41), h2 = hash(s * 13 + i * 3 + 7), h3 = hash(s * 7 + i * 5 + 2);
    const ig = igniteCurve(tb), ang = -Math.PI / 2 + (h1 - 0.5) * 1.8;
    const top = Math.min(1, 0.25 + ig) * REST.u * 0.42;
    return {
      i: 200 + s * 10 + i, t0: tb + 0.03 * i,
      x: REST.x + (h1 - 0.5) * 0.5 * REST.u * 0.44, y: REST_BASE - top * (0.6 + 0.4 * h3),
      vx: Math.cos(ang) * (80 + 70 * h2), vy: Math.sin(ang) * (80 + 70 * h2), life: 0.4 + 0.25 * h3, drag: 2.5, grav: -110,
    };
  }));

function cardRect(t: number, fold: number) {
  // the strip first draws in and drops ("breathes in"), then opens wide with a small overshoot
  const kw = backInOut(seg(t, 0, G.open1), 0.8), kh = backOut(seg(t, 0, G.open1 - 0.06), 1.2);
  const open = { x: lerp(STRIP.x, CARD.x, kw), y: lerp(STRIP.y, CARD.y, kh), w: lerp(STRIP.w, CARD.w, kw), h: lerp(STRIP.h, CARD.h, kh) };
  return foldTo({ ...open, r: lerp(14, 20, kh) }, fold);
}

/** Where a held greeting rests: after the hop has settled, before it would leave. */
export const HOLD_T = G.tuck1 + 0.35;

function play(t: number, hold = false): GreetFrame {
  if (hold) t = Math.min(t, HOLD_T);
  const card = easeOut(seg(t, 0, G.open1 * 0.7));
  const bursts = BURSTS.filter((b) => t >= b.t0 && t < b.t0 + 0.7);
  const sparks = [...bursts.flatMap(burstSparks), ...(t >= G.caught && t < G.ignite1 + 0.8 ? SURGE_SPARKS : [])];
  // the second strike leaves an ember that glows 0.2 s (spec §4) and dies
  const e2 = BURSTS[1].t0;
  const ember = t > e2 + EMBER_HOP && t < e2 + 0.35 ? { x: FLINT.x + 6, y: FLINT.y, r: 3 * (1 - seg(t, e2 + 0.25, e2 + 0.35)) } : null;
  if (t < G.catchAt) return { clock: t, card, header: card, minis: 0, toCompact: 0, cardRect: cardRect(t, 0), luch: null, ember, bursts, sparks };
  const fold = hold ? 0 : seg(t, G.leave0, G.leave0 + 0.24);
  return {
    clock: t, card: card * (1 - fold), header: card * (hold ? 1 : 1 - seg(t, G.leave0, G.leave0 + 0.1)),
    minis: hold ? 0 : easeOut(seg(t, G.end - 0.2, G.end)), toCompact: hold ? 0 : easeInOut(seg(t, G.leave0, G.end)), cardRect: cardRect(t, fold),
    luch: luchAt(t, hold), ember: null, bursts, sparks,
  };
}

/**
 * The greeting held by the cursor from `holdAt` (spec §4): it does not leave; it rests on the card
 * after the hop, still alive — it breathes, blinks now and then and glances about. Held while it
 * was already leaving, it comes back to the card first.
 */
function held(t: number, holdAt: number): GreetFrame {
  const b = play(t, true);
  if (b.luch && t > HOLD_T) {
    const d = t - HOLD_T, l = b.luch;
    const k = ((d + 1.4) % 3.6) / 0.14;
    const lid = k < 1 ? 1 - 0.92 * Math.sin(Math.PI * k) : 1;
    const br = 0.012 * Math.sin(d * 2.3) * seg(d, 0, 0.6);
    b.luch = { ...l, lid, sy: l.sy + br, sx: l.sx - br * 0.5, look: 0.45 * Math.sin(d * 0.8) * Math.sin(d * 0.31) };
  }
  if (holdAt <= G.leave0) return b;
  // it was on its way to the compact island: it flies back to rest on the card
  const a = play(Math.min(holdAt, G.end));
  const k = easeInOut(seg(t, holdAt, holdAt + 0.45));
  if (k >= 1) return b;
  const al = a.luch!, bl = b.luch!;
  return {
    ...b, card: lerp(a.card, b.card, k), header: lerp(a.header, b.header, k), minis: lerp(a.minis, 0, k), toCompact: lerp(a.toCompact, 0, k),
    cardRect: { x: lerp(a.cardRect.x, b.cardRect.x, k), y: lerp(a.cardRect.y, b.cardRect.y, k), w: lerp(a.cardRect.w, b.cardRect.w, k), h: lerp(a.cardRect.h, b.cardRect.h, k), r: lerp(a.cardRect.r, b.cardRect.r, k) },
    luch: { ...bl, x: lerp(al.x, bl.x, k), y: lerp(al.y, bl.y, k), u: lerp(al.u, bl.u, k), sx: lerp(al.sx, bl.sx, k), sy: lerp(al.sy, bl.sy, k) },
  };
}

/** The frame before any collapse: the plain run, or the held one from `holdAt`. */
function base(t: number, holdAt: number): GreetFrame {
  if (t >= holdAt) return held(t, holdAt);
  return play(Math.min(t, G.end + 10));
}

/**
 * The frame at t. `holdAt`: the cursor has held the greeting since then (Infinity = never). From `tc`
 * on it collapses from where it was to the compact pose in G.COLLAPSE. `clock` is the fire's own
 * time: it always runs, even while the pose rests.
 */
export function greetFrame(t: number, tc: number, holdAt = Number.POSITIVE_INFINITY): GreetFrame {
  if (t < tc) return { ...base(t, holdAt), clock: t };
  const a = base(tc, holdAt);
  const e = easeInOut(seg(t, tc, tc + G.COLLAPSE));
  const luch = a.luch && {
    ...a.luch, x: lerp(a.luch.x, SMALL.x, e), y: lerp(a.luch.y, SMALL.y, e), u: lerp(a.luch.u, SMALL.u, e),
    ignite: lerp(a.luch.ignite, 1, e), surge: a.luch.surge * (1 - e), tall: lerp(a.luch.tall, 1, e), coreA: lerp(a.luch.coreA, 1, e),
    sx: lerp(a.luch.sx, 1, e), sy: lerp(a.luch.sy, 1, e), handsOut: a.luch.handsOut * (1 - seg(t, tc, tc + 0.15)),
    waveT: -1, waveK: 0, eye: "open" as const, lid: 1, look: 0,
  };
  return {
    clock: t,
    card: a.card * (1 - seg(t, tc, tc + 0.18)), header: a.header * (1 - seg(t, tc, tc + 0.1)),
    minis: lerp(a.minis, 1, easeOut(seg(t, tc, tc + G.COLLAPSE))), toCompact: lerp(a.toCompact, 1, e), luch, ember: null,
    cardRect: foldTo(a.cardRect, seg(t, tc, tc + 0.18)),
    bursts: t > tc + G.COLLAPSE ? [] : a.bursts, sparks: t > tc + G.COLLAPSE ? [] : a.sparks,
  };
}

/** The sounds, in order (spec §4): a tick per strike, `pop` when the flame takes, `greet` with the wave. */
export const SOUNDS: readonly { t: number; name: "tick" | "pop" | "greet" }[] = [
  ...G.strikes.map((t) => ({ t, name: "tick" as const })),
  { t: G.caught, name: "pop" },
  { t: G.wave0, name: "greet" },
];
/** When the controller reports the greeting done (onComplete) on a plain run. */
export const COMPLETE_AT = G.end + 0.05;

function foldTo(r: GreetFrame["cardRect"], k: number) {
  const f = easeIn(k);
  return { x: lerp(r.x, COMPACT.x, f), y: lerp(r.y, COMPACT.y, f), w: lerp(r.w, COMPACT.w, f), h: lerp(r.h, COMPACT.h, f), r: lerp(r.r, 16, f) };
}
