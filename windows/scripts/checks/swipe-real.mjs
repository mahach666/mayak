// core/page-swipe.ts on real touchpad wheel streams, recorded in WebView2 from a precision
// touchpad with the cursor over the island (fixtures/*.json: [timeMs, deltaX, deltaY, deltaMode]).
// After the fingers lift, the momentum keeps sending decaying deltas for 3–4 s, never 200 ms
// apart: a stroke made during it must switch, the momentum itself never.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { PageSwipe } from "../../src/core/page-swipe.ts";

/** The island's views are 640 px wide, plus the 14 px gap the slide keeps between two pages. */
const WIDTH = 654;

function load(name) {
  return JSON.parse(readFileSync(new URL(`./fixtures/${name}.json`, import.meta.url), "utf8"));
}

/** Replays a recording as the island does (quiet first, then the event); returns the switches. */
function replay(events) {
  const swipe = new PageSwipe();
  swipe.width = WIDTH;
  const t0 = events[0][0];
  const switches = [];
  let lastSign = 0;
  const take = (step, t, sign) => {
    if (step?.kind === "commit") switches.push({ t: (t - t0) / 1000, delta: step.delta, sign });
  };
  for (const [t, dx, dy, mode] of events) {
    take(swipe.quiet(t), t, lastSign);
    if (dx !== 0) lastSign = Math.sign(dx);
    take(swipe.wheel({ deltaX: dx, deltaY: dy, deltaMode: mode, timeStamp: t }), t, Math.sign(dx));
  }
  const end = events.at(-1)[0] + 10_000;
  take(swipe.quiet(end), end, lastSign);
  return switches;
}

const show = (s) => s.map((x) => `${x.t.toFixed(2)}s${x.delta > 0 ? "R" : "L"}`).join(" ");

// Labelled: 5 swipes right, 3 left, each started inside the momentum of the one before; then one
// slow short swipe right, which slides back.
{
  const s = replay(load("touchpad-labeled"));
  console.log(`labeled ${s.length}: ${show(s)}`);
  const expected = [
    [0.05, 1], [0.69, 1], [1.52, 1], [2.32, 1], [3.12, 1],
    [6.45, -1], [7.40, -1], [8.12, -1],
  ];
  assert.equal(s.length, expected.length, show(s));
  expected.forEach(([t, delta], i) => {
    assert.equal(s[i].delta, delta, `switch ${i + 1}: ${show(s)}`);
    assert.ok(Math.abs(s[i].t - t) <= 0.3, `switch ${i + 1} at ${s[i].t}s, expected ~${t}s`);
  });
  assert.equal(s.filter((x) => x.t >= 11.5).length, 0, "the slow short swipe does not switch");
}

// Free play, back and forth: never two switches within 250 ms (one stroke, one page), and each
// switch goes the way its stroke went.
{
  const s = replay(load("touchpad-free"));
  console.log(`free ${s.length}: ${show(s)}`);
  assert.ok(s.length >= 10, "the strokes made during the momentum switch");
  for (let i = 1; i < s.length; i++) assert.ok(s[i].t - s[i - 1].t >= 0.25, `two switches 250 ms apart: ${show(s)}`);
  for (const x of s) assert.equal(x.delta, x.sign, `switch at ${x.t}s against its stroke`);
}

console.log("swipe-real: ok");
