// Sessions snapshot store — the island's copy of what the Rust observer
// publishes. No state logic lives here: ranks, attention, counts, hold and the
// aggregate are all computed in Rust (spec §10). This file only drops stale
// snapshots (rev guard) and counts the lines the sessions view will draw.

import type {
  Alert, AttentionKind, BotState, ProjectView, SessionView, SessionsSnapshot,
} from "./bridge";
import type { BotStateName, IslandViewName } from "./layout";
import type { SoundName } from "./sound";

/** Acked idle rows of one project beyond this many fold into one "+N idle" line. */
export const IDLE_FOLD_LIMIT = 3;

/**
 * The project's rows in display order: everything that is not an acked Idle row
 * keeps the snapshot order (Rust already ranks it); acked Idle rows go last.
 */
export function splitIdle(project: ProjectView): { live: SessionView[]; idle: SessionView[] } {
  const live: SessionView[] = [];
  const idle: SessionView[] = [];
  for (const s of project.sessions) {
    if (s.phase.kind === "idle" && s.acked && !s.silent) idle.push(s);
    else live.push(s);
  }
  return { live, idle };
}

export class SessionsStore {
  snapshot: SessionsSnapshot | null = null;
  /** Project keys whose "+N idle" fold is open. */
  readonly expandedIdle = new Set<string>();

  private lastRev = -1;
  private listeners = new Set<() => void>();

  /** False (ignored) when s.rev <= the last accepted rev; the first rev accepted is 0. */
  accept(s: SessionsSnapshot): boolean {
    if (s.rev <= this.lastRev) return false;
    this.lastRev = s.rev;
    this.snapshot = s;
    for (const fn of this.listeners) fn();
    return true;
  }

  /** Any project with at least one real session; a silent Codex row is not one. */
  get hasSessions(): boolean {
    return this.snapshot?.projects.some((p) => p.sessions.some((s) => !s.silent)) ?? false;
  }

  /** Any project with at least one row, silent ones included. */
  get hasRows(): boolean {
    return this.snapshot?.projects.some((p) => p.sessions.length > 0) ?? false;
  }

  find(key: string): { session: SessionView; project: ProjectView } | null {
    for (const project of this.snapshot?.projects ?? []) {
      const session = project.sessions.find((s) => s.key === key);
      if (session) return { session, project };
    }
    return null;
  }

  /** True when this project's acked Idle rows are drawn as one "+N idle" line. */
  folded(projectKey: string, ackedIdle: number): boolean {
    return ackedIdle > IDLE_FOLD_LIMIT && !this.expandedIdle.has(projectKey);
  }

  /**
   * 24 px lines the sessions view renders: project headers + visible rows +
   * "+N idle" rows. With no session at all the view draws one empty-state line.
   */
  rowCount(): number {
    let rows = 0;
    for (const project of this.snapshot?.projects ?? []) {
      if (project.sessions.length === 0) continue;
      const { live, idle } = splitIdle(project);
      rows += 1 + live.length + (this.folded(project.key, idle.length) ? 1 : idle.length);
    }
    return Math.max(1, rows);
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
}

export const Sessions = new SessionsStore();

/** The aggregate names are a subset of Luch's states: identity mapping. */
export function botStateName(s: BotState): BotStateName {
  return s;
}

/** The card an attention item opens. */
export function viewForAttention(k: Exclude<AttentionKind, null>): IslandViewName {
  switch (k) {
    case "permission":
    case "input":
      return "waiting";
    case "error":
      return "error";
    case "finished":
      return "finished";
  }
}

/** Existing WAVs until the sound set is replaced (sub-project 3). */
export const ALERT_SOUND: Record<Alert["kind"], SoundName> = {
  permission: "approval",
  input: "approval",
  error: "error",
  finished: "finish",
};
