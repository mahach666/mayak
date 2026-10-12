// The home page ("overview", variant B of the home mockup): Luch on the card's left, the integration
// tiles in the middle (the active integrations with a key, renderIntegrationCard as tiles), and on
// the right a 2 × 2 table of plan-limit rings (Claude and Codex, 5h and 7d) over the short balance
// line (OpenRouter, DeepSeek, Kimi). Every detail of a limit or a balance (windows, reset times, the
// pace, "new window", where the data come from) is in a popover per row: open on hover and keyboard
// focus, pinned by a click, closed by Esc, a page change or the island hiding. The pure parts are in
// home-model.ts and usage-format.ts. The limit texts are rewritten on each render, once a minute and
// when a window resets, only while the page is on screen (setActive), never through State.notify(),
// so the frame loop stays asleep; the tiles follow State like the integration cards did.

import { h, clear, svg } from "./dom";
import { ICONS } from "./icons";
import { Bridge, onMedia, onUsageRollover, type MediaState } from "../core/bridge";
import { Sessions } from "../core/sessions";
import { State } from "../core/state";
import { t } from "../core/i18n";
import { contextRing } from "./sessions";
import {
  balanceChips, homeTileIds, homeTileRows, placePopover, popoverBlocks, popoverRows, RING_SIZE, RING_STROKE,
  ringColumns, ringGeometry, ringRowLabel, ringRows, tileAlert, tileBadge, tileWide,
  type BalanceChip, type Box, type PopoverGroup, type PopoverRow, type RingCell, type RingProviderId,
} from "./home-model";
import { pillBadgeEl, renderIntegrationCard, type IntegrationCardHooks } from "./integrations";
import {
  formatTime, needsTick, nextRepeat, PLAYER_TICK_MS, SEEK_HOLD_MS, seekTarget, settleHold, shownPosition, startHold,
  trackKey, type SeekHold,
} from "../core/player-time";
import type { ViewActions, ViewHost } from "./views";

/** The refresh of "resets in …" and "updated …" while the page is on screen. */
const REFRESH_MS = 60_000;
/** A popover opened by hover stays this long after the pointer leaves, so it can be reached. */
const HIDE_DELAY_MS = 150;

const SVG_NS = "http://www.w3.org/2000/svg";

/** A ring of the limits table (home-model.ts ringGeometry): the percent inside, or "—". */
function limitRing(cell: RingCell): SVGSVGElement {
  const g = ringGeometry(RING_SIZE, RING_STROKE, cell.percent);
  const el = document.createElementNS(SVG_NS, "svg");
  el.setAttribute("class", `home-ring ${cell.tone}`);
  el.setAttribute("width", String(RING_SIZE));
  el.setAttribute("height", String(RING_SIZE));
  el.setAttribute("viewBox", `0 0 ${RING_SIZE} ${RING_SIZE}`);
  el.setAttribute("aria-hidden", "true");
  const circle = (cls: string) => {
    const c = document.createElementNS(SVG_NS, "circle");
    c.setAttribute("class", cls);
    c.setAttribute("cx", String(g.c));
    c.setAttribute("cy", String(g.c));
    c.setAttribute("r", String(g.r));
    c.setAttribute("stroke-width", String(RING_STROKE));
    return c;
  };
  el.append(circle("track"));
  if (g.dash) {
    const fill = circle("fill");
    fill.setAttribute("stroke-dasharray", g.dash);
    fill.setAttribute("transform", `rotate(-90 ${g.c} ${g.c})`);
    el.append(fill);
  }
  const text = document.createElementNS(SVG_NS, "text");
  text.setAttribute("x", String(g.c));
  text.setAttribute("y", String(g.c + 0.5));
  text.setAttribute("text-anchor", "middle");
  text.setAttribute("dominant-baseline", "central");
  text.textContent = cell.text;
  el.append(text);
  return el;
}

/** A sum of the balance line, with its 12 px ring where the key has a limit. */
function chipEl(c: BalanceChip): HTMLElement {
  const el = h("span", { class: c.tone === "dim" ? "home-bal" : `home-bal ${c.tone}` }, h("b", { text: c.text }));
  if (c.ring != null) el.prepend(contextRing(c.ring));
  return el;
}

