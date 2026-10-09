# Mayak Luch Character Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Mochi with Луч (Luch), a living, stylised fire spirit drawn in code: the character, its mini version, the launch greeting ("sparks, as from a lighter"), the app and tray icons, and the name.

**Architecture:**
- `BotEngine` keeps its whole logic: states, emotes, tweens, particles, hand mechanics and activities. Only its `draw()` changes. It calls one shared renderer, `luch/render.ts`, which:
  - draws the fire with a shared WebGL shader (`luch/fire.ts`, shader G4);
  - falls back to a simple 2D flame (`luch/fallback.ts`) when WebGL is missing;
  - puts the eyes and embers on top in 2D.
- Pure modules hold the palette, the per-state look and the greeting timeline (`luch/palette.ts`, `luch/look.ts`, `luch/greeting-timeline.ts`), and Node checks pin them.
- The greeting is rewritten on that timeline. A dev preview page is approved by the user before the greeting is wired into the island.

**Tech Stack:** TypeScript (`windows/src`), WebGL 1 (GLSL ES 1.0) in WebView2, Canvas 2D, Node 22 checks (`windows/scripts/checks/`), a Node icon rasteriser (`windows/scripts/gen-icons.mjs`), Rust/Tauri untouched.

**Spec:** `docs/superpowers/specs/2026-10-09-mayak-luch-character-design.md`. Prototype: `design/luch/luch-g4.frag.glsl` and `design/luch/luch-prototype.html`. Open the prototype at `http://127.0.0.1:1420/../design/luch/luch-prototype.html` (or with any static server) to see the target.

## Global Constraints

- **Character drawn in code:**
  - no images, Rive or Lottie;
  - the shader source is code, kept in `windows/src/luch/fire.frag.glsl` and imported with Vite's `?raw`;
  - no new npm or Rust dependencies.
- **0 % CPU while the island is hidden:** nothing renders and no loop runs when `State.mode === "hidden"`.
- **While the fire is the only thing moving** (no geometry spring, no tween, no greeting, no upload), the island loop runs at most **30 fps**. Otherwise the loop runs at full rate as today.
- **G4 measures** (p in −1…1 over the fire box, y up):
  - body centre (0.01, −0.11), half-size 0.44 × 0.33;
  - face centre (0.03, −0.1), half-size 0.32 × 0.22, radius 0.2;
  - eyes: pair centre (0.09, −0.095), step ±0.093, eye size 0.022 × 0.04, ink `#2A170B`;
  - hands at (−0.4, −0.28) and (0.48, −0.18);
  - flame base y = −0.44.
