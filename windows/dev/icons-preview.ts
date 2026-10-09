// Dev harness: the six app icons at 16, 22, 28 and 48 px on the island's black, next to the monograms
// they replace, then a row of the Apps tab built with the real appIcon() and styles. Not part of the app bundle.

import "../src/style.css";
import { appIcon } from "../src/views/app-icons";
import { styledIconFor } from "../src/views/app-icon-art";

const APPS: [string, string][] = [
  ["cursor", "Cursor"], ["vscode", "VS Code"], ["windsurf", "Windsurf"], ["zed", "Zed"], ["claude", "Claude"], ["codex", "Codex"],
];
const SIZES = [16, 22, 28, 48];
// the monograms these icons replace (app-icons.ts before the artwork; Cursor and VS Code never had one)
const OLD: Record<string, { letters: string; bg: string }> = {
  claude: { letters: "C", bg: "#fab387" },
  codex: { letters: "Cx", bg: "#a6e3a1" },
  windsurf: { letters: "W", bg: "#94e2d5" },
  zed: { letters: "Z", bg: "#89b4fa" },
};

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text;
  return e;
}

function oldMonogram(id: string, size: number): HTMLElement {
  const m = OLD[id];
  const span = el("span", "app-icon", m ? m.letters : "—");
  span.style.cssText = m
    ? `width:${size}px;height:${size}px;border-radius:${size * 0.22}px;background:${m.bg};color:#11111b;font:800 ${size * 0.42}px var(--font)`
    : `width:${size}px;height:${size}px;color:#4b5563`;
  return span;
}

const sizes = document.getElementById("sizes")!;
const head = el("tr");
head.append(el("th"), ...SIZES.map((s) => el("th", "", `${s}px`)), el("th", "", "old 28px / 48px"));
sizes.append(head);
for (const [id, name] of APPS) {
  const tr = el("tr");
  tr.append(el("td", "name", name));
  for (const s of SIZES) {
    const td = el("td");
    td.append(appIcon(id, s));
    tr.append(td);
  }
  const old = el("td", "sep");
  const cell = el("span", "cell");
  cell.append(oldMonogram(id, 28), oldMonogram(id, 48));
  old.append(cell);
  tr.append(old);
  sizes.append(tr);
}

// the Apps tab: same markup as views/apps.ts render(), plus one app without artwork (monogram fallback)
const real = document.getElementById("real")!;
const tab = el("div", "apps");
const grid = el("div", "apps-grid");
for (const [id, name] of APPS) {
  const tile = el("button", "apps-tile");
  tile.title = name;
  tile.append(appIcon(id, 28), el("span", "apps-name", name));
  grid.append(tile);
}
const grid2 = el("div", "apps-grid");
const extra = el("button", "apps-tile");
extra.append(appIcon("notepad", 28), el("span", "apps-name", "Notepad"));
grid2.append(extra);
tab.append(grid, grid2);
real.append(tab);

const lookups = document.getElementById("lookups")!;
for (const q of ["Cursor.exe", "Code.exe", "Visual Studio Code", "WINDSURF", "zed", "Claude", "codex.exe", "notepad"]) {
  const tr = el("tr");
  const icon = el("td");
  const svg = styledIconFor(q);
  if (svg) {
    const span = el("span", "app-icon");
    span.style.cssText = "width:22px;height:22px;border-radius:5px;background:#1e1e2e;padding:4px";
    span.innerHTML = svg;
    icon.append(span);
  } else icon.textContent = "null";
  tr.append(el("td", "name", JSON.stringify(q)), icon);
  lookups.append(tr);
}
