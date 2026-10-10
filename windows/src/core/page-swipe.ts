// The header pages by a horizontal swipe (ideas §40): two fingers left or right on a touchpad, or
// the mouse wheel tilted, over the open island. WebView2 hands both in as `wheel` events: a
// touchpad as a stream of small deltaX every ~8 ms, followed after the fingers lift by 3–4 s of
// momentum (decaying deltas, the last second ±1 px, never a 200 ms silence); a tilted wheel as
// discrete clicks of ~100 px. Windows gives a wheel event no phase, so where a stroke starts is
// read from the speed alone: a stroke is the deltas rising well above the lowest speed since the
// previous one (momentum only ever slows down), or turning the other way. Pure: no DOM, no
// timers; island/swipe-slide.ts draws what it decides.

/** The gesture's numbers, in CSS pixels and milliseconds. */
export const SWIPE = {
  /** No event for this long: the gesture is over (only after the momentum has run out). */
  QUIET_MS: 200,
  /** Movement summed before a gesture's axis is chosen. */
  LOCK_PX: 8,
  /** Horizontal only when |x| is at least this many times |y|: a scroll that drifts sideways stays a scroll. */
  DOMINANCE: 2,
  /** The speed is smoothed over about this long (exponential). */
  TAU_MS: 40,
  /** The shortest gap a speed is measured over (two events closer than this came together). */
  FRAME_MS: 8,
  /** px/ms. A stroke at least this fast is a flick; a new stroke inside a gesture is at least this fast. */
  VMIN: 1.5,
  /** A new stroke: the speed rose this many times above the lowest one since the previous stroke started. */
  RISE: 2.5,
  /**
   * Two strokes start at least this far apart: the jitter of one stroke never starts another. Wheel
   * clicks too: a held tilt repeats its clicks faster than this, and pages once per REFRACT_MS.
   */
  REFRACT_MS: 300,
  /** A flick switches once its own movement reaches this far, in its direction. */
  SWITCH_PX: 90,
  /** A slower stroke switches when it ends past this fraction of the page's width; short of it, back. */
  RELEASE: 0.35,
  /**
   * px/ms. A flick has ended once its speed falls under this, or under END_RATIO of its peak; a
   * slower stroke, once it goes back at least this fast.
   */
  END_V: 0.08,
  END_RATIO: 0.2,
  /** Past the page's width the page still moves, slower and slower, at most this much further. */
  OVERSHOOT_PX: 40,
  /** The page's width when the owner has not set one. */
  WIDTH_PX: 640,
  /** A wheel click: one step this large after a gap at least this long (a touchpad sends every ~8 ms). */
  TICK_PX: 48,
  TICK_GAP_MS: 30,
  /** deltaMode 1 (lines) and 2 (pages), turned into pixels. */
  LINE_PX: 40,
  PAGE_PX: 400,
  /** The slide to the next page, or back: its length clamped to these. */
  SLIDE_MIN_MS: 150,
  SLIDE_MAX_MS: 350,
  /** px/ms. A slide never starts slower than this, so a short way back is a short slide. */
  SLIDE_FLOOR_V: 2,
  /** A slide's length is about this many times the time its starting speed takes over the distance. */
  SLIDE_SLOPE: 3,
} as const;

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
  /** Taken by the swipe, nothing moves: the momentum of a stroke already decided. */
  | { kind: "hold" }
  /** The page follows the stroke: `offset` > 0 toward the next page (deltaX > 0, as →). */
  | { kind: "drag"; offset: number }
  /** Switch one page in `delta`'s direction, the slide starting from `offset` at `speed` px/ms. */
  | { kind: "commit"; delta: 1 | -1; offset: number; speed: number }
  /** The stroke ended short of a switch: the page slides back from `offset` at `speed` px/ms. */
  | { kind: "cancel"; offset: number; speed: number };

type Phase = "idle" | "undecided" | "vertical" | "drag" | "held";

function px(d: number, mode: number): number {
  return mode === 1 ? d * SWIPE.LINE_PX : mode === 2 ? d * SWIPE.PAGE_PX : d;
}

/** `x` up to ±`limit` as is; past it, resisted, never more than OVERSHOOT_PX further. */
export function rubberBand(x: number, limit: number): number {
  const a = Math.abs(x);
  if (a <= limit) return x;
  const over = a - limit;
  const r = SWIPE.OVERSHOOT_PX;
  return Math.sign(x) * (limit + (r * over) / (over + r));
}

/** A slide's length (ms) and CSS easing. */
export interface Slide {
  ms: number;
  ease: string;
}

/**
 * The slide over the `distance` px left when the stroke let go at `speed` px/ms: an ease-out that
 * starts at the stroke's own speed, so the page goes on from the finger without a jump, and as
 * long as that takes at most (clamped to SLIDE_MIN_MS…SLIDE_MAX_MS).
 */
