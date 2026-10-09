// The launch greeting, drawn from greetFrame (spec §4). Same controller API as before.

import { Sound } from "../core/sound";
import { BASE_K, CANVAS, COMPLETE_AT, FLINT, G, GREET_END, greetFrame, IGNITION, SOUNDS, sparkAt, type GreetFrame, type LuchPose, type Spark } from "./greeting-timeline";
import { fireLook } from "./look";
import { IDLE_FIRE, paletteFor } from "./palette";
import { drawLuch, LUCH, luchEyeSize } from "./render";

export const GREETING_END = GREET_END;
const LOOK = fireLook("idle", null, false);
const EYE_INK = "#2A170B";

const clamp01 = (k: number) => Math.min(1, Math.max(0, k));
const seg = (t: number, a: number, b: number) => clamp01((t - a) / (b - a));
const backOut = (k: number, c = 1.7) => { k = clamp01(k) - 1; return 1 + (c + 1) * k * k * k + c * k * k; };

/** The engine's pill, happy and closed eyes (same sizes), so the hand-off to the island does not jump. */
function eyes(x: CanvasRenderingContext2D, cx: number, cy: number, u: number, shape: LuchPose["eye"], lid: number, mini: boolean, look = 0) {
  const s = u / 2;
  const { w, h, step } = luchEyeSize(u, mini);
  x.fillStyle = x.strokeStyle = EYE_INK; x.lineCap = "round";
  for (const sd of [-1, 1]) {
    const ex = cx + LUCH.eyeCx * s + sd * step + look * LUCH.eyeStep * s * 0.35, ey = cy - LUCH.eyeCy * s;
    x.beginPath();
    if (shape === "open") {
      const hh = Math.max(h * lid, w * 0.3);
      x.roundRect(ex - w / 2, ey - hh / 2, w, hh, Math.min(w / 2, hh / 2)); x.fill();
    } else if (shape === "happy") {
      x.lineWidth = w * 0.5; x.arc(ex, ey + h * 0.18, w * 0.82, Math.PI * 1.12, Math.PI * 1.88); x.stroke();
    } else {
      x.lineWidth = w * 0.36; x.arc(ex, ey - h * 0.08, w * 0.78, Math.PI * 0.15, Math.PI * 0.85); x.stroke();
    }
  }
}

/** White-hot → yellow → orange → a dull red as the spark cools. */
function heatRGB(h: number): [number, number, number] {
  const stops: [number, number, number, number][] = [[0, 190, 40, 10], [0.35, 255, 110, 25], [0.7, 255, 196, 80], [1, 255, 250, 228]];
  for (let i = 1; i < stops.length; i++) {
    if (h <= stops[i][0]) {
      const [a, b] = [stops[i - 1], stops[i]], k = (h - a[0]) / (b[0] - a[0]);
      return [a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k, a[3] + (b[3] - a[3]) * k];
    }
  }
  return [255, 250, 228];
}

/** A spark: a tapered streak along its last 50 ms of flight, a hot head and a soft glow. */
function drawSpark(x: CanvasRenderingContext2D, s: Spark, t: number) {
  const p = sparkAt(s, t);
  if (!p || p.a < 0.01) return;
  // sparks twinkle as they tumble
  const a = p.a * (0.78 + 0.22 * Math.sin((t - s.t0) * 70 + s.i * 2.1));
  const [r, g, b] = heatRGB(p.heat);
  const N = 5;
  let lx = p.x, ly = p.y;
  for (let j = 1; j <= N; j++) {
    const q = sparkAt(s, Math.max(s.t0, t - j * 0.01));
    if (!q) break;
    const k = 1 - j / (N + 1);
    x.strokeStyle = `rgba(${r | 0},${(g * (0.75 + 0.25 * k)) | 0},${(b * k) | 0},${a * k * 0.9})`;
    x.lineWidth = 0.6 + 1.3 * k;
    x.beginPath(); x.moveTo(lx, ly); x.lineTo(q.x, q.y); x.stroke();
    lx = q.x; ly = q.y;
    if (q.x === s.x && q.y === s.y) break;
  }
  const gl = x.createRadialGradient(p.x, p.y, 0, p.x, p.y, 4);
  gl.addColorStop(0, `rgba(255,${(150 + 80 * p.heat) | 0},60,${0.3 * a})`); gl.addColorStop(1, "rgba(255,120,30,0)");
  x.fillStyle = gl; x.beginPath(); x.arc(p.x, p.y, 4, 0, Math.PI * 2); x.fill();
  x.fillStyle = `rgba(255,252,240,${a})`; x.beginPath(); x.arc(p.x, p.y, 0.6 + 0.7 * p.heat, 0, Math.PI * 2); x.fill();
}

