// The home page ("overview", usage-home spec §4): how much of the Claude and Codex plan limits is
// used and when they reset. Everything comes from the Rust snapshot. The texts are rewritten on
// each render and once a minute, only while the page is on screen (setActive), never through
// State.notify(), so the frame loop stays asleep.

import { h, clear, svg } from "./dom";
import { ICONS } from "./icons";
import { Bridge, onMedia, type MediaState } from "../core/bridge";
import { Sessions } from "../core/sessions";
import { State } from "../core/state";
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

/**
 * The player row at the bottom of the home page (night spec §B): `▶ Title — Artist · App` and the
 * prev / play-pause / next buttons. Shown only while Rust reports something playing; its state
 * comes from `media_state` once, then from the `media` events, never through State.notify().
 */
function buildPlayer(onVisibleChange: () => void): HTMLElement {
  const mark = h("span", { class: "player-mark" });
  const title = h("span", { class: "player-title" });
  const meta = h("span", { class: "player-meta" });
  const button = (label: string, onclick: () => void) =>
    h("button", { class: "icon-btn", title: label, onclick }) as HTMLButtonElement;
  const prev = button("Previous", () => void Bridge.mediaPrev());
  const toggle = button("Play or pause", () => void Bridge.mediaToggle());
  const next = button("Next", () => void Bridge.mediaNext());
  prev.append(svg(ICONS.skipPrev, 8));
  next.append(svg(ICONS.skipNext, 8));
  const el = h("div", { class: "player", hidden: true },
    mark, h("span", { class: "player-text" }, title, meta), h("span", { class: "player-buttons" }, prev, toggle, next));

  function show(s: MediaState | null) {
    const wasVisible = State.media != null;
    State.media = s;
    el.hidden = s == null;
    if (s) {
      mark.textContent = s.playing ? "▶" : "❚❚";
      title.textContent = s.title;
      meta.textContent = `${s.artist ? ` — ${s.artist}` : ""}${s.app ? ` · ${s.app}` : ""}`;
      clear(toggle);
      toggle.append(svg(s.playing ? ICONS.pause : ICONS.play, 8));
      toggle.title = s.playing ? "Pause" : "Play";
      prev.disabled = !s.canPrev;
      next.disabled = !s.canNext;
    }
    if (wasVisible !== (s != null)) onVisibleChange();
  }

  // An event that lands before the first answer is newer than it.
  let heard = false;
  void onMedia((s) => {
    heard = true;
    show(s);
  });
  void Bridge.mediaState().then((s) => {
    if (!heard) show(s);
  });
  return el;
}

export function buildHome(onHeightChange: () => void): ViewHost {
  const body = h("div", { class: "usage" });
  const card = h("div", { class: "card" }, body);
  const el = h("div", { class: "view" }, card);
  card.append(buildPlayer(() => {
    card.classList.toggle("with-player", State.media != null);
    // Only an open home page resizes now; any other view gets the height when it opens the home.
    if (State.mode === "expanded" && State.view === "overview") onHeightChange();
  }));
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
