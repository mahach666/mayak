// Frame pacing of the island loop (spec §2). A frame costs about the same few milliseconds of canvas,
// commit, compositing and present whatever it draws (measured: about half a percent of one core for
// each frame per second), so these rates are what the open island costs.

import { LOOK_SETTLED } from "../luch/eyes.ts";

/**
 * The fire alone moving: 20 fps on the open island, 15 in the compact bar (spec §2 caps it at 30).
 * The field is soft and slow, so 20 fps still reads as a living flame at full size; in the bar Луч is
 * about 30 px tall, where 15 fps reads the same. While Луч dances to music (`dancing`: the main Луч
 * shown, music playing, a calm state, luch/dance.ts) the fire moves faster: 30 fps open, 20 compact.
 */
export function fireFps(compact: boolean, dancing = false): number {
  if (dancing) return compact ? 20 : 30;
  return compact ? 15 : 20;
}

/**
 * The character alone moving (blinks, eyes, emotes, particles): about 60 fps (tooSoon gives the rate
 * per panel). A 120 or 144 Hz panel would otherwise draw every vsync for a blink nobody sees smoother
 * than at 60.
 */
export const CHARACTER_FPS = 60;

/**
 * True when a vsync `sinceLastMs` after the last frame is too early for `fps`: the loop waits for the
 * next one. At 70 % of the period, for 60 fps: a 60 Hz panel draws every vsync, a 120 Hz one every
 * other (60 fps), a 144 Hz one every other (72 fps); a 90 or 100 Hz one also every other, so 45 or
 * 50 fps there.
 */
export function tooSoon(sinceLastMs: number, fps: number): boolean {
  return sinceLastMs < (0.7 * 1000) / fps;
}

/**
 * The timer before the next fire-only frame, `spentMs` into this one: aimed 4 ms before the frame is
 * due, so that the vsync right after it draws the frame and the rate lands on `fps`, not below it.
 */
export function fireDelay(fps: number, spentMs: number): number {
  return Math.max(0, 1000 / fps - spentMs - 4);
}

/** What a cursor move changed (island.ts onCursor). */
export interface CursorMove {
  /** It entered or left the island (the FSM, the collapse countdown). */
  crossed: boolean;
  /** It came onto or off Луч (the hover blink, love). */
  hover: boolean;
  /** The drop sequence follows it. */
  dragging: boolean;
  /** How far (rad) Луч's eyes would turn to follow it; 0 when he is not shown or does not follow. */
  eyeLag: number;
}

/**
 * Whether a cursor move needs a frame now. The cursor poll sends a move up to 60 times a second while
 * the island shows: drawing each at once kept a fire-only island at the panel's rate for a mouse moving
 * anywhere on screen. A move nothing on screen answers yet waits: the next fire frame reads it.
 */
export function cursorNeedsFrame(m: CursorMove): boolean {
  return m.crossed || m.hover || m.dragging || m.eyeLag > LOOK_SETTLED;
}

/**
 * The mini Лучи (session pills, compact grid) draw no more often than the fire's paced rate (fireFps):
 * a fire-only frame draws them, as it always has, but while the loop runs at the panel's rate for the
 * big Луч (a blink, an emote, the island resizing) they keep to the fire's rate instead of drawing
 * every vsync. The remainder past a period carries over, so a vsync grid that does not divide the
 * period (144 Hz) still averages out at `fps`; after a stall at most one period carries.
 */
export class MiniPace {
  private owed = 0;

  /**
   * `dt` (s) since the last frame, the fire's rate now: whether this frame draws the minis. `now`: a
   * frame that draws them whatever the time owed, and starts the next period from it (the fire-only
   * loop's own paced frame; a state change, so the minis show it at once as before).
   */
  due(dt: number, fps: number, now = false): boolean {
    if (now) {
      this.owed = 0;
      return true;
    }
    this.owed += dt;
    const period = 1 / fps;
    if (this.owed < period) return false;
    this.owed = Math.min(period, this.owed - period);
    return true;
  }
}
