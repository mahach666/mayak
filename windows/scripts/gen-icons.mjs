// Draws Луч into the PNG/ICO set Tauri needs. No dependencies: the icons are
// rasterised here (a CPU port of the fire shader, src/luch/fire.frag.glsl, at one
// fixed moment) and encoded with node:zlib, so the app icon stays "drawn in code"
// like the character itself.
//
//   node scripts/gen-icons.mjs              writes src-tauri/icons
//   node scripts/gen-icons.mjs --out <dir>  writes <dir> instead, plus each ICO entry as
//                                           ico-<size>.png for scripts/icon-review.html

import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const outAt = process.argv.indexOf("--out");
if (outAt >= 0 && !process.argv[outAt + 1]) throw new Error("--out needs a directory");
const OUT = outAt >= 0
  ? resolve(process.argv[outAt + 1])
  : join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "icons");

// ── The fire field (port of fire.frag.glsl) ───────────────────────────────────

const hex01 = (h) => { const v = parseInt(h.slice(1), 16); return [((v >> 16) & 255) / 255, ((v >> 8) & 255) / 255, (v & 255) / 255]; };
// src/luch/palette.ts IDLE_FIRE and the engine's EYE_INK
const DEEP = hex01("#e0380a"), MID = hex01("#ff7410"), LIGHT = hex01("#ffbc34"), CORE = hex01("#fff0c2");
const INK = hex01("#2A170B");
const SQUARE = hex01("#121216");

// The pose: a fixed time, the idle look (live, tall, bright, grow, coreA = 1), no hands, no ignition.
const T = 1.7;

const fract = (x) => x - Math.floor(x);
const mix = (a, b, k) => a + (b - a) * k;
const clamp = (x, a, b) => Math.min(b, Math.max(a, x));
const smoothstep = (e0, e1, x) => { const k = clamp((x - e0) / (e1 - e0), 0, 1); return k * k * (3 - 2 * k); };
const mix3 = (a, b, k) => [mix(a[0], b[0], k), mix(a[1], b[1], k), mix(a[2], b[2], k)];

const hash = (x, y) => fract(Math.sin(x * 127.1 + y * 311.7) * 43758.5453);
function noise(x, y) {
  const ix = Math.floor(x), iy = Math.floor(y), fx = x - ix, fy = y - iy;
  const ux = fx * fx * (3 - 2 * fx), uy = fy * fy * (3 - 2 * fy);
  return mix(mix(hash(ix, iy), hash(ix + 1, iy), ux), mix(hash(ix, iy + 1), hash(ix + 1, iy + 1), ux), uy);
}
function fbm3(x, y) {
  let v = 0, a = 0.5;
  for (let i = 0; i < 3; i++) { v += a * noise(x, y); x = x * 2.02 + 1.7; y = y * 2.02 + 9.2; a *= 0.5; }
  return v;
}
function smax(a, b, k) { const h = clamp(0.5 + 0.5 * (a - b) / k, 0, 1); return mix(b, a, h) + k * h * (1 - h); }
function rot(x, y, a) { const c = Math.cos(a), s = Math.sin(a); return [c * x - s * y, s * x + c * y]; }
function drop(qx, qy, R, B, tall, taper) {
  if (qy < -B) return -(Math.abs(qy) - B) * 4 - Math.abs(qx) / R;
  if (qy > tall) return -(qy - tall) * 3 - Math.abs(qx) / R;
  const w = qy < 0 ? R * Math.sqrt(1 - (qy * qy) / (B * B)) : R * Math.pow(1 - qy / tall, taper);
  return (w - Math.abs(qx)) / R;
}
function rbox(px, py, bx, by, r) {
  const dx = Math.abs(px) - bx + r, dy = Math.abs(py) - by + r;
  return Math.hypot(Math.max(dx, 0), Math.max(dy, 0)) + Math.min(Math.max(dx, dy), 0) - r;
}
function lick(px, py, atx, aty, lean, R, tall, curl, t, seed) {
  let [qx, qy] = rot(px - atx, py - aty, lean);
  const y = Math.max(qy, 0);
  qx -= curl * Math.pow(Math.max(y / tall - 0.45, 0), 2) * tall + 0.035 * Math.sin(qy * 6 - t * 2.6 + seed) * y / tall;
  return drop(qx, qy, R, R * 0.7, tall * (0.9 + 0.1 * Math.sin(t * 2.3 + seed)), 0.9);
}