/** The strike: a flash that pops and dies fast, and its warm light on the card floor. */
function drawFlash(x: CanvasRenderingContext2D, bx: number, by: number, d: number, power: number) {
  if (d < 0 || d > 0.3) return;
  const a = Math.exp(-d * 16) * Math.min(1, d / 0.012);
  const r = 26 * power * backOut(d / 0.06, 1.4);
  const g = x.createRadialGradient(bx, by, 0, bx, by, r);
  g.addColorStop(0, `rgba(255,248,225,${0.95 * a})`); g.addColorStop(0.25, `rgba(255,190,90,${0.55 * a})`); g.addColorStop(1, "rgba(255,120,30,0)");
  x.fillStyle = g; x.beginPath(); x.arc(bx, by, r, 0, Math.PI * 2); x.fill();
  x.save(); x.translate(bx, by + 4); x.scale(1, 0.22);
  const f = x.createRadialGradient(0, 0, 0, 0, 0, 90 * power);
  f.addColorStop(0, `rgba(255,150,60,${0.35 * a})`); f.addColorStop(1, "rgba(255,120,30,0)");
  x.fillStyle = f; x.beginPath(); x.arc(0, 0, 90 * power, 0, Math.PI * 2); x.fill(); x.restore();
}

const MINI_COLORS = ["#E86A6A", "#3E86E0", "#EFAE5A", "#8C73F2"];
const MINI_FIRES = MINI_COLORS.map(paletteFor);
/**
 * The compact island's 2×2 grid (#mini-grid: 13 px slots, 3 px gap, centred 40 px from the right
 * edge), each mini drawn the way the engine draws one in a 13 px slot (u = 13 / 0.6 × 1.25).
 */
const GRID = { x: 320 + 288 / 2 - 40, y: 16, step: 8, u: (13 / 0.6) * 1.25 };

function drawMinis(x: CanvasRenderingContext2D, alpha: number, t: number) {
  if (alpha <= 0.01) return;
  const offsets: [number, number][] = [[-1, -1], [1, -1], [-1, 1], [1, 1]];
  offsets.forEach(([dx, dy], i) => {
    // they pop in one after the other, each with a small overshoot
    const k = backOut(seg(alpha, i * 0.12, i * 0.12 + 0.64), 2);
    if (k <= 0.01) return;
    const cx = GRID.x + dx * GRID.step, cy = GRID.y + dy * GRID.step + GRID.u * 0.1;
    const base = cy + BASE_K * GRID.u;
    x.save();
    x.translate(cx, base); x.scale(k, k); x.translate(-cx, -base);
    drawLuch(x, cx, cy, GRID.u, t + i * 1.7, { palette: MINI_FIRES[i], look: LOOK, mini: true });
    eyes(x, cx, cy, GRID.u, "open", 1, true);
    x.restore();
  });
}

function drawCard(x: CanvasRenderingContext2D, f: GreetFrame) {
  if (f.card <= 0.01) return;
  const r = f.cardRect;
  x.save(); x.globalAlpha = f.card; x.fillStyle = "#141518";
  x.beginPath(); x.roundRect(r.x, r.y, r.w, r.h, Math.min(r.r, r.h / 2, r.w / 2)); x.fill(); x.restore();
}

