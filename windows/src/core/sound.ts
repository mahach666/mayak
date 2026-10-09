// SoundEngine — port of SoundEngine.swift.
// The 28 WAVs are Mayak's own, synthesised by scripts/sounds/sfx.mjs and rendered to
// windows/sounds by `npm run sounds` (see SOUNDS_DIR in vite.config.ts); they are served at
// /sounds/<name>.wav. Default volume 0.12, slider range 0–0.2, exactly like the Mac player,
// and several sounds may overlap.
// A file of the user's own in the sounds folder (src-tauri/src/sounds.rs) replaces a sound;
// one that can't be read or decoded falls back to the built-in one.

import { Bridge } from "./bridge";
import { SOUND_NAMES, type SoundName } from "./sound-names";

export { SOUND_NAMES, type SoundName };

/** Goes to mayak.log (and the console); callers name the sound only, never a path. */
function logSound(message: string) {
  console.warn(`[mayak] ${message}`);
  void Bridge.log(message);
}

class SoundEngine {
  enabled = true;
  volume = 0.12;

  private ctx: AudioContext | null = null;
  private master: GainNode | null = null;
  /** What plays, by name: the user's sound where there is one, else the built-in one. */
  private buffers = new Map<string, AudioBuffer>();
  /** The decoded built-in WAVs, kept so a reload only reads the user's files again. */
  private builtins = new Map<string, AudioBuffer>();
  private loading: Promise<void> | null = null;
  private idleTimer: number | null = null;

  /** Creates the context and decodes every sound. Safe to call more than once. */
  preload(): Promise<void> {
    if (this.loading) return this.loading;
    this.loading = (async () => {
      const Ctor = window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
      if (!Ctor) return;
      const ctx = new Ctor();
      this.ctx = ctx;
      const master = ctx.createGain();
      master.gain.value = this.volume;
      master.connect(ctx.destination);
      this.master = master;
      await this.loadAll(ctx);
    })();
    return this.loading;
  }

  /**
   * Reads the sounds folder again (Settings → Reload sounds). Before the first load it is that
   * load; otherwise it runs after any load still going, so two never race on the same sounds.
   */
  reload(): Promise<void> {
    if (!this.loading) return this.preload();
    this.loading = this.loading.then(() => (this.ctx ? this.loadAll(this.ctx) : undefined));
    return this.loading;
  }

  private async loadAll(ctx: AudioContext) {
    const own = new Set((await Bridge.customSounds(SOUND_NAMES)) ?? []);
    await Promise.all(
      SOUND_NAMES.map(async (name) => {
        const mine = own.has(name) ? await this.decodeCustom(ctx, name) : null;
        const buf = mine ?? (await this.builtin(ctx, name));
        // A missing sound must never break the island: it just stays silent.
        if (buf) this.buffers.set(name, buf);
        else this.buffers.delete(name);
      }),
    );
  }

  /** The user's file for `name`, decoded; null (logged) when it can't be read or decoded. */
  private async decodeCustom(ctx: AudioContext, name: string): Promise<AudioBuffer | null> {
    const bytes = await Bridge.customSound(name);
    if (!bytes || bytes.byteLength === 0) {
      logSound(`sounds: ${name}: own file could not be read, the built-in one plays`);
      return null;
    }
    try {
      return await ctx.decodeAudioData(bytes);
    } catch {
      logSound(`sounds: ${name}: own file could not be decoded, the built-in one plays`);
      return null;
    }
  }

  /** The built-in WAV for `name`, fetched and decoded once; null (logged) when it fails. */
  private async builtin(ctx: AudioContext, name: string): Promise<AudioBuffer | null> {
    const cached = this.builtins.get(name);
    if (cached) return cached;
    try {
      const res = await fetch(`/sounds/${name}.wav`);
      if (!res.ok) {
        logSound(`sounds: ${name}: built-in sound missing (HTTP ${res.status})`);
        return null;
      }
      const buf = await ctx.decodeAudioData(await res.arrayBuffer());
      this.builtins.set(name, buf);
      return buf;
    } catch (err) {
      logSound(`sounds: ${name}: built-in sound failed: ${err instanceof Error ? err.name : "error"}`);
      return null;
    }
  }

  /** WebView2 can hand us a suspended context; call after any user input. */
  resume() {
    if (this.idleTimer != null) {
      window.clearTimeout(this.idleTimer);
      this.idleTimer = null;
    }
    void this.ctx?.resume();
  }

  /**
   * Called when the island goes quiet. A running AudioContext keeps an audio
   * thread and its render quantum alive even with nothing playing, which shows
   * up as a steady trickle of CPU on a machine that is supposed to be idle.
   *
   * The delay covers the tail of whatever just played — suspending mid-sound
   * would clip it — and `play()` resumes the context on its own.
   */
  idle() {
    if (!this.ctx || this.ctx.state !== "running" || this.idleTimer != null) return;
    this.idleTimer = window.setTimeout(() => {
      this.idleTimer = null;
      void this.ctx?.suspend();
    }, 1500);
  }

  setVolume(v: number) {
    this.volume = Math.max(0, Math.min(0.2, v));
    if (this.master) this.master.gain.value = this.volume;
  }

  setEnabled(on: boolean) {
    this.enabled = on;
  }

  play(name: SoundName | string) {
    if (!this.enabled) return;
    const ctx = this.ctx;
    const master = this.master;
    const buf = this.buffers.get(name);
    if (!ctx || !master || !buf) return;
    if (this.idleTimer != null) {
      window.clearTimeout(this.idleTimer);
      this.idleTimer = null;
    }
    if (ctx.state === "suspended") void ctx.resume();
    const src = ctx.createBufferSource();
    src.buffer = buf;
    src.connect(master);
    src.start();
  }
}

export const Sound = new SoundEngine();
