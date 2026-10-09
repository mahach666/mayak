// Luch — the engine behind it is a direct port of NotchBuddy/Sources/App/BotEngine.swift (Mochi)
// to Canvas 2D, with the body drawn as fire.
// Same constants, same tweens, same easings, same particles. The only intentional
// difference is the `happy`/`wink` eye arc, which follows the prototype
// (design/prototype/notch-buddy.html, the visual source of truth) — the Swift
// arc angles produce a different shape.

import { Ease, lerp, type EaseFn } from "../core/anim";
import { Sound } from "../core/sound";
import type { BotEmoteName, BotStateName } from "../core/layout";
import type { ActivityLook } from "../island/activity";
import { fireLook } from "./look";
import { IDLE_FIRE, mixHex, paletteFor, paletteForRGB, type FirePalette } from "./palette";
import { eyeOffset } from "./eyes";
import { drawLuch, LUCH, luchEyeSize } from "./render";

// ── Types ─────────────────────────────────────────────────────────────────────

export type EyeShape =
  | "pill" | "wide" | "dot" | "line" | "flat" | "happy" | "closed"
  | "spiral" | "heart" | "star" | "tired" | "wink" | "cup";

export type BadgeKind = "dots" | "bang" | "question" | "dot";

export interface Badge {
  kind: BadgeKind;
  color: RGB;
}

export type RGB = readonly [number, number, number]; // components 0…1

export type TweenKey = readonly [target: number, durationMs: number, ease: EaseFn];

interface Tween {
  prop: PropKey;
  keys: TweenKey[];
  index: number;
  from: number;
  startMs: number;
  onComplete?: () => void;
}

type PropKey =
  | "yaw" | "pitch" | "roll" | "tilt" | "open" | "sx" | "sy"
  | "oy" | "ox" | "morph" | "hands" | "es" | "badgeS";

interface BotStateCfg {
  color: RGB;
  eye: EyeShape;
  badge: Badge | null;
  bounces: boolean;
  scans: boolean;
  breathes: boolean;
  zz: boolean;
  look: readonly [number, number] | null;
  tilt: number;
}

interface Particle {
  /** `sweat` is drawn as a puff of smoke (spec §2: smoke instead of the sweat drop). */
  type: "heart" | "star" | "spark" | "sweat" | "z" | "note";
  x: number; y: number; vx: number; vy: number;
  age: number; life: number; rot: number; size: number;
  /** A note draws "♫" when true, "♪" when false (spec §A: they take turns). */
  alt?: boolean;
}

// ── Constants (MochiConst / PISTES.mochi) ─────────────────────────────────────

/** Луч's eye ink (spec §1). */
const EYE_INK = "#2A170B";
/** Eye shapes that reach past a pill's box (arcs, spiral, heart, star): the travel keeps them more room. */
const BROAD_EYES: ReadonlySet<EyeShape> = new Set(["happy", "closed", "wink", "spiral", "heart", "star"]);
/** The fire box is LUCH_BOX × the canvas width: its body then spans about Mochi's width. */
export const LUCH_BOX = 1.5;
/**
 * A mini's fire box. Луч is taller than Mochi (the tongue): at LUCH_BOX the compact grid's lower
 * row reached below the 32 px bar. At 1.25 the flame is about 1.15 × its slot tall and 0.92 × wide.
 */
const MINI_BOX = 1.25;

const C = {
  idle: [0xff / 255, 0x74 / 255, 0x10 / 255] as RGB, // the idle fire (luch/palette IDLE_FIRE_RGB)
  working: [0.231, 0.62, 1] as RGB,
  thinking: [0.545, 0.361, 0.965] as RGB,
  searching: [0.388, 0.396, 0.949] as RGB,
  approval: [0.961, 0.647, 0.141] as RGB,
  question: [0.133, 0.827, 0.933] as RGB,
  error: [0.957, 0.314, 0.369] as RGB,
  finished: [0.204, 0.831, 0.6] as RGB,
  ratelimit: [0.984, 0.573, 0.235] as RGB,
  sleeping: [0.58, 0.635, 0.722] as RGB,
  dizzy: [0.957, 0.447, 0.714] as RGB,
};

const base = {
  bounces: false, scans: false, breathes: false, zz: false,
  look: null, tilt: 0,
};

export const BOT_STATES: Record<BotStateName, BotStateCfg> = {
  idle: { ...base, color: C.idle, eye: "pill", badge: null },
  working: { ...base, color: C.working, eye: "pill", badge: { kind: "dots", color: C.working } },
  thinking: { ...base, color: C.thinking, eye: "pill", badge: { kind: "dots", color: C.thinking }, look: [0.55, 0.55] },
  searching: { ...base, color: C.searching, eye: "pill", badge: { kind: "dots", color: C.searching }, scans: true },
  approval: { ...base, color: C.approval, eye: "wide", badge: { kind: "bang", color: C.approval }, bounces: true },
  question: { ...base, color: C.question, eye: "pill", badge: { kind: "question", color: C.question }, tilt: 0.17 },
  error: { ...base, color: C.error, eye: "flat", badge: { kind: "dot", color: C.error } },
  finished: { ...base, color: C.finished, eye: "happy", badge: { kind: "dot", color: C.finished } },
  ratelimit: { ...base, color: C.ratelimit, eye: "tired", badge: { kind: "dot", color: C.ratelimit } },
  sleeping: { ...base, color: C.sleeping, eye: "closed", badge: null, breathes: true, zz: true },
  dizzy: { ...base, color: C.dizzy, eye: "spiral", badge: null },
};

/** State → sound, as in BotStateCfg.sound. */
export const STATE_SOUND: Partial<Record<BotStateName, string>> = {
  working: "work", thinking: "think", searching: "search", approval: "approval",
  question: "question", error: "error", finished: "finish", ratelimit: "rate",
  sleeping: "sleep", dizzy: "dizzy",
};

const EMOTE_EYE: Record<BotEmoteName, EyeShape> = {
  love: "heart", surprised: "dot", proud: "star", wink: "wink",
  yawn: "tired", happy: "happy", annoyed: "line",
};

