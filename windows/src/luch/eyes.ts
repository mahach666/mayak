// How far Луч's eyes travel with the look. Pure (no shader import), so a Node check can pin it.
// p-space is the fire shader's: x right, y up, the face panel a rounded box.

/** The G4 face panel (fire.frag.glsl: rbox(p - (0.03, -0.1), (0.32, 0.22), 0.2)). */
export const FACE = { cx: 0.03, cy: -0.1, hx: 0.32, hy: 0.22, r: 0.2 } as const;
/** Eyes stay this far inside the panel's edge: the cream is solid from there in. */
export const FACE_MARGIN = 0.03;

// Mochi's eye geometry (its drawEyes): the eyes sit EYE_SP either side of the look direction and
// EYE_P below it on the body sphere.
const EYE_SP = 0.37, EYE_P = -0.12;

/** The eye pair: centre and half-extents in p-space. */
export interface EyeBox { cx: number; cy: number; hx: number; hy: number }

function sdRoundBox(px: number, py: number): number {
  const dx = Math.abs(px - FACE.cx) - FACE.hx + FACE.r, dy = Math.abs(py - FACE.cy) - FACE.hy + FACE.r;
  return Math.hypot(Math.max(dx, 0), Math.max(dy, 0)) + Math.min(Math.max(dx, dy), 0) - FACE.r;
}

function fits(b: EyeBox, dx: number, dy: number): boolean {
  for (const sx of [-1, 1]) for (const sy of [-1, 1]) {
    if (sdRoundBox(b.cx + dx + sx * b.hx, b.cy + dy + sy * b.hy) > -FACE_MARGIN) return false;
  }
  return true;
}

/**
 * The pair's offset from its rest place for a look of `yaw` / `pitch` (radians, + = right / up).
 * The travel is Mochi's: the centre of its two eyes on the body sphere, as a fraction of the body
 * half-size, applied to the face panel half-size (0.54 of the half-width at yaw 0.62, 0.49 of the
 * half-height at pitch 0.5). It is pulled back along its own direction until every corner of the
 * eye box is inside the panel, so the rounded corners and the off-centre G4 eyes are respected.
 */
export function eyeOffset(yaw: number, pitch: number, box: EyeBox): { x: number; y: number } {
  const cp = Math.cos(EYE_P + pitch);
  const rawX = FACE.hx * Math.sin(yaw) * Math.cos(EYE_SP) * cp;
  const rawY = FACE.hy * (Math.sin(EYE_P + pitch) - Math.sin(EYE_P));
  if (!fits(box, 0, 0)) return { x: 0, y: 0 };
  if (fits(box, rawX, rawY)) return { x: rawX, y: rawY };
  // the fitting offsets form a convex set holding 0: bisect to its boundary on the way to the raw offset
  let lo = 0, hi = 1;
  for (let i = 0; i < 24; i++) {
    const mid = (lo + hi) / 2;
    if (fits(box, rawX * mid, rawY * mid)) lo = mid; else hi = mid;
  }
  return { x: rawX * lo, y: rawY * lo };
}
