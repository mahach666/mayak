// The CLI chat (src-tauri/src/chat): the wire types and the pure choices of its view. No DOM, no
// Tauri and no State, so Node can run these functions as they are (scripts/checks/agent-chat.mjs).
// The chat runs the user's own agent CLIs; nothing here knows a key.

import type { Key } from "./i18n";
import type { BotStateName } from "./layout";

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
  /** What the agent itself says it is doing (the MCP server's set_status): for the running turn only. */
  agentNote?: string | null;
  loginHint: boolean;
  /** When the turn under way, or the last one, started and ended (ms since the epoch, Rust's clock). */
  turnStarted: number | null;
  turnEnded: number | null;
  /** The tokens the CLI's last request sent, and the model's context window, when the CLI said them. */
  contextTokens: number | null;
  contextWindow: number | null;
  /** The plan's usage windows as the CLI last reported them (Claude Code); empty when it never did. */
  limits: PlanWindow[];
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

/** The agent's own line under Luch: shown only while its turn runs; "" for none. */
export function agentNoteText(c: Pick<Convo, "state" | "agentNote"> | null): string {
  if (!c || !isBusy(c.state)) return "";
  return (c.agentNote ?? "").trim();
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
export function pendingPermissions(c: Pick<Convo, "messages"> | null): Extract<ChatMsg, { kind: "permission" }>[] {
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

const MODE_HINT_KEYS: Record<string, Key> = {
  ask: "agentchat.modeHint.ask",
  plan: "agentchat.modeHint.plan",
  readOnly: "agentchat.modeHint.readOnly",
  workspaceWrite: "agentchat.modeHint.workspaceWrite",
  proposeOnly: "agentchat.modeHint.proposeOnly",
};

const MODE_SHORT_KEYS: Record<string, Key> = {
  ask: "agentchat.modeShort.ask",
  plan: "agentchat.modeShort.plan",
  readOnly: "agentchat.modeShort.readOnly",
  workspaceWrite: "agentchat.modeShort.workspaceWrite",
  proposeOnly: "agentchat.modeShort.proposeOnly",
};

/** The one-word name of a mode, for the composer's pill; null for a mode this page does not know. */
export function modeShortKey(mode: string): Key | null {
  return Object.hasOwn(MODE_SHORT_KEYS, mode) ? MODE_SHORT_KEYS[mode] : null;
}

/**
 * The colour of a mode's pill (text, and the same colour faint for its ground): green where every tool
 * asks for a click, blue where nothing is changed, and amber where files may be edited, so the
 * widest access is the one that stands out. A mode this page does not know is neutral.
 */
export function modeAccent(mode: string): { color: string; ground: string } {
  switch (mode) {
    case "ask": return { color: "#6EE7A8", ground: "rgba(52,211,153,0.17)" };
    case "plan": return { color: "#93A4FF", ground: "rgba(99,102,241,0.2)" };
    case "readOnly": return { color: "#67E0F2", ground: "rgba(34,211,238,0.15)" };
    case "workspaceWrite": return { color: "#F6BD5A", ground: "rgba(245,165,36,0.17)" };
    case "proposeOnly": return { color: "#C4B0FB", ground: "rgba(167,139,250,0.18)" };
    default: return { color: "#B0B5BE", ground: "rgba(255,255,255,0.09)" };
  }
}

/** The dictionary key of the sentence that says what a mode lets the agent do; null for a mode this page does not know. */
export function modeHintKey(mode: string): Key | null {
  return Object.hasOwn(MODE_HINT_KEYS, mode) ? MODE_HINT_KEYS[mode] : null;
}


/** The tool the Mayak MCP server's `ask_user` goes by in a chat: the agent is waiting for an answer on the island. */
const ASK_USER_TOOL = "mcp__mayak__ask_user";

/**
 * What Luch shows while this chat is on: the state of the island's own set that fits the run.
 * A request or a question waits for a click; a turn goes on (a tool running is "working", the answer
 * being written is "thinking"); a failed turn is an error, or the plan limit when the CLI said so;
 * a turn that just ended is "finished" for a moment (`justFinished`, the view times it).
 */
export function chatBotState(c: Pick<Convo, "state" | "messages"> | null, justFinished: boolean): BotStateName {
  if (c === null) return "idle";
  const running = c.messages.filter((m) => m.kind === "tool" && m.status === "running");
  if (c.state === "waiting" || pendingPermissions(c).length > 0) return "approval";
  if (c.state === "working") {
    if (running.some((m) => m.kind === "tool" && m.name === ASK_USER_TOOL)) return "question";
    return running.length > 0 ? "working" : "thinking";
  }
  if (c.state === "failed") {
    const last = [...c.messages].reverse().find((m) => m.kind === "error");
    return last?.kind === "error" && last.code === "rateLimit" ? "ratelimit" : "error";
  }
  return justFinished ? "finished" : "idle";
}

/** A tool's name as a chip shows it: an MCP tool is "server · tool" (mcp__mayak__ask_user is "mayak · ask_user"). */
export function toolLabel(name: string): string {
  const m = /^mcp__(.+?)__(.+)$/.exec(name);
  return m ? `${m[1]} · ${m[2]}` : name;
}

/** Rust `chat::agent::PlanWindow`: one of the plan's windows ("five_hour", "seven_day"), used 0 to 1. */
export interface PlanWindow {
  kind: string;
  used: number;
  /** Seconds since the epoch. */
  resetsAt: number | null;
}

/** The tool calls of the turn under way, or of the last one: the tool lines since the user's last message. */
export function turnTools(c: Pick<Convo, "messages"> | null): number {
  const msgs = c?.messages ?? [];
  let n = 0;
  for (let i = msgs.length - 1; i >= 0 && msgs[i].kind !== "user"; i--) if (msgs[i].kind === "tool") n++;
  return n;
}

/** A turn's length on a clock: "0:07", "12:05", "1:02:03". */
export function clockText(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

/** A token count in a few characters: "950", "26k", "1.2M". */
export function compactTokens(n: number): string {
  if (n < 1000) return String(Math.max(0, Math.round(n)));
  if (n < 1_000_000) return `${Math.round(n / 1000)}k`;
  const m = n / 1_000_000;
  return `${m < 10 ? Math.round(m * 10) / 10 : Math.round(m)}M`;
}

/** What the line under Luch says about the open chat: a dictionary key and its values. */
export function sideStatus(c: Pick<Convo, "state" | "messages" | "turnEnded"> | null): { key: Key; vars?: Record<string, string> } {
  if (c === null) return { key: "agentchat.side.ready" };
  if (c.state === "waiting" || pendingPermissions(c).length > 0) return { key: "agentchat.side.waiting" };
  if (c.state === "working") {
    const running = [...c.messages].reverse().find((m) => m.kind === "tool" && m.status === "running");
    if (running?.kind === "tool") {
      return running.name === ASK_USER_TOOL ? { key: "agentchat.side.asking" } : { key: "agentchat.side.running", vars: { tool: toolLabel(running.name) } };
    }
    return { key: "agentchat.side.thinking" };
  }
  if (c.state === "failed") {
    const last = [...c.messages].reverse().find((m) => m.kind === "error");
    const code = last?.kind === "error" ? last.code : "";
    return { key: code === "rateLimit" ? "agentchat.side.limit" : code === "notLoggedIn" ? "agentchat.side.signedOut" : "agentchat.side.failed" };
  }
  const last = c.messages.at(-1);
  if (last?.kind === "notice" && last.code === "stopped") return { key: "agentchat.side.stopped" };
  return { key: c.turnEnded !== null ? "agentchat.side.done" : "agentchat.side.ready" };
}

/**
 * The field's text when the view goes from conversation `from` to `to` (null: the new chat): what was
 * typed for `from` is kept in `drafts`, and what was kept for `to`, if anything, comes back.
 */
export function swapDraft(drafts: Map<number | null, string>, from: number | null, to: number | null, typed: string): string {
  if (typed.trim() === "") drafts.delete(from);
  else drafts.set(from, typed);
  const next = drafts.get(to) ?? "";
  drafts.delete(to);
  return next;
}