// ── Small helpers ─────────────────────────────────────────────────────────────

const now = () => performance.now() / 1000;

export function hexToRGB(hex: string): RGB {
  const h = hex.replace("#", "");
  const v = parseInt(h, 16);
  return [((v >> 16) & 255) / 255, ((v >> 8) & 255) / 255, (v & 255) / 255];
}

const rgba = (c: RGB, a = 1) =>
  `rgba(${Math.round(c[0] * 255)},${Math.round(c[1] * 255)},${Math.round(c[2] * 255)},${a})`;

const mix3 = (a: RGB, b: RGB, t: number): RGB => [
  lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t),
];

function roundRectPath(x: CanvasRenderingContext2D, X: number, Y: number, W: number, H: number, R: number) {
  const r = Math.max(0, Math.min(R, W / 2, H / 2));
  x.beginPath();
  x.moveTo(X + r, Y);
  x.arcTo(X + W, Y, X + W, Y + H, r);
  x.arcTo(X + W, Y + H, X, Y + H, r);
  x.arcTo(X, Y + H, X, Y, r);
  x.arcTo(X, Y, X + W, Y, r);
  x.closePath();
}

function heartPath(x: CanvasRenderingContext2D, s: number) {
  x.beginPath();
  x.moveTo(0, s * 0.38);
  x.bezierCurveTo(-s * 1.05, -s * 0.15, -s * 0.5, -s * 0.95, 0, -s * 0.38);
  x.bezierCurveTo(s * 0.5, -s * 0.95, s * 1.05, -s * 0.15, 0, s * 0.38);
  x.closePath();
}

function starPath(x: CanvasRenderingContext2D, ro: number, ri: number) {
  x.beginPath();
  for (let i = 0; i < 10; i++) {
    const r = i % 2 ? ri : ro;
    const a = -Math.PI / 2 + (i * Math.PI) / 5;
    x.lineTo(Math.cos(a) * r, Math.sin(a) * r);
  }
  x.closePath();
}

/**
 * The engine's animated state colour → its fire. Within IDLE_FADE of the idle colour the derived
 * palette fades into the idle fire, so a tween back to idle ends smoothly instead of snapping to
 * IDLE_FIRE at the last step. Every state colour is at least 0.17 from idle, so none is affected.
 */
const IDLE_FADE = 0.15;
function stateFire(c: RGB): FirePalette {
  const d = Math.max(...c.map((v, i) => Math.abs(v - C.idle[i])));
  if (d >= IDLE_FADE) return paletteForRGB(c);
  const p = paletteForRGB(c), k = d / IDLE_FADE;
  return {
    deep: mixHex(IDLE_FIRE.deep, p.deep, k), mid: mixHex(IDLE_FIRE.mid, p.mid, k),
    light: mixHex(IDLE_FIRE.light, p.light, k), core: mixHex(IDLE_FIRE.core, p.core, k),
  };
}

const FONT = `system-ui, "Segoe UI Variable Text", "Segoe UI", sans-serif`;

// ── Engine ────────────────────────────────────────────────────────────────────

export class BotEngine {
  isMini = false;
  /** The fire colour of mini bots / integration pills (null = the state colour). */
  bodyColor: RGB | null = null;

  // Animated state (BotEngine `s`)
  yaw = 0; pitch = 0; roll = 0; tilt = 0; open = 1;
  sx = 1; sy = 1; oy = 0; ox = 0;
  morph = 0; hands = 0; es = 1; badgeS = 0;

  // Targets
  tgYaw = 0; tgPitch = 0; tgTilt = 0; tgSy = 1; tgSx = 1; tgEs = 1;

  /** Extra canvas height above the body so hearts can fly out without clipping. */
  particleOverhang = 0;

  // Mouth spring (fraction of R)
  slotH = 0; slotHTarget = 0; slotHVel = 0; isChewing = false;

  col: RGB = C.idle;
  colT: RGB = C.idle;

  state: BotStateName = "idle";
  cfg: BotStateCfg = BOT_STATES.idle;

  eyeOverride: EyeShape | null = null;
  eyeOverrideUntil = 0;
  permanentEye: EyeShape | null = null;
  permanentEmote: BotEmoteName | null = null;
  /** The last triggered emote and when its look ends (its fire tint, height and smoke). */
  private lastEmote: BotEmoteName | null = null;
  private lastEmoteUntil = 0;
  miniNextBehavior = 0;

  badge: Badge | null = null;
  /**
   * What Luch is doing besides its state (spec §A): headphones and notes for music, a laptop or
   * a coffee for the pomodoro, sleep in the long break; `minutes` is the pomodoro time left, shown
   * in a pill in `minutesColor` when set. Never drawn on mini bots.
   */
  activity: (ActivityLook & { minutes: number | null; minutesColor: RGB }) | null = null;
  private badgeKey = "none";
  private badgeToken = 0;

  private tweens = new Map<PropKey, Tween>();
  private locks = new Set<PropKey>();
  private particles: Particle[] = [];
  /** Notes emitted so far: even ones draw "♪", odd ones "♫". */
  private noteCount = 0;

  lookX = 0;
  lookY = 0;

  lastTime = now();
  private t0 = now() - Math.random() * 5;
  private nextBlink = now() + 1.5 + Math.random() * 2;
  waveUntil = 0;
  waveStart = 0;
  private greetToken = 0;
  private lastAmbient = 0;
  private slapTimes: number[] = [];
  private miniLookTarget = { x: 0, y: 0 };
  private miniLookNextTime = 0;

  /** Fired when three slaps land inside 1.7 s (→ dizzy + confused view). */
  onDizzy: (() => void) | null = null;

  // ── Public API ──────────────────────────────────────────────────────────────

