// The Apps tab: the IDEs and agent apps found on this PC and the apps the user added, one tile each
// (own-apps, ideas §30). Rust lists them each time the tab opens (apps_list), in the user's order; a
// click opens the app (app_open) and folds the island. The last tile "+" adds an app through the
// Open dialog; a tile is dragged to move it; a right click opens Rename / Remove (a user app) or
// Hide (a built-in one). The order and the lists live in the settings (views/apps-model.ts).

import { h, clear } from "./dom";
import { Bridge, type AppTile, type PickOutcome } from "../core/bridge";
import { appIcon, imageIcon, monogram, rgbaToDataUrl, svgIcon } from "./app-icons";
import { styledIconForApp } from "./app-icon-art";
import {
  addApp, dropIndex, hideApp, removeApp, renameApp, reorder, type AppsFields, type Box,
} from "./apps-model";
import { t } from "../core/i18n";
import { State } from "../core/state";
import type { ViewActions, ViewHost } from "./views";

const FAILURE_MS = 3000;
const ICON_SIZE = 28;
/** How far the pointer moves before a press on a tile becomes a drag. */
const DRAG_START_PX = 5;
/** The tiles sliding into their new places while a tile is dragged. */
const SLIDE_MS = 160;

/** What the tab asks of Rust; the dev preview passes its own. */
export interface AppsBackend {
  list(): Promise<AppTile[] | null>;
  open(id: string): Promise<boolean | null>;
  pick(title: string, filter: string): Promise<PickOutcome | null>;
  icon(id: string): Promise<ArrayBuffer | null>;
  /** Writes State.settings. */
  save(): Promise<unknown>;
}

const BRIDGE_BACKEND: AppsBackend = {
  list: () => Bridge.appsList(),
  open: (id) => Bridge.appOpen(id),
  pick: (title, filter) => Bridge.appPick(title, filter),
  icon: (id) => Bridge.appIcon(id),
  save: () => Bridge.saveSettings(State.settings),
};

/** An exe's own icon per user app (id and exe name), as a data URL or null: read once per run. */
const exeIcons = new Map<string, Promise<string | null>>();

function exeIcon(backend: AppsBackend, app: AppTile): Promise<string | null> {
  const key = `${app.id}|${app.exe ?? ""}`;
  let icon = exeIcons.get(key);
  if (!icon) {
    icon = backend.icon(app.id).then((bytes) => (bytes ? rgbaToDataUrl(bytes) : null));
    exeIcons.set(key, icon);
  }
  return icon;
}

/** The icon of a tile: a built-in app's art, else a user app's styled icon, its exe's icon or a monogram. */
function tileIcon(backend: AppsBackend, app: AppTile): HTMLElement {
  if (!app.custom) return appIcon(app.id, ICON_SIZE);
  const styled = styledIconForApp(app.name, app.exe);
  if (styled) return svgIcon(styled, ICON_SIZE);
  const slot = monogram(app.name, ICON_SIZE);
  if (!app.missing) {
    void exeIcon(backend, app).then((url) => {
      if (url && slot.isConnected) slot.replaceWith(imageIcon(url, ICON_SIZE));
    });
  }
  return slot;
}

const fields = (): AppsFields => ({
  appOrder: State.settings.appOrder ?? [],
  hiddenApps: State.settings.hiddenApps ?? [],
  customApps: State.settings.customApps ?? [],
});

