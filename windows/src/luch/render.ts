// Луч's fire box: the GL fire (or the 2D fallback) plus rising embers. The eyes are the engine's.

import { drawEmber } from "./embers";
import { drawFallbackFire } from "./fallback";
import { renderFire } from "./fire";
import { brightAt, type FireLook } from "./look";
import type { FocusLevels } from "./focus";
import { mixFire, type FirePalette } from "./palette";

export const LUCH = { faceCx: 0.03, faceCy: -0.1, eyeCx: 0.09, eyeCy: -0.095, eyeStep: 0.093, eyeW: 0.022, eyeH: 0.04, baseY: -0.44 } as const;

/** Mini eyes never go below this (CSS px): the box-relative size is under a pixel in a 13 px slot. */
const MINI_EYE_W = 1.6, MINI_EYE_H = 2.6;
/**
 * Луч's eye size and spacing in CSS px for a fire box of u px: 1.7 × the G4 measures, with a readable
 * floor on minis. One place for the engine and the greeting, so the greeting's hand-off cannot drift.
 */
export function luchEyeSize(u: number, mini: boolean): { w: number; h: number; step: number } {
  const s = u / 2;
  let w = LUCH.eyeW * s * 1.7, h = LUCH.eyeH * s * 1.7, step = LUCH.eyeStep * s;
  if (mini) {
    w = Math.max(w, MINI_EYE_W); h = Math.max(h, MINI_EYE_H);
    step = Math.max(step, w * 0.95); // the floor must not merge the two eyes
  }
  return { w, h, step };
}

export interface LuchOpts {
  palette: FirePalette; look: FireLook;
  grow?: number; coreA?: number; handsOut?: number; waveT?: number;
  /** How far the right hand is in its wave pose (0…1); omitted = eased in from waveT. */ waveK?: number;
  /** The greeting's ignition, 0…1 (the fire catches inside the silhouette); omitted = 1. */ ignite?: number;
  /** The dance to music in the fire itself, 0…1, at dance clock `danceT` (luch/dance.ts); omitted = 0. */ dance?: number; danceT?: number;
  /** The pomodoro looks in the fire itself, at phase clock `focusT` (luch/focus.ts); omitted = none. */ focus?: FocusLevels; focusT?: number;
  /**
   * A colour change (luch/colour.ts): the palette before it (`palette` is the one it changes to) and its
   * eased progress 0…1; omitted = no change. The shader climbs the new colour up the flame; the 2D
   * fallback and the embers take the uniform mix.
   */
  colour?: { from: FirePalette; k: number; variant?: number };
  mini?: boolean; embers?: boolean;
}

const easeOut = (k: number) => 1 - Math.pow(1 - Math.min(1, Math.max(0, k)), 3);
const lerp = (a: number, b: number, k: number) => a + (b - a) * k;
const hash = (i: number) => { const s = Math.sin(i * 127.1 + 311.7) * 43758.5453; return s - Math.floor(s); };

/**
 * Hand centres in shader space: tucked near the body at 0, at the lower corners at 1; the right hand waves (Mochi's 13 rad/s).
 * `waveK` blends the hand in and out of the wave pose (by default it eases in over 0.2 s and never out).
 */
export function handsInBox(t: number, out: number, waveT: number, waveK?: number): [number, number, number, number] {
  const at = (sd: number): [number, number] => {
    let hx = lerp(sd * 0.22, sd < 0 ? -0.4 : 0.48, out);
    let hy = lerp(-0.3, sd < 0 ? -0.28 : -0.18, out);
    hy += Math.sin(t * 2.3 + sd * 1.3) * 0.02;
    if (sd > 0 && waveT >= 0) {
      const k = waveK ?? easeOut(waveT / 0.2);
      hx = lerp(hx, 0.56 + 0.05 * Math.cos(13 * waveT), k);
      hy = lerp(hy, 0.02 + 0.11 * Math.sin(13 * waveT), k);
    }
    return [hx, hy];
  };
  return [...at(-1), ...at(1)];
}

export function drawLuch(x: CanvasRenderingContext2D, cx: number, cy: number, u: number, t: number, o: LuchOpts) {
  const grow = o.grow ?? 1;
  if (grow <= 0.001 || u < 2) return;
  const look = o.look;
  // up to 1.2: the greeting's hands pop out a little past their place and settle back
  const handsOut = o.mini ? 0 : Math.min(1.2, Math.max(0, o.handsOut ?? 0));
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  const shown = o.colour ? mixFire(o.colour.from, o.palette, o.colour.k) : o.palette;
  x.save();
  x.translate(cx, cy);
  // The box is wider than the canvas: map box points to the canvas's device px (tilt and squash
  // included) so the shader fades the glow out before the canvas cuts it.
  const m = x.getTransform(), h = u / 2, cw = x.canvas.width, ch = x.canvas.height;
  const view = {
    x: [m.a * h, -m.c * h, m.e] as [number, number, number],
    y: [m.b * h, -m.d * h, m.f] as [number, number, number],
    fade: [cw, ch, Math.max(2, 0.15 * Math.min(cw, ch))] as [number, number, number],
  };
  const tex = renderFire(u * dpr, {
    time: t % 3600, grow, live: look.live, bright: brightAt(look, t), coreA: o.coreA ?? 1,
    handsOut, tall: look.tall, flares: o.mini ? 0 : 1, streaks: o.mini ? 0 : 1, palette: o.palette, hands: handsInBox(t, handsOut, o.waveT ?? -1, o.waveK), view, ignite: o.ignite ?? 1,
    dance: o.dance ?? 0, danceT: o.danceT ?? 0,
    work: o.focus?.work ?? 0, rest: o.focus?.rest ?? 0, ember: o.focus?.ember ?? 0, focusT: o.focusT ?? 0,
    from: o.colour?.from, colorK: o.colour?.k, colorV: o.colour?.variant,
  });
  if (tex) x.drawImage(tex.canvas, 0, 0, tex.px, tex.px, -u / 2, -u / 2, u, u);
  else {
    // the 2D fallback cannot catch inside the silhouette: it grows from the base and fades in instead
    const ig = Math.min(1, Math.max(0, o.ignite ?? 1));
    const ga = x.globalAlpha;
    x.globalAlpha = ga * Math.min(1, ig * 3);
    drawFallbackFire(x, u, t, shown, look, grow * (0.3 + 0.7 * ig), o.focus);
    x.globalAlpha = ga;
  }
  if (!o.mini && o.embers !== false && grow > 0.5) {
    x.globalCompositeOperation = "lighter";
    for (let i = 0; i < 10; i++) {
      const ph = (((t * (0.1 + 0.12 * hash(i + 3)) * look.live + hash(i)) % 1) + 1) % 1;
      const ex = (hash(i + 1) - 0.5) * u * 0.55 + Math.sin(t * 1.3 + i * 1.7) * u * 0.03 * ph;
      const ey = u * 0.05 - ph * u * 0.42;
      const a = Math.sin(ph * Math.PI) * 0.85 * (grow - 0.5) * 2;
      const r = u * (0.003 + 0.004 * hash(i + 7)) * (1 - 0.4 * ph);
      x.globalAlpha = a;
      drawEmber(x, ex, ey, r * 4, shown.light);
    }
  }
  x.restore();
}
