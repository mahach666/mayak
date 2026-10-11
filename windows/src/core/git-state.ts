// Git state of a project in the sessions list (ideas #5): what its header shows and when the list
// asks Rust again (src-tauri/src/git_state.rs). Asked when the list comes on screen, and while it
// stays there only for a project where a turn has just ended (a new idle row) or that just
// appeared. Never on a timer, never while the island is hidden. Pure: scripts/checks/git-state.mjs
// runs it in Node.

import type { Translate } from "./i18n-core";
import type { Key } from "./i18n";

export type GitOperation = "rebase" | "merge" | "cherryPick" | "revert";

/** git_state.rs `GitState`. */
export interface GitState {
  branch: string | null;
  commit: string | null;
  upstream: boolean;
  ahead: number;
  behind: number;
  changed: number;
  untracked: number;
  conflicts: number;
  insertions: number;
  deletions: number;
  operation: GitOperation | null;
}

/** Only what the turn check reads of a snapshot. */
interface ProjectRows {
  key: string;
  sessions: { key: string; since: number; silent?: unknown; phase: { kind: string } }[];
}

/** "dirty": uncommitted changes or an operation half-way; "conflict": unmerged files. */
export type GitMark = "clean" | "dirty" | "conflict";

export interface GitChip {
  /** The branch, or the short commit of a detached HEAD. */
  ref: string;
  mark: GitMark;
  /** "↑2 ↓1", "↑2", "" when in step or without an upstream. */
  sync: string;
  /** The operation's short word, or null. */
  op: string | null;
}

const OP_WORD: Record<GitOperation, Key> = {
  rebase: "git.op.rebase",
  merge: "git.op.merge",
  cherryPick: "git.op.cherryPick",
  revert: "git.op.revert",
};

export function gitMark(s: GitState): GitMark {
  if (s.conflicts > 0) return "conflict";
  return s.changed + s.untracked > 0 || s.operation !== null ? "dirty" : "clean";
}

/** What the project header shows. */
export function gitChip(s: GitState, tr: Translate<Key>): GitChip {
  const ref = s.branch ?? (s.commit ? tr("git.detachedShort", { commit: s.commit }) : "HEAD");
  const sync = [s.ahead > 0 ? `↑${s.ahead}` : "", s.behind > 0 ? `↓${s.behind}` : ""].filter(Boolean).join(" ");
  return { ref, mark: gitMark(s), sync, op: s.operation ? tr(OP_WORD[s.operation]) : null };
}

/** The chip's tooltip, one fact per line. */
export function gitTooltip(s: GitState, tr: Translate<Key>): string {
  const lines: string[] = [];
  if (s.branch !== null) lines.push(tr("git.branch", { branch: s.branch }));
  else lines.push(tr("git.detached", { commit: s.commit ?? "HEAD" }));
  if (s.commit === null) lines.push(tr("git.noCommits"));
  if (s.operation) lines.push(tr("git.inProgress", { op: tr(OP_WORD[s.operation]) }));
  if (s.conflicts > 0) lines.push(tr("git.conflicts", { n: s.conflicts }));
  if (s.changed > 0) {
    lines.push(tr("git.changed", { n: s.changed }));
    if (s.insertions + s.deletions > 0) lines.push(tr("git.lines", { add: s.insertions, del: s.deletions }));
  }
  if (s.untracked > 0) lines.push(tr("git.untracked", { n: s.untracked }));
  if (s.changed + s.untracked + s.conflicts === 0) lines.push(tr("git.clean"));
  if (s.branch !== null) {
    if (!s.upstream) lines.push(tr("git.noUpstream"));
    else {
      if (s.ahead > 0) lines.push(tr("git.ahead", { n: s.ahead }));
      if (s.behind > 0) lines.push(tr("git.behind", { n: s.behind }));
      if (s.ahead === 0 && s.behind === 0) lines.push(tr("git.inStep"));
    }
  }
  return lines.join("\n");
}

/** Per project key, its idle rows as "key@since": a new mark is a turn that has just ended. */
export function idleMarks(projects: readonly ProjectRows[]): Map<string, Set<string>> {
  const marks = new Map<string, Set<string>>();
  for (const p of projects) {
    const set = new Set<string>();
    for (const s of p.sessions) if (s.phase.kind === "idle" && !s.silent) set.add(`${s.key}@${s.since}`);
    marks.set(p.key, set);
  }
  return marks;
}

/**
 * The project keys to ask again after a new snapshot while the list stays on screen: a project
 * that was not there before, or one with an idle row it did not have (a turn ended). A row that
 * leaves idle (a turn starts) asks nothing.
 */
export function projectsToRefresh(prev: Map<string, Set<string>>, next: Map<string, Set<string>>): string[] {
  const keys: string[] = [];
  for (const [key, marks] of next) {
    const before = prev.get(key);
    if (!before || [...marks].some((m) => !before.has(m))) keys.push(key);
  }
  return keys;
}
