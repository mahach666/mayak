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
import { formatTime, mediaPosition, needsTick, nextRepeat, PLAYER_TICK_MS, seekTarget } from "../core/player-time";
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

/** The player footer: its row, and whether it is on screen (the island's syncDom). */
export interface Player {
  el: HTMLElement;
  /** On screen: the island is open on a main tab. The progress bar's timer runs only then. */
  setActive(on: boolean): void;
}

/**
 * The player row (night spec §B): the album art, `Title` over `Artist · App`, and the prev /
 * play-pause / next buttons. The island shows it in the footer of every main tab (layout.ts
 * showsPlayer), only while Rust reports something playing; its state comes from `media_state` once,
 * then from the `media` events, never through State.notify().
 *
 * Where the player reports them (ideas §35): a progress bar with the elapsed and total time under
 * the artist (a click or a drag seeks when the player allows it), and shuffle / repeat buttons
 * around prev and next. Without them the row is exactly the one above. The bar runs on from the
 * last report (core/player-time.ts) on a PLAYER_TICK_MS timer that runs only while the row is on
 * screen and playing; the buttons show what the player reports, never what was clicked.
 */
export function buildPlayer(onVisibleChange: () => void): Player {
  const img = h("img", { class: "player-cover", alt: "", hidden: true }) as HTMLImageElement;
  const fallback = h("span", { class: "player-cover none", text: "♪" });
  const title = h("div", { class: "player-title" });
  const meta = h("div", { class: "player-meta" });
  const elapsed = h("span", { class: "player-clock" });
  const fill = h("span", { class: "player-fill" });
  const bar = h("span", { class: "player-bar" }, h("span", { class: "player-track" }, fill));
  const total = h("span", { class: "player-clock" });
  const time = h("div", { class: "player-time", hidden: true }, elapsed, bar, total);
  const text = h("div", { class: "player-text" }, title, meta, time);
  // Comfortably clickable: 30 px buttons with 16 px icons, play/pause 32 px with an 18 px icon (style.css).
  const button = (label: string, onclick: () => void, cls = "icon-btn") =>
    h("button", { class: cls, title: label, onclick }) as HTMLButtonElement;
  let current: MediaState | null = null;
  const prev = button(t("common.previous"), () => void Bridge.mediaPrev());
  const toggle = button(t("player.playPause"), () => void Bridge.mediaToggle(), "icon-btn play");
  const next = button(t("common.next"), () => void Bridge.mediaNext());
  // Each asks for the next state after the one the player last reported; the result comes back
  // as `media`.
  const shuffle = button(t("player.shuffleOff"), () => {
    if (current?.shuffle != null) void Bridge.mediaShuffle(!current.shuffle);
  });
  const repeat = button(t("player.repeatOff"), () => {
    if (current?.repeat != null) void Bridge.mediaRepeat(nextRepeat(current.repeat));
  });
  shuffle.hidden = true;
  repeat.hidden = true;
  prev.append(svg(ICONS.skipPrev, 16));
  next.append(svg(ICONS.skipNext, 16));
  shuffle.append(svg(ICONS.shuffle, 16, { stroke: 2 }));
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
    img, fallback, text, h("span", { class: "player-buttons" }, shuffle, prev, toggle, next, repeat));

  // ── The progress bar ──
  let onScreen = false;
  let timer: number | null = null;
  /** While the bar is pressed: the position under the pointer, shown instead of the player's. */
  let dragMs: number | null = null;
  let dragging = false;

  function drawTime() {
    const tl = current?.timeline;
    if (!current || !tl) return;
    const pos = dragMs ?? mediaPosition(tl, current.playing, Date.now());
    fill.style.width = `${tl.durationMs > 0 ? (pos / tl.durationMs) * 100 : 0}%`;
    elapsed.textContent = formatTime(pos);
  }

  function retime() {
    const want = needsTick(onScreen, current?.playing === true, current?.timeline ?? null);
    if (want && timer == null) {
      timer = window.setInterval(drawTime, PLAYER_TICK_MS);
    } else if (!want && timer != null) {
      window.clearInterval(timer);
      timer = null;
    }
  }

  function dragTo(e: PointerEvent) {
    const tl = current?.timeline;
    if (!tl) return;
    const r = bar.getBoundingClientRect();
    dragMs = seekTarget(e.clientX, r.left, r.width, tl.durationMs);
    drawTime();
  }

  function endDrag() {
    dragging = false;
    dragMs = null;
    drawTime();
  }

  bar.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || !current?.timeline?.seekable) return;
    e.preventDefault();
    bar.setPointerCapture(e.pointerId);
    dragging = true;
    dragTo(e);
  });
  bar.addEventListener("pointermove", (e) => {
    if (dragging) dragTo(e);
  });
  bar.addEventListener("pointerup", (e) => {
    if (!dragging) return;
    dragTo(e);
    const target = dragMs;
    // Back to what the player reports: the new place shows once it says so.
    endDrag();
    if (target != null && current?.timeline?.seekable) void Bridge.mediaSeek(target);
  });
  bar.addEventListener("pointercancel", () => {
    if (dragging) endDrag();
  });
  bar.addEventListener("lostpointercapture", () => {
    if (dragging) endDrag();
  });

  function show(s: MediaState | null) {
    const wasVisible = State.media != null;
    State.media = s;
    current = s;
    el.hidden = s == null;
    if (s) {
      if (s.cover && s.cover === brokenCover) {
        img.removeAttribute("src");
        img.hidden = true;
        fallback.hidden = false;
      } else if (s.cover) {
        // The same cover again (a timeline or play/pause event) is not decoded again.
        if (img.getAttribute("src") !== s.cover) img.src = s.cover;
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

      const tl = s.timeline && s.timeline.durationMs > 0 ? s.timeline : null;
      time.hidden = tl == null;
      text.classList.toggle("timed", tl != null);
      bar.classList.toggle("seekable", tl?.seekable === true);
      if (tl) total.textContent = formatTime(tl.durationMs);
      if (!tl?.seekable && dragging) endDrag();
      drawTime();

      shuffle.hidden = s.shuffle == null;
      shuffle.classList.toggle("on", s.shuffle === true);
      shuffle.setAttribute("aria-pressed", String(s.shuffle === true));
      shuffle.title = t(s.shuffle ? "player.shuffleOn" : "player.shuffleOff");
      repeat.hidden = s.repeat == null;
      if (s.repeat != null) {
        repeat.classList.toggle("on", s.repeat !== "off");
        repeat.setAttribute("aria-pressed", String(s.repeat !== "off"));
        repeat.title = t(s.repeat === "one" ? "player.repeatOne" : s.repeat === "all" ? "player.repeatAll" : "player.repeatOff");
        clear(repeat);
        repeat.append(svg(s.repeat === "one" ? ICONS.repeatOne : ICONS.repeat, 16, { stroke: 2 }));
      }
    } else if (dragging) {
      endDrag();
    }
    retime();
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
  return {
    el,
    setActive(on: boolean) {
      if (on === onScreen) return;
      onScreen = on;
      // Opened: the bar catches up at once rather than at the next tick.
      if (on) drawTime();
      retime();
    },
  };
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
