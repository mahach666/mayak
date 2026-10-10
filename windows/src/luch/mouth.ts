// Луч's mouth slot (the mailbox's opening): a spring the engine steps every frame.
// No DOM and no imports: the Node checks load it as is.

/** Natural frequency: ω₀ = 2π / 0.25 s. */
export const MOUTH_OMEGA = (2 * Math.PI) / 0.25;
/** Damping ratio. */
export const MOUTH_ZETA = 0.6;
/**
 * The longest slice the spring is integrated over (seconds). The frame's dt can be up to the island's
 * clamp (0.1 s); a longer frame is cut into equal slices no longer than this, so the spring moves the
 * same at 15, 20 and 60 fps and stays as stable at the clamp (ω·h ≈ 0.5, well inside 0.05 s). A 60 fps
 * frame, late vsyncs included, is a single slice, exactly as before.
 */
export const MOUTH_MAX_SLICE_S = 0.02;

export interface MouthSpring {
  slot: number;
  vel: number;
}

/** One frame of the mouth spring toward `target`, in slices of at most MOUTH_MAX_SLICE_S. The slot never goes below 0. */
export function stepMouth(s: MouthSpring, target: number, dt: number): MouthSpring {
  const total = Math.max(0, dt);
  const slices = Math.max(1, Math.ceil(total / MOUTH_MAX_SLICE_S - 1e-9));
  const h = total / slices;
  let { slot, vel } = s;
  for (let i = 0; i < slices; i++) {
    const acc = MOUTH_OMEGA * MOUTH_OMEGA * (target - slot) - 2 * MOUTH_ZETA * MOUTH_OMEGA * vel;
    vel += acc * h;
    slot = Math.max(0, slot + vel * h);
  }
  return { slot, vel };
}