/**
 * The shader's main() at box point (px, py), -1…1, y up. Returns [r, g, b, a] premultiplied, as
 * gl_FragColor is. `glow: false` leaves out the halo, so `a` is the flame's own coverage.
 */
function fieldColor(px, py, { flares, streaks, glow: withGlow }) {
  const t = T;
  const edge = smoothstep(1, 0.6, Math.hypot(px, py));
  const p = [px, py]; // uGrow = 1: p = p0

  const h = clamp((p[1] + 0.44) / 1.1, 0, 1);
  const wx = fbm3(p[0] * 1.8, p[1] * 1.4 - t * 1.3) - 0.5;
  const wy = fbm3(p[0] * 1.8 + 4.1, p[1] * 1.4 - t * 1.3 + 2.7) - 0.5;
  const wk = 0.015 + 0.16 * h * h;
  const qx = p[0] + wx * wk, qy = p[1] + wy * wk;

  const qbx = (qx - 0.01) / 0.44, qby = (qy + 0.11) / 0.33;
  const fb = (1 - Math.pow(Math.pow(Math.abs(qbx), 2.1) + Math.pow(Math.abs(qby), 2), 1 / 2.05)) * 1.25;

  const ft = lick(qx, qy, 0.02, 0.06, -0.16, 0.34, 0.64, -1.6, t, 0);
  // the two side licks (fr, fl) are computed but unused in the shader's colour and alpha
  const f = smax(fb, ft, 0.1);
  const inner = lick(qx, qy, -0.02, -0.02, -0.18, 0.25, 0.56, -1.5, t + 0.25, 0.4);
  const calm = f;

  const er = fbm3(p[0] * 2.6, p[1] * 1.9 - t * 1.8);
  let fire = f - (er - 0.45) * (0.02 + 0.25 * h * h);

  for (let i = 0; i < 3 && flares > 0; i++) {
    const ph = fract(t * (0.2 + 0.05 * i) + hash(i, 7));
    const tip = i === 0 ? [-0.5, 0.1] : i === 1 ? [0.36, 0.24] : [-0.12, 0.62];
    const side = i === 1 ? 1 : -1;
    const cx = tip[0] + side * 0.05 * ph + 0.02 * Math.sin(t * 2 + i * 2), cy = tip[1] + 0.02 + ph * 0.28;
    const s = (1 - ph) * (i === 2 ? 0.7 : 1);
    let [fx, fy] = rot(p[0] - cx, p[1] - cy, -side * 0.35);
    fx /= Math.max(s, 0.001); fy /= Math.max(s, 0.001);
    fx += side * 1.2 * Math.pow(Math.max(fy, 0) / 0.1, 2) * 0.02;
    const fl2 = drop(fx, fy, 0.045, 0.035, 0.1, 1.2);
    fire = mix(fire, smax(fire, fl2 * 0.95, 0.06 * (1 - ph)), flares * smoothstep(0, 0.12, ph));
  }

  let col = mix3(DEEP, MID, smoothstep(0, 0.05, fire));
  const lightZone = Math.max(Math.max(smoothstep(0.22, 0.42, fb), smoothstep(0.12, 0.3, ft)), smoothstep(0, 0.14, inner));
  col = mix3(col, LIGHT, lightZone * smoothstep(0.03, 0.1, fire));
  col = mix3(col, mix3(LIGHT, CORE, 0.5), smoothstep(0.5, 0.8, Math.max(fb, inner * 0.8)));
  const streak = noise(p[0] * 10 + 0.6 * Math.sin(p[1] * 3), p[1] * 1.4 - t * 0.6) - 0.5;
  const sk = 1 + 0.07 * streaks * streak * smoothstep(0.05, 0.3, fire);
  col = col.map((c) => c * sk);
  const a = smoothstep(-0.008, 0.012, fire);

  const sd = rbox(p[0] - 0.03, p[1] + 0.1, 0.32, 0.22, 0.2);
  col = mix3(col, mix3(LIGHT, CORE, 0.55), smoothstep(0.07, 0, sd) * 0.6);
  const core = smoothstep(0.015, -0.025, sd);
  col = mix3(col, mix3(CORE, mix3(CORE, LIGHT, 0.4), smoothstep(-0.09, 0, sd)), core);

  if (!withGlow) return [col[0] * a, col[1] * a, col[2] * a, a];
  const outside = Math.max(0, -calm);
  const glow = (0.32 * Math.exp(-outside * 4.5) + 0.07 * Math.exp(-outside * 1.4)) * (1 - a) * edge;
  const gc = mix3(MID, DEEP, 0.4);
  return [col[0] * a + gc[0] * glow, col[1] * a + gc[1] * glow, col[2] * a + gc[2] * glow, a + glow * 0.85];
}

