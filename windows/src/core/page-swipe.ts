// The header pages by a horizontal swipe (ideas §40): two fingers left or right on a touchpad, or
// the mouse wheel tilted, over the open island. WebView2 hands both in as `wheel` events: a
// touchpad as a stream of small deltaX with its momentum after the fingers lift, a tilted wheel
// as discrete clicks. Pure: no DOM, no timers; island/page-swipe.ts draws what it decides.

/** The gesture's numbers, in CSS pixels and milliseconds. */
export const SWIPE = {
  /** No event for this long: the gesture is over (fingers lifted and their momentum ran out). */
  QUIET_MS: 200,
  /** Movement summed before the axis is chosen. */
  LOCK_PX: 8,
  /** Horizontal only when |Σx| is at least this many times Σ|y|: a scroll that drifts sideways stays a scroll. */
  DOMINANCE: 2,
  /** The page follows the finger this far, then goes on to the next one by itself. */
  COMMIT_PX: 110,
  /** A gesture that ends past this switches; short of it the page slides back. */
  RELEASE_PX: 40,
  /** A gesture's first event this large is a wheel click, not a finger: it switches at once. */
  TICK_PX: 48,
  /** deltaMode 1 (lines) and 2 (pages), turned into pixels. */
  LINE_PX: 40,
  PAGE_PX: 400,
} as const;

/** The parts of a WheelEvent read here. */
export interface WheelSample {
  deltaX: number;
  deltaY: number;
  /** 0 pixels, 1 lines, 2 pages. */
  deltaMode: number;
  /** Event.timeStamp (ms, same clock as performance.now()). */
  timeStamp: number;
}

export type SwipeStep =
  /** Not a swipe (a scroll, or not decided yet): the event goes on to the page as usual. */
  | { kind: "pass" }
  /** Taken by the swipe, nothing moves: the tail of a gesture that already switched. */
  | { kind: "hold" }
  /** The page follows the gesture: `offset` > 0 toward the next page (deltaX > 0, as →). */
  | { kind: "drag"; offset: number }
  /** Switch one page in `delta`'s direction, the slide starting from `offset`. */
  | { kind: "commit"; delta: 1 | -1; offset: number }
  /** The gesture ended short of a switch: the page slides back from `offset`. */
  | { kind: "cancel"; offset: number };

type Phase = "idle" | "undecided" | "vertical" | "horizontal" | "done";

function px(d: number, mode: number): number {
  return mode === 1 ? d * SWIPE.LINE_PX : mode === 2 ? d * SWIPE.PAGE_PX : d;
}

/**
 * One gesture at a time: the axis is chosen once, a gesture switches at most one page, and every
 * event after the switch is held until the deltas go quiet for QUIET_MS.
 */
export class PageSwipe {
  private phase: Phase = "idle";
  private x = 0;
  private absY = 0;
  private absX = 0;
  private events = 0;
  private last = 0;

  /** A gesture is under way: the owner keeps one timer to call `quiet` at `quietAt`. */
  get active(): boolean {
    return this.phase !== "idle";
  }

  /** When the gesture under way ends if no event comes, or null when none is under way. */
  get quietAt(): number | null {
    return this.active ? this.last + SWIPE.QUIET_MS : null;
  }

  /** Forgets the gesture without a step (the page left, the island folded). */
  reset() {
    this.phase = "idle";
    this.x = 0;
    this.absX = 0;
    this.absY = 0;
    this.events = 0;
  }

  /**
   * No event since QUIET_MS before `now`: the gesture ends. A drag past RELEASE_PX switches, a
   * shorter one slides back; null when no gesture ended (none under way, or not quiet yet).
   */
  quiet(now: number): SwipeStep | null {
    if (!this.active || now - this.last < SWIPE.QUIET_MS) return null;
    const phase = this.phase;
    const x = this.x;
    this.reset();
    if (phase !== "horizontal") return null;
    if (Math.abs(x) >= SWIPE.RELEASE_PX) return { kind: "commit", delta: x > 0 ? 1 : -1, offset: x };
    return { kind: "cancel", offset: x };
  }

  /**
   * One wheel event → what it does. The owner calls `quiet(e.timeStamp)` first, so a gesture that
   * ended without its timer having run is settled before this one starts.
   */
  wheel(e: WheelSample): SwipeStep {
    const dx = px(e.deltaX, e.deltaMode);
    const dy = px(e.deltaY, e.deltaMode);
    if (this.phase !== "idle" && e.timeStamp - this.last >= SWIPE.QUIET_MS) this.reset();
    this.last = e.timeStamp;
    if (this.phase === "idle") this.phase = "undecided";
    this.events += 1;

    switch (this.phase) {
      case "vertical":
        return { kind: "pass" };
      case "done":
        return { kind: "hold" };
      case "undecided": {
        this.x += dx;
        this.absX += Math.abs(dx);
        this.absY += Math.abs(dy);
        if (this.absX + this.absY < SWIPE.LOCK_PX) return { kind: "pass" };
        if (Math.abs(this.x) < SWIPE.DOMINANCE * this.absY || this.x === 0) {
          this.phase = "vertical";
          return { kind: "pass" };
        }
        this.phase = "horizontal";
        // A wheel click (lines or pages, or one large first step): no finger to follow.
        const click = this.events === 1 && (e.deltaMode !== 0 || Math.abs(dx) >= SWIPE.TICK_PX);
        if (click) return this.commit();
        return this.follow();
      }
      case "horizontal":
        this.x += dx;
        return this.follow();
      default:
        return { kind: "pass" };
    }
  }

  private follow(): SwipeStep {
    if (Math.abs(this.x) >= SWIPE.COMMIT_PX) return this.commit();
    return { kind: "drag", offset: this.x };
  }

  private commit(): SwipeStep {
    this.phase = "done";
    return { kind: "commit", delta: this.x > 0 ? 1 : -1, offset: this.x };
  }
}