/** A row of a popover: the markup the home page's blocks had (usage-line, sess-ctx and its ring). */
function popoverRowEl(r: PopoverRow): HTMLElement {
  switch (r.kind) {
    case "head":
      return h("div", { class: "home-pop-head" },
        h("span", { class: "usage-name", text: r.title }),
        r.updated ? h("span", { class: "usage-note", text: r.updated }) : null);
    case "line": {
      // The context label of a session row: the same ring, colours and thresholds. A balance line
      // (a sum of money) has the same label without the ring.
      const label = r.ring == null
        ? h("span", { class: "sess-ctx usage-amount" }, r.label)
        : h("span", { class: r.tone === "dim" ? "sess-ctx" : `sess-ctx ${r.tone}` }, r.label);
      if (r.ring != null) label.prepend(contextRing(r.ring));
      // A longer translation ends in an ellipsis; the whole text stays in the tooltip.
      return h("div", { class: "usage-line" }, label,
        h("span", { class: "usage-window", text: r.name }),
        h("span", { class: "usage-reset", title: r.resets, text: r.resets }));
    }
    case "pace":
      return h("div", { class: "home-pop-pace", title: r.text, text: r.text });
    case "empty":
      return h("div", { class: "usage-empty", text: r.text });
    case "note":
      return h("div", { class: "usage-note", text: r.text });
    case "sep":
      return h("div", { class: "home-pop-sep" });
  }
}

