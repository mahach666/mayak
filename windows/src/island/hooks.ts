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
import { ApprovalQueue, remainingMs } from "./approval-queue";
import { resumeChatRequest } from "./agent-chat";
import { shouldOpenForAlert } from "./focus";
import type { Island } from "./island";

/**
 * The card's upper bound, with the cursor kept on the island: Rust stops waiting at 590 s
 * (pipe.rs DECISION_TIMEOUT), after which a click could no longer reach the agent.
 */
const CARD_LIMIT_MS = 590_000;

let limitTimer: number | null = null;
/**
 * Requests that came while a card was up, oldest first. Each one's agent keeps waiting for its
 * click, so each has its own limit timer (the Rust wait started when the request came).
 */
const waiting = new ApprovalQueue<ApprovalInfo>();
const waitingTimers = new Map<string, number>();
let promoteScheduled = false;
/** A card was on the island at the last State change: its end is when the next request comes up. */
let cardUp = false;
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
  State.subscribe(() => watchCard(island));
}

function syncWaiting() {
  State.approvalWaiting = waiting.size;
}

/** A waiting request that is over: out of the line with its timer. */
function dropWaiting(requestId: string): boolean {
  const timer = waitingTimers.get(requestId);
  if (timer != null) window.clearTimeout(timer);
  waitingTimers.delete(requestId);
  const removed = waiting.remove(requestId);
  if (removed) syncWaiting();
  return removed;
}

/** Hands every waiting request back to its agent, which asks in its own window. */
function releaseWaiting() {
  for (const card of waiting.drain()) {
    const timer = waitingTimers.get(card.requestId);
    if (timer != null) window.clearTimeout(timer);
    waitingTimers.delete(card.requestId);
    void Bridge.log(`card released req=${card.requestId} (waiting)`);
    void Bridge.approvalDecline(card.requestId);
  }
  syncWaiting();
}

/**
 * Hands the card on screen and every waiting request back to their agents, without a decision:
 * for a change the cards would not survive (the page reload of a new language).
 */
export async function declineAllApprovals() {
  const card = State.pendingApproval;
  State.pendingApproval = null;
  const queued = waiting.drain();
  for (const q of queued) {
    const timer = waitingTimers.get(q.requestId);
    if (timer != null) window.clearTimeout(timer);
    waitingTimers.delete(q.requestId);
  }
  syncWaiting();
  await Promise.all([card, ...queued].filter((c) => c != null).map((c) => Bridge.approvalDecline(c.requestId)));
}

/**
 * Rust took the card back: the question of an MCP agent (src-tauri mcp_server) ran out of time, or
 * its server stopped. The request is over on Rust's side, so nothing is sent to it: the card is
 * cleared as a button would clear it, and a click on it could only have gone nowhere. A card that
 * is not the one on screen (answered since, or never shown) is left alone.
 */
function withdrawCard(island: Island, requestId: string) {
  if (dropWaiting(requestId)) {
    void Bridge.log(`card withdrawn req=${requestId} (waiting)`);
    State.notify();
    return;
  }
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
function watchCard(island: Island) {
  const card = State.pendingApproval;
  if (!card) {
    // Answered by a button (Island.closeCard), released, or none at all.
    shownId = null;
    if (limitTimer != null) window.clearTimeout(limitTimer);
    limitTimer = null;
    // The next request in line comes up once this State change is through: a hook's or an MCP
    // agent's, else a request of the CLI chat that this card kept off the island.
    const ended = cardUp;
    cardUp = false;
    if ((ended || waiting.size > 0) && !promoteScheduled) {
      promoteScheduled = true;
      queueMicrotask(() => {
        promoteScheduled = false;
        showNext(island);
      });
    }
    return;
  }
  cardUp = true;
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
  // A card let go of (folded, another view, paused) brings nothing else up in its place.
  cardUp = false;
  State.pendingApproval = null;
  void Bridge.approvalDecline(card.requestId);
  // Folding or pausing the island is a "not now" for every request, not for one: the others go
  // back to their agents too, rather than pop up again one after another.
  releaseWaiting();
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

  // One card on screen, and a second request must never quietly replace it — that would leave a
  // human staring at request B while request A waits for a decision nobody can give. It waits in
  // line behind the card instead (its agent keeps waiting for it), up to QUEUE_MAX; past that,
  // or for a request already known, it goes straight back to the agent.
  if (State.pendingApproval?.requestId === requestId || waiting.has(requestId)) return;
  if (State.pendingApproval != null || waiting.size > 0) {
    const now = Date.now();
    if (!waiting.push(card, now, State.pendingApproval != null ? 1 : 0)) {
      void Bridge.approvalDecline(requestId);
      return;
    }
    // The relay's short ack window closes in 800 ms: it is told now, the card comes up later.
    void Bridge.approvalAck(requestId);
    waitingTimers.set(
      requestId,
      window.setTimeout(() => {
        waitingTimers.delete(requestId);
        if (!waiting.remove(requestId)) return;
        syncWaiting();
        void Bridge.log(`card released req=${requestId} (waiting, limit)`);
        void Bridge.approvalDecline(requestId);
        State.notify();
      }, CARD_LIMIT_MS),
    );
    syncWaiting();
    State.notify();
    return;
  }
  showCard(island, card, CARD_LIMIT_MS);
  // The relay's short ack window closes in 800 ms; showCard is synchronous, so the card really is
  // up by the time it lands.
  void Bridge.approvalAck(requestId);
}

/** Puts a card on the island, for `limitMs` more at most with the cursor kept on it. */
function showCard(island: Island, card: ApprovalInfo, limitMs: number) {
  const requestId = card.requestId;
  if (limitTimer != null) window.clearTimeout(limitTimer);
  State.pendingApproval = card;
  Sound.play(card.kind === "question" ? "question" : "approval");
  // Opens the card, not pinned: it folds like any alert, and folding releases it.
  island.alert(cardView(card));
  island.armAutoClose();
  limitTimer = window.setTimeout(() => {
    limitTimer = null;
    if (State.pendingApproval?.requestId !== requestId) return;
    releaseOpenCard(island);
  }, limitMs);
  State.notify();
}

/** The same rule as for a request that comes now (handleHook). */
function mayAlert(card: ApprovalInfo): boolean {
  const letWaits = State.settings.focusLetWaitsThrough;
  return shouldOpenForAlert(card.kind === "question" ? "input" : "permission", { muting: State.muting, paused: State.paused, letWaits });
}

/** The card on screen is gone: the oldest request still in time takes its place. */
function showNext(island: Island) {
  if (State.pendingApproval != null) return;
  for (let next = waiting.shift(); next; next = waiting.shift()) {
    const timer = waitingTimers.get(next.card.requestId);
    if (timer != null) window.clearTimeout(timer);
    waitingTimers.delete(next.card.requestId);
    const left = remainingMs(next.arrivedAt, Date.now(), CARD_LIMIT_MS);
    // Out of time, or alerts held back since it came (a muted pomodoro): its agent asks itself.
    if (left <= 0 || !mayAlert(next.card)) {
      void Bridge.log(`card released req=${next.card.requestId} (waiting, ${left <= 0 ? "limit" : "held back"})`);
      void Bridge.approvalDecline(next.card.requestId);
      continue;
    }
    syncWaiting();
    showCard(island, next.card, left);
    return;
  }
  syncWaiting();
  State.notify();
  // Nothing of a hook's left: a request of the CLI chat may have been waiting behind the cards.
  void resumeChatRequest(island);
}