  setState(next: BotStateName, force = false) {
    if (this.state === next && !force) return;
    const prev = this.state;
    this.state = next;
    this.cfg = BOT_STATES[next];
    this.colT = this.cfg.color;
    if (!this.locks.has("tilt")) this.tgTilt = this.cfg.tilt;
    this.setBadge(this.cfg.badge);

    switch (next) {
      case "finished":
        this.doRoll(950, 1);
        setTimeout(() => this.emit("spark", 5), 500);
        break;
      case "error":
        this.anim("ox", [
          [0.08, 50, Ease.out], [-0.08, 70, Ease.inOut],
          [0.05, 70, Ease.inOut], [0, 90, Ease.out],
        ]);
        break;
      case "approval":
        this.anim("oy", [[-0.2, 150, Ease.out], [0, 300, Ease.back]]);
        break;
      case "dizzy":
        this.doRoll(1300, 2);
        break;
      case "question":
        this.blink();
        break;
      case "ratelimit":
        break; // Mochi's sweat drop is gone: Луч smokes only in error and annoyed (look.smoke)
      default:
        if (prev !== "idle" || next !== "idle") this.blink();
    }
  }

  setBadge(b: Badge | null) {
    const key = b ? `${b.kind}-${b.color.join(",")}` : "none";
    if (key === this.badgeKey) return;
    this.badgeKey = key;
    const tok = ++this.badgeToken;
    this.anim("badgeS", [[0, 90, Ease.inOut]]);
    setTimeout(() => {
      if (tok !== this.badgeToken) return;
      this.badge = b;
      if (b) this.anim("badgeS", [[1, 280, Ease.back]]);
    }, 100);
  }

  blink() {
    if (this.locks.has("open")) return;
    this.anim("open", [[0.06, 70, Ease.inOut], [1, 130, Ease.out]]);
  }

  squash() {
    this.anim("sy", [[0.78, 70, Ease.out], [1.1, 130, Ease.out], [1, 170, Ease.inOut]]);
    this.anim("sx", [[1.16, 70, Ease.out], [0.95, 130, Ease.out], [1, 170, Ease.inOut]]);
  }

  /** Mailbox swallow — opens the slot, chews, then closes. */
  gulp() {
    this.slotHTarget = 0.42;
    setTimeout(() => {
      this.slotHTarget = 0;
      this.isChewing = true;
      setTimeout(() => { this.isChewing = false; }, 800);
    }, 460);
    this.anim("sy", [[0.78, 80, Ease.out], [1.18, 130, Ease.out], [1, 220, Ease.back]]);
    this.anim("sx", [[1.28, 80, Ease.out], [0.92, 130, Ease.out], [1, 220, Ease.back]]);
    this.blink();
  }

  slap() {
    this.interruptGreet();
    if (this.state === "dizzy") return;
    const t = now();
    this.slapTimes = this.slapTimes.filter((s) => t - s < 1.7);
    this.slapTimes.push(t);
    Sound.play("slap");
    this.squash();
    if (this.slapTimes.length >= 3) {
      this.slapTimes = [];
      this.onDizzy?.();
    } else {
      this.eyeOverride = "line";
      this.eyeOverrideUntil = t + 0.8;
      // A click is Луч's annoyed (spec §3.2): the fire reddens and puffs smoke while the eyes last.
      this.lastEmote = "annoyed";
      this.lastEmoteUntil = t + 0.8;
      this.emit("sweat", 3);
      setTimeout(() => Sound.play("annoyed"), 60);
    }
  }

  doRoll(durationMs: number, turns: number) {
    this.roll = 0;
    this.anim("roll", [[Math.PI * 2 * turns, durationMs, Ease.inOut]], () => { this.roll = 0; });
  }

  /** Peek wave — the "coucou". Timings from BotEngine.greet(). */
  greet() {
    const t = now();
    const tok = ++this.greetToken;
    this.waveStart = t + 0.45;
    this.waveUntil = t + 1.55;

    this.eyeOverride = "happy";
    this.eyeOverrideUntil = t + 2.0;
    this.anim("oy", [[-0.06, 220, Ease.out], [0.0, 220, Ease.back]]);

    setTimeout(() => {
      if (this.greetToken !== tok) return;
      this.anim("hands", [[1, 280, Ease.out]]);
      this.anim("sy", [[0.95, 100, Ease.out], [1.0, 260, Ease.back]]);
      this.anim("sx", [[1.04, 100, Ease.out], [1.0, 260, Ease.back]]);
      Sound.play("greet");
    }, 250);

    setTimeout(() => { if (this.greetToken === tok) this.blink(); }, 550);
    setTimeout(() => { if (this.greetToken === tok) this.blink(); }, 1500);
    setTimeout(() => {
      if (this.greetToken !== tok) return;
      this.waveUntil = 0;
      this.anim("hands", [[0, 200, Ease.inOut]]);
    }, 1550);
    setTimeout(() => {
      if (this.greetToken !== tok) return;
      this.eyeOverride = "happy";
      this.eyeOverrideUntil = now() + 0.3;
    }, 1750);
  }

  interruptGreet() {
    if (this.hands <= 0.01 && now() >= this.waveUntil) return;
    this.greetToken++;
    this.waveUntil = 0;
    this.waveStart = 0;
    this.anim("hands", [[0, 150, Ease.inOut]]);
  }

  setPermanentEmote(emote: BotEmoteName | null) {
    this.permanentEmote = emote;
    if (emote === "wink") {
      this.miniNextBehavior = now() + 0.8 + Math.random() * 1.7;
      return;
    }
    this.permanentEye = emote ? EMOTE_EYE[emote] : null;
    if (this.permanentEye) {
      this.eyeOverride = this.permanentEye;
      this.eyeOverrideUntil = Number.POSITIVE_INFINITY;
    } else if (this.eyeOverrideUntil === Number.POSITIVE_INFINITY) {
      this.eyeOverride = null;
      this.eyeOverrideUntil = 0;
    }
    this.miniNextBehavior = now() + 0.8 + Math.random() * 1.7;
  }

