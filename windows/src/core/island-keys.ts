// Keys inside the open island (ideas §36, after upstream's islandKeyAction): ↑ ↓ walk the session
// rows, Enter opens the highlighted one, ← → switch the header pages. Pure: no DOM, no state.

import { OPEN_PAGES, isOpenPage, type OpenPage } from "./open-view.ts";

/** The parts of a KeyboardEvent read here. */
export interface KeyPress {
  key: string;
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/** Where the key landed. */
export interface KeyTarget {
  /** The page on screen (State.view). */
  view: string;
  /** A text field, a select or editable text has the focus: its own arrows and Enter. */
  inTextField: boolean;
  /** A button or link has the focus: Enter clicks it, as the browser does. */
  onControl: boolean;
}

export type IslandKeyAction =
  /** ↑ ↓ (or Ctrl+↑ Ctrl+↓) on the session list. */
  | { kind: "list"; delta: 1 | -1 }
  /** Enter or Ctrl+O on the session list: the highlighted session in its terminal or editor. */
  | { kind: "open" }
  /** ← → (or Ctrl+← Ctrl+→) on a header page: the next or previous one. */
  | { kind: "page"; delta: 1 | -1 };

/**
 * A key pressed in the open island → what it does, or null to leave it alone. Only the session list
 * and the header pages answer: the cards (permission, question, waiting…) never act on these keys.
 * Esc is not handled here (island.ts keeps folding the island on it).
 */
export function islandKeyAction(e: KeyPress, at: KeyTarget): IslandKeyAction | null {
  if (e.altKey || e.shiftKey || e.metaKey || at.inTextField) return null;
  const onList = at.view === "sessions";
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    return onList ? { kind: "list", delta: e.key === "ArrowDown" ? 1 : -1 } : null;
  }
  if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
    return isOpenPage(at.view) ? { kind: "page", delta: e.key === "ArrowRight" ? 1 : -1 } : null;
  }
  if (!onList) return null;
  if (e.key === "Enter" && !e.ctrlKey && !at.onControl) return { kind: "open" };
  if (e.ctrlKey && (e.code === "KeyO" || e.key.toLowerCase() === "o")) return { kind: "open" };
  return null;
}

/**
 * The highlighted row after a step: clamped at both ends (no wrap, as upstream's navigate). With
 * nothing highlighted, ↓ starts at the first row and ↑ at the last. Null when there is no row.
 */
export function moveSelection(keys: readonly string[], current: string | null, delta: 1 | -1): string | null {
  if (keys.length === 0) return null;
  const at = current == null ? -1 : keys.indexOf(current);
  if (at < 0) return delta > 0 ? keys[0] : keys[keys.length - 1];
  return keys[Math.max(0, Math.min(keys.length - 1, at + delta))];
}

/** The list changed under the highlight: the same session stays highlighted, or none if it left. */
export function keepSelection(keys: readonly string[], current: string | null): string | null {
  return current != null && keys.includes(current) ? current : null;
}

/** The header page `delta` steps from `view`, wrapping around, in the header's order. */
export function cyclePage(view: OpenPage, delta: 1 | -1): OpenPage {
  const n = OPEN_PAGES.length;
  return OPEN_PAGES[(OPEN_PAGES.indexOf(view) + delta + n) % n];
}
