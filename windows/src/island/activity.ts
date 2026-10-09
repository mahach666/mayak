// What Luch is doing besides its state (spec §A): no DOM, no Tauri, no State, so it can be
// checked on its own.

import type { FocusLook } from "../luch/focus";
import type { Phase } from "./pomodoro";

export interface ActivityLook {
  /** Music: Луч dances (drawn from the clock, luch/engine.ts). */
  dance: boolean;
  notes: boolean;
  /** The pomodoro phase's look of the fire itself (luch/focus.ts), null outside a pomodoro. */
  focus: FocusLook | null;
}

/**
 * Spec §A: music → a dance and notes; pomodoro work → a gathered flame, short break → a low breathing
 * flame, long break → embers. How the dance and the phases combine is the engine's (luch/focus.ts).
 */
export function activityFor(phase: Phase | null, playing: boolean): ActivityLook {
  return {
    dance: playing,
    notes: playing,
    focus: phase === "work" ? "focus" : phase === "short" ? "rest" : phase === "long" ? "embers" : null,
  };
}
