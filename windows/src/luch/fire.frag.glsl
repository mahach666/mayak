precision highp float;
varying vec2 vUv;
uniform float uTime, uGrow, uLive, uBright, uCoreA, uHandsOut, uTall;
// 0…1: the launch greeting's fire catching inside the full-size silhouette (1 = the normal fire)
uniform float uIgnite;
uniform float uFlares, uStreaks;
uniform vec3 uDeep, uMid, uLight, uCore;
uniform vec4 uHands;
// The target canvas in device px: v = (dot(uViewX, (p, 1)), dot(uViewY, (p, 1))) for the box point p;
// uViewFade = (width, height, fade). The glow fades out over `fade` px before the canvas edge so the
// box never shows as a square where the canvas cuts it. Off while fade <= 0.
uniform vec3 uViewX, uViewY, uViewFade;

// Coordinates: p in -1…1 over the box, y up — the same frame as the reference picture, so
// every measure below is read straight off it (body, tongue, licks, face, hands).

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
float noise(vec2 p) {
  vec2 i = floor(p), f = fract(p), u = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x), mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y);
}
float fbm3(vec2 p) {
  float v = 0.0, a = 0.5;
  for (int i = 0; i < 3; i++) { v += a * noise(p); p = p * 2.02 + vec2(1.7, 9.2); a *= 0.5; }
  return v;
}
float smax(float a, float b, float k) { float h = clamp(0.5 + 0.5 * (a - b) / k, 0.0, 1.0); return mix(b, a, h) + k * h * (1.0 - h); }
vec2 rot(vec2 p, float a) { float c = cos(a), s = sin(a); return vec2(c * p.x - s * p.y, s * p.x + c * p.y); }
float drop(vec2 q, float R, float B, float tall, float taper) {
  if (q.y < -B) return -(abs(q.y) - B) * 4.0 - abs(q.x) / R;
  if (q.y > tall) return -(q.y - tall) * 3.0 - abs(q.x) / R;
  float w = q.y < 0.0 ? R * sqrt(1.0 - (q.y * q.y) / (B * B)) : R * pow(1.0 - q.y / tall, taper);
  return (w - abs(q.x)) / R;
}
float rbox(vec2 p, vec2 b, float r) { vec2 d = abs(p) - b + r; return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0) - r; }

/** One lick of flame rooted at `at`, leaning by `lean` (radians, + = left), curling at the tip. */
float lick(vec2 p, vec2 at, float lean, float R, float tall, float curl, float t, float seed) {
  vec2 q = rot(p - at, lean);
  float y = max(q.y, 0.0);
  q.x -= curl * pow(max(y / tall - 0.45, 0.0), 2.0) * tall + 0.035 * sin(q.y * 6.0 - t * 2.6 + seed) * y / tall;
  return drop(q, R, R * 0.7, tall * (0.9 + 0.1 * sin(t * 2.3 + seed)), 0.9);
}

