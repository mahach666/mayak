// What Luch is doing besides its state (spec §A): no DOM, no Tauri, no State, so it can be
// checked on its own.

import type { Phase } from "./pomodoro";

export type Prop = "laptop" | "coffee" | null;

export interface ActivityLook {
  /** Music: Луч dances (drawn from the clock, luch/engine.ts). */
  dance: boolean;
  notes: boolean;
  prop: Prop;
  sleep: boolean;
}

/** Spec §A: music → a dance and notes; pomodoro work → laptop, short → coffee, long → sleep. */
export function activityFor(phase: Phase | null, playing: boolean): ActivityLook {
  return {
    dance: playing,
    notes: playing,
    prop: phase === "work" ? "laptop" : phase === "short" ? "coffee" : null,
    sleep: phase === "long",
  };
}
