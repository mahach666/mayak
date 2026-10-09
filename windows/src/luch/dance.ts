// Луч's dance to music (spec §A): the fire itself moves, not the box. The shader (fire.frag.glsl, the
// `if (uDance > 0.0)` block) deforms the field with these same numbers; this module gives the engine
// the beat, the amount's ease in and out, and where the face goes, so the eyes can follow it.
// No DOM and no imports: the Node checks load it as is.

/** How long the dance takes to come in or go (seconds): no pop when music starts or stops. */
export const DANCE_RAMP_S = 0.6;
/**
 * The dance clock (uDanceT) wraps after this many seconds. Every component makes whole cycles in it
 * (2 Hz, 1 Hz, 0.37 Hz), so the wrap shows no jump, and the float stays precise in the shader.
 */
export const DANCE_PERIOD_S = 100;
/** The beat (Hz): a stretch up and settle; the main sway takes one side per beat (half this rate). */
export const DANCE_BEAT_HZ = 2;
/** A slower sway under the main one, so the dance is not a metronome. */
export const DANCE_DRIFT_HZ = 0.37;

const TAU = 2 * Math.PI;
const BASE_Y = -0.44;

/** The dance clock for a time in seconds. */
export function danceClock(seconds: number): number {
  return ((seconds % DANCE_PERIOD_S) + DANCE_PERIOD_S) % DANCE_PERIOD_S;
}

/** The beat's envelope at dance time `T`: a quick rise after each beat, then a slow settle (peak about 0.6). */
export function danceEnvelope(T: number): number {
  const ph = T * DANCE_BEAT_HZ - Math.floor(T * DANCE_BEAT_HZ);
  return (1 - Math.exp(-ph * 22)) * Math.exp(-ph * 4);
}

/** The field's scale about the flame's base on the beat: a little taller, a touch narrower. */
export function danceStretch(T: number, amount: number): { sx: number; sy: number } {
  const env = danceEnvelope(T);
  return { sx: 1 - 0.03 * amount * env, sy: 1 + 0.08 * amount * env };
}

/**
 * The sideways sway at height `hd` (0 at the base, 1 at the crown), box units: a wave that travels up
 * (the base leads, the tip follows later and swings furthest).
 */
export function danceSway(T: number, hd: number, amount: number): number {
  const k = Math.min(1, Math.max(0, hd));
  return amount * Math.pow(k, 1.5) * (
    0.06 * Math.sin(TAU * (DANCE_BEAT_HZ / 2) * T - 1.8 * k) +
    0.025 * Math.sin(TAU * DANCE_DRIFT_HZ * T - 1.2 * k + 1.3)
  );
}

/** The brightness and glow pulse on the beat. */
export function dancePulse(T: number, amount: number): number {
  return 1 + 0.08 * amount * danceEnvelope(T);
}

/**
 * Where a point of the flame at rest (box units, y up) is drawn while dancing: the inverse of the
 * shader's warp, which samples the field at s = base + (p - base) / stretch, then s.x -= sway(s.y).
 */
export function danceMove(T: number, amount: number, x: number, y: number): { x: number; y: number } {
  if (amount <= 0) return { x, y };
  const { sx, sy } = danceStretch(T, amount);
  const hd = (y - BASE_Y) / 1.1;
  return { x: (x + danceSway(T, hd, amount)) * sx, y: BASE_Y + (y - BASE_Y) * sy };
}

/** The dance amount one step on: towards 1 while `on`, towards 0 otherwise, over DANCE_RAMP_S, smooth. */
export function stepDance(level: number, on: boolean, dt: number): number {
  const next = level + (on ? 1 : -1) * Math.max(0, dt) / DANCE_RAMP_S;
  return Math.min(1, Math.max(0, next));
}

/** The ramp's shape: a linear level eased in and out, so the dance never starts or stops with a jolt. */
export function danceAmount(level: number): number {
  const k = Math.min(1, Math.max(0, level));
  return k * k * (3 - 2 * k);
}
