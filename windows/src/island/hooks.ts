// Answers from the island (spec §2): the permission and question cards.
// The router forwards to the island only the requests it holds for an answer,
// each with a request_id, a `kind` and an `agent`: Claude permissions, Claude
// questions (AskUserQuestion) and, when enabled, Codex permissions.
// The relay sends no request without a session_id, and AskUserQuestion comes as
// a PreToolUse question, never as a permission (D87).
//
// The card is an ordinary alert: not pinned, it folds after autoCloseInterval with
// the cursor off the island. Whenever the island leaves the card (a fold, another
// view, the auto-close), the agent is released once and asks in its own window.

import { Bridge, onEvent } from "../core/bridge";
import type { HookAgent, HookCardKind } from "../core/bridge";
import type { IslandViewName } from "../core/layout";
import { Sound } from "../core/sound";
import { State, type ApprovalInfo } from "../core/state";
import { t } from "../core/i18n";
import { parseQuestions, withdrawsCard } from "./answers";
import { shouldOpenForAlert } from "./focus";
import type { Island } from "./island";

/**
 * The card's upper bound, with the cursor kept on the island: Rust stops waiting at 590 s
 * (pipe.rs DECISION_TIMEOUT), after which a click could no longer reach the agent.
 */
const CARD_LIMIT_MS = 590_000;

let limitTimer: number | null = null;
/** request_id of the card once its view has been on screen; leaving it after that releases it. */
let shownId: string | null = null;

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

/** The view a card is shown on. */
export function cardView(card: ApprovalInfo): IslandViewName {
  return card.kind === "question" ? "question" : "approval";
}

export function registerHookHandlers(island: Island) {
  void onEvent<HookPayload>("hook", (payload) => handleHook(island, payload));
  void onEvent<{ request_id?: string }>("hook-withdraw", (p) => withdrawCard(island, p.request_id ?? ""));
  State.subscribe(() => watchCard());
}

/**
 * Rust took the card back: the question of an MCP agent (src-tauri mcp_server) ran out of time, or
 * its server stopped. The request is over on Rust's side, so nothing is sent to it: the card is
 * cleared as a button would clear it, and a click on it could only have gone nowhere. A card that
 * is not the one on screen (answered since, or never shown) is left alone.
 */
function withdrawCard(island: Island, requestId: string) {
  const card = State.pendingApproval;
  if (!card || !withdrawsCard(card.requestId, requestId)) return;
  const onCard = State.mode === "expanded" && State.view === cardView(card);
  if (limitTimer != null) window.clearTimeout(limitTimer);
  limitTimer = null;
  shownId = null;
  State.pendingApproval = null;
  void Bridge.log(`card withdrawn req=${requestId}`);
  State.notify();
  if (onCard) island.closeCard();
}

/**
 * Every State change: once the card has been on screen, an island that no longer shows it
 * (folded, another view, auto-closed) releases the agent. A button clears the card first,
 * so an answered card is never released as well.
 */
function watchCard() {
  const card = State.pendingApproval;
  if (!card) {
    // Answered by a button (Island.closeCard), released, or none at all.
    shownId = null;
    if (limitTimer != null) window.clearTimeout(limitTimer);
    limitTimer = null;
    return;
  }
  if (State.mode === "expanded" && State.view === cardView(card)) {
    shownId = card.requestId;
    return;
  }
  // Still opening: alert() passes through the default view on its way to the card.
  if (shownId !== card.requestId) return;
  void Bridge.log(`card left req=${card.requestId}`);
  releaseCard();
}

/** Releases the agent of the card on the island, without a decision, and forgets the card. */
function releaseCard() {
  const card = State.pendingApproval;
  if (!card) return;
  if (limitTimer != null) window.clearTimeout(limitTimer);
  limitTimer = null;
  shownId = null;
  State.pendingApproval = null;
  void Bridge.approvalDecline(card.requestId);
  State.notify();
}

/**
 * Releases the card from outside the island's own moves: pause (the tray's or the island's quiet
 * button, spec §2.1) and the 590 s limit. A card still on screen is left as a button would leave it.
 */
export function releaseOpenCard(island: Island) {
  const card = State.pendingApproval;
  if (!card) return;
  const onCard = State.mode === "expanded" && State.view === cardView(card);
  void Bridge.log(`card released req=${card.requestId}`);
  releaseCard();
  if (onCard) island.closeCard();
}

/** Accepted by the router as a card: a Claude or Codex permission, or a Claude question. */
function isCard(payload: HookPayload): boolean {
  if (payload.kind === "permission") {
    return payload.hook_event_name === "PermissionRequest" && (payload.agent === "claude" || payload.agent === "codex");
  }
  return payload.kind === "question" && payload.agent === "claude" && payload.hook_event_name === "PreToolUse";
}

function handleHook(island: Island, payload: HookPayload) {
  const requestId = payload.request_id ?? "";
  if (!requestId) return;
  if (!isCard(payload)) {
    // No card for this request: the agent asks in its own window right away.
    void Bridge.approvalDecline(requestId);
    return;
  }
  const isQuestion = payload.kind === "question";
  // A question the card cannot show as it is (spec §2.3): the agent asks it itself.
  const questions = isQuestion ? parseQuestions(payload.tool_input) : [];
  if (questions === null) {
    void Bridge.approvalDecline(requestId);
    return;
  }

  const letWaits = State.settings.focusLetWaitsThrough;
  if (!shouldOpenForAlert(isQuestion ? "input" : "permission", { muting: State.muting, paused: State.paused, letWaits })) {
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
  // a decision nobody can give. Hand it straight back to the agent.
  if (State.pendingApproval && State.pendingApproval.requestId !== requestId) {
    void Bridge.approvalDecline(requestId);
    return;
  }

  if (limitTimer != null) window.clearTimeout(limitTimer);
  const agent = payload.agent === "codex" ? "codex" : "claude";
  const tool = payload.tool_name ?? t("approval.tool");
  const input = payload.tool_input ?? {};
  const cwdName = aliasProjectName(lastPathComponent(payload.cwd ?? "") || t("approval.session"));
  const card: ApprovalInfo = {
    requestId,
    sessionId: payload.session_id ?? "",
    tool,
    command: approvalTarget(tool, input),
    sessionKey: payload.session_key ?? null,
    projectName: payload.project_name || cwdName,
    kind: isQuestion ? "question" : "permission",
    agent,
    // Only Claude can keep a rule; Rust degrades "always" to "allow" anyway.
    canAlways: agent === "claude" && !isQuestion && payload.canAlways === true,
    questions,
  };
  State.pendingApproval = card;
  // The relay's short ack window closes in 800 ms; everything below this
  // line is synchronous, so the card really is up by the time it lands.
  void Bridge.approvalAck(requestId);
  Sound.play(isQuestion ? "question" : "approval");
  // Opens the card, not pinned: it folds like any alert, and folding releases it.
  island.alert(cardView(card));
  island.armAutoClose();
  limitTimer = window.setTimeout(() => {
    limitTimer = null;
    if (State.pendingApproval?.requestId !== requestId) return;
    releaseOpenCard(island);
  }, CARD_LIMIT_MS);
  State.notify();
}
