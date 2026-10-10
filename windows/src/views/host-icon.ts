// The styled icon id (app-icon-art.ts) of a session's host, for the badge of a session row
// (ideas §41). Pure: no DOM, no state; a Node check runs it as is.

import type { Host } from "../core/bridge.ts";

const HOST_ICON: Readonly<Record<Host, string | null>> = {
  claudeDesktop: "claude",
  cursor: "cursor",
  vsCode: "vscode",
  codexDesktop: "codex",
  antigravityIde: "antigravity",
  // agy is Antigravity's own CLI: its icon is Antigravity's.
  antigravityCli: "antigravity",
  // A terminal or anything else: the snapshot does not say which, so the row keeps its letter.
  unknown: null,
};

/** The app icon id for a session host, or null when the badge keeps the agent's letter. */
export function hostIconId(host: Host): string | null {
  return Object.hasOwn(HOST_ICON, host) ? HOST_ICON[host] : null;
}
