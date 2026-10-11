// The turn summary of a session (ideas §3 «Итог хода»): the texts of the row's "+N −M" label and
// of the card's files line. Pure: no DOM, no state; a Node check runs it as is. The paths come
// from the Rust snapshot (TurnView), which only holds what the agent's own transcript says.

import type { TurnView } from "../core/bridge.ts";
import type { Translate } from "../core/i18n-core.ts";

export type TurnKey = "turn.files" | "turn.length";

/** "+120 −34" (a real minus sign), "≥ +120 −34" when the totals are a lower bound. */
export function diffText(added: number, removed: number, partial = false): string {
  return `${partial ? "≥ " : ""}+${added} −${removed}`;
}

/** The row's label: the line totals when the turn changed files, else null. */
export function rowDiff(turn: TurnView | undefined): string | null {
  if (!turn || turn.fileCount === 0) return null;
  return diffText(turn.added, turn.removed, turn.partial);
}

/**
 * The card's summary: "Files: 4 · +120 −34 · 6m", without the files part when none changed
 * ("Turn 6m"), without the length when it is unknown; "" for no summary. `elapsed` formats ms
 * like the rows' clock.
 */
export function turnLine(turn: TurnView | undefined, tr: Translate<TurnKey>, elapsed: (ms: number) => string): string {
  if (!turn) return "";
  const time = turn.ms !== undefined ? elapsed(turn.ms) : null;
  if (turn.fileCount === 0) return time === null ? "" : tr("turn.length", { time });
  const parts = [tr("turn.files", { n: turn.fileCount }), diffText(turn.added, turn.removed, turn.partial)];
  if (time !== null) parts.push(time);
  return parts.join(" · ");
}

function trimSeparators(path: string): string {
  return path.replace(/[\\/]+$/, "");
}

/**
 * How a changed file is named: its path relative to the project root when it is inside it
 * (Windows paths compare without case), else the full path. `name` is the last segment.
 */
export function fileLabel(path: string, root: string): { name: string; relative: string } {
  const name = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  const base = trimSeparators(root);
  if (base && path.length > base.length + 1 && path.toLowerCase().startsWith(base.toLowerCase())) {
    const sep = path[base.length];
    if (sep === "\\" || sep === "/") return { name, relative: path.slice(base.length + 1) };
  }
  return { name, relative: path };
}
