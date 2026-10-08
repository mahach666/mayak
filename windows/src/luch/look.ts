// How the fire behaves per state, emote and activity (spec §3.1–3.3). Pure.

import type { BotEmoteName, BotStateName } from "../core/layout";
import type { Hex } from "./palette";

export interface FireLook {
  /** Speed of the fire's motion (1 = the prototype). */
  live: number;
  /** Height of the main tongue (1 = the prototype). */
  tall: number;
  /** Base brightness, 0…1. */
  bright: number;
  flicker: "none" | "flash" | "pulse" | "sputter";
  /** Smoke puffs above the flame (error, annoyed). */
  smoke: boolean;
  /** A colour that replaces the state colour while an emote lasts. */
  tint: Hex | null;
}

const base: FireLook = { live: 1, tall: 1, bright: 1, flicker: "none", smoke: false, tint: null };

const STATE: Record<BotStateName, Partial<FireLook>> = {
  idle: {},
  working: { live: 1.5 },
  thinking: { flicker: "pulse" },
  searching: { live: 1.2 },
  approval: { live: 1.3, tall: 1.15, flicker: "flash" },
  question: {},
  error: { live: 0.8, tall: 0.75, bright: 0.9, flicker: "sputter", smoke: true },
  finished: { live: 1.1, tall: 1.05 },
  ratelimit: { live: 0.6, tall: 0.6, bright: 0.75 },
  sleeping: { live: 0.35, tall: 0.5, bright: 0.55 },
  dizzy: { live: 1.4, tall: 0.9 },
};

const EMOTE: Record<BotEmoteName, Partial<FireLook>> = {
  love: { tint: "#ff6fa0" },
  surprised: { tall: 1.3 },
  proud: { tall: 1.25 },
  wink: {},
  yawn: { tall: 0.7, live: 0.6 },
  happy: {},
  annoyed: { tint: "#ff4a2a", smoke: true, live: 1.6 },
};

/** `sleepLook`: the long pomodoro break shows sleep whatever the idle state (spec §3.3). */
export function fireLook(state: BotStateName, emote: BotEmoteName | null, sleepLook: boolean): FireLook {
  const s = sleepLook ? STATE.sleeping : STATE[state];
  return { ...base, ...s, ...(emote ? EMOTE[emote] : {}) };
}

/** The look's brightness at time t (seconds), always in (0.3, 1]. */
export function brightAt(look: FireLook, t: number): number {
  let b = look.bright;
  if (look.flicker === "flash") b *= 0.8 + 0.2 * (0.5 + 0.5 * Math.sin(t * 10));
  if (look.flicker === "pulse") b *= 0.78 + 0.22 * Math.sin(t * 2.4);
  if (look.flicker === "sputter") b *= 0.86 + 0.14 * Math.sin(t * 17) * Math.sin(t * 5.3);
  return Math.min(1, Math.max(0.31, b));
}