// The G4 eyes as the engine draws them for the big Луч (src/luch/render.ts LUCH, luchEyeSize:
// 1.7 × the G4 measures): upright pills in box units, y up.
const EYE = { cx: 0.09, cy: -0.095, step: 0.093, w: 0.022 * 1.7, h: 0.04 * 1.7 };

/**
 * The two eyes in icon px for a box scale of `k` px per box unit, the box point (bcx, bcy) at the icon
 * centre `half`. Below 2 px wide they would blur into a smudge: like the engine's mini floor, they are
 * held at 2 × 4 px and put on whole pixels.
 */
function eyesAt(k, bcx, bcy, half) {
  let w = EYE.w * k, h = EYE.h * k;
  const snap = w < 2;
  if (snap) { w = 2; h = 4; }
  const at = (v) => (snap ? Math.round(v) : v);
  const cy = at(half - (EYE.cy - bcy) * k);
  const cxs = [-1, 1].map((sd) => at(half + (EYE.cx + sd * EYE.step - bcx) * k));
  const r = w / 2, straight = h / 2 - r;
  return (x, y) => cxs.some((cx) => {
    const dx = Math.abs(x - cx), dy = Math.max(Math.abs(y - cy) - straight, 0);
    return dx * dx + dy * dy <= r * r;
  });
}

/** The flame's own extent (a > 0.5) in box units: what the icons fit to their size. */
function flameBounds(opts) {
  const N = 400;
  let x0 = 1, x1 = -1, y0 = 1, y1 = -1;
  for (let j = 0; j < N; j++) {
    for (let i = 0; i < N; i++) {
      const px = -1 + (2 * (i + 0.5)) / N, py = -1 + (2 * (j + 0.5)) / N;
      if (fieldColor(px, py, { ...opts, glow: false })[3] <= 0.5) continue;
      x0 = Math.min(x0, px); x1 = Math.max(x1, px); y0 = Math.min(y0, py); y1 = Math.max(y1, py);
    }
  }
  return { cx: (x0 + x1) / 2, cy: (y0 + y1) / 2, w: x1 - x0, h: y1 - y0 };
}

const SS = 4; // supersampling factor

// App icon: the full fire (glow and streaks, no flares: a clean still) on a dark rounded square.
const APP = { flares: 0, streaks: 1, glow: true };
// Tray: the silhouette only, no glow, no streaks.
const TRAY = { flares: 0, streaks: 0, glow: false };
let appBounds, trayBounds;

