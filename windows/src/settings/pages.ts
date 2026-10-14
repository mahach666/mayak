// The Settings window's pages (ideas §37): a sidebar with one page per group, each made of
// sections. Pure, so the Node checks can see that every section sits on exactly one page.

import type { Key } from "../core/i18n";

/** One card (or a line between cards) of the Settings window. main.ts has one builder per id. */
export type SectionId =
  | "island" | "tabs"
  | "sound"
  | "alerts"
  | "sources" | "claudeCode" | "codex" | "antigravity" | "mcp" | "chats"
  | "limits" | "claudeApi" | "openrouter" | "deepseek" | "moonshot"
  | "integrations"
  | "system" | "about";

export type PageId = "island" | "sound" | "alerts" | "agents" | "limits" | "integrations" | "system";

export interface Page {
  id: PageId;
  /** The sidebar item and the page's title. */
  title: Key;
  /** In the order they are drawn. */
  sections: readonly SectionId[];
}

export const PAGES: readonly Page[] = [
  { id: "island", title: "set.pageIsland", sections: ["island", "tabs"] },
  { id: "sound", title: "settings.sound", sections: ["sound"] },
  { id: "alerts", title: "set.alerts", sections: ["alerts"] },
  { id: "agents", title: "set.pageAgents", sections: ["sources", "claudeCode", "codex", "antigravity", "mcp", "chats"] },
  { id: "limits", title: "set.pageLimits", sections: ["limits", "claudeApi", "openrouter", "deepseek", "moonshot"] },
  { id: "integrations", title: "set.integrations", sections: ["integrations"] },
  { id: "system", title: "set.pageSystem", sections: ["system", "about"] },
];

/** The page a window opens on when the session has none yet. */
export const FIRST_PAGE: PageId = PAGES[0].id;

/** A stored page id, or the first page for anything this build does not know (null, an old id). */
export function pageOrFirst(stored: string | null): PageId {
  return PAGES.some((p) => p.id === stored) ? (stored as PageId) : FIRST_PAGE;
}

/** The page ↑ (-1) or ↓ (+1) moves to from `current`; it stops at either end. */
export function stepPage(current: PageId, delta: -1 | 1): PageId {
  const i = PAGES.findIndex((p) => p.id === current);
  return PAGES[Math.max(0, Math.min(PAGES.length - 1, i + delta))].id;
}
