// The one WebGL context that draws Луч's fire (spec §2). Every caller renders into the top-left
// px×px of the shared canvas and copies it out with drawImage before the next call.

import FRAG from "./fire.frag.glsl?raw";
import type { FirePalette } from "./palette";

export interface FireUniforms {
  time: number; grow: number; live: number; bright: number; coreA: number;
  handsOut: number; tall: number; flares: number; palette: FirePalette; hands: [number, number, number, number];
}

const VERT = "attribute vec2 aPos; varying vec2 vUv; void main(){ vUv = aPos * 0.5 + 0.5; gl_Position = vec4(aPos, 0.0, 1.0); }";
const MAX = 512;
const NAMES = ["uTime", "uGrow", "uLive", "uBright", "uCoreA", "uHandsOut", "uTall", "uFlares", "uDeep", "uMid", "uLight", "uCore", "uHands"] as const;

interface Ctx { canvas: HTMLCanvasElement; gl: WebGLRenderingContext; u: Record<(typeof NAMES)[number], WebGLUniformLocation | null> }
let ctx: Ctx | null = null;
let broken = false;
let current: HTMLCanvasElement | null = null; // the newest canvas: events from older ones are ignored

const rgb01 = (h: string) => { const v = parseInt(h.slice(1), 16); return [((v >> 16) & 255) / 255, ((v >> 8) & 255) / 255, (v & 255) / 255]; };

function create(): Ctx | null {
  if (new URLSearchParams(location.search).has("luchNoGL")) return null;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = MAX;
  const gl = canvas.getContext("webgl", { premultipliedAlpha: true, alpha: true, preserveDrawingBuffer: true, antialias: false });
  if (!gl) return null;
  current = canvas;
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
  canvas.addEventListener("webglcontextlost", (e) => {
    e.preventDefault();
    if (canvas !== current) return; // a stale canvas must not drop the live context
    ctx = null; broken = true;
  });
  canvas.addEventListener("webglcontextrestored", () => { if (canvas === current) broken = false; });
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
  gl.uniform1f(u.uTall, f.tall); gl.uniform1f(u.uFlares, f.flares);
  gl.uniform3fv(u.uDeep, rgb01(f.palette.deep)); gl.uniform3fv(u.uMid, rgb01(f.palette.mid));
  gl.uniform3fv(u.uLight, rgb01(f.palette.light)); gl.uniform3fv(u.uCore, rgb01(f.palette.core));
  gl.uniform4fv(u.uHands, f.hands);
  gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  return { canvas, px: s };
}
