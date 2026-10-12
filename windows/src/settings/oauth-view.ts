// What the Connect controls of the GitHub and GitLab integrations say (src-tauri/src/oauth.rs).
// Pure, so the Node checks can see every state and every problem code maps to a text.

import type { Key } from "../core/i18n";
import type { ConnState, OAuthResult } from "../core/bridge";

export interface Notice {
  tone: "ok" | "warn" | "err";
  key: Key;
}

/** The account line: no token, connected, or a refresh token the service refused. */
export function connText(state: ConnState): Key {
  if (state === "connected") return "set.oauthStateConnected";
  if (state === "reconnect") return "set.oauthStateReconnect";
  return "set.oauthStateNone";
}

/** The account line's dot: green connected, amber needs reconnecting, red nothing. */
export function connColor(state: ConnState): string {
  if (state === "connected") return "#22c55e";
  if (state === "reconnect") return "#f5a524";
  return "#f4505e";
}

/** Connect, or Reconnect once there is (or was) a token. */
export function connectLabel(state: ConnState): Key {
  return state === "none" ? "set.oauthConnect" : "set.oauthReconnect";
}

const START_PROBLEMS: Record<string, Notice> = {
  notRegistered: { tone: "warn", key: "set.oauthNotRegistered" },
  selfManaged: { tone: "warn", key: "set.oauthSelfManaged" },
  invalidId: { tone: "err", key: "set.oauthInvalidId" },
  invalidHost: { tone: "err", key: "set.oauthInvalidHost" },
  paused: { tone: "warn", key: "set.oauthPaused" },
  failed: { tone: "err", key: "set.oauthError" },
};

/** The notice for a code `oauth_start` refused with (oauth::StartProblem::code); anything else is a failure. */
export function startProblem(code: string): Notice {
  return Object.hasOwn(START_PROBLEMS, code) ? START_PROBLEMS[code] : START_PROBLEMS.failed;
}

/** The notice for how a flow ended (an `oauth` event). */
export function flowNotice(state: OAuthResult["state"]): Notice {
  switch (state) {
    case "connected":
      return { tone: "ok", key: "set.oauthDone" };
    case "disconnected":
      return { tone: "ok", key: "set.oauthDisconnected" };
    case "expired":
      return { tone: "warn", key: "set.oauthExpired" };
    case "denied":
      return { tone: "err", key: "set.oauthDenied" };
    default:
      return { tone: "err", key: "set.oauthError" };
  }
}
