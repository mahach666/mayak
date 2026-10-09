// Frame pacing of the island loop (spec §2). A frame costs about the same few milliseconds of canvas,
// commit, compositing and present whatever it draws (measured: about half a percent of one core for
// each frame per second), so these rates are what the open island costs.

/**
 * The fire alone moving: 20 fps on the open island, 15 in the compact bar (spec §2 caps it at 30).
 * The field is soft and slow, so 20 fps still reads as a living flame at full size; in the bar Луч is
 * about 30 px tall, where 15 fps reads the same.
 */
export function fireFps(compact: boolean): number {
  return compact ? 15 : 20;
}

/**
 * The character alone moving (blinks, eyes, emotes, particles): about 60 fps. A 120 or 144 Hz panel
 * would otherwise draw every vsync for a blink nobody sees smoother than at 60.
 */
export const CHARACTER_FPS = 60;

/**
 * True when a vsync `sinceLastMs` after the last frame is too early for `fps`: the loop waits for the
 * next one. At 70 % of the period, for 60 fps: a 60 Hz panel draws every vsync, a 120 Hz one every
 * other (60 fps), a 144 Hz one every other (72 fps).
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