  triggerEmote(emote: BotEmoteName, duration = 1.8) {
    const t = now();
    this.eyeOverride = EMOTE_EYE[emote];
    this.eyeOverrideUntil = t + duration;
    this.lastEmote = emote;
    this.lastEmoteUntil = t + duration;

    switch (emote) {
      case "love":
        this.emit("heart", 4);
        this.anim("oy", [[-0.1, 160, Ease.out], [0, 300, Ease.back]]);
        break;
      case "surprised":
        this.anim("oy", [[-0.3, 140, Ease.out], [0, 380, Ease.back]]);
        this.anim("es", [[1.25, 120, Ease.out], [1, 500, Ease.inOut]]);
        break;
      case "proud":
        this.emit("star", 5);
        this.anim("tilt", [
          [-0.14, 220, Ease.out], [-0.14, (duration - 0.5) * 1000, Ease.lin], [0, 280, Ease.inOut],
        ]);
        break;
      case "wink":
        this.anim("tilt", [
          [0.12, 160, Ease.out], [0.12, (duration - 0.4) * 1000, Ease.lin], [0, 240, Ease.inOut],
        ]);
        break;
      case "yawn":
        this.anim("sy", [[1.12, 500, Ease.inOut], [1, 500, Ease.inOut]]);
        this.anim("sx", [[0.94, 500, Ease.inOut], [1, 500, Ease.inOut]]);
        setTimeout(() => { this.eyeOverride = "closed"; this.emit("z", 2); }, 700);
        break;
      case "happy":
        break;
      case "annoyed":
        this.eyeOverride = "line";
        this.eyeOverrideUntil = t + 0.8;
        this.lastEmoteUntil = t + 0.8;
        this.emit("sweat", 3);
        setTimeout(() => Sound.play("annoyed"), 60);
        break;
    }
  }

  emit(type: Particle["type"], count: number) {
    for (let i = 0; i < count; i++) {
      const isZ = type === "z";
      this.particles.push({
        type,
        x: (Math.random() - 0.5) * 0.9 + (isZ ? 0.55 : 0),
        y: -0.7 - Math.random() * 0.2,
        vx: (Math.random() - 0.5) * 0.35 + (isZ ? 0.18 : 0),
        vy: -(0.45 + Math.random() * 0.35),
        age: -i * 0.14,
        life: 1.3 + Math.random() * 0.5,
        rot: Math.random() * Math.PI * 2,
        size: 0.15 + Math.random() * 0.08,
        alt: type === "note" ? this.noteCount++ % 2 === 1 : undefined,
      });
    }
  }

  animateMorph(target: number, durationMs?: number) {
    const dur = durationMs ?? (target > 0.5 ? 550 : 650);
    this.anim("morph", [[target, dur, Ease.inOut]]);
  }

  resetMorph() {
    this.tweens.delete("morph");
    this.locks.delete("morph");
    this.morph = 0;
  }

  /** The long break: Luch sleeps while it has nothing else to show (spec §A). */
  private get sleepLook(): boolean {
    return this.activity?.sleep === true && !this.isMini && (this.state === "idle" || this.state === "finished");
  }

  /** The fire always moves: a shown Луч keeps the island loop running (throttled, see island.ts). */
  readonly fireAlive = true;

  /**
   * True while anything besides the fire is still moving: the island runs at full rate then,
   * and at most 30 fps otherwise (the fire alone).
   */
  get busy(): boolean {
    return (
      this.tweens.size > 0 ||
      this.particles.length > 0 ||
      this.cfg.bounces || this.cfg.scans || this.cfg.breathes || this.cfg.zz ||
      this.isMini ||
      (this.activity?.notes === true && !this.isMini) ||
      this.sleepLook ||
      Math.abs(this.tgYaw - this.yaw) > 0.002 ||
      Math.abs(this.tgPitch - this.pitch) > 0.002 ||
      Math.abs(this.tgTilt - this.tilt) > 0.002 ||
      Math.abs(this.tgSy - this.sy) > 0.002 ||
      Math.abs(this.tgSx - this.sx) > 0.002 ||
      Math.abs(this.tgEs - this.es) > 0.002 ||
      this.slotH > 0.001 || Math.abs(this.slotHVel) > 0.001 ||
      Math.abs(this.col[0] - this.colT[0]) > 0.003 ||
      Math.abs(this.col[1] - this.colT[1]) > 0.003 ||
      Math.abs(this.col[2] - this.colT[2]) > 0.003
    );
  }

  // ── Tweens ──────────────────────────────────────────────────────────────────

  anim(prop: PropKey, keys: TweenKey[], onComplete?: () => void) {
    this.tweens.set(prop, {
      prop, keys, index: 0, from: this[prop], startMs: performance.now(), onComplete,
    });
    this.locks.add(prop);
  }

  // ── Update ──────────────────────────────────────────────────────────────────

