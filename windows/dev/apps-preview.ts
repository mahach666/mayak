// Dev harness: the Apps tab built with the real buildApps() and styles, on a fake backend, in the
// states of own-apps (ideas §30): built-in apps only, user apps (styled icons, an exe's own icon, a
// missing app), the right-click menus, a rename, a note, and a panel to drag tiles in.
// Not part of the app bundle.

import "../src/style.css";
import { buildApps, type AppsBackend } from "../src/views/apps";
import type { ViewActions } from "../src/views/views";
import type { AppTile, PickOutcome } from "../src/core/bridge";
import { State, type Settings } from "../src/core/state";
import { fullOrder } from "../src/views/apps-model";

const INSTALLED: AppTile[] = [
  { id: "cursor", name: "Cursor", custom: false, missing: false, exe: null },
  { id: "vscode", name: "VS Code", custom: false, missing: false, exe: null },
  { id: "zed", name: "Zed", custom: false, missing: false, exe: null },
  { id: "claude", name: "Claude", custom: false, missing: false, exe: null },
  { id: "codex", name: "Codex", custom: false, missing: false, exe: null },
];
const GONE = new Set(["D:\\Old\\oldtool.exe"]);

/** Rust's apps::tiles over the fake installed list. */
function tiles(s: Settings): AppTile[] {
  const all: AppTile[] = [
    ...INSTALLED.filter((a) => !s.hiddenApps.includes(a.id)),
    ...s.customApps.map((a) => ({
      id: a.id, name: a.name, custom: true, missing: GONE.has(a.path), exe: a.path.split("\\").pop() ?? null,
    })),
  ];
  const order = fullOrder(s.appOrder, all.map((a) => a.id));
  return order.map((id) => all.find((a) => a.id === id)).filter((a): a is AppTile => a != null);
}

/** A 48 px RGBA square standing in for an exe's own icon: a rounded tile with a diagonal gradient. */
function fakeIcon(): ArrayBuffer {
  const n = 48;
  const px = new Uint8ClampedArray(n * n * 4);
  for (let y = 0; y < n; y++) {
    for (let x = 0; x < n; x++) {
      const i = (y * n + x) * 4;
      const inside = Math.hypot(Math.max(0, Math.abs(x - 23.5) - 14), Math.max(0, Math.abs(y - 23.5) - 14)) < 8;
      px[i] = 240 - x * 2;
      px[i + 1] = 90 + y * 2;
      px[i + 2] = 200;
      px[i + 3] = inside ? 255 : 0;
    }
  }
  return px.buffer;
}

function backend(pick: PickOutcome = { kind: "cancelled" }): AppsBackend {
  return {
    list: async () => tiles(State.settings),
    open: async () => true,
    pick: async () => pick,
    icon: async () => fakeIcon(),
    save: async () => {},
  };
}

const actions = new Proxy({}, { get: () => () => {} }) as ViewActions;

const USER_APPS: Partial<Settings> = {
  appOrder: ["user-chrome", "cursor", "vscode", "user-npp", "zed", "user-obs", "user-paint", "user-old", "claude"],
  hiddenApps: ["codex"],
  customApps: [
    { id: "user-chrome", name: "Google Chrome", path: "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe" },
    { id: "user-npp", name: "Notepad++", path: "C:\\Program Files\\Notepad++\\notepad++.exe" },
    { id: "user-obs", name: "Obsidian", path: "C:\\Users\\me\\AppData\\Local\\Programs\\Obsidian\\Obsidian.exe" },
    { id: "user-paint", name: "Paint.NET", path: "C:\\Program Files\\paint.net\\paintdotnet.exe" },
    { id: "user-old", name: "Old Tool", path: "D:\\Old\\oldtool.exe" },
  ],
};

const root = document.getElementById("panels")!;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function panel(id: string, title: string, settings: Partial<Settings>, after?: (el: HTMLElement) => void | Promise<void>, pick?: PickOutcome) {
  const wrap = document.createElement("div");
  wrap.className = "panel";
  wrap.id = id;
  const h2 = document.createElement("h2");
  h2.textContent = title;
  const stage = document.createElement("div");
  stage.className = "stage";
  wrap.append(h2, stage);
  root.append(wrap);
  State.settings = { ...State.settings, appOrder: [], hiddenApps: [], customApps: [], ...settings };
  const host = buildApps(actions, backend(pick));
  host.el.classList.add("on");
  stage.append(host.el);
  host.setActive?.(true);
  await sleep(80);
  await after?.(host.el);
}

function tileNamed(el: HTMLElement, name: string): HTMLButtonElement {
  return [...el.querySelectorAll<HTMLButtonElement>(".apps-tile")].find((t) => t.querySelector(".apps-name")?.textContent === name)!;
}

function rightClick(el: HTMLElement, name: string) {
  const tile = tileNamed(el, name);
  const r = tile.getBoundingClientRect();
  tile.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 }));
}

async function main() {
  State.mode = "expanded";
  State.view = "apps";
  await panel("builtin", "Built-in apps only, and the + tile", {});
  await panel("user", "User apps: styled icons by name or exe, an exe's own icon (Paint.NET), a missing app (dimmed)", USER_APPS);
  await panel("menu-user", "Right click on a user app: Rename, Remove", USER_APPS, (el) => rightClick(el, "Notepad++"));
  await panel("menu-builtin", "Right click on a built-in app: Hide", USER_APPS, (el) => rightClick(el, "Zed"));
  await panel("rename", "Rename in progress", USER_APPS, (el) => {
    rightClick(el, "Paint.NET");
    el.querySelector<HTMLButtonElement>(".apps-menu-item")!.click();
  });
  await panel("note", "A picked file that is not an app", USER_APPS, async (el) => {
    el.querySelector<HTMLButtonElement>(".apps-add")!.click();
    await sleep(30);
  }, { kind: "notAnApp" });
  await panel("missing", "Click on a missing app", USER_APPS, (el) => {
    tileNamed(el, "Old Tool").click();
  });
  // Last: State.settings stays this panel's, so a real drag here commits its own order.
  await panel("drag", "Drag here (pointer events, real mouse)", USER_APPS);
  document.body.dataset.ready = "1";
}

void main();
