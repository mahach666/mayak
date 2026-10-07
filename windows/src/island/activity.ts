// What Mochi is doing besides its state (spec §A): no DOM, no Tauri, no State, so it can be
// checked on its own.

import type { Phase } from "./pomodoro";

export type Prop = "laptop" | "coffee" | null;

export interface ActivityLook {
  headphones: boolean;
  notes: boolean;
  prop: Prop;
  sleep: boolean;
}

/** Spec §A: music → headphones and notes; pomodoro work → laptop, short → coffee, long → sleep. */
export function activityFor(phase: Phase | null, playing: boolean): ActivityLook {
  return {
    headphones: playing,
    notes: playing,
    prop: phase === "work" ? "laptop" : phase === "short" ? "coffee" : null,
    sleep: phase === "long",
  };
}
