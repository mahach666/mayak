// Луч's sounds, synthesised: warm soft tones plus fire (crackle, the "fyr" of a flame, a hiss).
// Pure and deterministic (seeded noise, no clock, no Math.random): the same code always renders
// the same samples. scripts/gen-sounds.mjs writes them to sounds/<name>.wav, and
// scripts/checks/sounds.mjs checks the committed files still match this script byte for byte.
// render(name, sr) -> Float32Array (mono, -1..1), RMS matched to TARGET_RMS[name];
// toWav(samples, sr) -> the 16-bit PCM stereo WAV the island plays.

export const SR = 48000;
const TAU = Math.PI * 2;

// the old set's loudness per sound (RMS), so the new set sits at the same level under the volume slider
export const TARGET_RMS = {
  annoyed: 0.028, approval: 0.066, approve: 0.038, attach: 0.049, blip: 0.009, close: 0.014,
  dizzy: 0.056, error: 0.072, finish: 0.103, greet: 0.064, gulp: 0.051, hover: 0.004, love: 0.045,
  open: 0.018, peek: 0.018, pop: 0.012, proud: 0.061, question: 0.052, rate: 0.04, search: 0.026,
  send: 0.016, slap: 0.025, sleep: 0.021, think: 0.028, tick: 0.002, wink: 0.007, work: 0.016,
  yawn: 0.036,
};

// ── building blocks ───────────────────────────────────────────────────────────

function rng(seed) {
  let s = seed >>> 0 || 1;
  return () => { s ^= s << 13; s >>>= 0; s ^= s >>> 17; s ^= s << 5; s >>>= 0; return s / 4294967296; };
}

const note = (n) => 440 * Math.pow(2, (n - 69) / 12); // MIDI number -> Hz
const N = { C4: 60, D4: 62, E4: 64, F4: 65, G4: 67, A4: 69, B4: 71, C5: 72, D5: 74, E5: 76, F5: 77, G5: 79, A5: 81, B5: 83, C6: 84, D6: 86, E6: 88, G6: 91, A3: 57, G3: 55, E3: 52, C3: 48 };

class Buf {
  constructor(sec, sr) { this.sr = sr; this.d = new Float32Array(Math.ceil(sec * sr)); }
  add(at, src, gain = 1) {
    const o = Math.round(at * this.sr);
    for (let i = 0; i < src.length && o + i < this.d.length; i++) if (o + i >= 0) this.d[o + i] += src[i] * gain;
  }
}

/** A warm mallet/bell voice: a sine with a few soft partials, a soft attack and an exponential decay. */
function bell(sr, f, dur, { decay = 6, attack = 0.004, bright = 0.35, glide = 0, vib = 0, vibHz = 5.5, inharm = 0 } = {}) {
  const n = Math.ceil(dur * sr), out = new Float32Array(n);
  const parts = [[1, 1], [2.0 + inharm, bright * 0.5], [3.0 + inharm * 2, bright * 0.18], [4.2, bright * 0.06]];
  const ph = parts.map(() => 0);
  for (let i = 0; i < n; i++) {
    const t = i / sr, k = t / dur;
    const fi = f * Math.pow(2, glide * k / 12) * (1 + vib * Math.sin(TAU * vibHz * t));
    const env = Math.min(1, t / attack) * Math.exp(-decay * t) * Math.min(1, (dur - t) / 0.02);
    let v = 0;
    parts.forEach(([m, a], j) => { ph[j] += TAU * fi * m / sr; v += a * Math.sin(ph[j]) * Math.exp(-decay * (m - 1) * 0.6 * t); });
    out[i] = v * env;
  }
  return out;
}

/** A soft rounded tone (sine + a touch of second harmonic) with an attack/release shape. */
function tone(sr, f, dur, { attack = 0.02, release = 0.08, glide = 0, vib = 0, vibHz = 5, h2 = 0.15 } = {}) {
  const n = Math.ceil(dur * sr), out = new Float32Array(n);
  let ph = 0;
  for (let i = 0; i < n; i++) {
    const t = i / sr, k = t / dur;
    ph += TAU * f * Math.pow(2, glide * k / 12) * (1 + vib * Math.sin(TAU * vibHz * t)) / sr;
    const env = Math.min(1, t / attack) * Math.min(1, (dur - t) / release);
    out[i] = (Math.sin(ph) + h2 * Math.sin(2 * ph)) * env;
  }
  return out;
}

