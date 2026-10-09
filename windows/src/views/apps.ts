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
  addApp, hideApp, moveItem, previewSlots, removeApp, renameApp, reorder, slotIndex, type AppsFields, type Box,
} from "./apps-model";
import { t } from "../core/i18n";
import { State } from "../core/state";
import type { ViewActions, ViewHost } from "./views";

const FAILURE_MS = 3000;
const ICON_SIZE = 28;
/** How far the pointer moves before a press on a tile becomes a drag: less is a click. */
const DRAG_START_PX = 4;
/** The other tiles sliding to their preview slots, and the dropped tile into its slot. */
const SLIDE_MS = 150;

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

  /** Writes the new fields and lists the tiles again (not after a drop: the tiles are already in place). */
  async function commit(next: AppsFields, relist = true) {
    State.settings = { ...State.settings, ...next };
    await backend.save();
    if (relist && active) refresh();
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
  // The slots' boxes are taken once when the drag starts and the DOM keeps its order until the
  // drop: the other tiles only slide (transform) to where they would go, the held tile follows the
  // pointer. Nothing is measured again during the drag, so a sliding tile never moves the target.

  function tilesOf(grid: HTMLElement): HTMLElement[] {
    return [...grid.querySelectorAll<HTMLElement>(".apps-tile[data-id]")];
  }

  function makeDraggable(grid: HTMLElement, tile: HTMLElement) {
    tile.addEventListener("pointerdown", (down) => {
      if (down.button !== 0 || tile.querySelector(".apps-rename")) return;
      /** A point in the scroller's content: it does not move when the card scrolls. */
      const local = (cx: number, cy: number) => {
        const area = scroller.getBoundingClientRect();
        return { x: cx - area.left, y: cy - area.top + scroller.scrollTop };
      };
      const start = local(down.clientX, down.clientY);
      let dragging = false;
      let tiles: HTMLElement[] = [];
      let slots: Box[] = [];
      let from = -1;
      let target = -1;

      const place = (to: number) => {
        target = to;
        const slotOf = previewSlots(tiles.length, from, to);
        tiles.forEach((t, i) => {
          if (i === from) return;
          const a = slots[i];
          const b = slots[slotOf[i]];
          t.style.transform = a === b ? "" : `translate(${b.x - a.x}px, ${b.y - a.y}px)`;
        });
      };

      const move = (e: PointerEvent) => {
        if (e.pointerId !== down.pointerId) return;
        const p = local(e.clientX, e.clientY);
        if (!dragging) {
          if (Math.hypot(p.x - start.x, p.y - start.y) < DRAG_START_PX) return;
          dragging = true;
          closeMenu();
          hold("drag", true);
          tile.setPointerCapture(e.pointerId);
          tiles = tilesOf(grid);
          from = tiles.indexOf(tile);
          slots = tiles.map((t) => {
            const r = t.getBoundingClientRect();
            const o = local(r.left, r.top);
            return { x: o.x, y: o.y, w: r.width, h: r.height };
          });
          for (const t of tiles) if (t !== tile) t.style.transition = `transform ${SLIDE_MS}ms ease-out`;
          tile.classList.add("apps-dragging");
          target = from;
        }
        tile.style.transform = `translate(${p.x - start.x}px, ${p.y - start.y}px) scale(1.06)`;
        const to = slotIndex(slots, p.x, p.y, target);
        if (to >= 0 && to !== target) place(to);
      };

      const end = (e: PointerEvent) => {
        if (e.pointerId !== down.pointerId) return;
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", end);
        window.removeEventListener("pointercancel", end);
        if (!dragging) return;
        if (tile.hasPointerCapture(e.pointerId)) tile.releasePointerCapture(e.pointerId);
        hold("drag", false);
        justDragged = true;
        window.setTimeout(() => { justDragged = false; }, 0);
        const cancelled = e.type === "pointercancel";
        const order = cancelled ? tiles : moveItem(tiles, from, target);
        // Where the held tile is now, to glide it from there into its slot.
        const held = tile.getBoundingClientRect();
        // One frame: the DOM takes the new order and every transform goes, without a transition,
        // so the other tiles stay exactly where they slid to.
        for (const t of tiles) {
          t.style.transition = "none";
          t.style.transform = "";
        }
        tile.classList.remove("apps-dragging");
        const add = grid.querySelector(".apps-add");
        for (const t of order) grid.insertBefore(t, add);
        const placed = tile.getBoundingClientRect();
        tile.animate(
          [{ transform: `translate(${held.left - placed.left}px, ${held.top - placed.top}px)` }, { transform: "none" }],
          { duration: SLIDE_MS, easing: "ease-out" },
        );
        for (const t of tiles) t.style.transition = "";
        const ids = order.map((t) => t.dataset.id!);
        if (!cancelled && ids.join("\n") !== shown.map((a) => a.id).join("\n")) {
          shown = ids.map((id) => shown.find((a) => a.id === id)!);
          void commit({ ...fields(), appOrder: reorder(fields().appOrder, ids) }, false);
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
