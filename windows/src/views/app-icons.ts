// Cursor and VS Code icons: Catppuccin vscode-icons (Mocha), MIT License,
// https://github.com/catppuccin/vscode-icons — see windows/THIRD_PARTY_NOTICES.md
// The other apps get a monogram drawn here.

import { h } from "./dom";

const LINES = 'fill="none" stroke-linecap="round" stroke-linejoin="round"';

const SVG_ICONS: Record<string, string> = {
  cursor:
    `<svg viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">` +
    `<path ${LINES} stroke="#cdd6f4" d="m 16.17866,12.334298 -7.0470492,4.068616 -7.0470494,-4.068616 0,-8.1372317 L 9.1316109,0.12845039 16.17866,4.1970663 Z" transform="matrix(.92099 0 0 .92169 -.41 .382)"/>` +
    `<path ${LINES} stroke="#cdd6f4" d="M 1.51,4.25 H 14.49 L 8,15.5 V 8 Z"/>` +
    `</svg>`,
  vscode:
    `<svg viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">` +
    `<path ${LINES} stroke="#89b4fa" d="M10.5 11 3 4.5h-.5l-1 1V6l9 8.5 4-2v-9l-4-2v13m0-13L5.3 6.41M3.53 8.08 1.5 10v.5l.98 1.1.52-.1 2.17-1.88m1.91-1.66L10.5 5"/>` +
    `</svg>`,
};

const MONOGRAMS: Record<string, { letters: string; bg: string }> = {
  claude: { letters: "C", bg: "#fab387" },
  codex: { letters: "Cx", bg: "#a6e3a1" },
  windsurf: { letters: "W", bg: "#94e2d5" },
  zed: { letters: "Z", bg: "#89b4fa" },
};

const UNKNOWN_BG = "#585b70";
const INK = "#11111b";

/** The icon of an app tile: an SVG for Cursor and VS Code, a monogram for the rest. */
export function appIcon(id: string, size: number): HTMLElement {
  const radius = `border-radius:${size * 0.22}px`;
  const box = `width:${size}px;height:${size}px;${radius}`;
  const svgSource = SVG_ICONS[id];
  if (svgSource) {
    return h("span", {
      class: "app-icon",
      style: `${box};background:#1e1e2e;padding:${size * 0.18}px`,
      html: svgSource,
    });
  }
  const mono = MONOGRAMS[id] ?? { letters: (id[0] ?? "?").toUpperCase(), bg: UNKNOWN_BG };
  return h("span", {
    class: "app-icon",
    style: `${box};background:${mono.bg};color:${INK};font:800 ${size * 0.42}px var(--font)`,
    text: mono.letters,
  });
}
