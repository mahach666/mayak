// Legacy Claude Code Allow/Deny (Settings → Advanced, off by default).
// The router forwards to the island only the requests it holds for an answer,
// each with a request_id, a `kind` and an `agent`: Claude permissions, Claude
// questions (AskUserQuestion) and, when enabled, Codex permissions. Only the
// Claude permission has a card here so far; the others are released at once.
// The relay sends no request without a session_id, and AskUserQuestion comes as
// a PreToolUse question, never as a permission (D87).

import { Bridge, onEvent } from "../core/bridge";
import type { HookAgent, HookCardKind } from "../core/bridge";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import { t } from "../core/i18n";
import { shouldOpenForAlert } from "./focus";
import type { Island } from "./island";

/** Clears the approval card if no decision was made before the hook gave up. */
let pendingTimeout: number | null = null;

interface HookPayload {
  hook_event_name?: string;
  request_id?: string;
  session_id?: string;
  cwd?: string;
  tool_name?: string;
  tool_input?: Record<string, unknown>;
  /** Router annotations (null when the observer has no match). */
  session_key?: string | null;
  project_name?: string | null;
  kind?: HookCardKind;
  agent?: HookAgent;
  /** A Claude permission with a rule to keep: "Always" may be offered. */
  canAlways?: boolean;
}

const PROJECT_ALIASES: Record<string, string> = {
  "notch-buddy": "Notch Buddy",
  notchbuddy: "Notch Buddy",
  notch_buddy: "Notch Buddy",
};

function aliasProjectName(name: string): string {
  return PROJECT_ALIASES[name.toLowerCase()] ?? name;
}

function lastPathComponent(p: string): string {
  const cleaned = p.replace(/[\\/]+$/, "");
  const idx = Math.max(cleaned.lastIndexOf("\\"), cleaned.lastIndexOf("/"));
  return idx >= 0 ? cleaned.slice(idx + 1) : cleaned;
}

/**
 * What the Allow button actually authorises. Approving "Write" tells you nothing
 * — approving `Write · C:\…\.env` tells you everything, and the difference is
 * the whole point of approving from the island rather than blind.
 *
 * Ordered by how specific the field is, so an unfamiliar tool still shows
 * whatever identifying string it carries instead of falling back to its name.
 */
const APPROVAL_FIELDS = [
  "command", // Bash, PowerShell
  "file_path", // Write, Edit, MultiEdit, NotebookEdit
  "path", // Read, LS
  "url", // WebFetch
  "query", // WebSearch
  "pattern", // Glob, Grep
  "prompt", // Task
] as const;

function approvalTarget(tool: string, input: Record<string, unknown>): string {
  for (const field of APPROVAL_FIELDS) {
    const value = input[field];
    if (typeof value === "string" && value.trim()) {
      return `${tool} · ${value.trim()}`;
    }
  }
  return tool;
}

export function registerHookHandlers(island: Island) {
  void onEvent<HookPayload>("hook", (payload) => handleHook(island, payload));
}

function handleHook(island: Island, payload: HookPayload) {
  const requestId = payload.request_id ?? "";
  if (!requestId) return;
  if (payload.kind !== "permission" || payload.agent !== "claude" || payload.hook_event_name !== "PermissionRequest") {
    // No card for this kind yet: the agent asks in its own window right away.
    void Bridge.approvalDecline(requestId);
    return;
  }

  const letWaits = State.settings.focusLetWaitsThrough;
  if (!shouldOpenForAlert("permission", { muting: State.muting, paused: State.paused, letWaits })) {
    // Silence here used to cost Claude Code nearly two minutes: the relay waited
    // for a decision from an island that had already decided not to look. Say so,
    // and the terminal takes the question immediately. A muted work interval that holds waits back
    // does not record it here: the observer's alert for the same wait, when one comes,
    // is what Island.handleAlerts records, so the wait is not counted twice.
    void Bridge.approvalDecline(requestId);
    return;
  }
  // One card, one request. A second one must never quietly replace the first
  // — that would leave a human staring at request B while request A waits for
  // a decision nobody can give. Hand it straight back to the terminal.
  if (State.pendingApproval && State.pendingApproval.requestId !== requestId) {
    void Bridge.approvalDecline(requestId);
    return;
  }

  if (pendingTimeout != null) window.clearTimeout(pendingTimeout);
  const tool = payload.tool_name ?? t("approval.tool");
  const input = payload.tool_input ?? {};
  const cwdName = aliasProjectName(lastPathComponent(payload.cwd ?? "") || t("approval.session"));
  State.pendingApproval = {
    requestId,
    sessionId: payload.session_id ?? "",
    tool,
    command: approvalTarget(tool, input),
    sessionKey: payload.session_key ?? null,
    projectName: payload.project_name || cwdName,
  };
  // The relay's short ack window closes in 800 ms; everything below this
  // line is synchronous, so the card really is up by the time it lands.
  void Bridge.approvalAck(requestId);
  State.isPinned = true;
  Sound.play("approval");
  // Opens the card, or replaces a waiting card of the same session in place.
  island.alert("approval");
  // The card stays up 110 s, as before; then it is handed back, so Claude Code
  // asks in the terminal at once instead of waiting out Mayak's 590 s.
  pendingTimeout = window.setTimeout(() => {
    pendingTimeout = null;
    if (!State.pendingApproval) return;
    void Bridge.approvalDecline(State.pendingApproval.requestId);
    State.pendingApproval = null;
    State.isPinned = false;
    island.dropPin();
    // A phase card that came while this request waited shows now.
    if (!island.releaseHeldCard() && State.view === "approval") island.setView(State.defaultView());
    State.notify();
  }, 110_000);
  State.notify();
}