  update(dt: number) {
    const n = now();
    const nowMs = performance.now();

    for (const tw of [...this.tweens.values()]) {
      const k = tw.keys[tw.index];
      const p = Math.min(1, Math.max(0, (nowMs - tw.startMs) / k[1]));
      this[tw.prop] = tw.from + (k[0] - tw.from) * k[2](p);
      if (p >= 1) {
        tw.from = k[0];
        tw.index += 1;
        tw.startMs = nowMs;
        if (tw.index >= tw.keys.length) {
          this.tweens.delete(tw.prop);
          this.locks.delete(tw.prop);
          tw.onComplete?.();
        }
      }
    }

    const t = n - this.t0;
    let ty = this.lookX * 0.62;
    let tp = this.lookY * 0.5;

    if (this.cfg.look) {
      ty = ty * 0.35 + this.cfg.look[0] * 0.55;
      tp = tp * 0.3 + this.cfg.look[1] * 0.5;
    }
    if (this.cfg.scans) {
      ty = Math.sin(t * 2.6) * 0.6;
      tp = -0.06;
    }
    if (this.state === "sleeping") { ty = 0; tp = -0.14; }
    if (this.state === "dizzy") { ty = Math.sin(t * 9) * 0.25; }

    // Mini bots never follow the mouse — they wander.
    if (this.isMini && !this.cfg.look && !this.cfg.scans && this.state !== "sleeping" && this.state !== "dizzy") {
      if (n > this.miniLookNextTime) {
        this.miniLookTarget = {
          x: -0.88 + Math.random() * 1.76,
          y: -0.55 + Math.random() * 1.0,
        };
        this.miniLookNextTime = n + 0.5 + Math.random() * 1.5;
      }
      ty = this.miniLookTarget.x * 0.62;
      tp = this.miniLookTarget.y * 0.5;
    }

    this.tgYaw = ty;
    this.tgPitch = tp;
    this.tgTilt = this.cfg.tilt;

    if (n > this.waveStart && n < this.waveUntil) {
      const wt = n - this.waveStart;
      this.tgTilt = -0.06 + Math.sin(2 * Math.PI * 1.2 * wt) * 0.07;
    }

    const bounce = this.cfg.bounces ? -Math.abs(Math.sin(t * 5.2)) * 0.07 : 0;
    const kGen = 1 - Math.pow(0.0008, dt);
    if (!this.locks.has("oy")) this.oy += (bounce - this.oy) * kGen;

    if (this.cfg.breathes) {
      const amp = this.isMini ? 0.07 : 0.035;
      this.tgSy = 1 + Math.sin(t * 1.8) * amp;
      this.tgSx = 1 - Math.sin(t * 1.8) * amp * 0.57;
    } else if (this.isMini) {
      this.tgSy = 1 + Math.sin(t * 2.2) * 0.04;
      this.tgSx = 1 - Math.sin(t * 2.2) * 0.02;
    } else {
      this.tgSy = 1;
      this.tgSx = 1;
    }

    if (this.isMini && n > this.miniNextBehavior) this.doMiniBehaviorLoop();

    const kLook = 1 - Math.pow(0.0025, dt);
    if (!this.locks.has("yaw")) this.yaw += (this.tgYaw - this.yaw) * kLook;
    if (!this.locks.has("pitch")) this.pitch += (this.tgPitch - this.pitch) * kLook;
    if (!this.locks.has("tilt")) this.tilt += (this.tgTilt - this.tilt) * kGen;
    if (!this.locks.has("sy")) this.sy += (this.tgSy - this.sy) * kGen;
    if (!this.locks.has("sx")) this.sx += (this.tgSx - this.sx) * kGen;
    if (!this.locks.has("es")) this.es += (this.tgEs - this.es) * kGen;

    this.col = mix3(this.col, this.colT, 1 - Math.pow(0.002, dt));

    if (n > this.nextBlink) {
      if (this.state !== "sleeping" && this.state !== "dizzy") {
        this.blink();
        if (Math.random() < 0.22) setTimeout(() => this.blink(), 230);
      }
      this.nextBlink = n + 2.2 + Math.random() * 3.2;
    }

    if (this.eyeOverride && n > this.eyeOverrideUntil) {
      this.eyeOverride = this.permanentEye;
      if (this.permanentEye) this.eyeOverrideUntil = Number.POSITIVE_INFINITY;
    }

    if (n - this.lastAmbient > 1.3) {
      this.lastAmbient = n;
      if (this.cfg.zz || this.sleepLook) this.emit("z", 1);
      if (this.activity?.notes && !this.isMini) this.emit("note", 1);
      if (!this.isMini && fireLook(this.state, this.activeEmote(), this.sleepLook).smoke && Math.random() < 0.5) {
        this.emit("sweat", 1);
      }
    }

    for (const p of this.particles) p.age += dt;
    this.particles = this.particles.filter((p) => p.age < p.life);

    // Mouth slot spring — ω₀ = 2π/0.25, ζ = 0.6
    const omega = (2 * Math.PI) / 0.25;
    const zeta = 0.6;
    const acc = omega * omega * (this.slotHTarget - this.slotH) - 2 * zeta * omega * this.slotHVel;
    this.slotHVel += acc * dt;
    this.slotH = Math.max(0, this.slotH + this.slotHVel * dt);

    this.lastTime = n;
  }

  private doMiniBehaviorLoop() {
    const n = now();
    switch (this.permanentEmote) {
      case "happy":
        if (this.locks.has("oy")) { this.miniNextBehavior = n + 0.4; return; }
        this.anim("oy", [[-0.3, 120, Ease.out], [0.03, 200, Ease.inOut], [0, 160, Ease.back]]);
        this.anim("sy", [[0.82, 80, Ease.out], [1.18, 130, Ease.out], [0.88, 160, Ease.inOut], [1, 200, Ease.back]]);
        this.anim("sx", [[1.15, 80, Ease.out], [0.88, 130, Ease.out], [1.06, 160, Ease.inOut], [1, 200, Ease.back]]);
        this.miniNextBehavior = n + 2.2 + Math.random() * 1.2;
        break;
      case "annoyed":
        if (this.locks.has("yaw")) { this.miniNextBehavior = n + 0.5; return; }
        this.anim("yaw", [
          [-0.65, 50, Ease.out], [0.65, 90, Ease.inOut], [-0.5, 80, Ease.inOut],
          [0.4, 75, Ease.inOut], [-0.2, 70, Ease.inOut], [0, 140, Ease.out],
        ]);
        this.miniNextBehavior = n + 3.0 + Math.random() * 2.5;
        break;
      case "wink":
        this.eyeOverride = "wink";
        this.eyeOverrideUntil = n + 0.55;
        this.anim("tilt", [[0.13, 100, Ease.out], [0.13, 320, Ease.lin], [0, 200, Ease.inOut]]);
        this.miniNextBehavior = n + 2.2 + Math.random() * 2.0;
        break;
      case "love":
        this.emit("heart", 2);
        this.anim("tilt", [[-0.1, 180, Ease.out], [0.1, 340, Ease.inOut], [0, 220, Ease.inOut]]);
        this.miniNextBehavior = n + 2.6 + Math.random() * 1.5;
        break;
      default:
        this.miniNextBehavior = n + 3.0 + Math.random() * 2.0;
    }
  }

  // ── Draw ────────────────────────────────────────────────────────────────────

