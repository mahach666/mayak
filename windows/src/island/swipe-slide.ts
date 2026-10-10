// The header pages by a horizontal swipe over the open island (ideas §40): core/page-swipe.ts
// decides, this draws. While a gesture runs, the page on screen follows it and the next page in
// its direction comes in beside it; a switch slides both the rest of the way, a gesture that ends
// short slides them back. Only inline styles on the two views, written from the wheel events
// themselves: no frame loop, and the one quiet timer and the one slide timer end with the gesture.

import { PageSwipe, type SwipeStep } from "../core/page-swipe";
import { cyclePage } from "../core/island-keys";
import type { OpenPage } from "../core/open-view";

/** How long the slide to the next page, or back, takes. */
export const SLIDE_MS = 260;
/** The space between the two pages while they slide side by side. */
const GAP_PX = 14;
const EASE = "cubic-bezier(0.22, 0.8, 0.3, 1)";

export interface SwipeHost {
  /** The header page on screen, or null when the island shows none (folded, a card, Settings…). */
  page(): OpenPage | null;
  /** A page's view element. */
  viewEl(page: OpenPage): HTMLElement | null;
  /** A page about to come in beside the one on screen: brought up to date first. */
  prepare(page: OpenPage): void;
  /** The switch itself, as a header tab click (sound, "last opened", the view). */
  switchTo(page: OpenPage): void;
}

export class SwipeSlide {
  private readonly swipe = new PageSwipe();
  /** The page the gesture started on, and the one shown beside it. */
  private base: OpenPage | null = null;
  private peer: OpenPage | null = null;
  /** The elements carrying inline styles, cleared when the slide ends. */
  private styled = new Set<HTMLElement>();
  private quietTimer: number | null = null;
  private slideTimer: number | null = null;
  private reduced = false;

  constructor(
    /** The views' container: its width is the slide's length. */
    private readonly area: HTMLElement,
    private readonly host: SwipeHost,
  ) {}

  /** The island's wheel listener (registered with passive: false). */
  onWheel(e: WheelEvent) {
    const page = this.host.page();
    // No page on screen, or another one than the gesture's (a card came up): the gesture goes.
    if (page == null || (this.base != null && page !== this.base)) {
      if (this.swipe.active) this.abandon();
      return;
    }
    // Ctrl: a pinch or a zoom. Shift: a wheel turned sideways by the user, meant as a scroll.
    // A held button: a tile being dragged, the player's seek bar, a text selection.
    if (e.ctrlKey || e.shiftKey || e.buttons !== 0) {
      if (this.swipe.active) this.abandon();
      return;
    }
    if (!this.swipe.active && scrollsSideways(e.target, this.area, e.deltaX)) return;

    const ended = this.swipe.quiet(e.timeStamp);
    if (ended) this.apply(ended);
    if (!this.swipe.active) {
      this.base = page;
      this.reduced = prefersReducedMotion();
    }
    const step = this.swipe.wheel(e);
    if (step.kind !== "pass") e.preventDefault();
    this.apply(step);
    this.armQuiet();
  }

  private apply(step: SwipeStep) {
    switch (step.kind) {
      case "pass":
      case "hold":
        return;
      case "drag":
        if (!this.reduced) this.follow(step.offset);
        return;
      case "commit":
        this.commit(step.delta, step.offset);
        return;
      case "cancel":
        this.slideBack();
        return;
    }
  }

  /** One timer per gesture: it re-arms itself until the deltas have gone quiet. */
  private armQuiet() {
    if (this.quietTimer != null) return;
    const at = this.swipe.quietAt;
    if (at == null) {
      if (this.slideTimer == null) this.base = null;
      return;
    }
    this.quietTimer = window.setTimeout(() => {
      this.quietTimer = null;
      const ended = this.swipe.quiet(performance.now());
      if (ended) this.apply(ended);
      if (this.swipe.active) this.armQuiet();
      else if (this.slideTimer == null) this.base = null;
    }, Math.max(0, at - performance.now()) + 1);
  }

  private span(): number {
    return this.area.clientWidth + GAP_PX;
  }

  /** The page on screen at -offset, its neighbour in the gesture's direction beside it. */
  private follow(offset: number) {
    const base = this.base;
    if (base == null || offset === 0) return;
    this.finishSlide();
    const dir: 1 | -1 = offset > 0 ? 1 : -1;
    const peer = cyclePage(base, dir);
    if (peer !== this.peer) {
      if (this.peer != null) this.clear(this.host.viewEl(this.peer));
      this.peer = peer;
      this.host.prepare(peer);
    }
    const span = this.span();
    const o = Math.max(-span, Math.min(span, offset));
    this.place(this.host.viewEl(base), -o, "none");
    this.place(this.host.viewEl(peer), dir * span - o, "none");
  }