/** State-variable band/low/high pass, per-sample cutoff from fc(t). */
function svf(src, sr, fc, q = 0.7, mode = "band") {
  const out = new Float32Array(src.length);
  let low = 0, band = 0;
  for (let i = 0; i < src.length; i++) {
    const f = 2 * Math.sin(Math.PI * Math.min(0.45 * sr, typeof fc === "function" ? fc(i / sr) : fc) / sr);
    low += f * band;
    const high = src[i] - low - band / q;
    band += f * high;
    out[i] = mode === "band" ? band : mode === "low" ? low : high;
  }
  return out;
}

/** White noise, seeded. */
function noise(sr, dur, seed) {
  const r = rng(seed), out = new Float32Array(Math.ceil(dur * sr));
  for (let i = 0; i < out.length; i++) out[i] = r() * 2 - 1;
  return out;
}

/** The "fyr" of a flame: band-passed noise whose centre sweeps from f0 to f1, with an envelope. */
function whoosh(sr, dur, f0, f1, seed, { attack = 0.3, q = 1.2 } = {}) {
  const src = noise(sr, dur, seed);
  const out = svf(src, sr, (t) => f0 * Math.pow(f1 / f0, t / dur), q, "band");
  for (let i = 0; i < out.length; i++) {
    const k = i / out.length;
    // The fall term is above 1 before `attack`, which pulls the peak a little ahead of it:
    // that is the shape the set was tuned with, so it stays.
    out[i] *= Math.pow(Math.min(1, k / attack), 1.5) * Math.pow(Math.max(0, 1 - (k - attack) / (1 - attack)), 1.6);
  }
  return out;
}

/** Fire crackle: sparse short pops of filtered noise, `rate` per second, fading over `dur`. */
function crackle(sr, dur, rate, seed, { bright = 2500, fade = 1.5 } = {}) {
  const r = rng(seed), out = new Float32Array(Math.ceil(dur * sr));
  let t = r() * 0.02;
  while (t < dur) {
    const len = 0.002 + r() * 0.006, amp = (0.4 + 0.6 * r()) * Math.exp(-fade * t / dur * 2);
    const o = Math.floor(t * sr), n = Math.floor(len * sr);
    for (let i = 0; i < n && o + i < out.length; i++) out[o + i] += (r() * 2 - 1) * amp * Math.exp(-i / (n * 0.3));
    t += -Math.log(1 - r()) / rate;
  }
  return svf(out, sr, bright, 0.8, "band").map((v, i) => v + out[i] * 0.15);
}

/** A hiss that dies away: high-passed noise with a slow fall (a flame going out). */
function hiss(sr, dur, seed, { attack = 0.03, fc = 3500 } = {}) {
  const out = svf(noise(sr, dur, seed), sr, fc, 0.7, "high");
  for (let i = 0; i < out.length; i++) { const t = i / sr; out[i] *= Math.min(1, t / attack) * Math.pow(1 - t / dur, 2.2); }
  return out;
}

/** A soft low thump (an impact), a sine dropping in pitch. */
function thump(sr, dur, f0, f1) {
  const n = Math.ceil(dur * sr), out = new Float32Array(n);
  let ph = 0;
  for (let i = 0; i < n; i++) { const t = i / sr; ph += TAU * (f1 + (f0 - f1) * Math.exp(-t * 30)) / sr; out[i] = Math.sin(ph) * Math.exp(-t * 14) * Math.min(1, t / 0.002); }
  return out;
}

const arp = (b, sr, at, notes, step, opts, gain = 1) => notes.forEach((m, i) => b.add(at + i * step, bell(sr, note(m), opts.dur ?? 0.6, opts), gain * (opts.gains?.[i] ?? 1)));

// ── the set ───────────────────────────────────────────────────────────────────
// Per sound: what it says (the island calls it at that moment) and how it is made.