export function buildApps(actions: ViewActions, backend: AppsBackend = BRIDGE_BACKEND): ViewHost {
  const body = h("div", { class: "apps-body" });
  const note = h("div", { class: "apps-note" });
  const scroller = h("div", { class: "apps" }, body, note);
  const el = h("div", { class: "view" }, h("div", { class: "card apps-card" }, scroller));

  // Only the answer to the latest request is shown, and none after the tab was left.
  let request = 0;
  let noteTimer: number | undefined;
  let active = false;
  /** The tiles on screen, in their order. */
  let shown: AppTile[] = [];
  let menu: HTMLElement | null = null;
  /** A drag just ended: the click that follows the release is not a launch. */
  let justDragged = false;
  /** Reasons the island is held open (the dialog, a rename, a drag); held while any is on. */
  const holds = new Set<string>();

  function hold(reason: string, on: boolean) {
    const before = holds.size > 0;
    if (on) holds.add(reason);
    else holds.delete(reason);
    if (before !== holds.size > 0) actions.holdOpen(holds.size > 0);
  }

  function message(text: string) {
    clear(body);
    body.append(h("div", { class: "apps-empty", text }));
  }

  function showNote(text: string) {
    note.textContent = text;
    window.clearTimeout(noteTimer);
    noteTimer = window.setTimeout(() => { note.textContent = ""; }, FAILURE_MS);
  }

  function clearNote() {
    window.clearTimeout(noteTimer);
    note.textContent = "";
  }

  function launch(app: AppTile) {
    if (app.missing) {
      showNote(t("apps.missing", { name: app.name }));
      return;
    }
    const mine = request;
    void backend.open(app.id).then((opened) => {
      // A late answer must not fold or annotate an island that has moved on.
      if (mine !== request || State.mode !== "expanded" || State.view !== "apps") return;
      if (opened) actions.collapse();
      else showNote(t("apps.couldntOpen", { name: app.name }));
    });
  }

  /** Writes the new fields and lists the tiles again. */
  async function commit(next: AppsFields) {
    State.settings = { ...State.settings, ...next };
    await backend.save();
    if (active) refresh();
  }

  // ── Context menu ───────────────────────────────────────────────────────────

  function closeMenu() {
    menu?.remove();
    menu = null;
  }

  function openMenu(app: AppTile, tile: HTMLElement, ev: MouseEvent) {
    closeMenu();
    const item = (text: string, run: () => void, danger = false) =>
      h("button", {
        class: danger ? "apps-menu-item danger" : "apps-menu-item",
        text,
        onclick: (e: Event) => {
          e.stopPropagation();
          closeMenu();
          run();
        },
      });
    const items = app.custom
      ? [
        item(t("apps.rename"), () => startRename(app, tile)),
        item(t("apps.remove"), () => void commit(removeApp(fields(), app.id)), true),
      ]
      : [item(t("apps.hide"), () => void commit(hideApp(fields(), app.id)))];
    menu = h("div", { class: "apps-menu", role: "menu" }, ...items);
    scroller.append(menu);
    // At the pointer, inside the card.
    const area = scroller.getBoundingClientRect();
    const w = menu.offsetWidth;
    const hgt = menu.offsetHeight;
    const x = Math.min(Math.max(4, ev.clientX - area.left), area.width - w - 4);
    const y = Math.min(Math.max(4, ev.clientY - area.top), area.height - hgt - 4);
    menu.style.left = `${Math.max(4, x)}px`;
    menu.style.top = `${Math.max(4, y) + scroller.scrollTop}px`;
  }

  // Any press outside the menu closes it.
  el.addEventListener("pointerdown", (e) => {
    if (menu && !menu.contains(e.target as Node)) closeMenu();
  });
  scroller.addEventListener("scroll", closeMenu);

  // ── Rename ────────────────────────────────────────────────────────────────

  function startRename(app: AppTile, tile: HTMLElement) {
    const label = tile.querySelector(".apps-name");
    if (!label) return;
    const input = h("input", { class: "apps-rename", value: app.name, maxlength: "40", spellcheck: "false" });
    let done = false;
    const finish = (save: boolean) => {
      if (done) return;
      done = true;
      hold("rename", false);
      void Bridge.focusWindow(false);
      if (save && input.value.trim() !== "" && input.value !== app.name) {
        void commit(renameApp(fields(), app.id, input.value));
      } else {
        input.replaceWith(label);
      }
    };
    input.addEventListener("keydown", (e) => {
      // The island's own keys (Esc folds it, arrows…) stay out of the field.
      e.stopPropagation();
      if (e.key === "Enter") finish(true);
      else if (e.key === "Escape") finish(false);
    });
    input.addEventListener("blur", () => finish(true));
    input.addEventListener("click", (e) => e.stopPropagation());
    input.addEventListener("pointerdown", (e) => e.stopPropagation());
    label.replaceWith(input);
    hold("rename", true);
    void Bridge.focusWindow(true);
    input.focus();
    input.select();
  }

  // ── Drag to reorder ───────────────────────────────────────────────────────

  function tilesOf(grid: HTMLElement): HTMLElement[] {
    return [...grid.querySelectorAll<HTMLElement>(".apps-tile[data-id]")];
  }

  /** Moves `tile` to slot `to` among the tiles, the others sliding to their new places. */
  function slide(grid: HTMLElement, tile: HTMLElement, to: number) {
    const tiles = tilesOf(grid);
    const from = tiles.indexOf(tile);
    if (from < 0 || to === from) return;
    const before = new Map(tiles.map((t) => [t, t.getBoundingClientRect()]));
    const rest = tiles.filter((t) => t !== tile);
    const anchor = rest[to] ?? grid.querySelector(".apps-add");
    grid.insertBefore(tile, anchor);
    for (const t of rest) {
      const a = before.get(t)!;
      const b = t.getBoundingClientRect();
      const dx = a.left - b.left;
      const dy = a.top - b.top;
      if (dx === 0 && dy === 0) continue;
      t.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: "none" }], {
        duration: SLIDE_MS,
        easing: "ease-out",
      });
    }
  }

  function makeDraggable(grid: HTMLElement, tile: HTMLElement) {
    tile.addEventListener("pointerdown", (down) => {
      if (down.button !== 0 || tile.querySelector(".apps-rename")) return;
      const startX = down.clientX;
      const startY = down.clientY;
      let ghost: HTMLElement | null = null;
      let offX = 0;
      let offY = 0;

      const move = (e: PointerEvent) => {
        if (e.pointerId !== down.pointerId) return;
        if (!ghost) {
          if (Math.hypot(e.clientX - startX, e.clientY - startY) < DRAG_START_PX) return;
          closeMenu();
          hold("drag", true);
          tile.setPointerCapture(e.pointerId);
          const r = tile.getBoundingClientRect();
          offX = startX - r.left;
          offY = startY - r.top;
          ghost = tile.cloneNode(true) as HTMLElement;
          ghost.classList.add("apps-ghost");
          ghost.removeAttribute("data-id");
          ghost.style.width = `${r.width}px`;
          ghost.style.height = `${r.height}px`;
          scroller.append(ghost);
          tile.classList.add("apps-dragging");
        }
        const area = scroller.getBoundingClientRect();
        ghost.style.left = `${e.clientX - area.left - offX}px`;
        ghost.style.top = `${e.clientY - area.top - offY + scroller.scrollTop}px`;
        const others = tilesOf(grid);
        const boxes: Box[] = others.map((t) => {
          const b = t.getBoundingClientRect();
          return { x: b.left, y: b.top, w: b.width, h: b.height };
        });
        const to = dropIndex(boxes, e.clientX, e.clientY);
        if (to >= 0) slide(grid, tile, to);
      };

      const end = (e: PointerEvent) => {
        if (e.pointerId !== down.pointerId) return;
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", end);
        window.removeEventListener("pointercancel", end);
        if (!ghost) return;
        ghost.remove();
        tile.classList.remove("apps-dragging");
        if (tile.hasPointerCapture(e.pointerId)) tile.releasePointerCapture(e.pointerId);
        hold("drag", false);
        justDragged = true;
        window.setTimeout(() => { justDragged = false; }, 0);
        const ids = tilesOf(grid).map((t) => t.dataset.id!);
        if (e.type === "pointercancel") {
          refresh();
          return;
        }
        if (ids.join("\n") !== shown.map((a) => a.id).join("\n")) {
          void commit({ ...fields(), appOrder: reorder(fields().appOrder, ids) });
        }
      };

      // On the window: a quick first move can leave the tile before the drag captures the pointer.
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", end);
      window.addEventListener("pointercancel", end);
    });
  }

  // ── Add ───────────────────────────────────────────────────────────────────

  let picking = false;

  async function add() {
    if (picking) return;
    picking = true;
    hold("pick", true);
    clearNote();
    try {
      const outcome = await backend.pick(t("apps.addTitle"), t("apps.addFilter"));
      if (outcome?.kind === "picked") {
        await commit(addApp(fields(), shown.map((a) => a.id), outcome, Date.now()));
      } else if (outcome?.kind === "notAnApp") {
        showNote(t("apps.notAnApp"));
      } else if (outcome == null || outcome.kind === "failed") {
        showNote(t("apps.addFailed"));
      }
    } finally {
      picking = false;
      hold("pick", false);
    }
  }

  // ── Tiles ─────────────────────────────────────────────────────────────────

  function render(apps: AppTile[]) {
    closeMenu();
    shown = apps;
    clear(body);
    const grid = h("div", { class: "apps-grid" });
    for (const app of apps) {
      const tile = h(
        "button",
        {
          class: app.missing ? "apps-tile missing" : "apps-tile",
          title: app.missing ? t("apps.missing", { name: app.name }) : app.name,
          "data-id": app.id,
          onclick: () => {
            if (!justDragged) launch(app);
          },
        },
        tileIcon(backend, app),
        h("span", { class: "apps-name", text: app.name }),
      );
      tile.addEventListener("contextmenu", (e) => {
        e.preventDefault();
        openMenu(app, tile, e);
      });
      makeDraggable(grid, tile);
      grid.append(tile);
    }
    grid.append(h(
      "button",
      { class: "apps-tile apps-add", title: t("apps.add"), "aria-label": t("apps.add"), onclick: () => void add() },
      h("span", { class: "app-icon apps-add-icon", style: `width:${ICON_SIZE}px;height:${ICON_SIZE}px`, "aria-hidden": "true", text: "+" }),
      h("span", { class: "apps-name", text: t("apps.addShort") }),
    ));
    body.append(grid);
    if (apps.length === 0) body.append(h("div", { class: "apps-empty", text: t("apps.none") }));
  }

  function refresh() {
    const mine = ++request;
    void backend.list().then((apps) => {
      if (mine !== request || !active) return;
      render(apps ?? []);
    });
  }

  function open() {
    clearNote();
    message(t("apps.looking"));
    refresh();
  }

  return {
    el,
    sync() {},
    setActive(on: boolean) {
      active = on;
      if (on) open();
      else {
        request++;
        clearNote();
        closeMenu();
        // A rename in progress ends with the tab (its blur saves it).
        const focused = document.activeElement;
        if (focused instanceof HTMLInputElement && scroller.contains(focused)) focused.blur();
      }
    },
  };
}
