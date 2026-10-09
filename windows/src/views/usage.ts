// The home page ("overview", usage-home spec §4): how much of the Claude and Codex plan limits is
// used and when they reset. Everything comes from the Rust snapshot. The texts are rewritten on
// each render and once a minute, only while the page is on screen (setActive), never through
// State.notify(), so the frame loop stays asleep.

import { h, clear, svg } from "./dom";
import { ICONS } from "./icons";
import { Bridge, onMedia, type MediaState } from "../core/bridge";
import { Sessions } from "../core/sessions";
import { State } from "../core/state";
import { t } from "../core/i18n";
import { contextRing } from "./sessions";
import { usageBlocks, type UsageBlock } from "./usage-format";
import type { ViewHost } from "./views";

/** The refresh of "resets in …" and "updated …" while the page is on screen. */
const REFRESH_MS = 60_000;

function blockEl(b: UsageBlock): HTMLElement {
  const el = h("div", { class: "usage-block" }, h("div", { class: "usage-name", text: b.title }));
  if (b.updated) el.append(h("div", { class: "usage-note", text: b.updated }));
  if (b.empty) el.append(h("div", { class: "usage-empty", text: t("usage.noData") }));
  for (const line of b.lines) {
    // The context label of a session row: the same ring, colours and thresholds.
    const label = h("span", { class: line.percent.tone === "dim" ? "sess-ctx" : `sess-ctx ${line.percent.tone}` }, line.percent.text);
    label.prepend(contextRing(line.percent.ring));
    el.append(h(
      "div",
      { class: "usage-line" },
      label,
      h("span", { class: "usage-window", text: line.name }),
      // A longer translation ends in an ellipsis; the whole text stays in the tooltip.
      h("span", { class: "usage-reset", title: line.resets, text: line.resets }),
    ));
  }
  if (b.note) el.append(h("div", { class: "usage-note", text: b.note }));
  return el;
}

/**
 * The player row (night spec §B): the album art, `Title` over `Artist · App`, and the prev /
 * play-pause / next buttons. The island shows it in the footer of every main tab (layout.ts
 * showsPlayer), only while Rust reports something playing; its state comes from `media_state` once,
 * then from the `media` events, never through State.notify().
 */
export function buildPlayer(onVisibleChange: () => void): HTMLElement {
  const img = h("img", { class: "player-cover", alt: "", hidden: true }) as HTMLImageElement;
  const fallback = h("span", { class: "player-cover none", text: "♪" });
  const title = h("div", { class: "player-title" });
  const meta = h("div", { class: "player-meta" });
  // Comfortably clickable: 30 px buttons with 16 px icons, play/pause 32 px with an 18 px icon (style.css).
  const button = (label: string, onclick: () => void, cls = "icon-btn") =>
    h("button", { class: cls, title: label, onclick }) as HTMLButtonElement;
  const prev = button(t("common.previous"), () => void Bridge.mediaPrev());
  const toggle = button(t("player.playPause"), () => void Bridge.mediaToggle(), "icon-btn play");
  const next = button(t("common.next"), () => void Bridge.mediaNext());
  prev.append(svg(ICONS.skipPrev, 16));
  next.append(svg(ICONS.skipNext, 16));
  // A cover that does not load (a broken image) falls back to the note, like no cover at all.
  // The cover that failed is remembered: the next media events with the same string keep the note
  // instead of loading it again. A different cover is tried again.
  let brokenCover: string | null = null;
  img.onerror = () => {
    brokenCover = img.getAttribute("src");
    img.removeAttribute("src");
    img.hidden = true;
    fallback.hidden = false;
  };
  const el = h("div", { class: "player", hidden: true },
    img, fallback, h("div", { class: "player-text" }, title, meta), h("span", { class: "player-buttons" }, prev, toggle, next));

  function show(s: MediaState | null) {
    const wasVisible = State.media != null;
    State.media = s;
    el.hidden = s == null;
    if (s) {
      if (s.cover && s.cover === brokenCover) {
        img.removeAttribute("src");
        img.hidden = true;
        fallback.hidden = false;
      } else if (s.cover) {
        img.src = s.cover;
        img.hidden = false;
        fallback.hidden = true;
      } else {
        img.removeAttribute("src");
        img.hidden = true;
        fallback.hidden = false;
      }
      title.textContent = s.title;
      meta.textContent = [s.artist, s.app].filter(Boolean).join(" · ");
      clear(toggle);
      toggle.append(svg(s.playing ? ICONS.pause : ICONS.play, 18));
      toggle.title = t(s.playing ? "player.pause" : "player.play");
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

export function buildHome(): ViewHost {
  const body = h("div", { class: "usage" });
  const card = h("div", { class: "card" }, body);
  const el = h("div", { class: "view" }, card);
  let renderedKey = "";
  let timer: number | null = null;

  function render() {
    clear(body);
    for (const b of usageBlocks(Sessions.snapshot?.usage, Date.now(), t)) body.append(blockEl(b));
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
