// The island's sound names. Kept apart from the engine (sound.ts) so that the Settings page and
// scripts/checks/sounds.mjs can read the list without the Web Audio code or the Tauri bridge.
// Every name has a WAV in windows/sounds, rendered by scripts/gen-sounds.mjs.

export const SOUND_NAMES = [
  "peek", "open", "close", "hover", "blip", "slap", "annoyed", "dizzy", "greet",
  "work", "finish", "error", "approval", "question", "approve", "gulp", "tick",
  "send", "love", "pop", "proud", "wink", "yawn", "attach", "think", "search",
  "rate", "sleep",
] as const;

export type SoundName = (typeof SOUND_NAMES)[number];