export const SOUNDS = {
  hover: { what: "pointer on a button", dur: 0.08, make: (b, sr) => { b.add(0, bell(sr, note(N.A5), 0.07, { decay: 45, bright: 0.2 }), 1); b.add(0, crackle(sr, 0.03, 120, 3, { bright: 5000 }), 0.3); } },
  tick: { what: "countdown tick", dur: 0.07, make: (b, sr) => { b.add(0, crackle(sr, 0.05, 40, 11, { bright: 3200 }), 1); b.add(0, bell(sr, note(N.E6), 0.04, { decay: 80, bright: 0 }), 0.4); } },
  blip: { what: "click, toggle", dur: 0.1, make: (b, sr) => b.add(0, bell(sr, note(N.E5), 0.1, { decay: 30, bright: 0.3 })) },
  peek: { what: "the island peeks out", dur: 0.45, make: (b, sr) => { b.add(0, crackle(sr, 0.12, 60, 5), 0.5); arp(b, sr, 0.03, [N.E5, N.A5], 0.09, { dur: 0.32, decay: 9, bright: 0.25 }); } },
  open: { what: "the island opens", dur: 0.5, make: (b, sr) => { b.add(0, whoosh(sr, 0.42, 300, 1800, 21, { attack: 0.45 }), 0.9); arp(b, sr, 0.12, [N.C5, N.G5], 0.08, { dur: 0.36, decay: 8, bright: 0.25 }, 0.6); } },
  close: { what: "the island closes", dur: 0.43, make: (b, sr) => { b.add(0, whoosh(sr, 0.36, 1500, 280, 22, { attack: 0.2 }), 0.9); arp(b, sr, 0.04, [N.G5, N.C5], 0.08, { dur: 0.3, decay: 10, bright: 0.2 }, 0.5); } },
  greet: { what: "hello, Луч appears", dur: 0.75, make: (b, sr) => { b.add(0, crackle(sr, 0.25, 70, 7), 0.6); b.add(0.02, whoosh(sr, 0.3, 400, 2200, 8), 0.5); arp(b, sr, 0.1, [N.C5, N.E5, N.G5], 0.1, { dur: 0.5, decay: 6, bright: 0.35 }); } },
  work: { what: "an agent starts working", dur: 0.38, make: (b, sr) => { b.add(0, crackle(sr, 0.3, 45, 12), 0.8); b.add(0, bell(sr, note(N.G4), 0.36, { decay: 9, bright: 0.2 }), 0.7); } },
  think: { what: "an agent is thinking", dur: 0.44, make: (b, sr) => { b.add(0, tone(sr, note(N.E5), 0.18, { attack: 0.03, release: 0.06, vib: 0.006 }), 0.6); b.add(0.2, tone(sr, note(N.G5), 0.22, { attack: 0.03, release: 0.1, vib: 0.006 }), 0.6); } },
  search: { what: "an agent is searching", dur: 0.46, make: (b, sr) => { b.add(0, whoosh(sr, 0.4, 800, 2600, 31, { attack: 0.5, q: 2 }), 0.6); arp(b, sr, 0.02, [N.C5, N.E5, N.G5, N.C6], 0.07, { dur: 0.2, decay: 16, bright: 0.2 }, 0.6); } },
  approval: { what: "a permission is needed (loud, it calls)", dur: 0.88, make: (b, sr) => { b.add(0, whoosh(sr, 0.35, 500, 2500, 41, { attack: 0.25 }), 0.5); [0, 0.22].forEach((at) => { b.add(at, bell(sr, note(N.A5), 0.5, { decay: 7, bright: 0.5 })); b.add(at, bell(sr, note(N.E5), 0.5, { decay: 7, bright: 0.3 }), 0.6); }); b.add(0.05, crackle(sr, 0.5, 30, 42), 0.4); } },
  question: { what: "an agent asks", dur: 0.7, make: (b, sr) => { b.add(0, bell(sr, note(N.G4), 0.3, { decay: 8, bright: 0.3 })); b.add(0.16, bell(sr, note(N.D5), 0.52, { decay: 5, bright: 0.35, glide: 1.2 })); } },
  approve: { what: "permission given", dur: 0.6, make: (b, sr) => { b.add(0, whoosh(sr, 0.3, 600, 2400, 51, { attack: 0.3 }), 0.6); arp(b, sr, 0.03, [N.E5, N.A5], 0.09, { dur: 0.48, decay: 7, bright: 0.4 }); } },
  finish: { what: "an agent finished", dur: 1.16, make: (b, sr) => { arp(b, sr, 0, [N.C5, N.E5, N.G5, N.C6], 0.11, { dur: 0.8, decay: 4.5, bright: 0.35, gains: [0.8, 0.85, 0.9, 1] }); b.add(0.3, crackle(sr, 0.8, 18, 61), 0.5); } },
  error: { what: "error", dur: 0.84, make: (b, sr) => { b.add(0, bell(sr, note(N.E4), 0.4, { decay: 6, bright: 0.25 })); b.add(0.15, bell(sr, note(N.C4), 0.6, { decay: 5, bright: 0.25, glide: -0.8 })); b.add(0.12, hiss(sr, 0.7, 71, { fc: 3000 }), 0.35); } },
  rate: { what: "hit a rate limit", dur: 0.67, make: (b, sr) => { arp(b, sr, 0, [N.G4, N.E4, N.C4], 0.13, { dur: 0.4, decay: 7, bright: 0.15 }); b.add(0.2, hiss(sr, 0.45, 81, { fc: 2500 }), 0.15); } },
  sleep: { what: "falls asleep", dur: 0.45, make: (b, sr) => { b.add(0, tone(sr, note(N.E4), 0.44, { attack: 0.08, release: 0.25, glide: -3, h2: 0.05 }), 0.6); b.add(0, svf(noise(sr, 0.44, 91), sr, 700, 0.6, "low").map((v, i) => v * Math.sin(Math.PI * i / (0.44 * sr))), 0.5); } },
  dizzy: { what: "dizzy (three slaps)", dur: 0.98, make: (b, sr) => { b.add(0, bell(sr, note(N.A4), 0.95, { decay: 2.8, bright: 0.3, vib: 0.04, vibHz: 7 })); b.add(0, bell(sr, note(N.E5), 0.95, { decay: 3, bright: 0.2, vib: 0.04, vibHz: 5.3 }), 0.5); b.add(0, whoosh(sr, 0.9, 1800, 600, 101, { attack: 0.3, q: 3 }), 0.3); } },
  slap: { what: "a slap on Луч", dur: 0.6, make: (b, sr) => { b.add(0, thump(sr, 0.3, 260, 90), 0.9); b.add(0, crackle(sr, 0.45, 90, 111, { bright: 2200 }), 0.9); b.add(0.01, whoosh(sr, 0.4, 2000, 500, 112, { attack: 0.08 }), 0.5); } },
  annoyed: { what: "annoyed", dur: 0.59, make: (b, sr) => { b.add(0, tone(sr, note(N.C3) * 1.5, 0.35, { attack: 0.02, release: 0.1, vib: 0.03, vibHz: 18, h2: 0.6, glide: -1 }), 0.5); b.add(0.15, hiss(sr, 0.42, 121, { fc: 2800 }), 0.5); } },
  love: { what: "hearts", dur: 0.79, make: (b, sr) => { arp(b, sr, 0, [N.G5, N.C6], 0.14, { dur: 0.6, decay: 5, bright: 0.3, vib: 0.004 }); b.add(0.25, crackle(sr, 0.5, 25, 131, { bright: 4500 }), 0.4); } },
  pop: { what: "a bubble, something appears", dur: 0.38, make: (b, sr) => { b.add(0, tone(sr, note(N.C5), 0.12, { attack: 0.003, release: 0.06, glide: 9, h2: 0 })); b.add(0, crackle(sr, 0.05, 80, 141), 0.3); } },
  proud: { what: "proud", dur: 0.86, make: (b, sr) => { arp(b, sr, 0, [N.G4, N.C5, N.E5], 0.08, { dur: 0.3, decay: 9, bright: 0.4 }); b.add(0.26, bell(sr, note(N.G5), 0.58, { decay: 4, bright: 0.45 })); b.add(0.26, bell(sr, note(N.C5), 0.58, { decay: 4, bright: 0.2 }), 0.5); b.add(0.2, whoosh(sr, 0.4, 600, 2600, 151), 0.4); } },
  wink: { what: "winks", dur: 0.21, make: (b, sr) => { b.add(0, bell(sr, note(N.E6), 0.2, { decay: 14, bright: 0.2, glide: 2 })); b.add(0, crackle(sr, 0.06, 80, 161, { bright: 5000 }), 0.3); } },
  yawn: { what: "yawns", dur: 0.73, make: (b, sr) => { const v = tone(sr, note(N.A4), 0.72, { attack: 0.1, release: 0.3, glide: -7, vib: 0.01, h2: 0.5 }); b.add(0, svf(v, sr, (t) => 900 + 700 * Math.sin(Math.PI * t / 0.72), 2.5, "band"), 2.5); b.add(0, svf(noise(sr, 0.72, 171), sr, 900, 0.6, "low").map((x, i) => x * Math.sin(Math.PI * i / (0.72 * sr))), 0.3); } },
  attach: { what: "file accepted", dur: 0.9, make: (b, sr) => { b.add(0, whoosh(sr, 0.35, 2500, 500, 181, { attack: 0.5 }), 0.7); b.add(0.3, whoosh(sr, 0.45, 400, 2400, 182, { attack: 0.2 }), 0.7); b.add(0.32, crackle(sr, 0.55, 60, 183), 0.7); b.add(0.34, bell(sr, note(N.A5), 0.5, { decay: 6, bright: 0.3 }), 0.6); } },
  gulp: { what: "Луч swallows the file", dur: 0.75, make: (b, sr) => { b.add(0, tone(sr, note(N.G4), 0.22, { attack: 0.01, release: 0.06, glide: -9, h2: 0.3 })); b.add(0.18, whoosh(sr, 0.45, 350, 1800, 191, { attack: 0.15 }), 0.8); b.add(0.2, crackle(sr, 0.5, 70, 192), 0.8); } },
  send: { what: "message sent", dur: 0.48, make: (b, sr) => { b.add(0, whoosh(sr, 0.4, 500, 3000, 201, { attack: 0.55 }), 0.9); b.add(0.25, bell(sr, note(N.C6), 0.22, { decay: 14, bright: 0.2 }), 0.4); } },
};