function drawFlame(x: CanvasRenderingContext2D, l: LuchPose, t: number) {
  const base = l.y + BASE_K * l.u;
  // warm light on the card: it swells with every surge of the catching fire and, once he burns, keeps
  // breathing gently (never a still frame)
  const wh = 0.2 * l.surge + 0.05 * l.ignite * (1 + 0.45 * Math.sin(t * 2.6) + 0.2 * Math.sin(t * 6.1 + 1));
  if (wh > 0.005) {
    const cy = base - (0.08 + 0.12 * l.ignite) * l.u, R = l.u * (0.3 + 0.2 * l.ignite);
    const g = x.createRadialGradient(l.x, cy, 0, l.x, cy, R);
    g.addColorStop(0, `rgba(255,170,70,${wh})`); g.addColorStop(1, "rgba(255,120,30,0)");
    x.save(); x.globalCompositeOperation = "lighter"; x.fillStyle = g;
    x.beginPath(); x.arc(l.x, cy, R, 0, Math.PI * 2); x.fill(); x.restore();
  }
  x.save();
  // squash and stretch pivot on the flame's base, where it stands
  x.translate(l.x, base); x.scale(l.sx, l.sy); x.translate(-l.x, -base);
  drawLuch(x, l.x, l.y, l.u, t, {
    palette: IDLE_FIRE, look: l.tall === 1 ? LOOK : { ...LOOK, tall: l.tall }, coreA: l.coreA, ignite: l.ignite,
    embers: l.ignite >= 1,
    // not a mini even when small: the compact island's Луч keeps its flares, so the hand-off matches
    handsOut: l.handsOut, waveT: l.waveT, waveK: l.waveK,
  });
  // the eyes come with the face panel, closed, and open once the fire has fully taken
  const ea = seg(l.coreA, 0.5, 1);
  if (ea > 0) {
    x.globalAlpha = ea;
    eyes(x, l.x, l.y, l.u, l.eye, l.lid, false, l.look);
  }
  x.restore();
}

/** The coal's glow (0…1): it breathes from the catch until the flame has really taken. */
function coalAt(t: number): number {
  const on = seg(t, G.catchAt - 0.02, G.catchAt + 0.08) * (1 - seg(t, G.caught - 0.05, G.caught + 0.25));
  return on * (0.7 + 0.3 * Math.sin((t - G.catchAt) * 9));
}

/** `holdAt`: the cursor has held the greeting since then (Infinity = never); `tc`: it collapses from then. */
export function drawGreeting(x: CanvasRenderingContext2D, t: number, tc: number, holdAt = Number.POSITIVE_INFINITY) {
  const f = greetFrame(t, tc, holdAt);
  t = f.clock; // the fire's own time: it runs on while a held pose rests
  x.clearRect(0, 0, CANVAS.w, CANVAS.h);
  drawCard(x, f);
  x.save(); x.globalCompositeOperation = "lighter"; x.lineCap = "round";
  const cut = Number.isFinite(tc) && t >= tc ? 1 - seg(t, tc, tc + 0.2) : 1;
  x.globalAlpha = cut;
  // a thumb on the wheel: a faint scrape-glint just before each strike
  for (const t0 of G.strikes) if (t > t0 - 0.08 && t < t0) drawFlash(x, FLINT.x, FLINT.y, t - (t0 - 0.07), 0.12);
  for (const b of f.bursts) drawFlash(x, b.x, b.y, t - b.t0, b.power);
  const coal = f.luch ? coalAt(t) : 0;
  if (coal > 0.01) {
    // the landed sparks glow on as a coal that breathes before the flame takes
    const r = 3 + 4 * coal, fl = 0.8 + 0.2 * Math.sin(t * 31) * Math.sin(t * 13);
    const g = x.createRadialGradient(IGNITION.x, IGNITION.y + 1, 0, IGNITION.x, IGNITION.y + 1, r * 2.2);
    g.addColorStop(0, `rgba(255,200,110,${0.8 * coal * fl})`); g.addColorStop(0.35, `rgba(255,110,30,${0.45 * coal * fl})`); g.addColorStop(1, "rgba(200,40,10,0)");
    x.fillStyle = g; x.beginPath(); x.arc(IGNITION.x, IGNITION.y + 1, r * 2.2, 0, Math.PI * 2); x.fill();
  }
  for (const s of f.sparks) {
    drawSpark(x, s, t);
    // a homing spark lands in the lighter tongue with a little flash
    if (s.home) drawFlash(x, s.home.x, s.home.y, t - s.home.t1, 0.35);
  }
  if (f.ember && f.ember.r > 0.2) {
    // it smoulders: a flickering red-orange coal that dims and shrinks
    const e = f.ember, fl = 0.75 + 0.25 * Math.sin(t * 55) * Math.sin(t * 23), k = e.r / 3;
    const g = x.createRadialGradient(e.x, e.y, 0, e.x, e.y, 2 + 4 * k);
    g.addColorStop(0, `rgba(255,${(150 + 70 * k) | 0},70,${0.45 * fl * k})`); g.addColorStop(1, "rgba(200,50,10,0)");
    x.fillStyle = g; x.beginPath(); x.arc(e.x, e.y, 2 + 4 * k, 0, Math.PI * 2); x.fill();
    x.fillStyle = `rgba(255,${(120 + 110 * k) | 0},${(40 + 100 * k) | 0},${fl})`;
    x.beginPath(); x.arc(e.x, e.y, 0.5 + 0.9 * k, 0, Math.PI * 2); x.fill();
  }
  x.restore();
  drawMinis(x, f.minis, t);
  if (f.luch) drawFlame(x, f.luch, t);
}

