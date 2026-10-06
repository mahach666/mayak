// Geometry of the visible strip of a hidden island (Windows), shared with
// src-tauri/src/strip.rs: same constants, same formula as `line_at` (spec §3).
// Rust decides the touch from it; the page only draws it. No DOM here.

export const STRIP_ZONE_W = 600;
const STRIP_LINE_W = 120;
const STRIP_LINE_H = 4;
const STRETCH_REACH = 150;
const STRETCH_LEN = 200;
const STRETCH_THICK = 8;
const STRETCH_FOLLOW = 0.5;

/** Logical px of the strip window, origin top-left. */
export interface LineBox { x: number; y: number; w: number; h: number }

const BASE: LineBox = { x: (STRIP_ZONE_W - STRIP_LINE_W) / 2, y: 0, w: STRIP_LINE_W, h: STRIP_LINE_H };

function distance(px: number, py: number, r: LineBox): number {
  const dx = Math.max(r.x - px, 0, px - (r.x + r.w));
  const dy = Math.max(r.y - py, 0, py - (r.y + r.h));
  return Math.hypot(dx, dy);
}

/** The line for a cursor near it, or at rest for null. */
export function lineAt(cursor: { x: number; y: number } | null): LineBox {
  if (!cursor) return BASE;
  const t = Math.min(1, Math.max(0, 1 - distance(cursor.x, cursor.y, BASE) / STRETCH_REACH));
  const len = STRIP_LINE_W + t * STRETCH_LEN;
  const thick = STRIP_LINE_H + t * STRETCH_THICK;
  const centre = STRIP_ZONE_W / 2;
  const pulled = centre + t * STRETCH_FOLLOW * (cursor.x - centre);
  const c = Math.min(STRIP_ZONE_W - len / 2, Math.max(len / 2, pulled));
  return { x: c - len / 2, y: 0, w: len, h: thick };
}