/** `el`'s box in `root`'s coordinates, from the offsets (a scaling transition of the view does not skew it). */
function boxWithin(el: HTMLElement, root: HTMLElement): Box {
  let left = 0;
  let top = 0;
  let n: HTMLElement | null = el;
  while (n && n !== root) {
    left += n.offsetLeft;
    top += n.offsetTop;
    const parent = n.offsetParent as HTMLElement | null;
    if (parent && parent !== root) {
      left += parent.clientLeft;
      top += parent.clientTop;
    }
    n = parent;
  }
  return { left, top, width: el.offsetWidth, height: el.offsetHeight };
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
 * screen and playing; after a seek it holds the target until the player confirms it (at most
 * SEEK_HOLD_MS). The buttons show what the player reports, never what was clicked.
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
  /** After a seek: the target the bar holds until the player confirms it (core/player-time.ts). */
  let hold: SeekHold | null = null;
  /** Ends the hold at SEEK_HOLD_MS even when no report comes (a paused player has no tick). */
  let holdTimer: number | null = null;

  function setHold(h: SeekHold | null) {
    hold = h;
    if (holdTimer != null) {
      window.clearTimeout(holdTimer);
      holdTimer = null;
    }
    if (h) {
      holdTimer = window.setTimeout(() => {
        holdTimer = null;
        hold = null;
        drawTime();
      }, SEEK_HOLD_MS);
    }
  }

  function drawTime() {
    const tl = current?.timeline;
    if (!current || !tl) return;
    const pos = dragMs ?? shownPosition(hold, tl, current.playing, Date.now());
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
    // The bar holds the target until the player reports it (or SEEK_HOLD_MS pass): the reports
    // that still show the old place do not pull it back.
    if (target != null && current?.timeline?.seekable) {
      setHold(startHold(target, Date.now(), trackKey(current)));
      void Bridge.mediaSeek(target);
    }
    endDrag();
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
      const kept = settleHold(hold, trackKey(s), tl, s.playing, Date.now());
      if (kept !== hold) setHold(kept);
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
    } else {
      if (dragging) endDrag();
      setHold(null);
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

export function buildHome(actions: ViewActions): ViewHost {
  // ── The rings column ──
  const [col5, col7] = ringColumns(t);
  const popId = "home-pop";
  const anchor = (group: PopoverGroup, cls: string) =>
    h("button", { class: `home-hit ${cls}`, "aria-expanded": "false", "aria-controls": popId, "data-group": group });
  const ringBtns: Record<RingProviderId, HTMLButtonElement> = {
    claude: anchor("claude", "home-rb"),
    codex: anchor("codex", "home-rb"),
  };
  const balBtn = anchor("balances", "home-bals");
  const anchors: Record<PopoverGroup, HTMLButtonElement> = { ...ringBtns, balances: balBtn };
  const rings = h("div", { class: "home-rings" },
    h("div", { class: "home-rb home-rb-head", "aria-hidden": "true" },
      h("span"), h("span", { class: "home-wlabel", text: col5 }), h("span", { class: "home-wlabel", text: col7 })),
    ringBtns.claude, ringBtns.codex, balBtn);

  // ── The tiles ──
  const tilesEl = h("div", { class: "home-tiles" });
  const hint = h("div", { class: "home-hint" },
    h("span", { class: "t", text: t("home.connectHint") }),
    h("button", {
      class: "link-btn",
      text: `${t("header.settings")} → ${t("set.integrations")}`,
      onclick: () => actions.openSettingsWindow(),
    }));

  const pop = h("div", { class: "home-pop", id: popId, role: "region", hidden: true });
  const card = h("div", { class: "card" }, h("div", { class: "home" }, h("div", { class: "home-luch" }), tilesEl, rings));
  const el = h("div", { class: "view home-view" }, card, pop);

  // ── The popover ──
  let open: PopoverGroup | null = null;
  let pinned = false;
  let hideTimer: number | null = null;

  function cancelHide() {
    if (hideTimer != null) {
      window.clearTimeout(hideTimer);
      hideTimer = null;
    }
  }

  function markOpen(group: PopoverGroup | null) {
    if (open) {
      anchors[open].classList.remove("open");
      anchors[open].setAttribute("aria-expanded", "false");
    }
    open = group;
    if (group) {
      anchors[group].classList.add("open");
      anchors[group].setAttribute("aria-expanded", "true");
    }
  }

  function hidePopover() {
    cancelHide();
    markOpen(null);
    pinned = false;
    pop.hidden = true;
  }

  /** Fills the popover for `group` and places it by its anchor, within the view (home-model.ts placePopover). */
  function fillPopover(group: PopoverGroup) {
    const blocks = popoverBlocks(group, Sessions.snapshot?.usage, Date.now(), t);
    pop.replaceChildren(...popoverRows(blocks, t).map(popoverRowEl));
    pop.setAttribute("aria-label", group === "balances" ? t("usage.balances") : blocks[0]?.title ?? "");
    pop.style.maxHeight = "none";
    pop.hidden = false;
    const at = placePopover(
      boxWithin(anchors[group], el),
      { width: pop.offsetWidth, height: pop.offsetHeight },
      { width: el.clientWidth, height: el.clientHeight },
    );
    pop.style.maxHeight = `${at.maxH}px`;
    pop.style.left = `${at.left}px`;
    pop.style.top = `${at.top}px`;
  }

  function showPopover(group: PopoverGroup) {
    cancelHide();
    if (open !== group) {
      markOpen(group);
      pop.scrollTop = 0;
    }
    fillPopover(group);
  }

  /** After the pointer or the focus left: closes unless pinned, or hovered or focused again meanwhile. */
  function hideSoon() {
    cancelHide();
    hideTimer = window.setTimeout(() => {
      hideTimer = null;
      if (!open || pinned) return;
      const a = anchors[open];
      if (pop.matches(":hover") || a.matches(":hover") || document.activeElement === a) return;
      hidePopover();
    }, HIDE_DELAY_MS);
  }

  for (const [group, a] of Object.entries(anchors) as [PopoverGroup, HTMLButtonElement][]) {
    a.addEventListener("pointerenter", () => {
      cancelHide();
      // A pinned popover stays put while the pointer passes over another row.
      if (!pinned || open === group) showPopover(group);
    });
    a.addEventListener("pointerleave", hideSoon);
    a.addEventListener("focus", () => {
      if (open !== group) pinned = false;
      showPopover(group);
    });
    a.addEventListener("blur", hideSoon);
    // A click or a tap pins it; a second one on the same row closes it.
    a.addEventListener("click", () => {
      if (pinned && open === group) {
        hidePopover();
      } else {
        showPopover(group);
        pinned = true;
      }
    });
  }
  pop.addEventListener("pointerenter", cancelHide);
  pop.addEventListener("pointerleave", hideSoon);
  // Esc closes the popover and nothing else: the island folds on the next Esc (island.ts).
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || !open) return;
    e.preventDefault();
    hidePopover();
  }, true);
  // A press anywhere else lets a pinned popover go.
  document.addEventListener("pointerdown", (e) => {
    if (!open || !pinned) return;
    const target = e.target as Node | null;
    if (target && (pop.contains(target) || Object.values(anchors).some((a) => a.contains(target)))) return;
    hidePopover();
  }, true);

  // ── The limits ──
  let renderedKey = "";
  let timer: number | null = null;
  // A window that has just reset reads "new window" at once rather than at the next minute tick.
  void onUsageRollover(() => {
    if (timer != null) render();
  });

  function render() {
    const usage = Sessions.snapshot?.usage;
    for (const row of ringRows(usage, Date.now())) {
      const b = ringBtns[row.id];
      b.replaceChildren(h("span", { class: "home-pname", text: row.title }), limitRing(row.cells[0]), limitRing(row.cells[1]));
      b.setAttribute("aria-label", ringRowLabel(row, t));
    }
    const chips = balanceChips(usage);
    balBtn.hidden = chips.length === 0;
    balBtn.replaceChildren(...chips.flatMap((c, i) => (i > 0 ? [h("span", { class: "home-bal-sep", text: "·" }), chipEl(c)] : [chipEl(c)])));
    balBtn.setAttribute("aria-label", `${t("usage.balances")}: ${chips.map((c) => `${c.title} ${c.text}`).join(", ")}`);
    if (open === "balances" && chips.length === 0) hidePopover();
    else if (open) fillPopover(open);
  }

  // ── The tiles ──
  const tiles = new Map<string, { el: HTMLElement; key: string }>();
  const detailOpen = new Map<string, boolean>();
  const hooksFor = (id: string): IntegrationCardHooks => ({
    get detailOpen() {
      return detailOpen.get(id) ?? false;
    },
    openDetail() {
      detailOpen.set(id, true);
      State.notify();
    },
    closeDetail() {
      detailOpen.set(id, false);
      State.notify();
    },
    openSettings: () => actions.openSettingsWindow(),
  });

  function renderTiles() {
    const ids = homeTileIds(State.tasks, State.integrations);
    for (const id of [...tiles.keys()]) if (!ids.includes(id)) tiles.delete(id);
    if (ids.length === 0) {
      if (tilesEl.firstChild !== hint || tilesEl.childNodes.length !== 1) tilesEl.replaceChildren(hint);
      tilesEl.classList.add("empty");
      tilesEl.style.gridTemplateRows = "";
      return;
    }
    tilesEl.classList.remove("empty");
    tilesEl.style.gridTemplateRows = `repeat(${homeTileRows(ids.length)}, minmax(0, 1fr))`;
    const wide = tileWide(ids.length);
    ids.forEach((id, i) => {
      const task = State.tasks.find((x) => x.id === id)!;
      const info = State.integrations[id];
      const badge = tileBadge(task);
      const key = [
        wide[i], detailOpen.get(id) ?? false, task.state, task.steps.join("|"), badge,
        info?.loaded, info?.error, info?.configured, JSON.stringify(info?.data ?? {}),
      ].join("~");
      const known = tiles.get(id);
      if (known?.key === key) return;
      let cls = "home-tile";
      if (wide[i]) cls += " wide";
      if (tileAlert(id, task, info?.data)) cls += " alert";
      if (badge) cls += " badged";
      // A click in a tile is what a click on its pill was: it takes the focus and clears the badge.
      const tileEl = h("div", { class: cls, onclick: () => State.setFocus(id) },
        renderIntegrationCard(task, hooksFor(id), { tile: true }));
      if (badge) tileEl.append(pillBadgeEl(badge));
      if (known) known.el.replaceWith(tileEl);
      tiles.set(id, { el: tileEl, key });
    });
    // The order or the set changed (a tile came or went): lay them out again. Otherwise each tile
    // was replaced in place, so a focused row elsewhere keeps the focus.
    const want = ids.map((id) => tiles.get(id)!.el);
    const have = [...tilesEl.children];
    if (want.length !== have.length || want.some((x, i) => x !== have[i])) tilesEl.replaceChildren(...want);
  }

  return {
    el,
    sync() {
      renderTiles();
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
        timer = window.setInterval(() => {
          // The provider balances again once they are 5 min old (Rust's gates decide; no key, no call).
          void Bridge.balancesRefresh();
          render();
        }, REFRESH_MS);
      } else if (!on && timer != null) {
        window.clearInterval(timer);
        timer = null;
      }
      // Another page, or the island hiding: the popover goes, pinned or not.
      if (!on) hidePopover();
    },
  };
}
