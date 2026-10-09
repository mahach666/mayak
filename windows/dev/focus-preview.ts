// Dev harness: the real Луч (engine, shader, eyes) in the pomodoro phases, at the open island's size, at
// the compact bar's size and enlarged, with music on or off and an automatic switch between phases to
// watch the transitions. Not part of the app bundle.

import { BotEngine } from "../src/luch/engine";
import { activityFor } from "../src/island/activity";
import type { Phase } from "../src/island/pomodoro";
import { fireFps } from "../src/island/pace";
import type { BotStateName } from "../src/core/layout";

type PhaseChoice = Phase | "none";

/** The island's canvas: Луч's box `w` (layout diameter / 0.6) plus the particle overhang (island.ts). */
const OVERHANG = 40;
const WORK_RGB = [167 / 255, 139 / 255, 250 / 255] as const;
const BREAK_RGB = [52 / 255, 211 / 255, 153 / 255] as const;

interface View { engine: BotEngine; canvas: HTMLCanvasElement; w: number; compact: boolean; minutes: boolean }

const stage = document.getElementById("stage")!;
const dpr = Math.min(2, window.devicePixelRatio || 1);

function view(label: string, w: number, compact: boolean, minutes: boolean): View {
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
  return { engine, canvas, w, compact, minutes };
}

// the home page's Луч is 58 px (layout.ts overview, box 58 / 0.6), the compact bar's 20 px (box 20 / 0.6)
const views = [
  view("enlarged ×2.5", Math.round((58 / 0.6) * 2.5), false, true),
  view("open island", Math.round(58 / 0.6), false, true),
  view("compact", Math.round(20 / 0.6), true, false),
];

let phase: PhaseChoice = "none";
let music = false;
let state: BotStateName = "idle";
const ORDER: PhaseChoice[] = ["none", "work", "short", "long"];

function apply() {
  const look = activityFor(phase === "none" ? null : phase, music);
  for (const v of views) {
    v.engine.activity = phase === "none" && !music ? null : {
      ...look,
      minutes: phase !== "none" && v.minutes ? (phase === "work" ? 25 : phase === "short" ? 5 : 15) : null,
      minutesColor: phase === "work" ? WORK_RGB : BREAK_RGB,
    };
  }
  for (const b of document.querySelectorAll<HTMLButtonElement>("button[data-phase]")) b.classList.toggle("on", b.dataset.phase === phase);
  const mb = document.getElementById("music")!;
  mb.textContent = `Music: ${music ? "on" : "off"}`;
  mb.classList.toggle("on", music);
}

function setPhase(p: PhaseChoice) { phase = p; apply(); }
function setMusic(on: boolean) { music = on; apply(); }
function setBotState(s: BotStateName) { state = s; for (const v of views) v.engine.setState(s); }

for (const b of document.querySelectorAll<HTMLButtonElement>("button[data-phase]")) b.onclick = () => setPhase(b.dataset.phase as PhaseChoice);
document.getElementById("music")!.onclick = () => setMusic(!music);
const stateSel = document.getElementById("state") as HTMLSelectElement;
stateSel.onchange = () => setBotState(stateSel.value as BotStateName);
const cycle = document.getElementById("cycle") as HTMLInputElement;
const every = document.getElementById("every") as HTMLInputElement;
const pace = document.getElementById("pace") as HTMLInputElement;
const status = document.getElementById("status")!;
apply();

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
  if (cycle.checked && nowMs - lastSwitch >= Math.max(1, Number(every.value) || 6) * 1000) {
    lastSwitch = nowMs;
    setPhase(ORDER[(ORDER.indexOf(phase) + 1) % ORDER.length]);
  } else if (!cycle.checked) lastSwitch = nowMs;
  views.forEach((v, i) => {
    // the island draws the fire alone at fireFps (pace.ts); the preview can follow it or draw every frame
    const fps = fireFps(v.compact, v.engine.dancing);
    if (pace.checked && nowMs - last[i] < 1000 / fps - 2) return;
    const dt = Math.min(0.1, (nowMs - (last[i] || nowMs)) / 1000);
    last[i] = nowMs;
    drawView(v, dt);
  });
  status.textContent = `${phase} · music ${music ? "on" : "off"} · ${state} · ${(1000 / Math.max(1, nowMs - prev)).toFixed(0)} rAF fps`;
  prev = nowMs;
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

// for scripted captures from the devtools: manual(true) freezes the clock, advance(dt) steps every view
(window as unknown as { __focusPreview?: object }).__focusPreview = {
  setPhase, setMusic, setBotState, canvases: views.map((v) => v.canvas), dpr,
  manual(on: boolean) {
    manual = on;
    if (on) { virtualMs = realNow(); performance.now = () => virtualMs; } else performance.now = realNow;
  },
  advance(dt: number) {
    virtualMs += dt * 1000;
    for (const v of views) drawView(v, dt);
  },
};
