// The 2D flame used when WebGL is missing or lost (spec §2): the same silhouette and face panel,
// gradients instead of the shader, a gentle sway instead of the fire field. The pomodoro looks are only
// approximated: a taller tongue for work, a lower one for the short break (no breath), and for the long
// break a stub of a tongue on a body tinted towards the deep colour (no cracks, glow or sparks).

import type { FocusLevels } from "./focus";
import type { FireLook } from "./look";
import type { FirePalette } from "./palette";

/** Draws centred on the current origin in a box of u CSS px (p −1…1 maps to ±u/2, y up → canvas y down). */
export function drawFallbackFire(x: CanvasRenderingContext2D, u: number, t: number, pal: FirePalette, look: FireLook, grow: number, focus?: FocusLevels) {
  const s = u / 2;
  const P = (px: number, py: number): [number, number] => [px * s, -py * s];
  x.save();
  x.translate(0, 0.44 * s); x.scale(grow, grow); x.translate(0, -0.44 * s);
  const sway = Math.sin(t * 2.1 * look.live) * 0.03;
  const w = focus?.work ?? 0, r = focus?.rest ?? 0, e = focus?.ember ?? 0;
  const tip = 0.66 * look.tall * (1 + 0.12 * w) * (1 - 0.25 * r) * (1 - 0.75 * e);
  const body = new Path2D();
  body.moveTo(...P(0.01, -0.44));
  body.bezierCurveTo(...P(0.42, -0.44), ...P(0.47, -0.05), ...P(0.3, 0.12));
  body.quadraticCurveTo(...P(0.12 + sway, tip * 0.55), ...P(-0.09 + sway * 2, tip));
  body.quadraticCurveTo(...P(-0.2, tip * 0.45), ...P(-0.36, 0.08));
  body.bezierCurveTo(...P(-0.47, -0.05), ...P(-0.42, -0.44), ...P(0.01, -0.44));
  const g = x.createRadialGradient(0, 0.1 * s, 0.05 * s, 0, 0.1 * s, 0.6 * s);
  g.addColorStop(0, pal.light); g.addColorStop(0.7, pal.mid); g.addColorStop(1, pal.deep);
  x.shadowColor = pal.mid; x.shadowBlur = 0.12 * s;
  x.fillStyle = g; x.fill(body);
  x.shadowBlur = 0;
  if (e > 0) {
    // embers: the body darkens towards the deep colour (source-atop keeps it inside the body)
    x.save();
    x.globalCompositeOperation = "source-atop"; x.globalAlpha *= 0.6 * e;
    x.fillStyle = pal.deep; x.fill(body);
    x.restore();
  }
  x.fillStyle = pal.core;
  x.beginPath(); x.roundRect((0.03 - 0.32) * s, (0.1 - 0.22) * s, 0.64 * s, 0.44 * s, 0.2 * s); x.fill();
  x.restore();
}
