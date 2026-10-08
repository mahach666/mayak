// A session card whose session moved on (spec §2.6): the pure part, with no DOM, no Tauri and
// no State, so it can be checked on its own. Decided on each observer snapshot, with no timer.

import type { AttentionKind, SessionView, SessionsSnapshot } from "../core/bridge";

export type AlertKind = Exclude<AttentionKind, null>;

/**
 * The episode a session card on screen stands for (`waiting`, `finished` or `error`), with
 * `since` the episode start (Alert.since). A permission or question card has none: it ends only by
 * the user, the island leaving it, pause, a language reload or its 590 s limit, never on its own.
 */
export interface CardEpisode {
  key: string;
  kind: "waiting" | "finished" | "error";
  since: number;
}

/** The next card to show: an unacknowledged alert of the snapshot. */
export interface NextAlert {
  key: string;
  kind: AlertKind;
  /** Its episode start, the `since` of the card that shows it. */
  since: number;
}

/** Rust's attention_rank (observe/model.rs), the scale of Alert.rank: most important = smallest. */
export const ATTENTION_RANK: Record<AlertKind, number> = { permission: 0, input: 1, error: 2, finished: 3 };

/** Rust's episode_start: Waiting → since; Idle with a finished or error outcome → its time; else null. */
export function episodeStart(s: SessionView): number | null {
  if (s.phase.kind === "waiting") return s.since;
  if (s.phase.kind === "idle" && (s.outcome?.kind === "finished" || s.outcome?.kind === "error")) return s.outcome.at;
  return null;
}

function findSession(snapshot: SessionsSnapshot, key: string): SessionView | null {
  for (const project of snapshot.projects) {
    const s = project.sessions.find((x) => x.key === key);
    if (s) return s;
  }
  return null;
}

/**
 * True when the snapshot shows the card's session went on past the card's episode: a wait was
 * answered (working or idle again, or a newer wait), a finished or error turn was followed by a
 * new turn. An ack never makes a card stale,
 * and neither does an unknown phase or a session missing from the snapshot: the card then stays.
 */
export function staleCard(card: CardEpisode, snapshot: SessionsSnapshot): boolean {
  const s = findSession(snapshot, card.key);
  if (!s) return false;
  const phase = s.phase.kind;
  switch (card.kind) {
    case "waiting":
      return phase === "working" || phase === "idle" || (phase === "waiting" && s.since > card.since);
    case "finished":
    case "error":
      if (phase === "working" || phase === "waiting") return true;
      return phase === "idle" && s.outcome != null && s.outcome.at > card.since;
  }
}

/**
 * The most important unacknowledged alert of the snapshot that `allowed` lets open, by rank, ties
 * in the snapshot's attention order (Rust's cmp_sessions). The card's own episode (`except`) is
 * skipped; a newer episode of the same session is not. Null when there is none.
 */
export function nextAlert(
  snapshot: SessionsSnapshot,
  except: { key: string; since: number },
  allowed: (kind: AlertKind) => boolean,
): NextAlert | null {
  let best: NextAlert | null = null;
  for (const key of snapshot.attention) {
    const s = findSession(snapshot, key);
    if (!s || s.silent || s.acked || s.attention == null) continue;
    const since = episodeStart(s);
    if (since == null || !allowed(s.attention)) continue;
    if (key === except.key && since <= except.since) continue;
    if (best == null || ATTENTION_RANK[s.attention] < ATTENTION_RANK[best.kind]) {
      best = { key, kind: s.attention, since };
    }
  }
  return best;
}
