// The icon of an app tile. The artwork and its licence notice live in app-icon-art.ts;
// an app without artwork gets a monogram drawn here.

import { h } from "./dom";
import { SVG_ICONS } from "./app-icon-art";

const MONOGRAM_BG = "#585b70";
const INK = "#11111b";

/** The icon of an app tile: its SVG when there is one, a monogram otherwise. */
export function appIcon(id: string, size: number): HTMLElement {
  const radius = `border-radius:${size * 0.22}px`;
  const box = `width:${size}px;height:${size}px;${radius}`;
  const svgSource = Object.hasOwn(SVG_ICONS, id) ? SVG_ICONS[id] : undefined;
  if (svgSource) {
    return h("span", {
      class: "app-icon",
      style: `${box};background:#1e1e2e;padding:${size * 0.18}px`,
      "aria-hidden": "true",
      html: svgSource,
    });
  }
  return h("span", {
    class: "app-icon",
    style: `${box};background:${MONOGRAM_BG};color:${INK};font:800 ${size * 0.42}px var(--font)`,
    "aria-hidden": "true",
    text: (id[0] ?? "?").toUpperCase(),
  });
}