  /**
   * Draws Луч (fire, eyes), the badge, particles and activity props into a canvas of
   * `W`×`H` CSS pixels (the caller has already applied the DPR transform).
   */
  draw(x: CanvasRenderingContext2D, W: number, H: number) {
    const R = W * 0.3;
    const cx = W / 2 + this.ox * R;
    const cy = H / 2 + this.particleOverhang / 2 + this.oy * R + R * 0.06;
    const u = W * (this.isMini ? MINI_BOX : LUCH_BOX);
    // Луч sits a little lower than Mochi's body (body centre p.y = -0.11 at cy + 0.105 u), so the
    // whole silhouette, tongue included, is centred there: the tongue stays inside the compact bar.
    const fy = cy + u * 0.05;

    const emote = this.activeEmote();
    const look = fireLook(this.state, emote, this.sleepLook);
    const palette = look.tint ? paletteFor(look.tint) : this.bodyColor ? paletteForRGB(this.bodyColor) : stateFire(this.col);
    const t = now() - this.t0;
    const waveT = now() > this.waveStart && now() < this.waveUntil ? now() - this.waveStart : -1;
    // Mochi's eye roll (finished, dizzy) becomes a sway of the whole flame
    const tilt = this.tilt + Math.sin(this.roll) * 0.12;

    x.save();
    x.translate(cx, fy + u * 0.22); // pivot at the flame base for squash and tilt
    if (tilt !== 0) x.rotate(tilt);
    x.scale(this.sx, this.sy);
    x.translate(-cx, -(fy + u * 0.22));
    drawLuch(x, cx, fy, u, t, {
      palette, look, grow: 1 - 0.75 * this.morph, coreA: 1 - this.morph,
      handsOut: this.hands, waveT, mini: this.isMini, embers: !this.isMini,
    });
    if (this.morph < 0.6) this.drawLuchEyes(x, u, cx, fy, emote);
    const a = this.activity;
    if (a?.headphones && !this.isMini) {
      // on Луч's body (centre p.y = -0.11), inside the flame's tilt and squash
      x.save(); x.translate(cx, fy + u * 0.055); this.drawHeadphones(x, R, R * 1.14, R * 0.88); x.restore();
    }
    x.restore();

    if (this.badge && this.badgeS > 0.01 && this.morph < 0.25) this.drawBadge(x, this.badge, R, cx, cy);
    this.drawParticles(x, R, cx, cy);
    if (a?.prop === "laptop" && !this.isMini) this.drawLaptop(x, R, cx, cy);
    if (a?.prop === "coffee" && !this.isMini) this.drawCoffee(x, R, cx, cy);
    if (a?.minutes != null && !this.isMini) this.drawMinutes(x, a.minutes, a.minutesColor, R, cx, cy);
  }

  /**
   * The fire draw() shows once the colour tween has arrived. The launch greeting lands Луч in it,
   * as the engine is not updated (nor drawn) while the greeting plays.
   */
  settledFire(): FirePalette {
    const look = fireLook(this.state, this.activeEmote(), this.sleepLook);
    return look.tint ? paletteFor(look.tint) : this.bodyColor ? paletteForRGB(this.bodyColor) : stateFire(this.colT);
  }

  /**
   * The greeting has just handed Луч over in its settled colour: no tween from the colour the engine
   * had before it, and the badge, which the greeting does not draw, pops in rather than being there.
   */
  takeOverFromGreeting() {
    this.col = [...this.colT] as RGB;
    if (this.badge) {
      this.badgeS = 0;
      this.anim("badgeS", [[1, 280, Ease.back]]);
    }
  }

  /** The emote whose look the fire wears now: a permanent one, else the last triggered one while it lasts. */
  private activeEmote(): BotEmoteName | null {
    if (this.permanentEmote) return this.permanentEmote;
    return now() < this.lastEmoteUntil ? this.lastEmote : null;
  }

  /** Eyes on the face panel (G4 measures), using the engine's eye shapes; the yawn adds an oval mouth. */
  private drawLuchEyes(x: CanvasRenderingContext2D, u: number, cx: number, fy: number, emote: BotEmoteName | null) {
    const s = u / 2;
    const shape: EyeShape = this.sleepLook ? "closed" : this.eyeOverride ?? this.cfg.eye;
    const size = luchEyeSize(u, this.isMini), step = size.step;
    const ew = size.w * this.es, eh = size.h * this.es;
    // Mochi's travel on the face panel; the extents of the drawn shapes keep the eyes inside it
    const wide = BROAD_EYES.has(shape), halfW = (wide ? 1.2 : 0.8) * ew, halfH = wide ? Math.max(0.6 * eh, 1.2 * ew) : 0.6 * eh;
    const off = eyeOffset(this.yaw, this.pitch, { cx: LUCH.eyeCx, cy: LUCH.eyeCy, hx: (step + halfW) / s, hy: halfH / s });
    const lookX = off.x * s, lookY = -off.y * s;
    x.fillStyle = x.strokeStyle = EYE_INK;
    for (const sd of [-1, 1]) {
      x.save();
      x.translate(cx + LUCH.eyeCx * s + sd * step + lookX, fy - LUCH.eyeCy * s + lookY);
      this.drawEyeShape(x, shape, ew, eh, sd, EYE_INK);
      x.restore();
    }
    if (emote === "yawn") {
      x.beginPath();
      x.ellipse(cx + LUCH.eyeCx * s + lookX, fy - (LUCH.eyeCy - 0.09) * s + lookY, ew * 0.7, eh * 0.45, 0, 0, Math.PI * 2);
      x.fill();
    }
  }

