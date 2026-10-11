// The «next waiting session» shortcut (ideas §2): which session a press goes to. Pure: the island
// passes the snapshot's attention list (Rust already orders it: permissions and questions waiting
// longest first, then errors and unacknowledged finished turns) and what earlier presses visited.

/** A session a press went to, with the episode it was in: a new wait of the same session is new. */
export type Visited = ReadonlyMap<string, number>;

export interface NextWaiting {
  key: string;
  /** What the next press skips: the sessions visited in this round, this one included. */
  visited: Map<string, number>;
}

/**
 * The first session of `attention` this round has not visited yet, in its current episode
 * (`episodeOf`; a session without one is not waiting and is skipped). A session visited in an
 * earlier episode counts as new. When every waiting session was visited the round starts again
 * at the first. `null` when nothing waits.
 */
export function nextWaiting(
  attention: readonly string[],
  episodeOf: (key: string) => number | null,
  visited: Visited,
): NextWaiting | null {
  const waiting: { key: string; episode: number }[] = [];
  for (const key of attention) {
    const episode = episodeOf(key);
    if (episode != null) waiting.push({ key, episode });
  }
  if (waiting.length === 0) return null;
  // Only what still waits, in the same episode, stays visited.
  const kept = new Map<string, number>();
  for (const { key, episode } of waiting) {
    if (visited.get(key) === episode) kept.set(key, episode);
  }
  const fresh = waiting.find(({ key }) => !kept.has(key));
  if (fresh) {
    kept.set(fresh.key, fresh.episode);
    return { key: fresh.key, visited: kept };
  }
  const first = waiting[0];
  return { key: first.key, visited: new Map([[first.key, first.episode]]) };
}
