// Legacy Claude Code Allow/Deny (Settings → Advanced, off by default).
// The router forwards to the island only Claude PermissionRequest payloads that
// carry a request_id (spec §5.3); every other Claude event is read by the
// observer, and Codex never gets an answer from the island. The relay sends no
// request without a session_id and none for AskUserQuestion (D87).

import { Bridge, onEvent } from "../core/bridge";
import { Sound } from "../core/sound";
import { State } from "../core/state";
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
  if (payload.hook_event_name !== "PermissionRequest" || !requestId) return;

  const letWaits = State.settings.focusLetWaitsThrough;
  if (!shouldOpenForAlert("permission", { muting: State.focusOn, paused: State.paused, letWaits })) {
    // Silence here used to cost Claude Code nearly two minutes: the relay waited
    // for a decision from an island that had already decided not to look. Say so,
    // and the terminal takes the question immediately. A focus that holds waits back
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
  const tool = payload.tool_name ?? "Tool";
  const input = payload.tool_input ?? {};
  const cwdName = aliasProjectName(lastPathComponent(payload.cwd ?? "") || "Session");
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
  // Mayak answers within 108 s or not at all; after that the terminal has
  // taken over and the card would be lying.
  pendingTimeout = window.setTimeout(() => {
    pendingTimeout = null;
    if (!State.pendingApproval) return;
    State.pendingApproval = null;
    State.isPinned = false;
    island.dropPin();
    // A focus that ended while this request waited shows its summary now.
    if (!island.flushFocusDone() && State.view === "approval") island.setView(State.defaultView());
    State.notify();
  }, 110_000);
  State.notify();
}
