/*! Cursor and VS Code icons: Catppuccin vscode-icons (Mocha), MIT License,
    https://github.com/catppuccin/vscode-icons — see windows/THIRD_PARTY_NOTICES.md.
    Windsurf, Zed, Claude and Codex icons: Mayak's own simplified line drawings in the Catppuccin style
    (Catppuccin has none for them); the products' names and marks belong to their owners. */
// The "/*!" keeps this notice in the minified bundle. No imports: a Node check runs this file as is.

const LINES = 'fill="none" stroke-linecap="round" stroke-linejoin="round"';

const svg = (body: string) =>
  `<svg viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">${body}</svg>`;

/** The icon artwork per app id (apps.rs ids): 16×16 line art in Catppuccin Mocha colours. */
export const SVG_ICONS: Readonly<Record<string, string>> = {
  cursor: svg(
    `<path ${LINES} stroke="#cdd6f4" d="m 16.17866,12.334298 -7.0470492,4.068616 -7.0470494,-4.068616 0,-8.1372317 L 9.1316109,0.12845039 16.17866,4.1970663 Z" transform="matrix(.92099 0 0 .92169 -.41 .382)"/>` +
      `<path ${LINES} stroke="#cdd6f4" d="M 1.51,4.25 H 14.49 L 8,15.5 V 8 Z"/>`,
  ),
  vscode: svg(
    `<path ${LINES} stroke="#89b4fa" d="M10.5 11 3 4.5h-.5l-1 1V6l9 8.5 4-2v-9l-4-2v13m0-13L5.3 6.41M3.53 8.08 1.5 10v.5l.98 1.1.52-.1 2.17-1.88m1.91-1.66L10.5 5"/>`,
  ),
  // Three waves rising to the right, the longest at the bottom.
  windsurf: svg(
    `<path ${LINES} stroke="#94e2d5" d="M1.5 12.5c2.5 0 3-2.5 5.5-2.5s3 2 7.5-1.5M3.5 8.5c2 0 2.5-2.25 4.75-2.25S11 8 14.5 5M6 4.5c1.5 0 2-1.75 3.75-1.75S12 4 14.5 1.75"/>`,
  ),
  // A Z in a rounded frame.
  zed: svg(
    `<rect ${LINES} stroke="#74c7ec" x="1.5" y="1.5" width="13" height="13" rx="2.5"/>` +
      `<path ${LINES} stroke="#74c7ec" d="M5 5h6l-6 6h6"/>`,
  ),
  // A twelve-ray starburst, long and short rays alternating.
  claude: svg(
    `<path ${LINES} stroke="#fab387" d="M8 5.5V1.5M9.25 5.83l1.13-1.94M10.17 6.75l3.46-2M10.5 8h2.25M10.17 9.25l3.46 2M9.25 10.17l1.13 1.94M8 10.5v4M6.75 10.17l-1.12 1.94M5.83 9.25l-3.46 2M5.5 8H3.25M5.83 6.75l-3.46-2M6.75 5.83 5.62 3.89"/>`,
  ),
  // A cloud with a terminal prompt.
  codex: svg(
    `<path ${LINES} stroke="#cdd6f4" d="M4.5 13.5a3.5 3.5 0 0 1-.4-6.98 4.1 4.1 0 0 1 7.8-.9 3.75 3.75 0 0 1-.4 7.88Z"/>` +
      `<path ${LINES} stroke="#a6e3a1" d="m5 8.5 2 1.75-2 1.75m3.5 0h2.5"/>`,
  ),
};

/** Names and exe names (lower case, without ".exe") that mean each app id. */
const NAMES: Readonly<Record<string, string>> = {
  cursor: "cursor",
  code: "vscode",
  vscode: "vscode",
  "vs code": "vscode",
  "visual studio code": "vscode",
  windsurf: "windsurf",
  zed: "zed",
  claude: "claude",
  codex: "codex",
};

/** The app id an app name or exe name (a bare name or a path, any case, ".exe" optional) stands for, or null. */
export function styledIconId(nameOrExe: string): string | null {
  const base = nameOrExe.trim().split(/[\\/]/).pop() ?? "";
  const key = base.toLowerCase().replace(/\.exe$/, "").trim();
  return Object.hasOwn(NAMES, key) ? NAMES[key] : null;
}

/** The icon SVG for an app name or exe name, or null when Mayak has none (a monogram is used then). */
export function styledIconFor(nameOrExe: string): string | null {
  const id = styledIconId(nameOrExe);
  return id === null ? null : SVG_ICONS[id];
}
