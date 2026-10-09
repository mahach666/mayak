// Dev harness: the real Луч (engine, shader, eyes) changing colour (luch/colour.ts) at the open island's
// size, at the compact bar's size and enlarged: a button per state, the love and annoyed tints, a burst
// of changes and a random state every few seconds. sheet(name) draws a contact sheet of a scripted
// change on a virtual clock (for review captures). Not part of the app bundle.

import { BotEngine } from "../src/luch/engine";
import { COLOUR_VARIANTS, colourVariant, setColourVariant, type ColourVariant } from "../src/luch/colour";
import { fireFps, CHARACTER_FPS } from "../src/island/pace";
import type { BotStateName } from "../src/core/layout";

/** The island's canvas: Луч's box `w` (layout diameter / 0.6) plus the particle overhang (island.ts). */
const OVERHANG = 40;
const STATES: BotStateName[] = ["idle", "working", "thinking", "searching", "approval", "question", "error", "finished", "ratelimit", "sleeping"];

interface View { engine: BotEngine; canvas: HTMLCanvasElement; w: number; compact: boolean }

if (new URLSearchParams(location.search).has("luchNoGL")) (document.getElementById("nogl") as HTMLInputElement).checked = true;
(document.getElementById("nogl") as HTMLInputElement).onchange = (e) => {
  const u = new URL(location.href);
  if ((e.target as HTMLInputElement).checked) u.searchParams.set("luchNoGL", "1"); else u.searchParams.delete("luchNoGL");
  location.href = u.href;
};

const stage = document.getElementById("stage")!;
const dpr = Math.min(2, window.devicePixelRatio || 1);

function view(label: string, w: number, compact: boolean): View {
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(w * dpr);
  canvas.height = Math.round((w + OVERHANG) * dpr);
  canvas.style.width = `${w}px`;
  canvas.style.height = `${w + OVERHANG}px`;
  const fig = document.createElement("figure");
  const cap = document.createElement("figcaption");
  cap.textContent = label;
  fig.append(canvas, cap);
  stage.append(fig);
  const engine = new BotEngine();
  engine.particleOverhang = OVERHANG;
  return { engine, canvas, w, compact };
}

// the home page's Луч is 58 px (layout.ts overview, box 58 / 0.6), the compact bar's 20 px (box 20 / 0.6)
const views = [
  view("enlarged ×2.5", Math.round((58 / 0.6) * 2.5), false),
  view("open island", Math.round(58 / 0.6), false),
  view("compact", Math.round(20 / 0.6), true),
];

let state: BotStateName = "idle";
const timers: number[] = [];

function setBotState(s: BotStateName) {
  state = s;
  for (const v of views) v.engine.setState(s);
  for (const b of document.querySelectorAll<HTMLButtonElement>("button[data-state]")) b.classList.toggle("on", b.dataset.state === s);
}
function love() { for (const v of views) v.engine.triggerEmote("love"); }
function annoyed() { for (const v of views) v.engine.triggerEmote("annoyed"); }
/** Three changes 0.15 s apart, from whatever is shown now. */
function rapid(after: (ms: number, f: () => void) => void = (ms, f) => { timers.push(window.setTimeout(f, ms)); }) {
  after(0, () => setBotState("working"));
  after(150, () => setBotState("finished"));
  after(300, () => setBotState("error"));
}

const statesBar = document.getElementById("states")!;
for (const s of STATES) {
  const b = document.createElement("button");
  b.textContent = s; b.dataset.state = s;
  b.onclick = () => setBotState(s);
  statesBar.append(b);
}
document.getElementById("love")!.onclick = love;
document.getElementById("annoyed")!.onclick = annoyed;
document.getElementById("rapid")!.onclick = () => rapid();
const cycle = document.getElementById("cycle") as HTMLInputElement;
const every = document.getElementById("every") as HTMLInputElement;
const pace = document.getElementById("pace") as HTMLInputElement;
const status = document.getElementById("status")!;
setBotState("idle");