export function slide(distance: number, speed: number): Slide {
  const d = Math.max(1, Math.abs(distance));
  const v = Math.max(speed, SWIPE.SLIDE_FLOOR_V);
  const ms = Math.round(Math.min(SWIPE.SLIDE_MAX_MS, Math.max(SWIPE.SLIDE_MIN_MS, (SWIPE.SLIDE_SLOPE * d) / v)));
  // The curve's starting slope is its speed relative to the average one (d / ms): x1 = 0.3, y1 = 0.3 × slope.
  const slope = Math.min(1 / 0.3, Math.max(1, (v * ms) / d));
  const y1 = Math.round(0.3 * slope * 1000) / 1000;
  return { ms, ease: `cubic-bezier(0.3, ${y1}, 0.3, 1)` };
}

/**
 * One gesture at a time, from its first event to QUIET_MS without one; inside it, strokes. The
 * axis is chosen once per gesture. A stroke moves the page and switches at most one page: a flick
 * as soon as it has gone SWITCH_PX, a slower stroke when it ends (past RELEASE of the width, or
 * back). After that, every event is held until the next stroke starts: momentum never switches.
 */
export class PageSwipe {
  /** The page's width (px): the owner sets it when a gesture starts. */
  width: number = SWIPE.WIDTH_PX;