void main() {
  vec2 p0 = vUv * 2.0 - 1.0;
  float edge = smoothstep(1.0, 0.6, length(p0));
  vec2 base = vec2(0.0, -0.44);
  float g = max(uGrow, 0.001);
  vec2 p = base + (p0 - base) / g;
  float t = uTime * uLive;

  // a slow, broad drift that grows with height: the body breathes, the crown sways
  float h = clamp((p.y + 0.44) / 1.1, 0.0, 1.0);
  vec2 warp = vec2(fbm3(vec2(p.x * 1.8, p.y * 1.4 - t * 1.3)), fbm3(vec2(p.x * 1.8 + 4.1, p.y * 1.4 - t * 1.3 + 2.7))) - 0.5;
  vec2 q = p + warp * (0.015 + 0.16 * h * h);

  // the body: a wide, soft-cornered blob — centre (0.01, -0.12), half-size 0.44 × 0.32
  vec2 qb = (q - vec2(0.01, -0.11)) * vec2(1.0 / 0.44, 1.0 / 0.33);
  float fb = (1.0 - pow(pow(abs(qb.x), 2.1) + pow(abs(qb.y), 2.0), 1.0 / 2.05)) * 1.25;

  // the crown: the main tongue (tip at about (-0.09, 0.66)), a lick off the right shoulder,
  // a lick on the left side
  float ft = lick(q, vec2(0.02, 0.06), -0.16, 0.34, 0.64 * uTall, -1.6, t, 0.0);
  float fr = lick(q, vec2(0.3, 0.08), 0.35, 0.09, 0.15 * uTall, -0.2, t, 1.7);
  float fl = lick(q, vec2(-0.37, -0.04), -0.6, 0.08, 0.14 * uTall, 0.25, t, 3.1);
  float f = smax(fb, ft, 0.1);
  // a lighter inner tongue inside the main one: the painted, layered look of the reference
  float inner = lick(q, vec2(-0.02, -0.02), -0.18, 0.25, 0.56 * uTall, -1.5, t + 0.25, 0.4);
  float calm = f;

  // light erosion near the top only
  float er = fbm3(vec2(p.x * 2.6, p.y * 1.9 - t * 1.8));
  float fire = f - (er - 0.45) * (0.02 + 0.25 * h * h);

  // flares: little flames that pinch off a lick and float up, shrinking
  for (int i = 0; i < 3; i++) {
    float fi = float(i);
    float ph = fract(t * (0.2 + 0.05 * fi) + hash(vec2(fi, 7.0)));
    vec2 tip = i == 0 ? vec2(-0.5, 0.1) : i == 1 ? vec2(0.36, 0.24) : vec2(-0.12, 0.62);
    float side = i == 1 ? 1.0 : -1.0;
    vec2 c = tip + vec2(side * 0.05 * ph + 0.02 * sin(t * 2.0 + fi * 2.0), 0.02 + ph * 0.28);
    float s = (1.0 - ph) * (i == 2 ? 0.7 : 1.0);
    vec2 qf = rot(p - c, -side * 0.35) / max(s, 0.001);
    qf.x += side * 1.2 * pow(max(qf.y, 0.0) / 0.1, 2.0) * 0.02;
    float fl2 = drop(qf, 0.045, 0.035, 0.1, 1.2);
    // gate the merge, not the input: smax(fire, 0, k) would lift the whole outside field to 0 —
    // both for the flares switch and for the fade-in ramp (a field scaled by ~0 floods the box)
    fire = mix(fire, smax(fire, fl2 * 0.95, 0.06 * (1.0 - ph)), uFlares * smoothstep(0.0, 0.12, ph));
  }

  // ignition: no scaling — the fire catches inside the full-size silhouette. At the start the seed is a
  // coal on the base, then a thin lighter tongue; a burning front then climbs from it, its centre
  // drifting side to side (it licks up one side, then the other), the body's hot interior catching
  // ahead of its edges, torn by a fast upright flicker; the tongue's tip catches last. At uIgnite = 1
  // the block is skipped and the fire is exactly as before.
  float igK = 1.0 - clamp(uIgnite, 0.0, 1.0);
  float seed = -1.0, sk = 0.0, frontV = 1.0;
  if (igK > 0.0) {
    float ig = 1.0 - igK;
    // two octaves rising at different speeds: broad licks and fine flicker on the front
    // (on the fire's own warped coordinates, so the front sways and breathes with the flame)
    float fk = 0.6 * fbm3(vec2(q.x * 4.0, q.y * 3.5 - t * 5.0)) + 0.4 * noise(vec2(q.x * 11.0, q.y * 6.0 - t * 11.0));
    float reach = 1.05 * max(0.0, ig - 0.12) / 0.88;
    float ss = smoothstep(0.0, 0.2, reach);
    vec2 o = vec2(0.12 * sin(t * 1.9) * igK, -0.4);
    // a flame-shaped reach: wide at the base, narrowing as it climbs
    vec2 dq = q - o;
    float dist = length(vec2(dq.x * (1.0 + 0.9 * max(dq.y, 0.0)), dq.y * 0.7));
    float front = (reach - dist + igK * ss * (0.25 * clamp(f, 0.0, 1.0) + (fk - 0.5) * 0.7)) * 1.6;
    fire = min(fire, front); calm = min(calm, front);
    frontV = front;
    float on = smoothstep(0.0, 0.1, ig);
    seed = lick(p, vec2(0.0, -0.41), 0.06 * sin(t * 9.0) * on, 0.055, 0.03 + 0.17 * on + 0.05 * sin(t * 13.0) * on, -0.4, t * 3.0, 5.0);
    sk = 1.0 - smoothstep(0.1, 0.3, ig);
    fire = mix(fire, smax(fire, seed, 0.04), sk);
    calm = mix(calm, max(calm, seed), sk);
  }

  // paint: a red-orange rim about 0.06 thick, then bright yellow, a paler heart
  vec3 col = mix(uDeep, uMid, smoothstep(0.0, 0.05, fire));
  float lightZone = max(max(smoothstep(0.22, 0.42, fb), smoothstep(0.12, 0.3, ft)), smoothstep(0.0, 0.14, inner));
  col = mix(col, uLight, lightZone * smoothstep(0.03, 0.1, fire));
  col = mix(col, mix(uLight, uCore, 0.5), smoothstep(0.5, 0.8, max(fb, inner * 0.8)));
  float streak = noise(vec2(p.x * 10.0 + 0.6 * sin(p.y * 3.0), p.y * 1.4 - t * 0.6)) - 0.5;
  col *= 1.0 + 0.07 * uStreaks * streak * smoothstep(0.05, 0.3, fire);
  float a = smoothstep(-0.008, 0.012, fire);
  if (igK > 0.0) {
    // the lighter's tongue is pale yellow, and the catching fire has a bluish base
    col = mix(col, mix(uLight, uCore, 0.4), sk * smoothstep(0.0, 0.35, seed));
    // the burning front is a red-orange edge, like the rim of the flame
    col = mix(col, mix(uDeep, uMid, 0.55), igK * (1.0 - smoothstep(0.0, 0.16, frontV)) * 0.85);
    col = mix(col, vec3(0.42, 0.6, 1.0), igK * smoothstep(-0.37, -0.45, p.y) * 0.65);
  }

  // the face: a big cream panel — centre (0.03, -0.095), half-size 0.33 × 0.225
  float sd = rbox(p - vec2(0.03, -0.1), vec2(0.32, 0.22), 0.2);
  col = mix(col, mix(uLight, uCore, 0.55), smoothstep(0.07, 0.0, sd) * uCoreA * 0.6);
  float core = smoothstep(0.015, -0.025, sd) * uCoreA;
  col = mix(col, mix(uCore, mix(uCore, uLight, 0.4), smoothstep(-0.09, 0.0, sd)), core);

  // hands: cream ovals with an orange rim, just off the body's lower corners
  if (uHandsOut > 0.01) {
    float dl = length(rot(p - uHands.xy, 0.3) * vec2(1.0, 1.07)) - 0.072 * uHandsOut;
    float dr = length(rot(p - uHands.zw, -0.55) * vec2(1.0, 1.12)) - 0.078 * uHandsOut;
    float d = min(dl, dr);
    float hm = smoothstep(0.007, -0.007, d);
    col = mix(col, mix(uMid, mix(uCore, uLight, 0.25), smoothstep(0.0, -0.022, d)), hm);
    a = max(a, hm);
    calm = max(calm, -d * 5.0);
  }
  col *= uBright;

  float outside = max(0.0, -calm);
  float glow = (0.32 * exp(-outside * 4.5) + 0.07 * exp(-outside * 1.4)) * (1.0 - a) * uBright * edge;
  if (uViewFade.z > 0.0) {
    vec2 v = vec2(dot(uViewX, vec3(p0, 1.0)), dot(uViewY, vec3(p0, 1.0)));
    vec2 e = min(v, uViewFade.xy - v);
    glow *= smoothstep(0.0, uViewFade.z, e.x) * smoothstep(0.0, uViewFade.z, e.y);
  }
  gl_FragColor = vec4(col * a + mix(uMid, uDeep, 0.4) * glow, a + glow * 0.85);
}