  private commit(delta: 1 | -1, offset: number) {
    const base = this.base;
    if (base == null) return;
    const to = cyclePage(base, delta);
    if (this.reduced) {
      // No slide, and no cross-fade either: the new page is simply there.
      this.clearAll();
      this.peer = null;
      this.switchTo(to);
      this.clear(this.host.viewEl(base));
      this.clear(this.host.viewEl(to));
      return;
    }
    // A wheel click comes with no drag before it: the pages start from where the click put them.
    this.follow(offset === 0 ? delta : offset);
    const from = this.host.viewEl(base);
    const into = this.host.viewEl(to);
    // The positions just written are where the transition starts from.
    void this.area.offsetWidth;
    const span = this.span();
    const ease = `transform ${SLIDE_MS}ms ${EASE}`;
    this.place(from, -delta * span, ease);
    this.place(into, 0, ease);
    this.switchTo(to);
    this.endSlideIn(SLIDE_MS);
  }

  /** The switch; the rest of the gesture (its momentum, held) now belongs to the new page. */
  private switchTo(page: OpenPage) {
    const from = this.base;
    this.host.switchTo(page);
    this.base = page;
    // The island moves the views' "on" class at its next frame; moved here now, so the slide (or
    // the instant switch) owns the change and the stylesheet's cross-fade never plays over it.
    if (from != null) this.host.viewEl(from)?.classList.remove("on");
    this.host.viewEl(page)?.classList.add("on");
  }

  private slideBack() {
    const base = this.base;
    const peer = this.peer;
    if (base == null || peer == null || this.styled.size === 0) {
      this.clearAll();
      return;
    }
    const dir = cyclePage(base, 1) === peer ? 1 : -1;
    const ease = `transform ${SLIDE_MS}ms ${EASE}`;
    this.place(this.host.viewEl(base), 0, ease);
    this.place(this.host.viewEl(peer), dir * this.span(), ease);
    this.endSlideIn(SLIDE_MS);
  }

  private endSlideIn(ms: number) {
    if (this.slideTimer != null) window.clearTimeout(this.slideTimer);
    this.slideTimer = window.setTimeout(() => {
      this.slideTimer = null;
      this.clearAll();
      this.peer = null;
      if (!this.swipe.active) this.base = null;
    }, ms + 20);
  }

  /** A slide still running when a new gesture moves the pages: it ends where it was going. */
  private finishSlide() {
    if (this.slideTimer == null) return;
    window.clearTimeout(this.slideTimer);
    this.slideTimer = null;
    this.clearAll();
    this.peer = null;
  }

  /** The page changed under the gesture, or a drag began: everything back to the stylesheet. */
  private abandon() {
    this.swipe.reset();
    if (this.quietTimer != null) window.clearTimeout(this.quietTimer);
    this.quietTimer = null;
    if (this.slideTimer != null) window.clearTimeout(this.slideTimer);
    this.slideTimer = null;
    this.clearAll();
    this.base = null;
    this.peer = null;
  }

  private place(el: HTMLElement | null, x: number, transition: string) {
    if (!el) return;
    this.styled.add(el);
    el.style.transition = transition;
    // Both stay opaque while they slide: the island's clip hides what leaves it.
    el.style.opacity = "1";
    el.style.transform = `translateX(${x}px)`;
  }

  /**
   * Back to the stylesheet at once: the view's own transition (the fade of `.view`) must not play
   * from where the slide left it, or the page that went would flash back in the middle.
   */
  private clear(el: HTMLElement | null) {
    if (!el) return;
    this.styled.delete(el);
    el.style.transition = "none";
    el.style.opacity = "";
    el.style.transform = "";
    void el.offsetWidth;
    el.style.transition = "";
  }

  private clearAll() {
    for (const el of [...this.styled]) this.clear(el);
  }
}

function prefersReducedMotion(): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** Something under the cursor, inside the views, that can still scroll sideways that way keeps the gesture. */
function scrollsSideways(target: EventTarget | null, area: HTMLElement, dx: number): boolean {
  if (dx === 0) return false;
  for (let el = target instanceof Element ? target : null; el && el !== area; el = el.parentElement) {
    if (el.scrollWidth <= el.clientWidth) continue;
    const overflow = getComputedStyle(el).overflowX;
    if (overflow !== "auto" && overflow !== "scroll") continue;
    if (dx > 0 ? el.scrollLeft + el.clientWidth < el.scrollWidth - 1 : el.scrollLeft > 0) return true;
  }
  return false;
}