function renderApp(size) {
  appBounds ??= flameBounds(APP);
  const px = new Uint8Array(size * size * 4);
  const R = size * 0.22, half = size / 2;
  // Луч fills 80 % of the square; box units → px
  const k = (0.8 * size) / Math.max(appBounds.w, appBounds.h);
  const inEye = eyesAt(k, appBounds.cx, appBounds.cy, half);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let r = 0, g = 0, b = 0, a = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const ux = x + (sx + 0.5) / SS, uy = y + (sy + 0.5) / SS;
          if (rbox(ux - half, uy - half, half, half, R) > 0) continue;
          const bx = appBounds.cx + (ux - half) / k, by = appBounds.cy - (uy - half) / k;
          let c;
          if (inEye(ux, uy)) c = [...INK, 1];
          else {
            const f = fieldColor(bx, by, APP);
            c = [f[0] + SQUARE[0] * (1 - f[3]), f[1] + SQUARE[1] * (1 - f[3]), f[2] + SQUARE[2] * (1 - f[3]), 1];
          }
          r += Math.min(1, c[0]); g += Math.min(1, c[1]); b += Math.min(1, c[2]); a += 1;
        }
      }
      writePixel(px, (y * size + x) * 4, r, g, b, a, SS * SS);
    }
  }
  return px;
}

/**
 * Tray: the flame's silhouette filled with the mid colour, a darker rim and two ink dots, snapped
 * to the pixel grid. Windows draws the tray from the ICO's 32 px entry shrunk to 16, so every
 * measure is in 16 px units and scales with the size: the 32 px icon is the 16 px one at 2×.
 */
const RIM = DEEP.map((c) => c * 0.62);
function renderTray(size) {
  trayBounds ??= flameBounds(TRAY);
  const u = size / 16;
  const k = size / Math.max(trayBounds.w, trayBounds.h);
  const N = size * SS;
  const mask = new Uint8Array(N * N);
  for (let j = 0; j < N; j++) {
    for (let i = 0; i < N; i++) {
      const bx = trayBounds.cx + ((i + 0.5) / SS - size / 2) / k, by = trayBounds.cy - ((j + 0.5) / SS - size / 2) / k;
      mask[j * N + i] = fieldColor(bx, by, TRAY)[3] > 0.5 ? 1 : 0;
    }
  }
  // the fill is the silhouette eroded by the rim width (a disk), at the supersampled resolution
  const rr = u * SS, ri = Math.ceil(rr);
  const fill = new Uint8Array(N * N);
  for (let j = 0; j < N; j++) {
    for (let i = 0; i < N; i++) {
      if (!mask[j * N + i]) continue;
      let all = 1;
      for (let dj = -ri; dj <= ri && all; dj++) {
        for (let di = -ri; di <= ri; di++) {
          if (di * di + dj * dj > rr * rr) continue;
          const ii = i + di, jj = j + dj;
          if (ii < 0 || jj < 0 || ii >= N || jj >= N || !mask[jj * N + ii]) { all = 0; break; }
        }
      }
      fill[j * N + i] = all;
    }
  }
  // the eyes: the G4 centres, a little wider apart so the two dots never merge, as whole pixels
  const ew = Math.round(1 * u), eh = Math.round(2 * u), gap = Math.round(2 * u);
  const ecx = size / 2 + (EYE.cx - trayBounds.cx) * k, ecy = size / 2 - (EYE.cy - trayBounds.cy) * k;
  const eyeL = Math.round(ecx - gap / 2 - ew), eyeT = Math.round(ecy - eh / 2);
  const isEye = (x, y) => y >= eyeT && y < eyeT + eh &&
    ((x >= eyeL && x < eyeL + ew) || (x >= eyeL + ew + gap && x < eyeL + 2 * ew + gap));

  const px = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let cov = 0, inner = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const o = (y * SS + sy) * N + x * SS + sx;
          cov += mask[o]; inner += fill[o];
        }
      }
      if (cov === 0) continue;
      const body = isEye(x, y) ? INK : MID;
      const r = body[0] * inner + RIM[0] * (cov - inner);
      const g = body[1] * inner + RIM[1] * (cov - inner);
      const b = body[2] * inner + RIM[2] * (cov - inner);
      writePixel(px, (y * size + x) * 4, r, g, b, cov, SS * SS);
    }
  }
  return px;
}

