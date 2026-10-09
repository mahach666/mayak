// Dev harness: plays the launch greeting in a plain browser with a time slider, pause, frame steps
// and an interrupt, so the choreography can be judged frame by frame. Not part of the app bundle.

import { drawGreeting } from "../src/luch/greeting";

const c = document.getElementById("stage") as HTMLCanvasElement;
const dpr = Math.min(2, window.devicePixelRatio || 1);
c.width = 640 * dpr; c.height = 150 * dpr;
const x = c.getContext("2d")!;
const slider = document.getElementById("t") as HTMLInputElement;
const clock = document.getElementById("clock")!;
const playBtn = document.getElementById("play") as HTMLButtonElement;
let playing = true, start = performance.now() / 1000, tc = Number.POSITIVE_INFINITY, holdAt = Number.POSITIVE_INFINITY, frozen = 0;

const pause = () => { playing = false; playBtn.textContent = "Play"; };
playBtn.onclick = () => {
  playing = !playing; playBtn.textContent = playing ? "Pause" : "Play";
  if (playing) start = performance.now() / 1000 - frozen;
};
document.getElementById("restart")!.onclick = () => {
  start = performance.now() / 1000; tc = holdAt = Number.POSITIVE_INFINITY; playing = true; playBtn.textContent = "Pause";
};
const step = (d: number) => { pause(); frozen = Math.max(0, frozen + d); slider.value = String(frozen); };
document.getElementById("back")!.onclick = () => step(-1 / 60);
document.getElementById("fwd")!.onclick = () => step(1 / 60);
const now = () => (playing ? performance.now() / 1000 - start : frozen);
document.getElementById("cut")!.onclick = () => { tc = now(); };
// the cursor entering the island: the greeting stays on the card until "Interrupt now" (or Restart)
document.getElementById("hold")!.onclick = () => { if (!Number.isFinite(holdAt)) holdAt = now(); };
slider.oninput = () => { pause(); frozen = Number(slider.value); };

function frame() {
  // a held greeting does not loop: it rests until interrupted
  const raw = performance.now() / 1000 - start;
  const t = playing ? (Number.isFinite(holdAt) && !Number.isFinite(tc) ? raw : raw % 6.2) : frozen;
  if (playing) {
    if (t < frozen) tc = holdAt = Number.POSITIVE_INFINITY; // a new loop starts uninterrupted
    frozen = t; slider.value = String(t);
  }
  clock.textContent = `${t.toFixed(2)} s`;
  x.setTransform(dpr, 0, 0, dpr, 0, 0);
  drawGreeting(x, t, tc, holdAt);
  requestAnimationFrame(frame);
}
frame();