/** Renders a sound: mono, RMS matched to the old set, peaks soft-clipped to about 0.95. */
export function render(name, sr = SR) {
  const s = Object.hasOwn(SOUNDS, name) ? SOUNDS[name] : null;
  if (!s) throw new Error(`unknown sound: ${name}`);
  const b = new Buf(s.dur, sr);
  s.make(b, sr);
  const d = b.d;
  // short fades so nothing clicks at the ends
  const fi = Math.floor(0.002 * sr), fo = Math.floor(0.015 * sr);
  for (let i = 0; i < fi && i < d.length; i++) d[i] *= i / fi;
  for (let i = 0; i < fo && i < d.length; i++) d[d.length - 1 - i] *= i / fo;
  let ss = 0;
  for (const v of d) ss += v * v;
  const rms = Math.sqrt(ss / d.length) || 1;
  const g = TARGET_RMS[name] / rms;
  for (let i = 0; i < d.length; i++) d[i] = Math.tanh(d[i] * g * 1.05) / 1.05;
  return d;
}

export const NAMES = Object.keys(SOUNDS);

/**
 * A WAV file of `samples` (mono, -1..1): 16-bit PCM, two identical channels, the format of the
 * set it replaces. The same samples always give the same bytes.
 */
export function toWav(samples, sr = SR) {
  const channels = 2, bytesPerSample = 2;
  const dataBytes = samples.length * channels * bytesPerSample;
  const out = Buffer.alloc(44 + dataBytes);
  out.write("RIFF", 0, "ascii");
  out.writeUInt32LE(36 + dataBytes, 4);
  out.write("WAVE", 8, "ascii");
  out.write("fmt ", 12, "ascii");
  out.writeUInt32LE(16, 16); // fmt chunk size
  out.writeUInt16LE(1, 20); // PCM
  out.writeUInt16LE(channels, 22);
  out.writeUInt32LE(sr, 24);
  out.writeUInt32LE(sr * channels * bytesPerSample, 28); // byte rate
  out.writeUInt16LE(channels * bytesPerSample, 32); // block align
  out.writeUInt16LE(bytesPerSample * 8, 34);
  out.write("data", 36, "ascii");
  out.writeUInt32LE(dataBytes, 40);
  for (let i = 0; i < samples.length; i++) {
    const v = Math.round(Math.max(-1, Math.min(1, samples[i])) * 32767);
    out.writeInt16LE(v, 44 + i * 4);
    out.writeInt16LE(v, 46 + i * 4);
  }
  return out;
}
