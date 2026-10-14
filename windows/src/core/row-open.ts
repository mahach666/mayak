// The «↗» of a sessions-list row (ideas §43): «↗ Open» of the session card in one press, without
// the card. A pure choice with no DOM, no Tauri and no State, so it can be checked on its own.

import type { Host } from "./bridge";
import type { Translate } from "./i18n-core";
import type { Key } from "./i18n";

/**
 * Whether the row of `key` gets the button: every row that Open (and Enter) can open. A pure
 * silent Codex row (`silent:<id>`) is no real session and opens nothing; a «Recent» row
 * (`recent:…`) has its own Resume and Copy buttons, and Agent Orchestrator's fold (`ao:…`) opens
 * and closes. A known session whose turn ran without hooks keeps its key and opens like any other row.
 */
export function rowOpens(key: string): boolean {
  return !key.startsWith("silent:") && !key.startsWith("recent:") && !key.startsWith("ao:");
}

/**
 * The button's tooltip: "Open in Cursor" with the host's short name, plain "Open" for an unknown host,
 * and "Open in the Agents chat" for a session Mayak's own chat runs (`chat`: it opens there).
 */
export function rowOpenTitle(host: Host, hostName: string, tr: Translate<Key>, chat = false): string {
  if (chat) return tr("sess.openChat");
  return host === "unknown" ? tr("common.open") : tr("sess.openIn", { host: hostName });
}
