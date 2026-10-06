// The home page ("overview", usage-home spec §4): how much of the Claude and Codex plan limits is
// used and when they reset. Everything comes from the Rust snapshot. The texts are rewritten on
// each render and once a minute, only while the page is on screen (setActive), never through
// State.notify(), so the frame loop stays asleep.

import { h, clear } from "./dom";
import { Bridge } from "../core/bridge";
import { Sessions } from "../core/sessions";
import { contextRing } from "./sessions";
import { usageBlocks, type UsageBlock } from "./usage-format";
import type { ViewHost } from "./views";

/** The refresh of "resets in …" and "updated …" while the page is on screen. */
const REFRESH_MS = 60_000;

function blockEl(b: UsageBlock): HTMLElement {
  const el = h("div", { class: "usage-block" }, h("div", { class: "usage-name", text: b.title }));
  if (b.updated) el.append(h("div", { class: "usage-note", text: b.updated }));
  if (b.empty) el.append(h("div", { class: "usage-empty", text: "No data yet" }));
  for (const line of b.lines) {
    // The context label of a session row: the same ring, colours and thresholds.
    const label = h("span", { class: line.percent.tone === "dim" ? "sess-ctx" : `sess-ctx ${line.percent.tone}` }, line.percent.text);
    label.prepend(contextRing(line.percent.ring));
    el.append(h(
      "div",
      { class: "usage-line" },
      label,
      h("span", { class: "usage-window", text: line.name }),
      h("span", { class: "usage-reset", text: line.resets }),
    ));
  }
  if (b.note) el.append(h("div", { class: "usage-note", text: b.note }));
  return el;
}

export function buildHome(): ViewHost {
  const body = h("div", { class: "usage" });
  const el = h("div", { class: "view" }, h("div", { class: "card" }, body));
  let renderedKey = "";
  let timer: number | null = null;

  function render() {
    clear(body);
    for (const b of usageBlocks(Sessions.snapshot?.usage, Date.now())) body.append(blockEl(b));
  }

  return {
    el,
    sync() {
      const key = JSON.stringify(Sessions.snapshot?.usage ?? null);
      if (key === renderedKey) return;
      renderedKey = key;
      render();
    },
    setActive(on: boolean) {
      if (on && timer == null) {
        // Opening the home page reads Claude's cache again (usage-home spec §2.1).
        void Bridge.usageRefresh();
        render();
        timer = window.setInterval(render, REFRESH_MS);
      } else if (!on && timer != null) {
        window.clearInterval(timer);
        timer = null;
      }
    },
  };
}
