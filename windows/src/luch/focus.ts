// Луч in the pomodoro (spec §A): each phase changes the fire itself, like the dance to music, never a
// prop drawn over it. Work gathers the flame (taller, narrower, upright, a hotter heart), the short
// break lowers it into a slow visible breath, the long break burns it down to smouldering embers.
// The shader (fire.frag.glsl, the `if (uWork + uRest + uEmber > 0.0)` block) shapes the field with
// these same numbers; this module gives the engine the eased amounts, the clock and where the face
// goes, so the eyes follow it. No DOM and no imports: the Node checks load it as is.
//
// Music and the phases: the dance wins in work and the short break (their looks ease out while Луч
// dances and back in when the music stops); the long break wins over the dance (no dance in embers).

/** The fire's look for a pomodoro phase: work, short break, long break. */
export type FocusLook = "focus" | "rest" | "embers";

/** How long a phase look takes to come in or go (seconds): slower than the dance, the phases are calm. */
export const FOCUS_RAMP_S = 1.5;
/**
 * The phase clock (uFocusT) wraps after this many seconds. Every component makes whole cycles in it
 * (0.22 Hz, 0.18 Hz, 0.05 Hz), so the wrap shows no jump, and the float stays precise in the shader.
 */
export const FOCUS_PERIOD_S = 100;
/** The short break's breath (Hz): the flame rises and settles about every 4.5 s. */
export const REST_BREATH_HZ = 0.22;
/** The embers' slow glow (Hz). */
export const EMBER_GLOW_HZ = 0.18;

const TAU = 2 * Math.PI;
const BASE_Y = -0.44;

/** The three looks' amounts (each 0…1); at most one is wanted at a time, two meet while one hands over. */
export interface FocusLevels { work: number; rest: number; ember: number }

export const NO_FOCUS: FocusLevels = Object.freeze({ work: 0, rest: 0, ember: 0 });

/** The phase clock for a time in seconds. */
export function focusClock(seconds: number): number {
  return ((seconds % FOCUS_PERIOD_S) + FOCUS_PERIOD_S) % FOCUS_PERIOD_S;
}

/** The short break's breath at phase time `T`, -1…1 (+1 = risen). */
export function restBreath(T: number): number {
  return Math.sin(TAU * REST_BREATH_HZ * T);
}

/** The embers' glow at phase time `T`, 0…1. */
export function emberGlow(T: number): number {
  return 0.5 + 0.5 * Math.sin(TAU * EMBER_GLOW_HZ * T);
}

/**
 * The field's scale about the flame's base: work a little taller and narrower, the short break lower
 * and breathing, the embers a low wide mound. 1, 1 when every amount is 0.
 */
export function focusStretch(T: number, a: FocusLevels): { sx: number; sy: number } {
  const b = restBreath(T);
  return {
    sx: (1 - 0.06 * a.work) * (1 + 0.03 * a.rest) * (1 + 0.05 * a.ember),
    sy: (1 + 0.08 * a.work) * (1 - 0.08 * a.rest + 0.03 * a.rest * b) * (1 - 0.12 * a.ember),
  };
}

/**
 * Where a point of the flame at rest (box units, y up) is drawn: the inverse of the shader's warp,
 * which samples the field at s = base + (p - base) / stretch. Apply before dance.ts danceMove (the
 * shader moves the dance first, then the phase).
 */
export function focusMove(T: number, a: FocusLevels, x: number, y: number): { x: number; y: number } {
  if (a.work <= 0 && a.rest <= 0 && a.ember <= 0) return { x, y };
  const { sx, sy } = focusStretch(T, a);
  return { x: x * sx, y: BASE_Y + (y - BASE_Y) * sy };
}

/** The levels one step on: the wanted look towards 1, the others towards 0, over FOCUS_RAMP_S. */
export function stepFocus(levels: FocusLevels, wanted: FocusLook | null, dt: number): FocusLevels {
  const d = Math.max(0, dt) / FOCUS_RAMP_S;
  const to = (v: number, on: boolean) => Math.min(1, Math.max(0, v + (on ? d : -d)));
  return { work: to(levels.work, wanted === "focus"), rest: to(levels.rest, wanted === "rest"), ember: to(levels.ember, wanted === "embers") };
}

/** The ramp's shape: each linear level eased in and out, so a phase never starts or ends with a jolt. */
export function focusAmounts(levels: FocusLevels): FocusLevels {
  const e = (v: number) => { const k = Math.min(1, Math.max(0, v)); return k * k * (3 - 2 * k); };
  return { work: e(levels.work), rest: e(levels.rest), ember: e(levels.ember) };
}

/** Any phase look shown. */
export function focusActive(a: FocusLevels): boolean {
  return a.work > 0 || a.rest > 0 || a.ember > 0;
}
