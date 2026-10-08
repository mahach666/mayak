// Which page the island opens on when the user opens it (quick-wins spec §2): a pure choice with
// no DOM, no Tauri and no State, so it can be checked on its own.

import type { IslandViewName } from "./layout";

/** The header tabs the setting can name, in the header's order. */
export const OPEN_PAGES = ["sessions", "overview", "stats", "focus", "apps"] as const;

export type OpenPage = (typeof OPEN_PAGES)[number];

/** Settings → General → "Open on click": a page, or the last one the user opened by its tab. */
export type OpenOnClick = OpenPage | "last";

export function isOpenPage(view: string): view is OpenPage {
  return (OPEN_PAGES as readonly string[]).includes(view);
}

/** The setting's values as the select lists them. */
export const OPEN_ON_CLICK_VALUES: readonly OpenOnClick[] = [...OPEN_PAGES, "last"];

/**
 * The view for a user-initiated open. An unknown `setting` (a newer build's value) opens the
 * sessions; `last` opens the last tab the user chose, the sessions while there is none. The
 * sessions page with no sessions to list is the overview, as it always was.
 */
export function viewOnOpen(setting: string, last: OpenPage | null, hasSessions: boolean): IslandViewName {
  const page: OpenPage = isOpenPage(setting) ? setting : setting === "last" ? last ?? "sessions" : "sessions";
  return page === "sessions" && !hasSessions ? "overview" : page;
}
