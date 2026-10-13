// The CLI chat (src-tauri/src/chat): the wire types and the pure choices of its view. No DOM, no
// Tauri and no State, so Node can run these functions as they are (scripts/checks/agent-chat.mjs).
// The chat runs the user's own agent CLIs; nothing here knows a key.

import type { Key } from "./i18n";

/** Wire names of Rust `chat::AgentKind`. */
export type AgentKind = "claudeCode" | "codex" | "geminiCli" | "cursorAgent";

export const AGENT_KINDS: readonly AgentKind[] = ["claudeCode", "codex", "geminiCli", "cursorAgent"];

/** An empty `id` is the CLI's own default model. */
export interface ModelChoice {
  id: string;
  /** A product name; empty for the default. */
  label: string;
}

export type Login = "ready" | "notLoggedIn" | "unknown";

/** Rust `chat::AgentInfo`. */
export interface AgentInfo {
  kind: AgentKind;
  label: string;
  /** The CLI is on PATH. */
  found: boolean;
  /** False: written from the CLI's documentation, never run. */
  verified: boolean;
  verifiedWith: string | null;
  models: ModelChoice[];
  /** The access modes the user may pick; the first is the default. */
  modes: string[];
  login: Login;
}

export type ConvoState = "idle" | "working" | "waiting" | "failed";

/** Rust `chat::Summary`. */
export interface ConvoSummary {
  id: number;
  agent: AgentKind;
  cwd: string;
  title: string;
  state: ConvoState;
  rev: number;
}

export type ToolStatus = "running" | "ok" | "failed";
export type PermissionStatus = "pending" | "allowed" | "denied" | "cancelled";

/** Rust `chat::convo::Msg`, flat with a `kind` tag. */
export type ChatMsg =
  | { id: number; kind: "user"; text: string }
  | { id: number; kind: "assistant"; text: string }
  | { id: number; kind: "tool"; toolId: string; name: string; summary: string; status: ToolStatus; output: string | null }
  | {
      id: number; kind: "permission"; requestId: string; tool: string; summary: string;
      detail: string | null; toolUseId: string | null; status: PermissionStatus;
    }
  | { id: number; kind: "notice"; code: string; text: string }
  | { id: number; kind: "error"; code: string; text: string };

/** Rust `chat::Convo`. */
export interface Convo {
  id: number;
  agent: AgentKind;
  cwd: string;
  model: string;
  mode: string;
  title: string;
  /** The CLI's own id of the conversation. */
  session: string | null;
  usedModel: string | null;
  state: ConvoState;
  /** A line shown while a turn goes on (a reconnect). */
  status: string | null;
  loginHint: boolean;
  rev: number;
  messages: ChatMsg[];
}

/** Rust `chat::OpenRequest`. An empty `cwd` is the home folder. */
export interface OpenRequest {
  agent: AgentKind;
  cwd: string;
  model: string;
  mode: string;
  resume: string | null;
}

/** What Rust rejects with: a stable code and the English sentence. */
export interface ChatError {
  code: string;
  message: string;
}

/** The payload of the `agent-chat-permission` event. */
export interface PermissionEvent {
  conv: number;
  requestId: string;
  tool: string;
}

/** What a rejected call carries: Rust's `{code, message}`, an Error, or a bare string. */
export function normalizeError(err: unknown): ChatError {
  if (typeof err === "object" && err !== null) {
    const e = err as Record<string, unknown>;
    if (typeof e.code === "string" && typeof e.message === "string") return { code: e.code, message: e.message };
    if (typeof e.message === "string") return { code: "internal", message: e.message };
  }
  return { code: "internal", message: typeof err === "string" ? err : "Something went wrong." };
}

/** The CLI's command name on PATH, as the error texts and the sign-in hint name it. */
export function agentCommand(kind: AgentKind): string {
  switch (kind) {
    case "claudeCode": return "claude";
    case "codex": return "codex";
    case "geminiCli": return "gemini";
    case "cursorAgent": return "cursor-agent";
  }
}

/** What to type in a terminal to sign in; null where it is not known (the CLI's own message guides). */
export function loginCommand(kind: AgentKind): string | null {
  switch (kind) {
    case "claudeCode": return "claude auth login";
    case "codex": return "codex login";
    case "cursorAgent": return "cursor-agent login";
    case "geminiCli": return null;
  }
}