/**
 * Runs the greeting animation on its own canvas. `onComplete` fires once at COMPLETE_AT (also while
 * the cursor holds it: the FSM's hover timer decides then) so the FSM can move on; an interruption
 * cancels it, as the FSM has already moved on.
 */
export class Greeting {
  private startMs = 0;
  private tc = Number.POSITIVE_INFINITY;
  /** When the cursor started holding the greeting (Infinity = not held). */
  private holdAt = Number.POSITIVE_INFINITY;
  private fired = false;
  private timers: number[] = [];

  onComplete: (() => void) | null = null;

  start() {
    this.startMs = performance.now();
    this.tc = Number.POSITIVE_INFINITY;
    this.holdAt = Number.POSITIVE_INFINITY;
    this.fired = false;
    this.cancelTimers();
    this.timers.push(
      ...SOUNDS.map((s) => window.setTimeout(() => Sound.play(s.name), s.t * 1000)),
      window.setTimeout(() => this.fire(), COMPLETE_AT * 1000),
    );
  }

  /**
   * Mouse entered the island during the greeting: hold it (spec §4). It plays on to its rest after
   * the hop and stays there, alive, instead of leaving; the FSM's hover timer (10 s) or the mouse
   * leaving ends it through interrupt().
   */
  hover() {
    if (Number.isFinite(this.tc) || Number.isFinite(this.holdAt)) return;
    this.holdAt = this.elapsed;
  }

  /** Mouse left (or an alert, or the hover timer): collapse from where it is now, held or not. */
  interrupt() {
    const t = (performance.now() - this.startMs) / 1000;
    if (!Number.isFinite(this.tc) || this.tc > t) this.tc = t;
    this.cancelTimers();
  }

  get elapsed(): number {
    return (performance.now() - this.startMs) / 1000;
  }

  get done(): boolean {
    return this.fired;
  }

  private fire() {
    if (this.fired) return;
    this.fired = true;
    this.cancelTimers();
    this.onComplete?.();
  }

  private cancelTimers() {
    this.timers.forEach((id) => window.clearTimeout(id));
    this.timers = [];
  }

  draw(x: CanvasRenderingContext2D) {
    const t = this.elapsed;
    if (!this.fired && t >= COMPLETE_AT && this.tc >= G.autoLeave) this.fire();
    drawGreeting(x, t, this.tc, this.holdAt);
  }
}
