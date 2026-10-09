// Луч's fire box: the GL fire (or the 2D fallback) plus rising embers. The eyes are the engine's.

import { drawFallbackFire } from "./fallback";
import { renderFire } from "./fire";
import { brightAt, type FireLook } from "./look";
import type { FirePalette } from "./palette";

export const LUCH = { faceCx: 0.03, faceCy: -0.1, eyeCx: 0.09, eyeCy: -0.095, eyeStep: 0.093, eyeW: 0.022, eyeH: 0.04, baseY: -0.44 } as const;

export interface LuchOpts {
  palette: FirePalette; look: FireLook;
  grow?: number; coreA?: number; handsOut?: number; waveT?: number;
  mini?: boolean; embers?: boolean;
}

const easeOut = (k: number) => 1 - Math.pow(1 - Math.min(1, Math.max(0, k)), 3);
const lerp = (a: number, b: number, k: number) => a + (b - a) * k;
const hash = (i: number) => { const s = Math.sin(i * 127.1 + 311.7) * 43758.5453; return s - Math.floor(s); };

/** Hand centres in shader space: tucked near the body at 0, at the lower corners at 1; the right hand waves (Mochi's 13 rad/s). */
export function handsInBox(t: number, out: number, waveT: number): [number, number, number, number] {
  const at = (sd: number): [number, number] => {
    let hx = lerp(sd * 0.22, sd < 0 ? -0.4 : 0.48, out);
    let hy = lerp(-0.3, sd < 0 ? -0.28 : -0.18, out);
    hy += Math.sin(t * 2.3 + sd * 1.3) * 0.02;
    if (sd > 0 && waveT >= 0) {
      const k = easeOut(waveT / 0.2);
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
  const handsOut = o.mini ? 0 : Math.min(1, Math.max(0, o.handsOut ?? 0));
  const dpr = Math.min(2, window.devicePixelRatio || 1);
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
    handsOut, tall: look.tall, flares: o.mini ? 0 : 1, streaks: o.mini ? 0 : 1, palette: o.palette, hands: handsInBox(t, handsOut, o.waveT ?? -1), view,
  });
  if (tex) x.drawImage(tex.canvas, 0, 0, tex.px, tex.px, -u / 2, -u / 2, u, u);
  else drawFallbackFire(x, u, t, o.palette, look, grow);
  if (!o.mini && o.embers !== false && grow > 0.5) {
    x.globalCompositeOperation = "lighter";
    for (let i = 0; i < 10; i++) {
      const ph = (((t * (0.1 + 0.12 * hash(i + 3)) * look.live + hash(i)) % 1) + 1) % 1;
      const ex = (hash(i + 1) - 0.5) * u * 0.55 + Math.sin(t * 1.3 + i * 1.7) * u * 0.03 * ph;
      const ey = u * 0.05 - ph * u * 0.42;
      const a = Math.sin(ph * Math.PI) * 0.85 * (grow - 0.5) * 2;
      const r = u * (0.003 + 0.004 * hash(i + 7)) * (1 - 0.4 * ph);
      const g = x.createRadialGradient(ex, ey, 0, ex, ey, r * 4);
      g.addColorStop(0, o.palette.light); g.addColorStop(1, "rgba(0,0,0,0)");
      x.globalAlpha = a; x.fillStyle = g; x.beginPath(); x.arc(ex, ey, r * 4, 0, Math.PI * 2); x.fill();
    }
  }
  x.restore();
}
