// Луч's rising embers from one sprite: each ember is the same radial glow (the fire's light colour at the
// centre to transparent at its edge), so instead of building a gradient for each of them in every
// frame, the glow is drawn once per colour into a sprite, and each ember fills its circle with that
// sprite as a pattern, moved and scaled onto it. The circle is filled as before (same coverage at its
// edge); only the gradient's value at each pixel is read from the sprite, bilinearly. Stamping the
// sprite with drawImage would not do: most embers are 0.3 to 3 device px across, and the gradient
// sampled at pixel centres there is not what a resampled image gives (embers vanished or doubled).

/** The sprite's glow radius (px): large enough that bilinear reads of it match the gradient. */
export const EMBER_SPRITE_R = 32;

/** The sprite's side (px): the glow and a transparent pixel round it; its centre is at side / 2. */
export const EMBER_SPRITE_SIDE = 2 * EMBER_SPRITE_R + 2;

/**
 * The pattern transform (a, d, e, f; no rotation: the context's own transform tilts it) that puts the
 * sprite's glow on a circle of `radius` centred on (ex, ey) in the context's units.
 */
export function emberPlacement(
  ex: number,
  ey: number,
  radius: number,
  out: { scale: number; e: number; f: number } = { scale: 0, e: 0, f: 0 },
): { scale: number; e: number; f: number } {
  const scale = radius / EMBER_SPRITE_R;
  const half = (EMBER_SPRITE_SIDE / 2) * scale;
  out.scale = scale;
  out.e = ex - half;
  out.f = ey - half;
  return out;
}

let colour = "";
let pattern: CanvasPattern | null = null;
let canvas: OffscreenCanvas | HTMLCanvasElement | null = null;
/** Reused for every ember (setTransform copies it); made on first use. */
let place: DOMMatrix | null = null;
/** Reused for every ember, so placing one makes no object. */
const placed = { scale: 0, e: 0, f: 0 };

/** The glow's pattern in `light`; redrawn in place when the colour changes (a colour change of the fire). */
function glow(x: CanvasRenderingContext2D, light: string): CanvasPattern | null {
  if (pattern && colour === light) return pattern;
  // a plain canvas where OffscreenCanvas is missing (older WebKitGTK on Linux)
  canvas ??=
    typeof OffscreenCanvas !== "undefined"
      ? new OffscreenCanvas(EMBER_SPRITE_SIDE, EMBER_SPRITE_SIDE)
      : Object.assign(document.createElement("canvas"), { width: EMBER_SPRITE_SIDE, height: EMBER_SPRITE_SIDE });
  const g = canvas.getContext("2d") as OffscreenCanvasRenderingContext2D | CanvasRenderingContext2D | null;
  if (!g) return null;
  const c = EMBER_SPRITE_SIDE / 2;
  g.clearRect(0, 0, EMBER_SPRITE_SIDE, EMBER_SPRITE_SIDE);
  const grad = g.createRadialGradient(c, c, 0, c, c, EMBER_SPRITE_R);
  grad.addColorStop(0, light);
  grad.addColorStop(1, "rgba(0,0,0,0)");
  g.fillStyle = grad;
  g.beginPath();
  g.arc(c, c, EMBER_SPRITE_R, 0, Math.PI * 2);
  g.fill();
  // the pattern reads the canvas as it is now: a new one after each redraw
  pattern = x.createPattern(canvas, "no-repeat");
  colour = pattern ? light : "";
  return pattern;
}

/**
 * Fills one ember: a circle of `radius` centred on (ex, ey) in the context's units with the radial
 * glow `light` → transparent, as a radial gradient over that circle would, at the context's alpha
 * and compositing.
 */
export function drawEmber(x: CanvasRenderingContext2D, ex: number, ey: number, radius: number, light: string) {
  const p = glow(x, light);
  if (p) {
    const { scale, e, f } = emberPlacement(ex, ey, radius, placed);
    place ??= new DOMMatrix();
    place.a = scale; place.b = 0; place.c = 0; place.d = scale; place.e = e; place.f = f;
    p.setTransform(place);
    x.fillStyle = p;
  } else {
    // no sprite (no 2D context for it): the gradient itself, as before
    const grad = x.createRadialGradient(ex, ey, 0, ex, ey, radius);
    grad.addColorStop(0, light);
    grad.addColorStop(1, "rgba(0,0,0,0)");
    x.fillStyle = grad;
  }
  x.beginPath();
  x.arc(ex, ey, radius, 0, Math.PI * 2);
  x.fill();
}
