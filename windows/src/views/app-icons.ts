// The icon of an app tile. The artwork and its licence notice live in app-icon-art.ts;
// an app without artwork gets a monogram drawn here.

import { h } from "./dom";
import { SVG_ICONS } from "./app-icon-art";

const MONOGRAM_BG = "#585b70";
const INK = "#11111b";

/** The icon of a built-in app's tile: its SVG when there is one, a monogram otherwise. */
export function appIcon(id: string, size: number): HTMLElement {
  const svgSource = Object.hasOwn(SVG_ICONS, id) ? SVG_ICONS[id] : undefined;
  return svgSource ? svgIcon(svgSource, size) : monogram(id, size);
}

const box = (size: number) => `width:${size}px;height:${size}px;border-radius:${size * 0.22}px`;

/** A styled SVG icon at `size`. */
export function svgIcon(svgSource: string, size: number): HTMLElement {
  return h("span", {
    class: "app-icon",
    style: `${box(size)};background:#1e1e2e;padding:${size * 0.18}px`,
    "aria-hidden": "true",
    html: svgSource,
  });
}

/** The first letter of `name` on a grey square. */
export function monogram(name: string, size: number): HTMLElement {
  return h("span", {
    class: "app-icon",
    style: `${box(size)};background:${MONOGRAM_BG};color:${INK};font:800 ${size * 0.42}px var(--font)`,
    "aria-hidden": "true",
    text: ([...name.trim()][0] ?? "?").toUpperCase(),
  });
}

/** An exe's own icon (an image URL) in the same box. */
export function imageIcon(url: string, size: number): HTMLElement {
  const img = h("img", { src: url, alt: "", draggable: "false" });
  img.style.cssText = "width:100%;height:100%;display:block";
  const span = h("span", { class: "app-icon", style: `width:${size}px;height:${size}px`, "aria-hidden": "true" }, img);
  return span;
}

/** A PNG data URL of square RGBA bytes (Rust `app_icon`); null when they are not a square. */
export function rgbaToDataUrl(bytes: ArrayBuffer): string | null {
  const side = Math.round(Math.sqrt(bytes.byteLength / 4));
  if (side === 0 || side * side * 4 !== bytes.byteLength) return null;
  const canvas = document.createElement("canvas");
  canvas.width = side;
  canvas.height = side;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  ctx.putImageData(new ImageData(new Uint8ClampedArray(bytes), side, side), 0, 0);
  return canvas.toDataURL("image/png");
}