  private phase: Phase = "idle";
  private last = 0;
  /** Smoothed speed (px/ms), signed: > 0 as deltaX > 0. */
  private v = 0;
  private vy = 0;
  /** The current stroke: when it started, its direction, its movement and its peak speed. */
  private onset = 0;
  private dir = 0;
  private x = 0;
  private peak = 0;
  private fast = false;
  /** The lowest |v| since the current stroke started. */
  private low = 0;
  /** Before the axis is chosen: the movement so far. */
  private absX = 0;
  private absY = 0;

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
    this.v = 0;
    this.vy = 0;
    this.dir = 0;
    this.x = 0;
    this.peak = 0;
    this.fast = false;
    this.low = 0;
    this.absX = 0;
    this.absY = 0;
  }

  /**
   * No event since QUIET_MS before `now`: the gesture ends. A stroke still moving the page is
   * decided as it stands (switch past RELEASE, else back); null when nothing was moving.
   */
  quiet(now: number): SwipeStep | null {
    if (!this.active || now - this.last < SWIPE.QUIET_MS) return null;
    const step = this.phase === "drag" ? this.decide(0) : null;
    this.reset();
    return step;
  }

  /**
   * One wheel event → what it does. The owner calls `quiet(e.timeStamp)` first, so a gesture that
   * ended without its timer having run is settled before this one starts.
   */
  wheel(e: WheelSample): SwipeStep {
    const dx = px(e.deltaX, e.deltaMode);
    const dy = px(e.deltaY, e.deltaMode);
    const t = e.timeStamp;
    if (this.phase !== "idle" && t - this.last >= SWIPE.QUIET_MS) this.reset();
    const gap = this.phase === "idle" ? Infinity : t - this.last;
    const dt = this.phase === "idle" ? 2 * SWIPE.FRAME_MS : Math.max(gap, 0);
    const k = 1 - Math.exp(-dt / SWIPE.TAU_MS);
    this.v += (dx / Math.max(dt, SWIPE.FRAME_MS) - this.v) * k;
    this.vy += (dy / Math.max(dt, SWIPE.FRAME_MS) - this.vy) * k;
    this.last = t;
    const sp = Math.abs(this.v);

    switch (this.phase) {
      case "idle":
        if (this.isClick(dx, dy, e.deltaMode, gap)) return this.click(dx, t);
        this.phase = "undecided";
        this.onset = t;
        this.low = sp;
        return this.undecided(dx, dy, t);
      case "undecided":
        return this.undecided(dx, dy, t);
      case "drag":
        this.x += dx;
        this.low = Math.min(this.low, sp);
        this.peak = Math.max(this.peak, this.v * this.dir);
        this.becameFast(sp, t);
        return this.dragStep();
      case "vertical":
      case "held": {
        if (this.isClick(dx, dy, e.deltaMode, gap)) {
          return t - this.onset >= SWIPE.REFRACT_MS ? this.click(dx, t) : this.ignored();
        }
        if (this.startsStroke(sp, t)) {
          this.stroke(t, dx, sp);
          return this.dragStep();
        }
        this.low = Math.min(this.low, sp);
        // A scroll during the momentum goes on to the page; the momentum itself is held.
        if (this.phase === "vertical" || Math.abs(dy) > SWIPE.DOMINANCE * Math.abs(dx)) return { kind: "pass" };
        return { kind: "hold" };
      }
    }
  }

  /** An event the swipe does not use: a scroll goes on to the page, the swipe's own tail is held. */
  private ignored(): SwipeStep {
    return this.phase === "vertical" ? { kind: "pass" } : { kind: "hold" };
  }

  /** A wheel click, sideways: lines or pages, or one large step after a gap no touchpad leaves. */
  private isClick(dx: number, dy: number, mode: number, gap: number): boolean {
    if (dx === 0 || Math.abs(dx) < SWIPE.DOMINANCE * Math.abs(dy)) return false;
    return mode !== 0 || (Math.abs(dx) >= SWIPE.TICK_PX && gap >= SWIPE.TICK_GAP_MS);
  }

  /** A click switches at once, from where it put the page; it counts as a stroke of its own. */
  private click(dx: number, t: number): SwipeStep {
    const delta = dx > 0 ? 1 : -1;
    this.onset = t;
    this.low = Math.abs(this.v);
    this.dir = delta;
    this.phase = "held";
    return { kind: "commit", delta, offset: rubberBand(dx, this.width), speed: 0 };
  }

  /**
   * A new stroke inside a gesture (its momentum, or a scroll): fast, clearly sideways, REFRACT_MS
   * after the previous one, and either rising RISE times above the lowest speed since then or
   * going the other way.
   */
  private startsStroke(sp: number, t: number): boolean {
    if (sp <= SWIPE.VMIN || t - this.onset < SWIPE.REFRACT_MS) return false;
    if (sp < SWIPE.DOMINANCE * Math.abs(this.vy)) return false;
    const reversed = this.dir !== 0 && Math.sign(this.v) !== this.dir;
    return sp > SWIPE.RISE * this.low || reversed;
  }

  private stroke(t: number, dx: number, sp: number) {
    this.phase = "drag";
    this.onset = t;
    this.low = sp;
    this.dir = Math.sign(this.v);
    this.x = dx;
    this.peak = sp;
    this.fast = sp > SWIPE.VMIN;
  }

  private undecided(dx: number, dy: number, t: number): SwipeStep {
    this.x += dx;
    this.absX += Math.abs(dx);
    this.absY += Math.abs(dy);
    this.low = Math.min(this.low, Math.abs(this.v));
    if (this.absX + this.absY < SWIPE.LOCK_PX) return { kind: "pass" };
    if (this.x === 0 || Math.abs(this.x) < SWIPE.DOMINANCE * this.absY) {
      this.phase = "vertical";
      return { kind: "pass" };
    }
    this.phase = "drag";
    this.dir = Math.sign(this.x);
    this.peak = Math.max(0, this.v * this.dir);
    this.becameFast(Math.abs(this.v), t);
    return this.dragStep();
  }

  /**
   * A stroke that began slowly turns into a flick: it is a stroke from here on, as one that starts
   * fast inside a gesture, so the next one is told apart from where this one became fast.
   */
  private becameFast(sp: number, t: number) {
    if (this.fast || sp <= SWIPE.VMIN) return;
    this.fast = true;
    this.onset = t;
    this.low = sp;
  }

  /**
   * The stroke under way: a flick switches, a flick that slowed down is decided (what follows is
   * its momentum), else the page follows. A stroke that never became fast leaves no momentum: it
   * follows the fingers however slowly or unevenly they move, pauses included, and is decided only
   * when the deltas stop (`quiet`) or the fingers turn back.
   */
  private dragStep(): SwipeStep {
    if (this.fast && this.x * this.dir >= SWIPE.SWITCH_PX) {
      return this.commit(this.dir as 1 | -1, Math.max(0, this.v * this.dir));
    }
    const ahead = this.v * this.dir;
    const ended = this.fast
      ? this.peak >= SWIPE.END_V && ahead < Math.max(SWIPE.END_V, SWIPE.END_RATIO * this.peak)
      : ahead < -SWIPE.END_V;
    if (ended) return this.decide(this.v);
    return { kind: "drag", offset: rubberBand(this.x, this.width) };
  }

  /** The stroke ended at speed `v`: past RELEASE, and not on its way back, it switches; else back. */
  private decide(v: number): SwipeStep {
    const sign = Math.sign(this.x);
    if (sign !== 0 && Math.abs(this.x) >= SWIPE.RELEASE * this.width && v * sign >= 0) {
      return this.commit(sign as 1 | -1, v * sign);
    }
    const offset = rubberBand(this.x, this.width);
    this.phase = "held";
    this.x = 0;
    return { kind: "cancel", offset, speed: Math.max(0, -v * sign) };
  }

  private commit(delta: 1 | -1, speed: number): SwipeStep {
    const offset = rubberBand(this.x, this.width);
    this.phase = "held";
    this.x = 0;
    return { kind: "commit", delta, offset, speed };
  }
}