/** Sums of premultiplied colour and coverage over `n` samples → one straight-alpha RGBA pixel. */
function writePixel(px, o, r, g, b, a, n) {
  if (a <= 0) return;
  px[o] = Math.round(Math.min(1, r / a) * 255);
  px[o + 1] = Math.round(Math.min(1, g / a) * 255);
  px[o + 2] = Math.round(Math.min(1, b / a) * 255);
  px[o + 3] = Math.round(Math.min(1, a / n) * 255);
}

/** Луч at `size` px, RGBA with straight alpha (what encodePNG writes). */
function renderLuch(size, { tray }) {
  return tray ? renderTray(size) : renderApp(size);
}

// ── PNG ───────────────────────────────────────────────────────────────────────

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function encodePNG(size, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0; // filter: none
    Buffer.from(rgba.buffer, y * size * 4, size * 4).copy(raw, y * (size * 4 + 1) + 1);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ── ICO (PNG-in-ICO, Vista and later) ─────────────────────────────────────────

function encodeICO(entries) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(entries.length, 4);
  const dir = Buffer.alloc(16 * entries.length);
  let offset = header.length + dir.length;
  entries.forEach((e, i) => {
    const o = i * 16;
    dir[o] = e.size >= 256 ? 0 : e.size;
    dir[o + 1] = e.size >= 256 ? 0 : e.size;
    dir[o + 2] = 0;
    dir[o + 3] = 0;
    dir.writeUInt16LE(1, o + 4);
    dir.writeUInt16LE(32, o + 6);
    dir.writeUInt32LE(e.png.length, o + 8);
    dir.writeUInt32LE(offset, o + 12);
    offset += e.png.length;
  });
  return Buffer.concat([header, dir, ...entries.map((e) => e.png)]);
}

// ── Go ────────────────────────────────────────────────────────────────────────

mkdirSync(OUT, { recursive: true });

// Up to 32 px the tray's silhouette: Windows builds the tray from the ICO's 32 px entry (the
// default window icon) and Linux from 32x32.png; a full fire would be a blob there.
const isTray = (size) => size <= 32;
const cache = new Map();
const png = (size) => {
  if (!cache.has(size)) cache.set(size, encodePNG(size, renderLuch(size, { tray: isTray(size) })));
  return cache.get(size);
};

const files = {
  "32x32.png": png(32),
  "128x128.png": png(128),
  "128x128@2x.png": png(256),
  "icon.png": png(512),
  // the tray's own icon (src-tauri/src/tray.rs): the default window icon is the ICO entry at the
  // system icon size, which above 100 % scale is the dark-square app icon. 32 px is the 16 px tray
  // at 2× and the tray's size at 200 %.
  "tray.png": encodePNG(32, renderLuch(32, { tray: true })),
};
for (const [name, data] of Object.entries(files)) {
  writeFileSync(join(OUT, name), data);
  console.log(`${name} — ${data.length} bytes`);
}

const ICO_SIZES = [16, 24, 32, 48, 64, 128, 256];
const ico = encodeICO(ICO_SIZES.map((size) => ({ size, png: png(size) })));
writeFileSync(join(OUT, "icon.ico"), ico);
console.log(`icon.ico — ${ico.length} bytes`);

if (outAt >= 0) {
  for (const size of ICO_SIZES) writeFileSync(join(OUT, `ico-${size}.png`), png(size));
  console.log(`ico-<size>.png — ${ICO_SIZES.join(", ")}`);
}