  private drawEyeShape(
    x: CanvasRenderingContext2D, shape: EyeShape,
    w: number, h: number, sd: number, ink: string,
  ) {
    const t = now();
    switch (shape) {
      case "wide":
        this.drawEyeShape(x, "pill", w * 1.16, h * 1.12, sd, ink);
        break;
      case "pill": {
        const hh = Math.max(h * this.open, w * 0.3);
        roundRectPath(x, -w / 2, -hh / 2, w, hh, Math.min(w / 2, hh / 2));
        x.fill();
        break;
      }
      case "dot":
        x.beginPath();
        x.arc(0, 0, w * 0.45, 0, Math.PI * 2);
        x.fill();
        break;
      case "line":
        x.rotate(-sd * 0.2);
        roundRectPath(x, -w * 0.78, -w * 0.21, w * 1.56, w * 0.42, w * 0.21);
        x.fill();
        break;
      case "flat":
        roundRectPath(x, -w * 0.72, -w * 0.2, w * 1.44, w * 0.4, w * 0.2);
        x.fill();
        break;
      case "happy":
        x.lineWidth = w * 0.5;
        x.lineCap = "round";
        x.beginPath();
        x.arc(0, h * 0.18, w * 0.82, Math.PI * 1.12, Math.PI * 1.88);
        x.stroke();
        break;
      case "closed":
        x.lineWidth = w * 0.36;
        x.lineCap = "round";
        x.beginPath();
        x.arc(0, -h * 0.08, w * 0.78, Math.PI * 0.15, Math.PI * 0.85);
        x.stroke();
        break;
      case "spiral": {
        x.lineWidth = w * 0.22;
        x.lineCap = "round";
        x.beginPath();
        for (let a = 0; a < 4.4 * Math.PI; a += 0.2) {
          const r = w * 0.06 + a * w * 0.058;
          const aa = a + t * 9 * sd;
          const px = Math.cos(aa) * r;
          const py = Math.sin(aa) * r;
          if (a === 0) x.moveTo(px, py);
          else x.lineTo(px, py);
        }
        x.stroke();
        break;
      }
      case "heart":
        x.fillStyle = "#FF4D6D";
        heartPath(x, w * 1.2);
        x.fill();
        x.fillStyle = ink;
        break;
      case "star":
        x.fillStyle = "#F7B32B";
        x.rotate(t * 1.5 * sd);
        starPath(x, w * 1.05, w * 0.46);
        x.fill();
        x.fillStyle = ink;
        break;
      case "tired":
        roundRectPath(x, -w / 2, -h * 0.02, w, h * 0.38, w / 2);
        x.fill();
        roundRectPath(x, -w * 0.62, -h * 0.1, w * 1.24, w * 0.22, w * 0.11);
        x.fill();
        break;
      case "wink":
        if (sd < 0) {
          const hh = Math.max(h * this.open, w * 0.3);
          roundRectPath(x, -w / 2, -hh / 2, w, hh, Math.min(w / 2, hh / 2));
          x.fill();
        } else {
          x.lineWidth = w * 0.5;
          x.lineCap = "round";
          x.beginPath();
          x.arc(0, h * 0.18, w * 0.82, Math.PI * 1.12, Math.PI * 1.88);
          x.stroke();
        }
        break;
      case "cup": {
        // Flat top, rounded bottom corners (U shape) — used while the box is open
        const hh = Math.max(h * this.open, w * 0.3);
        const cr = Math.min(w / 2, hh / 2);
        x.beginPath();
        x.moveTo(-w / 2, -hh / 2);
        x.lineTo(w / 2, -hh / 2);
        x.lineTo(w / 2, hh / 2 - cr);
        x.quadraticCurveTo(w / 2, hh / 2, w / 2 - cr, hh / 2);
        x.lineTo(-w / 2 + cr, hh / 2);
        x.quadraticCurveTo(-w / 2, hh / 2, -w / 2, hh / 2 - cr);
        x.closePath();
        x.fill();
        break;
      }
    }
  }

  /** A band over the head and a cup on each side, in the body's own coordinates. */
  private drawHeadphones(x: CanvasRenderingContext2D, R: number, rx: number, ry: number) {
    x.save();
    x.strokeStyle = "#585b70";
    x.fillStyle = "#cba6f7";
    x.lineWidth = R * 0.11;
    x.lineCap = "round";
    x.beginPath();
    x.arc(0, -ry * 0.05, rx * 1.06, Math.PI, Math.PI * 2);
    x.stroke();
    const cw = R * 0.24;
    const ch = R * 0.46;
    for (const sd of [-1, 1]) {
      roundRectPath(x, sd * rx * 1.02 - cw / 2, -ch * 0.35, cw, ch, cw / 2);
      x.fill();
    }
    x.restore();
  }

  /** A small laptop in front of Luch's lower edge: the pomodoro work phase. */
  private drawLaptop(x: CanvasRenderingContext2D, R: number, cx: number, cy: number) {
    const w = R * 1.5;
    const h = R * 0.5;
    const px = cx;
    const py = cy + R * 0.78;
    x.save();
    roundRectPath(x, px - w / 2, py - h / 2, w, h, R * 0.08);
    x.fillStyle = "#1e1e2e";
    x.fill();
    x.strokeStyle = "#45475a";
    x.lineWidth = 1;
    x.stroke();
    x.fillStyle = "#cdd6f4";
    x.beginPath();
    x.arc(px, py, R * 0.05, 0, Math.PI * 2);
    x.fill();
    x.restore();
  }

  /** A cup with a handle and two wisps of steam beside Luch: the pomodoro short break. */
  private drawCoffee(x: CanvasRenderingContext2D, R: number, cx: number, cy: number) {
    const w = R * 0.42;
    const h = R * 0.46;
    const px = cx + R * 1.15;
    const py = cy + R * 0.35;
    x.save();
    x.fillStyle = "#f5e0dc";
    roundRectPath(x, px - w / 2, py - h / 2, w, h, R * 0.08);
    x.fill();
    x.strokeStyle = "#f5e0dc";
    x.lineWidth = R * 0.06;
    x.lineCap = "round";
    x.beginPath();
    x.arc(px + w / 2, py, h * 0.28, -Math.PI / 2, Math.PI / 2);
    x.stroke();
    x.strokeStyle = "rgba(205,214,244,0.6)";
    x.lineWidth = R * 0.04;
    for (const sd of [-1, 1]) {
      x.beginPath();
      x.moveTo(px + sd * w * 0.2, py - h / 2 - R * 0.05);
      x.lineTo(px + sd * w * 0.2, py - h / 2 - R * 0.2);
      x.stroke();
    }
    x.restore();
  }