let lastSwitch = performance.now();
const last = views.map(() => 0);
let prev = performance.now();

/** Scripted captures step a virtual clock instead (manual mode): the engine reads performance.now(). */
let manual = false;
const realNow = performance.now.bind(performance);
let virtualMs = 0;

function drawView(v: View, dt: number) {
  v.engine.update(dt);
  const x = v.canvas.getContext("2d")!;
  x.setTransform(dpr, 0, 0, dpr, 0, 0);
  x.clearRect(0, 0, v.w, v.w + OVERHANG);
  v.engine.draw(x, v.w, v.w + OVERHANG);
}

function frame(nowMs: number) {
  if (manual) { requestAnimationFrame(frame); return; }
  if (cycle.checked && nowMs - lastSwitch >= Math.max(0.2, Number(every.value) || 3) * 1000) {
    lastSwitch = nowMs;
    const others = STATES.filter((s) => s !== state);
    setBotState(others[Math.floor(Math.random() * others.length)]);
  } else if (!cycle.checked) lastSwitch = nowMs;
  views.forEach((v, i) => {
    // the island: full rate (about CHARACTER_FPS) while anything besides the fire moves, a colour
    // change included (engine.busy), the fire's rate otherwise (pace.ts)
    const fps = v.engine.busy ? CHARACTER_FPS : fireFps(v.compact, v.engine.dancing);
    if (pace.checked && nowMs - last[i] < 1000 / fps - 2) return;
    const dt = Math.min(0.05, (nowMs - (last[i] || nowMs)) / 1000);
    last[i] = nowMs;
    drawView(v, dt);
  });
  const ch = views[0].engine.colourChanging;
  status.textContent = `${state} · colour ${ch ? "changing" : "settled"} · ${(1000 / Math.max(1, nowMs - prev)).toFixed(0)} rAF fps`;
  prev = nowMs;
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

// ── Contact sheets (virtual clock) ────────────────────────────────────────────

function setManual(on: boolean) {
  manual = on;
  // the virtual clock never runs back: an emote from an earlier sheet must not outlive it
  if (on) { virtualMs = Math.max(realNow(), virtualMs); performance.now = () => virtualMs; } else performance.now = realNow;
}
function advance(dt: number) {
  virtualMs += dt * 1000;
  for (const v of views) drawView(v, dt);
}
/** Advance `s` seconds in 1/60 s steps (the island's full rate while the colour changes). */
function run(s: number) { for (let i = 0; i < Math.round(s * 60); i++) advance(1 / 60); }

type Script = { start: BotStateName; events: [number, () => void][]; until: number; step: number; label: string };
const SCRIPTS: Record<string, Script> = {
  "idle-working": { start: "idle", events: [[0, () => setBotState("working")]], until: 1.3, step: 0.1, label: "idle → working" },
  "working-finished": { start: "working", events: [[0, () => setBotState("finished")]], until: 1.3, step: 0.1, label: "working → finished" },
  "working-error": { start: "working", events: [[0, () => setBotState("error")]], until: 1.3, step: 0.1, label: "working → error" },
  "love-short": { start: "idle", events: [[0, love]], until: 2.6, step: 0.2, label: "love: the tint in and out (1.8 s)" },
  "idle-error": { start: "idle", events: [[0, () => setBotState("error")]], until: 1.3, step: 0.1, label: "idle → error" },
  "love": { start: "idle", events: [[0, love]], until: 3.2, step: 0.2, label: "love: the tint in (1.8 s) and out" },
  "rapid": { start: "idle", events: [[0, () => setBotState("working")], [0.15, () => setBotState("finished")], [0.3, () => setBotState("error")]], until: 2.2, step: 0.15, label: "rapid: working, finished, error 0.15 s apart" },
};

/** Draws the scripted change `name` as a contact sheet: one column per capture, a row per size. */
function sheet(name: string, oneRow = false): string {
  const sc = SCRIPTS[name];
  if (!sc) throw new Error(`no script ${name}`);
  setManual(true);
  setBotState(sc.start);
  for (const v of views) { v.engine.lookX = 0; v.engine.lookY = 0; }
  run(2.5); // settle in the start colour
  const times: number[] = [];
  for (let t = 0; t <= sc.until + 1e-9; t += sc.step) times.push(Math.round(t * 1000) / 1000);
  const rows = [views[0], views[2]];
  const cw = views[0].canvas.width, chs = rows.map((v) => v.canvas.height);
  const cols = oneRow ? times.length : Math.min(times.length, 9);
  const lines = Math.ceil(times.length / cols);
  const cellH = chs.reduce((a, b) => a + b, 0) + 22 * dpr;
  const out = document.getElementById("sheet") as HTMLCanvasElement;
  out.width = cols * cw; out.height = lines * cellH + 26 * dpr;
  out.style.width = `${out.width / dpr}px`;
  const g = out.getContext("2d")!;
  g.fillStyle = "#000"; g.fillRect(0, 0, out.width, out.height);
  g.fillStyle = "#bbb"; g.font = `${13 * dpr}px system-ui`;
  g.fillText(`${sc.label} · ${colourVariant()}`, 8 * dpr, 18 * dpr);
  let t = 0, ev = 0;
  times.forEach((at, i) => {
    while (t < at - 1e-9) {
      while (ev < sc.events.length && sc.events[ev][0] <= t + 1e-9) sc.events[ev++][1]();
      advance(1 / 60); t += 1 / 60;
    }
    while (ev < sc.events.length && sc.events[ev][0] <= t + 1e-9) sc.events[ev++][1]();
    if (i === 0) { advance(1 / 60); t += 1 / 60; } // the first capture: the change has just begun
    const cx = (i % cols) * cw, cy = 26 * dpr + Math.floor(i / cols) * cellH;
    let y = cy;
    for (const v of rows) {
      g.drawImage(v.canvas, cx + (cw - v.canvas.width) / 2, y);
      y += v.canvas.height;
    }
    g.fillStyle = "#888"; g.font = `${11 * dpr}px ui-monospace, monospace`;
    g.fillText(`${at.toFixed(2)} s`, cx + 6 * dpr, y + 15 * dpr);
  });
  setManual(false);
  return out.toDataURL("image/png");
}

/** One look's sheet: idle → working, working → error and love in and out, one row each. */
function variantSheet(v: ColourVariant): string {
  const was = colourVariant();
  setVariant(v);
  const parts = ["idle-working", "working-error", "love-short"].map((n) => {
    sheet(n, true);
    const src = document.getElementById("sheet") as HTMLCanvasElement;
    const c = document.createElement("canvas");
    c.width = src.width; c.height = src.height;
    c.getContext("2d")!.drawImage(src, 0, 0);
    return c;
  });
  const out = document.getElementById("sheet") as HTMLCanvasElement;
  out.width = Math.max(...parts.map((c) => c.width));
  out.height = parts.reduce((h, c) => h + c.height, 0);
  out.style.width = `${out.width / dpr}px`;
  const g = out.getContext("2d")!;
  g.fillStyle = "#000"; g.fillRect(0, 0, out.width, out.height);
  let y = 0;
  for (const c of parts) { g.drawImage(c, 0, y); y += c.height; }
  setVariant(was);
  return out.toDataURL("image/png");
}

function setVariant(v: ColourVariant) {
  setColourVariant(v);
  variantSel.value = v;
}
const variantSel = document.getElementById("variant") as HTMLSelectElement;
for (const v of COLOUR_VARIANTS) variantSel.append(new Option(v, v));
variantSel.value = colourVariant();
variantSel.onchange = () => setVariant(variantSel.value as ColourVariant);

(window as unknown as { __colourPreview?: object }).__colourPreview = {
  setBotState, love, annoyed, rapid, sheet, variantSheet, setVariant, scripts: Object.keys(SCRIPTS), manual: setManual, advance, dpr,
};