- **Idle palette:** deep `#E0380A`, mid `#FF7410`, light `#FFBC34`, core `#FFF0C2`.
- **State palette from a colour c:**
  - deep = mix(c, #000000, 0.25);
  - mid = c;
  - light = mix(c, #FFFFFF, 0.45);
  - core = mix(#FFFBF2, c, 0.08).
- **Hands only for gestures:** today only the greeting wave, with Mochi's timings.
- **Mini Luch:** fire and eyes only — no embers, flares, hands or streaks.
- **Greeting beats** (spec §4, about 4.4 s):
  - open 0–0.35;
  - strikes at 0.45, 0.85, 1.25 — the third one catches;
  - rise 1.6–2.2, land 2.2;
  - wave 2.45–3.55, tuck 3.6–3.9;
  - leave 3.9–4.4.
  - Hover holds it up to 10 s; an alert interrupts it with its timers.
  - Sounds: strikes `tick`, catch `pop`, wave `greet`.
- **The user approves the greeting preview and the icons** before they are wired or generated. The controller stops and asks.
- **Names:** «Луч» (ru), "Luch" (en, de, fr, es), «ルーチ» (ja), «卢奇» (zh).
- **Do not touch:** `NotchBuddy/`, `design/prototype`, `design/captures`, `design/animations`, `design/outfits`. The sounds are sub-project B and stay out of scope.
- **Commits:**
  - one line, no trailers, no `Co-Authored-By`;
  - never mention Claude or AI;
  - stage by explicit path.
- **Checks after every task:**
  - `cd windows && npx tsc --noEmit && npx vite build && npm run check`;
  - Rust is untouched; still run `cargo test --workspace -j 1` (with `CARGO_TARGET_DIR=C:/tmp/mayak-target`) in the final task.
- **Live checks** use the dev app: `C:\tmp\mayak-spike\startdev.ps1` starts it; CDP lives at `node C:/tmp/mayak-spike/cdp.mjs "<expr>"`.
  - **Never click** Apply, Install, Remove, Enable or Disable in the running app.
  - **Never stop the user's own Mayak.**

## Review Focus

1. **WebGL context lost or unavailable** (driver reset, GPU blocklist). Expected: the island keeps drawing a 2D flame with the same face, no exception, no blank Mochi slot. Owner: Task 4 Step 5, which tests `?luchNoGL=1` and a forced context loss through the dev hook `__luchLoseContext`.
2. **Many mini Lucs at once** (8 pills plus compact grid plus the main bot) on a 2× DPR screen. Expected: one shared GL context — never one context per canvas, since browsers cap them at about 16 — and no flicker between minis. Owner: Task 5, which counts `getContext("webgl")` calls through a CDP probe.
3. **An island left open on an idle Luch for minutes.** Expected: the loop is throttled to ≤ 30 fps and the CPU stays bounded; hiding the island drops it to 0 %. Owner: Task 4, which measures fps with a CDP `requestAnimationFrame` counter in both states.
4. **The greeting interrupted at any moment** (alert at 0.5 s, 1.3 s, 3.0 s), or hovered then left. Expected: it collapses to the compact pose from where it is, with no leftover sparks or sounds and `onComplete` fired exactly once. Owner: Task 6, which checks `greetFrame(t, tc)` for tc at those times, plus a Task 8 live test.
5. **A state change while the fire tweens between palettes** (idle → error → finished in under a second). Expected: the colour moves smoothly, without a frame of the grey Mochi idle colour, and the badge stays correct. Owner: Task 2, whose `paletteForRGB` test covers the idle sentinel and a mid-tween colour.

---

## File Structure

| File | Responsibility |
|---|---|
| `windows/src/luch/` (moved from `windows/src/mochi/`) | the character package |
| `luch/engine.ts` | `BotEngine`: logic as before; `draw()` delegates to `render.ts`; badge, particles and props reused |
| `luch/palette.ts` (new, pure) | hex/RGB helpers, `IDLE_FIRE`, `paletteFor`, `paletteForRGB` |
| `luch/look.ts` (new, pure) | `fireLook(state, emote, sleepLook)` and `brightAt(look, t)` |
| `luch/fire.frag.glsl` (new) | the G4 shader, copied from `design/luch/luch-g4.frag.glsl` |
| `luch/fire.ts` (new) | the shared WebGL renderer: `renderFire(px, uniforms)` → `{ canvas, px } \| null` |
| `luch/fallback.ts` (new) | `drawFallbackFire(x, cx, cy, u, t, palette, look, grow)` — 2D flame |
| `luch/render.ts` (new) | `drawLuch(x, cx, cy, u, t, opts)`: fire (GL or fallback), eyes, embers; shared by engine, greeting, preview |
| `luch/minibots.ts` (moved) | unchanged API; mini engines get palette from `task.color` |
| `luch/greeting-timeline.ts` (new, pure) | `G` beat table, `greetFrame(t, tc)`, `sparkPoints(burst, t)` |
| `luch/greeting.ts` (rewritten) | `Greeting` class, same API; draws from `greetFrame` |
| `windows/dev/greeting-preview.html` + `.ts` (new) | scrubber preview of the greeting (not shipped) |
| `windows/scripts/gen-icons.mjs` | draws Luch instead of Mochi (CPU port of the field) |
| `windows/scripts/checks/luch-look.mjs`, `luch-greeting.mjs` (new) | Node checks |
| i18n `set.integrationsNote` ×7, `README.md`, `windows/README.md` | name |

---

### Task 1: Move `mochi/` to `luch/`, no behaviour change

**Files:**
- Move: `windows/src/mochi/{engine,greeting,minibots}.ts` → `windows/src/luch/`.
- Modify imports:
  - `windows/src/core/state.ts:4`;
  - `windows/src/island/island.ts:19-21`;
  - `windows/src/views/views.ts:18`;
  - any other `mochi/` import found by grep.

**Interfaces:** Produces the `windows/src/luch/` paths every later task uses.

- [ ] **Step 1: Move the folder with history**

```bash
cd windows && git mv src/mochi src/luch
```

- [ ] **Step 2: Fix every import**

```bash
grep -rln "mochi/" src --include=*.ts | xargs sed -i 's#\.\./mochi/#../luch/#g; s#\./mochi/#./luch/#g'
grep -rn "mochi/" src --include=*.ts
```

Expected: the second grep prints nothing.

- [ ] **Step 3: Run the checks**

```bash
npx tsc --noEmit && npx vite build && npm run check
```

Expected: all pass, `9/9 checks passed`.

- [ ] **Step 4: Commit**

```bash
git add -A src/luch src/mochi src/core/state.ts src/island/island.ts src/views/views.ts
git commit -m "refactor: move the character package from mochi/ to luch/"
```

---

### Task 2: Palette and look (pure) with checks

**Files:**
- Create: `windows/src/luch/palette.ts`, `windows/src/luch/look.ts`.
- Test: `windows/scripts/checks/luch-look.mjs`.

**Interfaces:**
- Consumes: `BotStateName`, `BotEmoteName` from `windows/src/core/layout.ts`.
- Produces:
  - `type Hex = string`;
  - `interface FirePalette { deep: Hex; mid: Hex; light: Hex; core: Hex }`;
  - `IDLE_FIRE: FirePalette`;
  - `mixHex(a: Hex, b: Hex, k: number): Hex`;
  - `rgbToHex(c: readonly [number, number, number]): Hex` (components 0…1);
  - `paletteFor(c: Hex | null): FirePalette`;
  - `paletteForRGB(c: readonly [number, number, number]): FirePalette`;
  - `IDLE_FIRE_RGB: readonly [number, number, number]` — mid of `IDLE_FIRE` as 0…1;
  - `interface FireLook { live: number; tall: number; bright: number; flicker: "none" | "flash" | "pulse" | "sputter"; smoke: boolean; tint: Hex | null }`;
  - `fireLook(state: BotStateName, emote: BotEmoteName | null, sleepLook: boolean): FireLook`;
  - `brightAt(look: FireLook, t: number): number`.

- [ ] **Step 1: Write the failing check `windows/scripts/checks/luch-look.mjs`**

```js
import assert from "node:assert/strict";
import { IDLE_FIRE, IDLE_FIRE_RGB, mixHex, rgbToHex, paletteFor, paletteForRGB } from "../../src/luch/palette.ts";
import { fireLook, brightAt } from "../../src/luch/look.ts";

// mixHex / rgbToHex
assert.equal(mixHex("#000000", "#ffffff", 0.5), "#808080");
assert.equal(mixHex("#ff7410", "#ff7410", 0.3), "#ff7410");
assert.equal(rgbToHex([1, 0.5, 0]), "#ff8000");

// idle palette (spec §1) and the derived palette (spec §3.1)
assert.deepEqual(paletteFor(null), IDLE_FIRE);
assert.deepEqual(IDLE_FIRE, { deep: "#e0380a", mid: "#ff7410", light: "#ffbc34", core: "#fff0c2" });
const blue = paletteFor("#3b9eff");
assert.equal(blue.mid, "#3b9eff");
assert.equal(blue.deep, mixHex("#3b9eff", "#000000", 0.25));
assert.equal(blue.light, mixHex("#3b9eff", "#ffffff", 0.45));
assert.equal(blue.core, mixHex("#fffbf2", "#3b9eff", 0.08));

// RGB entry: the idle sentinel gives the idle palette; anything else is derived
assert.deepEqual(paletteForRGB(IDLE_FIRE_RGB), IDLE_FIRE);
const mid = paletteForRGB([0.6, 0.3, 0.2]);
assert.equal(mid.mid, rgbToHex([0.6, 0.3, 0.2]));
// a colour half way from idle to red is derived, never Mochi's grey
assert.notEqual(paletteForRGB([1, 0.38, 0.2]).mid, "#e6e9ee");

// look table (spec §3.1–3.2)
assert.equal(fireLook("idle", null, false).live, 1);
assert.equal(fireLook("working", null, false).live, 1.5);
assert.equal(fireLook("approval", null, false).flicker, "flash");
assert.equal(fireLook("thinking", null, false).flicker, "pulse");
const err = fireLook("error", null, false);
assert.ok(err.tall < 1 && err.smoke && err.flicker === "sputter");
const sleep = fireLook("sleeping", null, false);
assert.ok(sleep.live <= 0.35 && sleep.tall <= 0.5 && sleep.bright < 0.6);
assert.deepEqual(fireLook("idle", null, true), sleep, "the long break sleeps");
assert.ok(fireLook("ratelimit", null, false).tall < 0.7);
assert.equal(fireLook("idle", "love", false).tint, "#ff6fa0");
assert.equal(fireLook("idle", "annoyed", false).tint, "#ff4a2a");
assert.ok(fireLook("idle", "annoyed", false).smoke);
assert.ok(fireLook("idle", "proud", false).tall > 1.1);
assert.ok(fireLook("idle", "yawn", false).tall < 0.8);
assert.equal(fireLook("working", null, false).tint, null);

// brightness stays in range for every flicker
for (const s of ["idle", "approval", "thinking", "error", "sleeping"]) {
  for (let t = 0; t < 5; t += 0.137) {
    const b = brightAt(fireLook(s, null, false), t);
    assert.ok(b > 0.3 && b <= 1.0001, `${s} ${t} ${b}`);
  }
}
console.log("luch-look: all passed");
```

- [ ] **Step 2: Run it to see it fail**

Run: `npm run check -- luch-look`
Expected: FAIL — cannot find module `palette.ts`.

- [ ] **Step 3: Write `windows/src/luch/palette.ts`**

```ts
// Fire palettes for Луч (spec §1, §3.1). Pure: no DOM, no WebGL.

export type Hex = string;
export interface FirePalette { deep: Hex; mid: Hex; light: Hex; core: Hex }

/** The idle fire, read off the reference (spec §1). */
export const IDLE_FIRE: FirePalette = { deep: "#e0380a", mid: "#ff7410", light: "#ffbc34", core: "#fff0c2" };

const toRgb = (h: Hex): [number, number, number] => {
  const v = parseInt(h.replace("#", ""), 16);
  return [(v >> 16) & 255, (v >> 8) & 255, v & 255];
};
const toHex = (c: readonly number[]): Hex =>
  "#" + c.map((v) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, "0")).join("");

export function mixHex(a: Hex, b: Hex, k: number): Hex {
  const A = toRgb(a), B = toRgb(b);
  return toHex(A.map((v, i) => v + (B[i] - v) * k));
}

/** Components 0…1, as `BotEngine` keeps its colours. */
export function rgbToHex(c: readonly [number, number, number]): Hex {
  return toHex(c.map((v) => v * 255));
}

/** The idle mid colour as 0…1 — the engine's idle state colour, so its colour tween never passes Mochi's grey. */
export const IDLE_FIRE_RGB: readonly [number, number, number] = [0xff / 255, 0x74 / 255, 0x10 / 255];

/** Spec §3.1: a state or pill colour → its fire; `null` → the idle fire. */
export function paletteFor(c: Hex | null): FirePalette {
  if (c == null) return IDLE_FIRE;
  const m = c.toLowerCase();
  return { deep: mixHex(m, "#000000", 0.25), mid: m, light: mixHex(m, "#ffffff", 0.45), core: mixHex("#fffbf2", m, 0.08) };
}

/** The engine's animated colour → its fire. Within 1/255 of the idle colour it is the idle fire. */
export function paletteForRGB(c: readonly [number, number, number]): FirePalette {
  const near = c.every((v, i) => Math.abs(v - IDLE_FIRE_RGB[i]) < 1 / 255);
  return near ? IDLE_FIRE : paletteFor(rgbToHex(c));
}
```

- [ ] **Step 4: Write `windows/src/luch/look.ts`**

```ts
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
```

- [ ] **Step 5: Run the check to see it pass**

Run: `npm run check -- luch-look`
Expected: `ok luch-look.mjs  luch-look: all passed`.

- [ ] **Step 6: Commit**

```bash
git add src/luch/palette.ts src/luch/look.ts scripts/checks/luch-look.mjs
git commit -m "feat(luch): fire palette and per-state look"
```

---

### Task 3: The shared fire renderer, the 2D fallback and `drawLuch`

**Files:**
- Create:
  - `windows/src/luch/fire.frag.glsl` — copy of `design/luch/luch-g4.frag.glsl`;
  - `windows/src/luch/fire.ts`;
  - `windows/src/luch/fallback.ts`;
  - `windows/src/luch/render.ts`.

**Interfaces:**
- Consumes: `FirePalette`, `FireLook`, `brightAt` (Task 2).
- Produces:
  - `renderFire(px: number, u: FireUniforms): { canvas: HTMLCanvasElement; px: number } | null`;
  - `interface FireUniforms { time: number; grow: number; live: number; bright: number; coreA: number; handsOut: number; tall: number; palette: FirePalette; hands: [number, number, number, number] }`;
  - `fireAvailable(): boolean`;
  - `drawFallbackFire(x: CanvasRenderingContext2D, u: number, t: number, palette: FirePalette, look: FireLook, grow: number): void` — draws centred on the current origin;
  - `interface LuchOpts { palette: FirePalette; look: FireLook; grow?: number; coreA?: number; handsOut?: number; waveT?: number; mini?: boolean; embers?: boolean }`;
  - `drawLuch(x: CanvasRenderingContext2D, cx: number, cy: number, u: number, t: number, o: LuchOpts): void` — draws the fire box of size `u` centred on (cx, cy). It does **not** draw eyes, so the engine keeps its eye shapes.
  - `handsInBox(t: number, handsOut: number, waveT: number): [number, number, number, number]` — hand centres in shader space (spec measures, Mochi wave).
  - `LUCH = { faceCx: 0.03, faceCy: -0.1, eyeCx: 0.09, eyeCy: -0.095, eyeStep: 0.093, eyeW: 0.022, eyeH: 0.04, baseY: -0.44 }` — the G4 measures, for the eyes.

- [ ] **Step 1: Copy the shader**

```bash
cp ../design/luch/luch-g4.frag.glsl src/luch/fire.frag.glsl
```

`windows/tsconfig.json` already has `"types": ["vite/client"]`, which declares `*?raw` modules as `string`: nothing else is needed.

`vite/client` declares `*?raw` modules as `string`.

- [ ] **Step 2: Write `windows/src/luch/fire.ts`**

```ts
// The one WebGL context that draws Луч's fire (spec §2). Every caller renders into the top-left
// px×px of the shared canvas and copies it out with drawImage before the next call.

import FRAG from "./fire.frag.glsl?raw";
import type { FirePalette } from "./palette";

export interface FireUniforms {
  time: number; grow: number; live: number; bright: number; coreA: number;
  handsOut: number; tall: number; palette: FirePalette; hands: [number, number, number, number];
}

const VERT = "attribute vec2 aPos; varying vec2 vUv; void main(){ vUv = aPos * 0.5 + 0.5; gl_Position = vec4(aPos, 0.0, 1.0); }";
const MAX = 512;
const NAMES = ["uTime", "uGrow", "uLive", "uBright", "uCoreA", "uHandsOut", "uTall", "uDeep", "uMid", "uLight", "uCore", "uHands"] as const;

interface Ctx { canvas: HTMLCanvasElement; gl: WebGLRenderingContext; u: Record<(typeof NAMES)[number], WebGLUniformLocation | null> }
let ctx: Ctx | null = null;
let broken = false;

const rgb01 = (h: string) => { const v = parseInt(h.slice(1), 16); return [((v >> 16) & 255) / 255, ((v >> 8) & 255) / 255, (v & 255) / 255]; };

function create(): Ctx | null {
  if (new URLSearchParams(location.search).has("luchNoGL")) return null;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = MAX;
  const gl = canvas.getContext("webgl", { premultipliedAlpha: true, alpha: true, preserveDrawingBuffer: true, antialias: false });
  if (!gl) return null;
  const sh = (type: number, src: string) => {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src); gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? "shader");
    return s;
  };
  const p = gl.createProgram()!;
  gl.attachShader(p, sh(gl.VERTEX_SHADER, VERT)); gl.attachShader(p, sh(gl.FRAGMENT_SHADER, FRAG));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? "link");
  gl.useProgram(p);
  const b = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, b);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
  const a = gl.getAttribLocation(p, "aPos"); gl.enableVertexAttribArray(a); gl.vertexAttribPointer(a, 2, gl.FLOAT, false, 0, 0);
  const u = Object.fromEntries(NAMES.map((n) => [n, gl.getUniformLocation(p, n)])) as Ctx["u"];
  canvas.addEventListener("webglcontextlost", (e) => { e.preventDefault(); ctx = null; broken = true; });
  canvas.addEventListener("webglcontextrestored", () => { broken = false; });
  return { canvas, gl, u };
}

/** False when WebGL is missing or the context is lost: callers draw the 2D fallback. */
export function fireAvailable(): boolean {
  if (broken) return false;
  if (!ctx) {
    try { ctx = create(); } catch (err) { console.error("luch: fire shader failed", err); ctx = null; }
    if (!ctx) broken = true;
  }
  return ctx != null;
}

export function renderFire(px: number, f: FireUniforms): { canvas: HTMLCanvasElement; px: number } | null {
  if (!fireAvailable() || !ctx) return null;
  const { gl, u, canvas } = ctx;
  const s = Math.min(MAX, Math.max(8, Math.round(px)));
  gl.viewport(0, MAX - s, s, s);
  gl.enable(gl.SCISSOR_TEST); gl.scissor(0, MAX - s, s, s);
  gl.clearColor(0, 0, 0, 0); gl.clear(gl.COLOR_BUFFER_BIT);
  gl.uniform1f(u.uTime, f.time); gl.uniform1f(u.uGrow, f.grow); gl.uniform1f(u.uLive, f.live);
  gl.uniform1f(u.uBright, f.bright); gl.uniform1f(u.uCoreA, f.coreA); gl.uniform1f(u.uHandsOut, f.handsOut);
  gl.uniform1f(u.uTall, f.tall);
  gl.uniform3fv(u.uDeep, rgb01(f.palette.deep)); gl.uniform3fv(u.uMid, rgb01(f.palette.mid));
  gl.uniform3fv(u.uLight, rgb01(f.palette.light)); gl.uniform3fv(u.uCore, rgb01(f.palette.core));
  gl.uniform4fv(u.uHands, f.hands);
  gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  return { canvas, px: s };
}
```

`broken` is set once WebGL has failed: the app does not retry every frame. A lost context clears `ctx`. On `webglcontextrestored` the next `fireAvailable()` recreates it.

- [ ] **Step 3: Write `windows/src/luch/fallback.ts`**

```ts
// The 2D flame used when WebGL is missing or lost (spec §2): the same silhouette and face panel,
// gradients instead of the shader, a gentle sway instead of the fire field.

import type { FireLook } from "./look";
import type { FirePalette } from "./palette";

/** Draws centred on the current origin in a box of u CSS px (p −1…1 maps to ±u/2, y up → canvas y down). */
export function drawFallbackFire(x: CanvasRenderingContext2D, u: number, t: number, pal: FirePalette, look: FireLook, grow: number) {
  const s = u / 2;
  const P = (px: number, py: number): [number, number] => [px * s, -py * s];
  x.save();
  x.translate(0, 0.44 * s); x.scale(grow, grow); x.translate(0, -0.44 * s);
  const sway = Math.sin(t * 2.1 * look.live) * 0.03;
  const tip = 0.66 * look.tall;
  const body = new Path2D();
  body.moveTo(...P(0.01, -0.44));
  body.bezierCurveTo(...P(0.42, -0.44), ...P(0.47, -0.05), ...P(0.3, 0.12));
  body.quadraticCurveTo(...P(0.12 + sway, tip * 0.55), ...P(-0.09 + sway * 2, tip));
  body.quadraticCurveTo(...P(-0.2, tip * 0.45), ...P(-0.36, 0.08));
  body.bezierCurveTo(...P(-0.47, -0.05), ...P(-0.42, -0.44), ...P(0.01, -0.44));
  const g = x.createRadialGradient(0, 0.1 * s, 0.05 * s, 0, 0.1 * s, 0.6 * s);
  g.addColorStop(0, pal.light); g.addColorStop(0.7, pal.mid); g.addColorStop(1, pal.deep);
  x.shadowColor = pal.mid; x.shadowBlur = 0.12 * s;
  x.fillStyle = g; x.fill(body);
  x.shadowBlur = 0;
  x.fillStyle = pal.core;
  x.beginPath(); x.roundRect((0.03 - 0.32) * s, (0.1 - 0.22) * s, 0.64 * s, 0.44 * s, 0.2 * s); x.fill();
  x.restore();
}
```

- [ ] **Step 4: Write `windows/src/luch/render.ts`**

```ts
// Луч's fire box: the GL fire (or the 2D fallback) plus rising embers. The eyes are the engine's.

import { drawFallbackFire } from "./fallback";
import { renderFire } from "./fire";
import { brightAt, type FireLook } from "./look";
import type { FirePalette } from "./palette";

export const LUCH = { faceCx: 0.03, faceCy: -0.1, eyeCx: 0.09, eyeCy: -0.095, eyeStep: 0.093, eyeW: 0.022, eyeH: 0.04, baseY: -0.44 } as const;

export interface LuchOpts {
  palette: FirePalette; look: FireLook;
  grow?: number; coreA?: number; handsOut?: number; waveT?: number;
  mini?: boolean; embers?: boolean;
}

const easeOut = (k: number) => 1 - Math.pow(1 - Math.min(1, Math.max(0, k)), 3);
const lerp = (a: number, b: number, k: number) => a + (b - a) * k;
const hash = (i: number) => { const s = Math.sin(i * 127.1 + 311.7) * 43758.5453; return s - Math.floor(s); };

/** Hand centres in shader space: tucked near the body at 0, at the lower corners at 1; the right hand waves (Mochi's 13 rad/s). */
export function handsInBox(t: number, out: number, waveT: number): [number, number, number, number] {
  const at = (sd: number): [number, number] => {
    let hx = lerp(sd * 0.22, sd < 0 ? -0.4 : 0.48, out);
    let hy = lerp(-0.3, sd < 0 ? -0.28 : -0.18, out);
    hy += Math.sin(t * 2.3 + sd * 1.3) * 0.02;
    if (sd > 0 && waveT >= 0) {
      const k = easeOut(waveT / 0.2);
      hx = lerp(hx, 0.56 + 0.05 * Math.cos(13 * waveT), k);
      hy = lerp(hy, 0.02 + 0.11 * Math.sin(13 * waveT), k);
    }
    return [hx, hy];
  };
  return [...at(-1), ...at(1)];
}

export function drawLuch(x: CanvasRenderingContext2D, cx: number, cy: number, u: number, t: number, o: LuchOpts) {
  const grow = o.grow ?? 1;
  if (grow <= 0.001 || u < 2) return;
  const look = o.look;
  const handsOut = o.mini ? 0 : Math.min(1, Math.max(0, o.handsOut ?? 0));
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  const tex = renderFire(u * dpr, {
    time: t, grow, live: look.live, bright: brightAt(look, t), coreA: o.coreA ?? 1,
    handsOut, tall: look.tall, palette: o.palette, hands: handsInBox(t, handsOut, o.waveT ?? -1),
  });
  x.save();
  x.translate(cx, cy);
  if (tex) x.drawImage(tex.canvas, 0, 0, tex.px, tex.px, -u / 2, -u / 2, u, u);
  else drawFallbackFire(x, u, t, o.palette, look, grow);
  if (!o.mini && o.embers !== false && grow > 0.5) {
    x.globalCompositeOperation = "lighter";
    for (let i = 0; i < 10; i++) {
      const ph = (t * (0.1 + 0.12 * hash(i + 3)) * look.live + hash(i)) % 1;
      const ex = (hash(i + 1) - 0.5) * u * 0.55 + Math.sin(t * 1.3 + i * 1.7) * u * 0.03 * ph;
      const ey = u * 0.05 - ph * u * 0.42;
      const a = Math.sin(ph * Math.PI) * 0.85 * (grow - 0.5) * 2;
      const r = u * (0.003 + 0.004 * hash(i + 7)) * (1 - 0.4 * ph);
      const g = x.createRadialGradient(ex, ey, 0, ex, ey, r * 4);
      g.addColorStop(0, o.palette.light); g.addColorStop(1, "rgba(0,0,0,0)");
      x.globalAlpha = a; x.fillStyle = g; x.beginPath(); x.arc(ex, ey, r * 4, 0, Math.PI * 2); x.fill();
    }
  }
  x.restore();
}
```

- [ ] **Step 5: Type-check and build**

Run: `npx tsc --noEmit && npx vite build`
Expected: PASS. The build inlines the shader string. Confirm it with `grep -c "smax" dist/assets/*.js` (≥ 1).

- [ ] **Step 6: Commit**

```bash
git add src/luch/fire.frag.glsl src/luch/fire.ts src/luch/fallback.ts src/luch/render.ts
git commit -m "feat(luch): shared WebGL fire renderer with a 2D fallback"
```

(The live check of the renderer happens through the engine in Task 4.)

---

### Task 4: The engine draws Луч; the loop stays alive at ≤ 30 fps

**Files:**
- Modify `windows/src/luch/engine.ts`:
  - `C.idle` (line ~76);
  - `draw()` (lines ~663-710);
  - `busy` getter (~473);
  - delete `bodyPath`, `drawBody`, `drawHandsBehind` and the blush block;
  - keep `drawEyeShape`, `drawBadge`, `drawParticles`, `drawLaptop`, `drawCoffee`, `drawHeadphones`, `drawMinutes`.
- Modify `windows/src/island/island.ts`, the frame loop end (~1428-1445).

**Interfaces:**
- Consumes:
  - `drawLuch`, `LUCH` (Task 3);
  - `fireLook`, `paletteForRGB`, `paletteFor`, `IDLE_FIRE_RGB` (Task 2).
- Produces:
  - `BotEngine.draw(x, W, H)` — same signature;
  - `BotEngine.fireAlive: boolean` — true for every engine (the fire always moves);
  - `LUCH_BOX = 1.5` — the fire box is 1.5 × the canvas width.

- [ ] **Step 1: The idle colour becomes the idle fire**

In `engine.ts`, the `C` table:

```ts
  idle: [0xff / 255, 0x74 / 255, 0x10 / 255] as RGB, // the idle fire (luch/palette IDLE_FIRE_RGB)
```

- [ ] **Step 2: Replace `draw()`**

The badge, particles and props keep `R = W * 0.3` and `cx`, `cy` as before. Only the body changes:

```ts
  draw(x: CanvasRenderingContext2D, W: number, H: number) {
    const R = W * 0.3;
    const cx = W / 2 + this.ox * R;
    const cy = H / 2 + this.particleOverhang / 2 + this.oy * R + R * 0.06;
    const u = W * LUCH_BOX;
    // the fire box is centred so the body centre (p.y = -0.11) sits where Mochi's body did
    const fy = cy - u * 0.055;

    const emote = this.activeEmote();
    const look = fireLook(this.state, emote, this.sleepLook);
    const palette = look.tint ? paletteFor(look.tint) : this.bodyColor ? paletteForRGB(this.bodyColor) : paletteForRGB(this.col);
    const t = now() - this.t0;
    const waveT = now() > this.waveStart && now() < this.waveUntil ? now() - this.waveStart : -1;

    x.save();
    x.translate(cx, fy + u * 0.22);           // pivot at the flame base for squash and tilt
    if (this.tilt !== 0) x.rotate(this.tilt);
    x.scale(this.sx, this.sy);
    x.translate(-cx, -(fy + u * 0.22));
    drawLuch(x, cx, fy, u, t, {
      palette, look, grow: 1 - 0.75 * this.morph, coreA: 1 - this.morph,
      handsOut: this.hands, waveT, mini: this.isMini, embers: !this.isMini,
    });
    if (this.morph < 0.6) this.drawLuchEyes(x, u, cx, fy);
    x.restore();

    if (this.badge && this.badgeS > 0.01 && this.morph < 0.25) this.drawBadge(x, this.badge, R, cx, cy);
    this.drawParticles(x, R, cx, cy);
    const a = this.activity;
    if (a?.headphones && !this.isMini) {
      // drawHeadphones works in body-local coordinates, as it did inside Mochi's transform
      x.save(); x.translate(cx, cy); this.drawHeadphones(x, R, R * 1.14, R * 0.88); x.restore();
    }
    if (a?.prop === "laptop" && !this.isMini) this.drawLaptop(x, R, cx, cy);
    if (a?.prop === "coffee" && !this.isMini) this.drawCoffee(x, R, cx, cy);
    if (a?.minutes != null && !this.isMini) this.drawMinutes(x, a.minutes, a.minutesColor, R, cx, cy);
  }

  /** The emote whose look the fire wears now: a permanent one, else the last triggered one while its eyes last. */
  private activeEmote(): BotEmoteName | null {
    if (this.permanentEmote) return this.permanentEmote;
    return this.eyeOverride && now() < this.eyeOverrideUntil ? this.lastEmote : null;
  }

  /** Eyes on the face panel (G4 measures), using the engine's eye shapes. */
  private drawLuchEyes(x: CanvasRenderingContext2D, u: number, cx: number, fy: number) {
    const s = u / 2;
    const shape: EyeShape = this.eyeOverride ?? this.cfg.eye;
    const ew = LUCH.eyeW * s * 1.7 * this.es, eh = LUCH.eyeH * s * 1.7 * this.es;
    const lookX = this.yaw * LUCH.eyeStep * s * 0.35, lookY = -this.pitch * LUCH.eyeStep * s * 0.3;
    for (const sd of [-1, 1]) {
      x.save();
      x.translate(cx + (LUCH.eyeCx + sd * LUCH.eyeStep) * s + lookX, fy - LUCH.eyeCy * s + lookY);
      x.fillStyle = x.strokeStyle = "#2A170B";
      this.drawEyeShape(x, shape, ew, eh, sd, "#2A170B");
      x.restore();
    }
  }
```

`triggerEmote` remembers `this.lastEmote = emote` (add the field `private lastEmote: BotEmoteName | null = null;`). The `sx`/`sy` scale is applied after the tilt, so squash and stretch work as before. Delete `bodyPath`, `drawBody`, `drawHandsBehind`, the blush and the `drawEyes`/`drawMouth` calls. Keep `drawMouth` only if the yawn needs it: the spec's yawn mouth is an oval, so call `this.drawMouth` with the face's `fy` only when `emote === "yawn"`. Otherwise delete it.

Imports to add: `drawLuch`, `LUCH` from `./render`; `fireLook` from `./look`; `paletteFor`, `paletteForRGB` from `./palette`.

- [ ] **Step 3: Make the fire always alive, and throttle the loop when it is the only thing moving**

In `engine.ts`:

```ts
  /** The fire always moves: a shown Луч keeps the island loop running (throttled, see island.ts). */
  readonly fireAlive = true;
```

In `island.ts` at the end of `frame`, replace the `busy` block:

```ts
    const settling = this.width.animating || this.height.animating || this.radius.animating;
    const bot = botPosition(State.mode, State.view, this.height.value, State.uploadProgress);
    const botShown = bot.opacity > 0 && bot.diameter > 0;
    const moving = State.mode === "hidden"
      ? settling
      : settling || !this.botCx.settled || !this.botCy.settled || !this.botSize.settled ||
        greetingActive || UploadSeq.isActive || (botShown && this.engine.busy);
    // Луч's fire never stops; nothing else moving → at most 30 fps (spec §2, plan Global Constraints).
    const fireOnly = State.mode !== "hidden" && !moving && (botShown || miniBotCount() > 0);

    if (moving) {
      requestAnimationFrame(this.frame);
    } else if (fireOnly) {
      window.setTimeout(() => requestAnimationFrame(this.frame), 1000 / 30);
    } else {
      this.running = false;
      Sound.idle();
    }
```

`engine.busy` stays as it is: without `fireAlive` in it, so an idle Луч counts as "fire only". Import `miniBotCount` from `../luch/minibots`.

- [ ] **Step 4: Type-check and build**

Run: `npx tsc --noEmit && npx vite build && npm run check`
Expected: PASS.

- [ ] **Step 5: Live check every state and emote**

1. Start (or restart) the dev app with `C:\tmp\mayak-spike\startdev.ps1`, bumping the log name.
2. Open the island: `node C:/tmp/mayak-spike/cdp.mjs 'window.__TAURI_INTERNALS__.invoke("plugin:event|emit",{event:"tray",payload:"open"})'`.
3. For each state, call `engine.setState` from the page and take a screenshot. Reach `engine` through the island instance as earlier sessions did: find the module URL with `performance.getEntriesByType("resource")`.

Compare each screenshot with the spec §3.1 table. Also check:
- **Emotes:** `triggerEmote("love"|"proud"|"annoyed"|"yawn")` → tint, tall and smoke as in §3.2.
- **Fps:** CDP probe `let n=0; const f=()=>{n++; requestAnimationFrame(f)}; f(); setTimeout(()=>console.log(n/3),3000)`.
  - Expected ≈ 30 for an idle open island and 60 during a geometry change.
  - With the island hidden (`tray` → `hide`), the island's own loop stops. Confirm with Task Manager: CPU of `mayak.exe` is 0 %.
- **Fallback:** reload the page with `?luchNoGL=1`. Expected: the 2D flame with the same eyes, and no console errors.
- **Context loss:** in `fire.ts` add a dev-only hook, `if (import.meta.env.DEV) (window as unknown as { __luchLoseContext?: () => void }).__luchLoseContext = () => ctx?.gl.getExtension("WEBGL_lose_context")?.loseContext();`, and call it through CDP. Expected: the 2D fallback from the next frame, no exception; the island keeps running.

- [ ] **Step 6: Commit**

```bash
git add src/luch/engine.ts src/island/island.ts
git commit -m "feat(luch): the engine draws Луч; the island keeps the fire alive at up to 30 fps"
```

---

### Task 5: Mini Луч in pills

**Files:**
- Modify: `windows/src/luch/minibots.ts` (comment only), `windows/src/luch/engine.ts` (mini branch of `draw()` already covered).
- Test: live.

**Interfaces:**
- Consumes: `draw()` (Task 4) with `isMini = true` and `bodyColor = hexToRGB(task.color)`.

- [ ] **Step 1: Make sure the mini path is fire and eyes only**

In `draw()`, `mini: this.isMini` already turns off hands and embers inside `drawLuch`. Check that the props are skipped for minis (they are, under `!this.isMini`) and that the badge keeps its mini scale. Update the comment at the top of `minibots.ts`: "Mini Lucs (pills + compact grid)".

- [ ] **Step 2: Live check**

Open the compact island with several sessions or pills.
- Expected: each pill shows a small living flame in its pill colour, with eyes, readable at 22 px. There is no rectangle around it, no embers and no hands.
- CDP probe for the number of WebGL contexts: run `HTMLCanvasElement.prototype.getContext` wrapped in a counter before the island loads (inject via `cdp.mjs` and reload). Expected: exactly 1 `webgl` context, whatever the number of pills.

- [ ] **Step 3: Commit**

```bash
git add src/luch/minibots.ts
git commit -m "feat(luch): mini Луч in the pill colour"
```

---

### Task 6: Greeting timeline (pure) with checks

**Files:**
- Create: `windows/src/luch/greeting-timeline.ts`.
- Test: `windows/scripts/checks/luch-greeting.mjs`.

**Interfaces:**
- Produces:
  - `G` (beat table);
  - `GREET_END = G.end`;
  - `interface Burst { t0: number; seed: number; n: number; x: number; y: number; power: number }`;
  - `BURSTS: readonly Burst[]`;
  - `interface SparkPoint { x: number; y: number; px: number; py: number; a: number; heat: number }` — head, tail 35 ms back, alpha, heat 1→0;
  - `sparkPoints(b: Burst, t: number): SparkPoint[]`;
  - `interface LuchPose { x: number; y: number; u: number; grow: number; sx: number; sy: number; eye: "closed" | "open" | "happy"; handsOut: number; waveT: number }`;
  - `interface GreetFrame { card: number; header: number; minis: number; luch: LuchPose | null; ember: { x: number; y: number; r: number } | null; bursts: readonly Burst[] }`;
  - `greetFrame(t: number, tc: number): GreetFrame` — `tc` is the collapse start (`Infinity` while not collapsing);
  - `CANVAS = { w: 640, h: 150 }`;
  - `CARD = { x: 10, y: 36, w: 620, h: 104 }`;
  - `FLINT = { x: 320, y: 128 }`;
  - `REST = { x: 320, y: 95, u: 170 }`;
  - `SMALL = { x: 320 - COMPACT_W / 2 + 40, y: 16, u: 44 }`.

- [ ] **Step 1: Write the failing check `windows/scripts/checks/luch-greeting.mjs`**

```js
import assert from "node:assert/strict";
import { G, GREET_END, BURSTS, sparkPoints, greetFrame, REST, SMALL, FLINT } from "../../src/luch/greeting-timeline.ts";

const I = Number.POSITIVE_INFINITY;
assert.equal(GREET_END, G.end);
assert.ok(Math.abs(G.end - 4.4) < 0.25, "about 4.4 s (spec §4)");

// three strikes at the spec beats, the third catches
assert.deepEqual(BURSTS.map((b) => b.t0), [0.45, 0.85, 1.25]);
assert.ok(BURSTS[2].n > BURSTS[0].n);

// no Луч before the catch; a growing ember right after it
assert.equal(greetFrame(0.2, I).luch, null);
assert.equal(greetFrame(1.0, I).luch, null);
const c = greetFrame(1.4, I);
assert.ok(c.luch && c.luch.grow > 0 && c.luch.grow < 0.5, "a small flame after the third strike");
assert.ok(c.luch.y > REST.y, "it starts low, near the flint");

// it rises, then lands at REST with a squash
const land = greetFrame(G.land + 0.04, I).luch;
assert.ok(Math.abs(land.y - REST.y) < 4 && land.grow > 0.95);
assert.ok(land.sy < 1 && land.sx > 1, "squash on landing");
assert.equal(greetFrame(G.land + 0.6, I).luch.sy.toFixed(2), "1.00");

// eyes closed until they open after landing
assert.equal(greetFrame(G.land - 0.1, I).luch.eye, "closed");
assert.notEqual(greetFrame(G.land + 0.3, I).luch.eye, "closed");

// the wave: hands out, the right hand waving, then tucked back
const w = greetFrame((G.wave0 + G.wave1) / 2, I).luch;
assert.equal(w.handsOut, 1); assert.ok(w.waveT > 0); assert.equal(w.eye, "happy");
assert.equal(greetFrame(G.tuck1 + 0.05, I).luch.handsOut, 0);

// it leaves for the compact island
const end = greetFrame(G.end, I).luch;
assert.ok(Math.abs(end.x - SMALL.x) < 1 && Math.abs(end.y - SMALL.y) < 1 && Math.abs(end.u - SMALL.u) < 1);
assert.ok(greetFrame(G.end, I).minis > 0.99);

// interrupted at any moment: collapses from where it is to SMALL within G.COLLAPSE, no hands, no sparks after
for (const tc of [0.5, 1.3, 3.0]) {
  const f = greetFrame(tc + G.COLLAPSE + 0.01, tc);
  assert.equal(f.card, 0); assert.equal(f.header, 0);
  if (f.luch) {
    assert.ok(Math.abs(f.luch.x - SMALL.x) < 1 && Math.abs(f.luch.u - SMALL.u) < 1, `tc ${tc}`);
    assert.equal(f.luch.handsOut, 0);
  }
  assert.equal(f.bursts.length, 0, `no sparks after the collapse (tc ${tc})`);
}

// sparks: start at the flint, rise, fall under gravity, fade out
const pts0 = sparkPoints(BURSTS[0], BURSTS[0].t0 + 0.02);
assert.ok(pts0.length >= 6);
for (const p of pts0) assert.ok(Math.hypot(p.x - FLINT.x, p.y - FLINT.y) < 20);
const later = sparkPoints(BURSTS[0], BURSTS[0].t0 + 0.2);
assert.ok(later.some((p) => p.y < FLINT.y - 10), "they fly up");
assert.equal(sparkPoints(BURSTS[0], BURSTS[0].t0 + 1.2).length, 0, "all gone");
for (const p of later) assert.ok(p.a >= 0 && p.a <= 1 && p.heat >= 0 && p.heat <= 1);
console.log("luch-greeting: all passed");
```

- [ ] **Step 2: Run it to see it fail**

Run: `npm run check -- luch-greeting`
Expected: FAIL — module not found.

- [ ] **Step 3: Write `windows/src/luch/greeting-timeline.ts`**

```ts
// The launch greeting "sparks, as from a lighter" (spec §4) as a pure function of time.
// Canvas 640×150, the same frame as the old greeting; island.ts sizes the island around it.

import { COMPACT_W } from "../core/layout";

export const G = {
  open1: 0.35, strikes: [0.45, 0.85, 1.25] as const, catchAt: 1.3,
  rise0: 1.6, land: 2.2, eyes: 2.3, wave0: 2.45, wave1: 3.55, tuck1: 3.9,
  leave0: 3.9, end: 4.4, autoLeave: 4.9, COLLAPSE: 0.34,
};
export const GREET_END = G.end;
export const CANVAS = { w: 640, h: 150 };
export const CARD = { x: 10, y: 36, w: 620, h: 104 };
export const FLINT = { x: 320, y: 128 };
export const REST = { x: 320, y: 95, u: 170 };
export const SMALL = { x: 320 - COMPACT_W / 2 + 40, y: 16, u: 44 };

export interface Burst { t0: number; seed: number; n: number; x: number; y: number; power: number }
export const BURSTS: readonly Burst[] = [
  { t0: 0.45, seed: 11, n: 8, x: FLINT.x, y: FLINT.y, power: 0.8 },
  { t0: 0.85, seed: 12, n: 11, x: FLINT.x, y: FLINT.y, power: 0.9 },
  { t0: 1.25, seed: 13, n: 16, x: FLINT.x, y: FLINT.y, power: 1.1 },
];

const clamp01 = (k: number) => Math.min(1, Math.max(0, k));
const seg = (t: number, a: number, b: number) => clamp01((t - a) / (b - a));
const lerp = (a: number, b: number, k: number) => a + (b - a) * k;
const easeOut = (k: number) => 1 - Math.pow(1 - clamp01(k), 3);
const easeInOut = (k: number) => { k = clamp01(k); return k < 0.5 ? 4 * k * k * k : 1 - Math.pow(-2 * k + 2, 3) / 2; };
const hash = (i: number) => { const s = Math.sin(i * 127.1 + 311.7) * 43758.5453; return s - Math.floor(s); };

export interface SparkPoint { x: number; y: number; px: number; py: number; a: number; heat: number }
const GRAV = 520;

export function sparkPoints(b: Burst, t: number): SparkPoint[] {
  const dt = t - b.t0;
  if (dt < 0) return [];
  const out: SparkPoint[] = [];
  for (let i = 0; i < b.n; i++) {
    const h1 = hash(b.seed * 97 + i), h2 = hash(b.seed * 31 + i * 7), h3 = hash(b.seed * 13 + i * 3);
    const ang = -Math.PI / 2 + (h1 - 0.5) * 1.5;
    const v = 260 * b.power * (0.45 + 0.55 * h2);
    const life = 0.55 * (0.6 + 0.4 * h3);
    if (dt > life) continue;
    const pos = (d: number) => [b.x + Math.cos(ang) * v * d, b.y + Math.sin(ang) * v * d + 0.5 * GRAV * d * d];
    const [x, y] = pos(dt), [px, py] = pos(Math.max(0, dt - 0.035));
    out.push({ x, y, px, py, a: 1 - dt / life, heat: 1 - dt / life });
  }
  return out;
}

export interface LuchPose { x: number; y: number; u: number; grow: number; sx: number; sy: number; eye: "closed" | "open" | "happy"; handsOut: number; waveT: number }
export interface GreetFrame {
  card: number; header: number; minis: number;
  luch: LuchPose | null; ember: { x: number; y: number; r: number } | null; bursts: readonly Burst[];
}

function play(t: number): GreetFrame {
  const card = easeOut(seg(t, 0, G.open1));
  const bursts = BURSTS.filter((b) => t >= b.t0 && t < b.t0 + 0.7);
  // the second strike leaves an ember that glows a moment and dies
  const e2 = BURSTS[1].t0;
  const ember = t > e2 + 0.15 && t < e2 + 0.45 ? { x: FLINT.x + 6, y: FLINT.y, r: 3 * (1 - seg(t, e2 + 0.25, e2 + 0.45)) } : null;
  if (t < G.catchAt) return { card, header: card, minis: 0, luch: null, ember, bursts };

  // caught: a small flame breathes in at the flint, then rises to REST and lands with a squash
  const breathe = easeOut(seg(t, G.catchAt, G.rise0));
  const rise = easeInOut(seg(t, G.rise0, G.land));
  const grow = lerp(0.18 + 0.2 * breathe, 1, rise);
  let x = REST.x, y = lerp(FLINT.y - 8, REST.y, rise), u = REST.u, sx = 1, sy = 1;
  if (t > G.rise0 && t < G.land) { const k = Math.sin(Math.PI * seg(t, G.rise0, G.land)); sx = 1 - 0.08 * k; sy = 1 + 0.12 * k; }
  if (t >= G.land && t < G.land + 0.5) {
    const k = seg(t, G.land, G.land + 0.5);
    const d = Math.exp(-6 * k) * Math.cos(k * Math.PI * 3);
    sx = 1 + 0.1 * d; sy = 1 - 0.12 * d;
  }
  // the happy hop after the wave
  if (t >= G.wave1 + 0.05 && t < G.tuck1) y -= 10 * Math.sin(Math.PI * seg(t, G.wave1 + 0.05, G.tuck1));
  // leave for the compact island
  const lv = easeInOut(seg(t, G.leave0, G.end));
  x = lerp(x, SMALL.x, lv); y = lerp(y, SMALL.y, lv); u = lerp(u, SMALL.u, lv);

  const eye = t < G.eyes ? "closed" : t >= G.wave0 - 0.1 && t < G.tuck1 ? "happy" : "open";
  const handsOut = t < G.wave0 ? 0 : t < G.wave0 + 0.28 ? easeOut((t - G.wave0) / 0.28) : t < G.wave1 ? 1 : 1 - easeOut((t - G.wave1) / 0.25);
  const waveT = t >= G.wave0 + 0.15 && t < G.wave1 ? t - G.wave0 - 0.15 : -1;
  return {
    card: card * (1 - seg(t, G.leave0, G.leave0 + 0.2)), header: card * (1 - seg(t, G.leave0, G.leave0 + 0.1)),
    minis: easeOut(seg(t, G.end - 0.2, G.end)),
    luch: { x, y, u, grow, sx, sy, eye, handsOut: t >= G.tuck1 ? 0 : handsOut, waveT }, ember: null, bursts,
  };
}

/** The frame at t; from `tc` on it collapses from where it was to the compact pose in G.COLLAPSE. */
export function greetFrame(t: number, tc: number): GreetFrame {
  if (t < tc) return play(Math.min(t, G.end + 10));
  const a = play(tc);
  const e = easeInOut(seg(t, tc, tc + G.COLLAPSE));
  const luch = a.luch && {
    ...a.luch, x: lerp(a.luch.x, SMALL.x, e), y: lerp(a.luch.y, SMALL.y, e), u: lerp(a.luch.u, SMALL.u, e),
    sx: lerp(a.luch.sx, 1, e), sy: lerp(a.luch.sy, 1, e), handsOut: a.luch.handsOut * (1 - seg(t, tc, tc + 0.15)),
    waveT: -1, eye: "open" as const,
  };
  return {
    card: a.card * (1 - seg(t, tc, tc + 0.18)), header: a.header * (1 - seg(t, tc, tc + 0.1)),
    minis: easeOut(seg(t, tc + 0.24, tc + 0.42)), luch, ember: null,
    bursts: t > tc + G.COLLAPSE ? [] : a.bursts,
  };
}
```

- [ ] **Step 4: Run the check to see it pass**

Run: `npm run check -- luch-greeting`
Expected: `ok luch-greeting.mjs  luch-greeting: all passed`. If a beat assertion fails, adjust the timeline, not the check. The check is the spec.

- [ ] **Step 5: Commit**

```bash
git add src/luch/greeting-timeline.ts scripts/checks/luch-greeting.mjs
git commit -m "feat(luch): greeting timeline — sparks as from a lighter"
```

---

### Task 7: The greeting drawing and its preview page (user approval)

**Files:**
- Rewrite: `windows/src/luch/greeting.ts`. Keep the `Greeting` class API: `start()`, `hover()`, `interrupt()`, `elapsed`, `done`, `onComplete`, `draw(x)`, and the export `GREETING_END`.
- Create: `windows/dev/greeting-preview.html`, `windows/dev/greeting-preview.ts`.

**Interfaces:**
- Consumes:
  - `greetFrame`, `sparkPoints`, `G`, `CARD`, `CANVAS` (Task 6);
  - `drawLuch` (Task 3);
  - `IDLE_FIRE`, `fireLook` (Task 2).
- Produces:
  - `drawGreeting(x: CanvasRenderingContext2D, t: number, tc: number): void`, exported for the preview;
  - `Greeting` (same API as today).

- [ ] **Step 1: Write the drawing in `greeting.ts`**

```ts
// The launch greeting, drawn from greetFrame (spec §4). Same controller API as before.

import { Sound } from "../core/sound";
import { BURSTS, CARD, CANVAS, G, GREET_END, greetFrame, sparkPoints } from "./greeting-timeline";
import { fireLook } from "./look";
import { IDLE_FIRE } from "./palette";
import { drawLuch, LUCH } from "./render";

export const GREETING_END = GREET_END;
const LOOK = fireLook("idle", null, false);

function eyes(x: CanvasRenderingContext2D, cx: number, cy: number, u: number, shape: "closed" | "open" | "happy", t: number) {
  const s = u / 2;
  x.fillStyle = x.strokeStyle = "#2A170B"; x.lineCap = "round"; x.lineWidth = Math.max(1, u * 0.01);
  const blink = (t % 4.1) < 0.16 ? Math.sin(((t % 4.1) / 0.16) * Math.PI) : 0;
  for (const sd of [-1, 1]) {
    const ex = cx + (LUCH.eyeCx + sd * LUCH.eyeStep) * s, ey = cy - LUCH.eyeCy * s;
    const w = LUCH.eyeW * s, h = LUCH.eyeH * s;
    x.beginPath();
    if (shape === "open") { x.ellipse(ex, ey, w / 2, (h / 2) * (1 - 0.9 * blink), 0, 0, Math.PI * 2); x.fill(); }
    else if (shape === "happy") { x.arc(ex, ey + h * 0.2, w * 0.75, Math.PI * 1.15, Math.PI * 1.85); x.stroke(); }
    else { x.arc(ex, ey - h * 0.1, w * 0.75, Math.PI * 0.15, Math.PI * 0.85); x.stroke(); }
  }
}

export function drawGreeting(x: CanvasRenderingContext2D, t: number, tc: number) {
  const f = greetFrame(t, tc);
  x.clearRect(0, 0, CANVAS.w, CANVAS.h);
  if (f.card > 0.01) {
    x.save(); x.globalAlpha = f.card; x.fillStyle = "#141518";
    x.beginPath(); x.roundRect(CARD.x, CARD.y, CARD.w, CARD.h, 20); x.fill(); x.restore();
  }
  // sparks: glowing streaks, white-hot to orange
  x.save(); x.globalCompositeOperation = "lighter"; x.lineCap = "round";
  for (const b of f.bursts) {
    // a flash at the flint for each strike
    const k = (t - b.t0) / 0.15;
    if (k >= 0 && k < 1) {
      const g = x.createRadialGradient(b.x, b.y, 0, b.x, b.y, 26);
      g.addColorStop(0, `rgba(255,240,200,${0.9 * (1 - k)})`); g.addColorStop(1, "rgba(255,140,40,0)");
      x.fillStyle = g; x.beginPath(); x.arc(b.x, b.y, 26, 0, Math.PI * 2); x.fill();
    }
    for (const p of sparkPoints(b, t)) {
      const r = Math.round(255), gC = Math.round(150 + 100 * p.heat), bC = Math.round(40 + 160 * p.heat);
      x.strokeStyle = `rgba(${r},${gC},${bC},${p.a})`; x.lineWidth = 1.6; x.shadowColor = "#ff8a1f"; x.shadowBlur = 6;
      x.beginPath(); x.moveTo(p.px, p.py); x.lineTo(p.x, p.y); x.stroke();
    }
  }
  if (f.ember && f.ember.r > 0.2) {
    x.fillStyle = "rgba(255,170,60,0.9)"; x.shadowBlur = 8; x.beginPath(); x.arc(f.ember.x, f.ember.y, f.ember.r, 0, Math.PI * 2); x.fill();
  }
  x.restore();
  const l = f.luch;
  if (l) {
    // the caught flame has a bluish base, like a lighter's, until it rises
    if (t < G.land && t >= G.catchAt) {
      const a = 0.6 * (1 - Math.min(1, (t - G.rise0) / 0.4));
      if (a > 0) {
        const g = x.createRadialGradient(l.x, l.y + l.u * 0.2, 0, l.x, l.y + l.u * 0.2, l.u * 0.12 * l.grow);
        g.addColorStop(0, `rgba(120,170,255,${a})`); g.addColorStop(1, "rgba(120,170,255,0)");
        x.fillStyle = g; x.beginPath(); x.arc(l.x, l.y + l.u * 0.2, l.u * 0.12 * l.grow, 0, Math.PI * 2); x.fill();
      }
    }
    x.save();
    x.translate(l.x, l.y + l.u * 0.22); x.scale(l.sx, l.sy); x.translate(-l.x, -(l.y + l.u * 0.22));
    drawLuch(x, l.x, l.y, l.u, t, { palette: IDLE_FIRE, look: LOOK, grow: l.grow, coreA: Math.min(1, Math.max(0, (l.grow - 0.5) / 0.35)), handsOut: l.handsOut, waveT: l.waveT, mini: l.u < 60 });
    if (l.grow > 0.85) eyes(x, l.x, l.y, l.u, l.eye, t);
    x.restore();
  }
}
```

The `Greeting` class:
- It is the old controller with the old `T` replaced by `G`.
- `start()` schedules the sounds: `tick` at each `BURSTS[i].t0`, `pop` at `G.catchAt`, `greet` at `G.wave0`, and `fire()` at `G.end + 0.05`.
- `draw(x)` calls `drawGreeting(x, this.elapsed, this.tc)`, after the same auto-leave logic: `if (!this.fired && t >= G.end && this.tc >= G.autoLeave) this.fire()`.

Delete everything that drew Mochi:
- `mochiPath`, `drawMochi`, `drawHandL`, `drawHandR`;
- the old particles;
- the old `greetPose`, `smallPose`, `pose`.

`drawMinis` stays, drawing small flames: it calls `drawLuch` with `mini: true`, `u: 14` and the four `MINI_COLORS` through `paletteFor`.

- [ ] **Step 2: Write the preview `windows/dev/greeting-preview.html` and `.ts`**

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <title>Mayak — greeting preview</title>
    <style>
      body { margin: 0; background: #1b1b1f; font: 12px system-ui; color: #ddd; }
      #stage { width: 640px; height: 150px; margin: 24px auto 8px; background: #000; border-radius: 0 0 22px 22px; }
      #bar { width: 640px; margin: 0 auto; display: flex; gap: 8px; align-items: center; }
      #t { flex: 1; } #clock { width: 60px; font: 600 13px ui-monospace, monospace; }
    </style>
  </head>
  <body>
    <canvas id="stage"></canvas>
    <div id="bar">
      <button id="play">Pause</button><button id="restart">Restart</button>
      <button id="cut">Interrupt now</button>
      <input id="t" type="range" min="0" max="5.2" step="0.01" value="0" /><span id="clock">0.00 s</span>
    </div>
    <script type="module" src="./greeting-preview.ts"></script>
  </body>
</html>
```

```ts
import { drawGreeting } from "../src/luch/greeting";

const c = document.getElementById("stage") as HTMLCanvasElement;
const dpr = Math.min(2, window.devicePixelRatio || 1);
c.width = 640 * dpr; c.height = 150 * dpr;
const x = c.getContext("2d")!;
const slider = document.getElementById("t") as HTMLInputElement;
const clock = document.getElementById("clock")!;
let playing = true, start = performance.now() / 1000, tc = Number.POSITIVE_INFINITY, frozen = 0;

document.getElementById("play")!.onclick = (e) => {
  playing = !playing; (e.target as HTMLButtonElement).textContent = playing ? "Pause" : "Play";
  if (playing) start = performance.now() / 1000 - frozen;
};
document.getElementById("restart")!.onclick = () => { start = performance.now() / 1000; tc = Number.POSITIVE_INFINITY; playing = true; };
document.getElementById("cut")!.onclick = () => { tc = performance.now() / 1000 - start; };
slider.oninput = () => { playing = false; frozen = Number(slider.value); };

function frame() {
  const t = playing ? (performance.now() / 1000 - start) % 5.4 : frozen;
  if (playing) { frozen = t; slider.value = String(t); }
  clock.textContent = `${t.toFixed(2)} s`;
  x.setTransform(dpr, 0, 0, dpr, 0, 0);
  drawGreeting(x, t, tc);
  requestAnimationFrame(frame);
}
frame();
```

Vite serves it in dev at `http://127.0.0.1:1420/dev/greeting-preview.html`. It is not in `rollupOptions.input`, so it never ships, the same as `upload-preview.html`.

- [ ] **Step 3: Type-check, build, checks**

Run: `npx tsc --noEmit && npx vite build && npm run check`
Expected: PASS.

- [ ] **Step 4: Frame-by-frame review, then STOP for the user**

Open the preview in the built-in browser and capture frames at 0.3, 0.5, 0.9, 1.3, 1.5, 1.9, 2.25, 2.4, 3.0, 3.7, 4.1 and 4.4 s with the slider. Check the spec §4 quality bar:
- no linear motion;
- the sparks fall under gravity;
- the landing squash;
- the hands only during the wave.

Fix what does not meet it. Then the controller shows the preview URL to the user and **waits for an explicit approval** before Task 8. Every change the user asks for goes back through Step 4.

- [ ] **Step 5: Commit (after approval)**

```bash
git add src/luch/greeting.ts dev/greeting-preview.html dev/greeting-preview.ts
git commit -m "feat(luch): the sparks greeting and its preview page"
```

---

### Task 8: Wire the greeting into the island

**Files:**
- Modify: `windows/src/island/island.ts`. The import path is already `../luch/greeting` after Task 1. Check that the `greet()` sound is not played twice: the engine's `greet()` is not called by the greeting.

**Interfaces:**
- Consumes: `Greeting` (Task 7).

- [ ] **Step 1: Check the call sites**

```bash
grep -n "greeting\.\|Greeting" src/island/island.ts
```

`start`, `hover`, `interrupt`, `draw`, `onComplete`, `done` and `elapsed` must all exist on the new class with the same meaning. No edit is needed if they match.

- [ ] **Step 2: Live check**

1. Restart the dev app. The greeting plays at launch.
2. Watch it in the island (screenshot sequence through CDP). It must look like the approved preview.
3. Hover during the greeting. Expected: it holds, then collapses after the cursor leaves.
4. Trigger an alert during it. Inject one with `cdp.mjs`, calling the island's alert path as the answers tests did. Expected: the greeting collapses at once, its timed sounds stop, and the alert card opens.
5. Quiet mode on: the greeting plays without sound.

- [ ] **Step 3: Commit (only if Step 1 needed an edit)**

```bash
git add src/island/island.ts
git commit -m "feat(luch): the island plays the sparks greeting"
```

---

### Task 9: Icons (user approval)

**Files:**
- Modify: `windows/scripts/gen-icons.mjs` — replace `renderMochi` and the tray drawing with a CPU port of the G4 field at a fixed time.
- Generated: `windows/src-tauri/icons/*`.

**Interfaces:**
- Produces `renderLuch(size: number, opts: { tray: boolean }): Uint8Array` (RGBA, premultiplied the same way the file already encodes).

- [ ] **Step 1: Port the field to JS**

In `gen-icons.mjs`, add pure JS versions of the shader functions:
- `hash`, `noise`, `fbm3`, `smax`, `rot`, `drop`, `rbox`, `lick`;
- `main()` as `fieldColor(px, py)`, which returns `[r, g, b, a]` with the G4 constants and idle palette, `t = 1.7` (a pleasant pose), `uHandsOut = 0`, `uCoreA = 1`, `uTall = 1`, `uGrow = 1`.

Port them line by line from `src/luch/fire.frag.glsl`. `fract`, `mix`, `smoothstep` and `clamp` become small JS helpers. The eyes are the G4 ovals in ink.

Then:
- **App icon:** a dark rounded square (`#121216`, radius 22 % of the size) with Luch filling 80 % of it, supersampled ×4 as the file already does.
- **Tray icon:** at 16 and 32 px, a simplified pass. Only the silhouette (`a > 0.5`), filled with the mid colour, a 1 px darker rim and two 1–2 px ink dots. No glow, no streaks.

- [ ] **Step 2: Render a review sheet and STOP for the user**

Write `windows/scripts/icon-review.html` (not shipped). It shows the generated PNGs at 16, 32, 48 and 256 px on light and dark backgrounds. Generate into a temp folder first:

```bash
node scripts/gen-icons.mjs --out C:/tmp/mayak-spike/icons-review
```

The `--out` flag writes elsewhere instead of `src-tauri/icons`. Add it with a small `process.argv` check. The controller shows the sheet to the user and **waits for approval**.

- [ ] **Step 3: Generate the real icons and build**

```bash
npm run icons && npx vite build
```

Expected: `src-tauri/icons/*` changed. The tray shows the new icon after a dev restart.

- [ ] **Step 4: Commit**

```bash
git add scripts/gen-icons.mjs scripts/icon-review.html src-tauri/icons
git commit -m "feat(luch): app and tray icons drawn from the fire field"
```

---

### Task 10: Name, strings, README

**Files:**
- Modify:
  - `windows/src/core/i18n/{en,ru,de,fr,es,ja,zh}.ts` (`set.integrationsNote`);
  - `README.md`, `windows/README.md`;
  - comments mentioning Mochi in `windows/src/luch/*`, `windows/src/core/layout.ts`, `windows/src/island/*`.

- [ ] **Step 1: Replace the name in `set.integrationsNote`**

| Language | Name |
|---|---|
| ru | Луч |
| en, de, fr, es | Luch |
| ja | ルーチ |
| zh | 卢奇 |

Keep every `{var}` as is.

```bash
npm run check -- i18n
```

Expected: both i18n checks pass.

- [ ] **Step 2: Update the comments**

Run `grep -rn "Mochi" src --include=*.ts`. Change the comments that describe the current character to Луч/Luch. Keep upstream port references, such as "port of BotEngine.swift", unchanged.

- [ ] **Step 3: Update the READMEs**

- `windows/README.md`:
  - "Mochi" becomes "Luch (Луч), Mayak's fire spirit" in the text;
  - remove the five `screenshots/*.png` image lines that show Mochi;
  - add one line: "Luch and the icons are drawn in code (`src/luch/`, `scripts/gen-icons.mjs`)".
- Root `README.md`: the Windows section says the character is Luch.
- Leave `NotchBuddy` docs alone.

- [ ] **Step 4: Checks and commit**

```bash
npx tsc --noEmit && npx vite build && npm run check
git add src/core/i18n src/luch src/core/layout.ts src/island README.md ../README.md
git commit -m "docs: Луч replaces Mochi in strings, comments and READMEs"
```

---

### Task 11: Final verification and ideas update

**Files:**
- Modify: `docs/superpowers/ideas/2026-10-06-mayak-next-ideas.md` (§26 status).

- [ ] **Step 1: Full checks**

```bash
cd windows && npx tsc --noEmit && npx vite build && npm run check
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target && cargo test --workspace -j 1
```

Expected: everything passes.

- [ ] **Step 2: Load measurement**

With the dev app open on an idle Луч for 60 s:
- record the `mayak.exe` and `msedgewebview2.exe` CPU from Task Manager or `Get-Counter`;
- hide the island and record again.

Expected:
- the hidden island adds 0 %;
- the open island stays within twice the Mochi figure. Measure Mochi first on `main` with the same method if no figure exists, and record both numbers in the ledger.

- [ ] **Step 3: Record §26 status**

Under §26 in the ideas file add:

> **Итог (дата):** персонаж Луч сделан (`feature/luch-character`): облик G4, приветствие «искры», иконки, имя. Осталось: подпроект Б — свои звуки; до него Mayak не распространять.

- [ ] **Step 4: Commit**

```bash
git add ../docs/superpowers/ideas/2026-10-06-mayak-next-ideas.md
git commit -m "docs: §26 — Луч done, sounds remain"
```
