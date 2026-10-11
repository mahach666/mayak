// The session list's row menu (idea #8): its keys and its place. Pure: no DOM, no state. The items
// come from Rust (session_menu.rs); views/sessions.ts draws them and runs them.

import type { KeyPress } from "./island-keys.ts";

export type MenuKeyAction =
  /** The context-menu key or Shift+F10 on the highlighted row: its menu. */
  | { kind: "open" }
  /** ↑ ↓ in the open menu. */
  | { kind: "move"; delta: 1 | -1 }
  /** Enter or Space in the open menu: the highlighted item. */
  | { kind: "run" }
  /** Esc, Tab or the menu key again: the menu closes (Esc does not fold the island then). */
  | { kind: "close" };

const isMenuKey = (e: KeyPress) =>
  (e.key === "ContextMenu" && !e.shiftKey) || (e.key === "F10" && e.shiftKey);

/**
 * A key pressed on the session list → what it does to the row menu, or null to leave it to the
 * island (islandKeyAction). With the menu closed only the menu key answers; with it open the menu
 * takes ↑ ↓ Enter Space Esc Tab, and every other key goes on as usual.
 */
export function menuKeyAction(e: KeyPress, menuOpen: boolean): MenuKeyAction | null {
  if (e.ctrlKey || e.altKey || e.metaKey) return null;
  if (!menuOpen) return isMenuKey(e) ? { kind: "open" } : null;
  if (isMenuKey(e)) return { kind: "close" };
  if (e.shiftKey && e.key !== "Tab") return null;
  switch (e.key) {
    case "ArrowDown":
      return { kind: "move", delta: 1 };
    case "ArrowUp":
      return { kind: "move", delta: -1 };
    case "Enter":
    case " ":
      return { kind: "run" };
    case "Escape":
    case "Tab":
      return { kind: "close" };
    default:
      return null;
  }
}

/** The highlighted item after a step, wrapping around. From none (-1), ↓ starts at the first, ↑ at the last. */
export function stepItem(index: number, count: number, delta: 1 | -1): number {
  if (count <= 0) return -1;
  if (index < 0 || index >= count) return delta > 0 ? 0 : count - 1;
  return (index + delta + count) % count;
}

/**
 * Where the menu goes inside its card: its top-left corner at the point, pushed back inside so it
 * keeps a 4 px margin (and pinned to the top-left when the card is smaller than the menu).
 */
export function menuPlace(
  point: { x: number; y: number },
  menu: { w: number; h: number },
  area: { w: number; h: number },
): { left: number; top: number } {
  const clamp = (v: number, size: number, room: number) => Math.max(4, Math.min(v, room - size - 4));
  return { left: clamp(point.x, menu.w, area.w), top: clamp(point.y, menu.h, area.h) };
}
