// The Apps tab: the IDEs and agent apps found on this PC, one tile each. Rust looks for them
// each time the tab opens (apps_list); a click opens the app (app_open) and folds the island.

import { h, clear } from "./dom";
import { Bridge, type AppEntry } from "../core/bridge";
import { appIcon } from "./app-icons";
import { t } from "../core/i18n";
import { State } from "../core/state";
import type { ViewActions, ViewHost } from "./views";

const FAILURE_MS = 3000;

export function buildApps(actions: ViewActions): ViewHost {
  const body = h("div", { class: "apps-body" });
  const note = h("div", { class: "apps-note" });
  const el = h("div", { class: "view" }, h("div", { class: "card apps-card" }, h("div", { class: "apps" }, body, note)));

  // Only the answer to the latest request is shown, and none after the tab was left.
  let request = 0;
  let noteTimer: number | undefined;

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

  function launch(app: AppEntry) {
    const mine = request;
    void Bridge.appOpen(app.id).then((opened) => {
      // A late answer must not fold or annotate an island that has moved on.
      if (mine !== request || State.mode !== "expanded" || State.view !== "apps") return;
      if (opened) actions.collapse();
      else showNote(t("apps.couldntOpen", { name: app.name }));
    });
  }

  function render(apps: AppEntry[]) {
    if (apps.length === 0) {
      message(t("apps.none"));
      return;
    }
    clear(body);
    const grid = h("div", { class: "apps-grid" });
    for (const app of apps) {
      grid.append(h(
        "button",
        { class: "apps-tile", title: app.name, onclick: () => launch(app) },
        appIcon(app.id, 28),
        h("span", { class: "apps-name", text: app.name }),
      ));
    }
    body.append(grid);
  }

  function open() {
    clearNote();
    message(t("apps.looking"));
    const mine = ++request;
    void Bridge.appsList().then((apps) => {
      if (mine !== request) return;
      render(apps ?? []);
    });
  }

  return {
    el,
    sync() {},
    setActive(on: boolean) {
      if (on) open();
      else {
        request++;
        clearNote();
      }
    },
  };
}