  /** The minutes left in a dark pill at Luch's top right, opposite the badge. */
  private drawMinutes(x: CanvasRenderingContext2D, minutes: number, color: RGB, R: number, cx: number, cy: number) {
    const text = String(minutes);
    const fs = R * 0.34;
    x.save();
    x.font = `800 ${fs}px ${FONT}`;
    const pw = Math.max(fs * 1.5, x.measureText(text).width + fs * 0.8);
    const ph = fs * 1.35;
    const bx = cx + R * 0.78 * this.sx;
    const by = cy - R * 0.72 * this.sy;
    roundRectPath(x, bx - pw / 2, by - ph / 2, pw, ph, ph / 2);
    x.fillStyle = "#000";
    x.fill();
    x.fillStyle = rgba(color);
    x.textAlign = "center";
    x.textBaseline = "middle";
    x.fillText(text, bx, by + fs * 0.04);
    x.restore();
  }

  private drawBadge(x: CanvasRenderingContext2D, badge: Badge, R: number, cx: number, cy: number) {
    const bs = this.badgeS * (this.isMini ? 1.25 : 1);
    // upper left as on Mochi, but further out: at Mochi's 0.72 R it sat on Луч's tongue
    const bx = cx - R * 0.9 * this.sx;
    const by = cy - R * 0.85 * this.sy;
    const t = now();

    x.save();
    x.translate(bx, by);
    x.scale(bs, bs);
    const col = rgba(badge.color);

    if (badge.kind === "dots") {
      if (this.isMini) {
        const phase = (t * 2.4) % 1;
        const dotR = R * 0.22 * (1 + 0.25 * Math.sin(phase * Math.PI * 2));
        x.fillStyle = "#000";
        x.beginPath();
        x.arc(0, 0, R * 0.2, 0, Math.PI * 2);
        x.fill();
        x.fillStyle = col;
        x.beginPath();
        x.arc(0, 0, dotR, 0, Math.PI * 2);
        x.fill();
      } else {
        const pw = R * 0.72;
        const ph = R * 0.36;
        roundRectPath(x, -pw / 2, -ph / 2, pw, ph, ph / 2);
        x.fillStyle = col;
        x.fill();
        for (let i = 0; i < 3; i++) {
          const phase = (((t * 2.4 - i * 0.22) % 1) + 1) % 1;
          const dotR = R * 0.055 * (1 + 0.4 * Math.max(0, Math.sin(phase * Math.PI * 2)));
          x.fillStyle = "#fff";
          x.beginPath();
          x.arc((i - 1) * R * 0.18, 0, dotR, 0, Math.PI * 2);
          x.fill();
        }
      }
    } else if (badge.kind === "bang" || badge.kind === "question") {
      x.fillStyle = "#000";
      x.beginPath();
      x.arc(0, 0, R * 0.3, 0, Math.PI * 2);
      x.fill();
      x.fillStyle = col;
      x.beginPath();
      x.arc(0, 0, R * 0.23, 0, Math.PI * 2);
      x.fill();
      if (!this.isMini) {
        x.fillStyle = "#fff";
        x.font = `900 ${R * 0.32}px ${FONT}`;
        x.textAlign = "center";
        x.textBaseline = "middle";
        x.fillText(badge.kind === "bang" ? "!" : "?", 0, R * 0.02);
      }
    } else {
      x.fillStyle = "#000";
      x.beginPath();
      x.arc(0, 0, R * 0.2, 0, Math.PI * 2);
      x.fill();
      x.fillStyle = col;
      x.beginPath();
      x.arc(0, 0, R * 0.135, 0, Math.PI * 2);
      x.fill();
    }
    x.restore();
  }

  private drawParticles(x: CanvasRenderingContext2D, R: number, cx: number, cy: number) {
    for (const p of this.particles) {
      if (p.age <= 0) continue;
      const k = p.age / p.life;
      const a = k < 0.2 ? k / 0.2 : 1 - (k - 0.2) / 0.8;
      const px = cx + (p.x + p.vx * p.age) * R * 1.3;
      const py = cy + (p.y + p.vy * p.age) * R * 1.3;
      const sz = R * p.size * (1 + k * 0.4);

      x.save();
      x.translate(px, py);
      x.globalAlpha = Math.min(1, Math.max(0, a));
      switch (p.type) {
        case "heart":
          x.rotate(Math.sin(p.age * 6) * 0.3);
          x.fillStyle = "#FF4D6D";
          heartPath(x, sz);
          x.fill();
          break;
        case "star":
          x.rotate(p.rot + p.age * 2);
          x.fillStyle = "#F7B32B";
          starPath(x, sz, sz * 0.45);
          x.fill();
          break;
        case "spark":
          x.rotate(p.rot);
          x.fillStyle = "#fff";
          starPath(x, sz * 0.8, sz * 0.18);
          x.fill();
          break;
        case "sweat": {
          // a soft grey puff of smoke that swells as it rises and fades
          const r = sz * (0.8 + 1.4 * k);
          const g = x.createRadialGradient(0, 0, 0, 0, 0, r);
          g.addColorStop(0, "rgba(150,146,150,0.55)");
          g.addColorStop(0.6, "rgba(120,116,122,0.3)");
          g.addColorStop(1, "rgba(100,96,104,0)");
          x.fillStyle = g;
          x.beginPath();
          x.arc(0, 0, r, 0, Math.PI * 2);
          x.fill();
          break;
        }
        case "note":
          x.fillStyle = "#f5c2e7";
          x.font = `700 ${sz * 2.1}px ${FONT}`;
          x.textAlign = "center";
          x.textBaseline = "middle";
          x.fillText(p.alt ? "♫" : "♪", 0, 0);
          break;
        case "z":
          x.fillStyle = "rgb(209,219,235)";
          x.font = `700 ${sz * 1.9}px ${FONT}`;
          x.textAlign = "center";
          x.textBaseline = "middle";
          x.fillText("z", 0, 0);
          break;
      }
      x.restore();
    }
  }
}
