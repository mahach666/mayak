// «Recent» on the sessions list (ideas §7): the Claude and Codex sessions of the last 24 hours that
// are no longer listed, read by Rust (resume.rs) only when the fold is opened. Pure: no DOM, no
// Tauri, no dictionary, so Node can check it (scripts/checks/recent.mjs).

import type { RecentSession, ResumeAgent, RecentEnd } from "./bridge";

/** The fold's state. `list` is null while the scan runs; `rev` changes with every change drawn. */
export interface RecentState {
  open: boolean;
  list: RecentSession[] | null;
  /** A line shown in place of one row's detail after Resume or Copy. */
  note: { key: string; kind: RecentNote } | null;
  rev: number;
}

/** What the last Resume or Copy on a row did. */
export type RecentNote = "copied" | "noCli" | "failed";

export function closedRecent(): RecentState {
  return { open: false, list: null, note: null, rev: 0 };
}

/** The 24 px lines «Recent» adds to the list: its fold line, then one line per row (one for "…" or "none"). */
export function recentLines(r: RecentState): number {
  if (!r.open) return 1;
  return 1 + Math.max(1, r.list?.length ?? 1);
}

/** The ↑ ↓ key of a recent row; never a session key (those are `claude:`, `codex:`, `antigravity:`, `silent:`). */
export function recentKey(r: Pick<RecentSession, "agent" | "id">): string {
  return `recent:${r.agent}:${r.id}`;
}

/** The row of a `recentKey` in the list, or null. */
export function findRecent(list: readonly RecentSession[] | null, key: string): RecentSession | null {
  return list?.find((r) => recentKey(r) === key) ?? null;
}

/** The row's text: its title, else its last phrase, else its folder's name. */
export function recentDetail(r: RecentSession): string {
  return r.title || r.last || r.project;
}

/**
 * The phase word of a row, as a dictionary key with its tone. A finished turn has none: only the
 * sessions that stopped short stand out.
 */
export function recentEndWord(end: RecentEnd | null): { key: "phase.error" | "phase.interrupted" | "recent.unfinished"; tone: "red" | "amber" | "dim" } | null {
  switch (end) {
    case "error":
      return { key: "phase.error", tone: "red" };
    case "unfinished":
      return { key: "recent.unfinished", tone: "amber" };
    case "interrupted":
      return { key: "phase.interrupted", tone: "dim" };
    case "finished":
    case null:
      return null;
  }
}

/** The command a row resumes with, as shown in its tooltip (resume.rs ResumeAgent::args). */
export function resumeCommand(agent: ResumeAgent, id: string): string {
  return agent === "claude" ? `claude --resume ${id}` : `codex resume ${id}`;
}

/** The note after Resume: started → none (the island folds), copied → the CLI was not found. */
export function noteAfterResume(outcome: "started" | "copied" | "failed" | null): RecentNote | null {
  if (outcome === "started") return null;
  return outcome === "copied" ? "noCli" : "failed";
}

/**
 * The session and error cards offer «Copy resume» for a Claude or Codex session whose turn stopped
 * short (error or interrupted): the window may be gone, the command brings the session back.
 */
export function offersResumeCopy(agent: string, outcome: string | null | undefined): boolean {
  return (agent === "claude" || agent === "codex") && (outcome === "error" || outcome === "interrupted");
}