/** The dot of a conversation: the colours of the island's own states (core/layout.ts botGlowColor). */
export function stateColor(state: ConvoState): string {
  switch (state) {
    case "working": return "#3B9EFF";
    case "waiting": return "#F5A524";
    case "failed": return "#F4505E";
    case "idle": return "rgba(255,255,255,0.4)";
  }
}

/** The agent to start with: the saved one while it is still installed, else the first installed one. */
export function chooseAgentKind(agents: readonly AgentInfo[], saved: AgentKind | null): AgentKind | null {
  const found = agents.filter((a) => a.found);
  return found.find((a) => a.kind === saved)?.kind ?? found[0]?.kind ?? null;
}

/** The model to keep when the agent changes: the same id if the new agent has it, else its default. */
export function keepModel(agent: AgentInfo | undefined, model: string): string {
  return agent?.models.some((m) => m.id === model) ? model : "";
}

/** The mode to keep when the agent changes: the same one if the new agent has it, else its first. */
export function keepMode(agent: AgentInfo | undefined, mode: string): string {
  const modes = agent?.modes ?? [];
  return modes.includes(mode) ? mode : modes[0] ?? "";
}

/** The conversation to show after the list changed: the open one while it exists, else none (a new chat). */
export function reconcileActive(list: readonly ConvoSummary[], active: number | null): number | null {
  return active !== null && list.some((c) => c.id === active) ? active : null;
}

/** A conversation is running a turn: a message cannot be sent, a stop can. */
export function isBusy(state: ConvoState | undefined): boolean {
  return state === "working" || state === "waiting";
}

/** Whether the send button works: a message, an agent to send it to, nothing running, no call under way. */
export function canSend(text: string, state: ConvoState | undefined, hasAgent: boolean, sending: boolean): boolean {
  return text.trim() !== "" && hasAgent && !isBusy(state) && !sending;
}

/** The tab's title: the conversation's first line, or "New chat" (the caller's word) while it has none. */
export function tabTitle(c: { title: string }, fallback: string): string {
  const t = c.title.trim();
  return t === "" ? fallback : t;
}

/** The last path component, for a project folder shown by name. */
export function folderName(path: string): string {
  const cleaned = path.replace(/[\\/]+$/, "");
  const i = Math.max(cleaned.lastIndexOf("\\"), cleaned.lastIndexOf("/"));
  return i >= 0 ? cleaned.slice(i + 1) || cleaned : cleaned;
}

/**
 * What decides whether a message's line must be drawn again: the same text means the same DOM. A
 * growing answer, a tool that ended and a request that was answered each change it.
 */
export function messageSig(m: ChatMsg): string {
  switch (m.kind) {
    case "user":
    case "assistant":
      return `${m.kind}:${m.text.length}:${m.text.slice(-24)}`;
    case "tool":
      return `tool:${m.status}:${m.summary}:${m.output ?? ""}`;
    case "permission":
      return `permission:${m.status}`;
    case "notice":
    case "error":
      return `${m.kind}:${m.code}:${m.text}`;
  }
}

/** Permission requests still waiting for a click, oldest first. */
export function pendingPermissions(c: Convo | null): Extract<ChatMsg, { kind: "permission" }>[] {
  return (c?.messages ?? []).filter((m): m is Extract<ChatMsg, { kind: "permission" }> => m.kind === "permission" && m.status === "pending");
}

/** The word for each access mode the adapters offer (Rust `Adapter::modes`). */
const MODE_KEYS: Record<string, Key> = {
  ask: "agentchat.mode.ask",
  plan: "agentchat.mode.plan",
  readOnly: "agentchat.mode.readOnly",
  workspaceWrite: "agentchat.mode.workspaceWrite",
  proposeOnly: "agentchat.mode.proposeOnly",
};

/** The dictionary key of a mode; null for one this page has no word for (it shows the id). */
export function modeKey(mode: string): Key | null {
  return Object.hasOwn(MODE_KEYS, mode) ? MODE_KEYS[mode] : null;
}
