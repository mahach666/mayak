// Thin wrapper over the Tauri commands/events. Every call is a no-op when the
// page is opened in a plain browser, so the island can be iterated on with
// `npm run dev` alone.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { Settings } from "./state";

export const IS_TAURI =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  if (!IS_TAURI) return null;
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    console.error(`[mayak] ${cmd} failed`, err);
    return null;
  }
}

export interface BootInfo {
  settings: Settings;
  /** Logical screen rect of the monitor the island lives on. */
  screen: { x: number; y: number; width: number; height: number; scale: number };
  version: string;
  hookPath: string;
  /** False where the OS has no global cursor (Wayland): see Island.followPageCursor. */
  cursorPoll: boolean;
}

export const Bridge = {
  boot: () => call<BootInfo>("boot"),

  saveSettings: (settings: Settings) => call<void>("save_settings", { settings }),

  /** Shrink the window down to the invisible wake strip (hidden) or back to full. */
  setCollapsed: (collapsed: boolean) => call<void>("set_collapsed", { collapsed }),

  /**
   * Pushes the island shape in window coordinates. Rust flips click-through from
   * its own cursor poll, so the flag is never a frame behind a click.
   */
  setIslandRect: (x: number, y: number, width: number, height: number) =>
    call<void>("set_island_rect", { x, y, width, height }),

  /** Give the window keyboard focus (chat field) and take it away again. */
  focusWindow: (focused: boolean) => call<void>("focus_window", { focused }),

  reposition: () => call<void>("reposition"),

  openUrl: (url: string) => call<void>("open_url", { url }),

  /** "Open terminal" → opens the folder in VS Code when `code` is on PATH. */
  openInVSCode: (path: string | null) => call<boolean>("open_in_vscode", { path }),

  quit: () => call<void>("quit_app"),

  openSettingsWindow: () => call<void>("open_settings_window"),

  /** Writes to %LOCALAPPDATA%\Mayak\mayak.log, next to the Rust lines. */
  log: (message: string) => call<void>("log_line", { message }),

  // ── Agent sessions (observer) ─────────────────────────────────────────────
  /** Latest published snapshot; the island also receives it as the `sessions` event. */
  sessionsSnapshot: () => call<SessionsSnapshot>("sessions_snapshot"),
  /** "Seen": hides the current episode of that row. Never changes its phase. */
  sessionAck: (key: string) => call<void>("session_ack", { key }),
  /** Opens the project root in the session's host editor; false → Explorer was used. */
  sessionOpen: (key: string) => call<boolean>("session_open", { key }),

  // ── Hook installers (Codex hooks.json, legacy Claude settings.json) ──────────
  hooksStatus: (target: HookTarget = "claude") => call<HookStatus>("hooks_status", { target }),
  /** Diff to show before anything is written. `install: false` previews removal. */
  hooksPreview: (install: boolean, target: HookTarget = "claude") =>
    callOrThrow<HookPreview>("hooks_preview", { target, install }),
  /**
   * Writes the target's config file — only ever after an explicit click, and only
   * when the file still matches the preview the user looked at.
   */
  hooksApply: (install: boolean, fingerprint: string, target: HookTarget = "claude") =>
    callOrThrow<string>("hooks_apply", { target, install, fingerprint }),

  approvalDecision: (requestId: string, decision: "allow" | "deny") =>
    call<void>("approval_decision", { requestId, decision }),
  /** "The card is up" — until this lands the relay only waits a moment. */
  approvalAck: (requestId: string) => call<void>("approval_ack", { requestId }),
  /** "Nobody can act on this" — Claude Code asks in the terminal right away. */
  approvalDecline: (requestId: string) => call<void>("approval_decline", { requestId }),

  // ── Chat, files, secrets ──────────────────────────────────────────────────
  /** One chat turn. The API key and any file bytes never leave Rust. */
  chatSend: (query: string, context: ChatContext | null) =>
    callOrThrow<{ text: string }>("chat_send", { query, context }),
  chatReset: () => call<void>("chat_reset"),
  /** Copies a dropped file into the inbox. */
  ingestFile: (path: string) => callOrThrow<DroppedFile>("ingest_file", { path }),
  /** Only ever tells you whether a key exists — never its value. */
  secretPresent: (key: string) => call<boolean>("secret_present", { key }),
  secretSet: (key: string, value: string) => callOrThrow<void>("secret_set", { key, value }),
  secretClear: (key: string) => callOrThrow<void>("secret_clear", { key }),

  // ── Integrations ──────────────────────────────────────────────────────────
  refreshIntegration: (id: string) => call<void>("refresh_integration", { id }),
  /** Opens the configured n8n instance in the browser. */
  openN8n: () => call<void>("open_n8n"),

  /** Tray → Pause. Stops the integration pollers, not just the island. */
  setPaused: (paused: boolean) => call<void>("set_paused", { paused }),
};

export interface IntegrationUpdate {
  id: string;
  data: Record<string, unknown>;
  error: string | null;
  event: { success: boolean; label: string; detail: string | null } | null;
}

export type ChatContext =
  | { kind: "file"; name: string; path: string }
  | { kind: "window"; appName: string; title: string; url?: string };

export interface DroppedFile {
  name: string;
  path: string;
  size: number;
}

// ── Sessions snapshot — mirrors the Rust wire types (spec §10) ────────────────

export type Host = "claudeDesktop" | "cursor" | "vsCode" | "codexDesktop" | "unknown";
export type Phase =
  | { kind: "working" }
  | { kind: "waiting"; wait: "permission" | "input" | "other"; label: string }
  | { kind: "idle" }
  | { kind: "unknown"; label: string };
export type OutcomeView = null | { kind: "finished" | "error" | "interrupted"; summary: string; at: number };
export type AttentionKind = null | "permission" | "input" | "error" | "finished";
export type BotState = "approval" | "question" | "error" | "working" | "finished" | "idle";
export type Health = "ok" | "missing" | { error: string } | { degraded: { unreadable: number } };
export type CodexHooksState =
  | "not_installed" | "installed_unseen" | { receiving: { hosts: Host[]; since: number } };
export type SilentReason = "hooksNotInstalled" | "folderMissing" | "hooksNotReaching";
export interface SessionView {
  key: string; agent: "claude" | "codex"; host: Host; title: string | null;
  phase: Phase; since: number; activity: string | null; detail: string | null;
  outcome: OutcomeView; attention: AttentionKind; acked: boolean;
  silent?: SilentReason;
}
export interface ProjectView { key: string; name: string; root: string; sessions: SessionView[] }
export interface SessionsSnapshot {
  rev: number; now: number; projects: ProjectView[]; attention: string[];
  counts: { waiting: number; error: number; finished: number; working: number };
  hold: boolean; aggregate: BotState;
  sources: { claude: Health; codex: Health; codexHooks: CodexHooksState };
}
export interface Alert {
  key: string; kind: "permission" | "input" | "error" | "finished"; rank: number;
  agent: "claude" | "codex"; host: Host; project: string; title: string | null;
  detail: string | null; text: string; since: number;
}

// ── Hook installers ───────────────────────────────────────────────────────────

export type HookTarget = "claude" | "codex";
export interface HookStatus {
  target: HookTarget; installed: boolean; outdated: boolean; settingsPath: string;
  hookPath: string; hookReady: boolean; notes: string[];
}
export interface HookPreview {
  /** Hand `fingerprint` back to hooksApply so only the reviewed diff is ever written. */
  diff: string; backup: string; settingsPath: string; fingerprint: string; notes: string[];
  /** Nothing would be written (the entry is already there): Apply stays disabled. */
  unchanged: boolean;
}

/** Same as `call`, but surfaces the error so the UI can show what went wrong. */
async function callOrThrow<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!IS_TAURI) throw new Error("not running inside Mayak");
  return invoke<T>(cmd, args);
}

export type BridgeEvent =
  | { name: "cursor"; payload: { x: number; y: number } }
  | { name: "tray"; payload: string }
  | { name: "hook"; payload: Record<string, unknown> }
  | { name: "screen-changed"; payload: null }
  | { name: "sessions"; payload: SessionsSnapshot }
  | { name: "session-alert"; payload: Alert[] };

export interface DragDropPayload {
  type: "enter" | "over" | "drop" | "leave";
  paths?: string[];
}

/** Files dragged onto the island. Only reaches us when the window takes the mouse. */
export async function onDragDrop(handler: (e: DragDropPayload) => void) {
  if (!IS_TAURI) return () => {};
  return getCurrentWebview().onDragDropEvent((event) => {
    handler(event.payload as DragDropPayload);
  });
}

export async function onEvent<T>(name: string, handler: (payload: T) => void) {
  if (!IS_TAURI) return () => {};
  return listen<T>(name, (e) => handler(e.payload));
}

/** Every published sessions snapshot (emit_to "island"). */
export function onSessions(handler: (s: SessionsSnapshot) => void): Promise<() => void> {
  return onEvent<SessionsSnapshot>("sessions", handler);
}

/** One batch of model alerts, emitted just before the snapshot that contains them. */
export function onSessionAlert(handler: (alerts: Alert[]) => void): Promise<() => void> {
  return onEvent<Alert[]>("session-alert", handler);
}

/** A new silent Codex row appeared: the island shows itself once, with no sound and no card. */
export function onSessionSilent(handler: () => void): Promise<() => void> {
  return onEvent<null>("session-silent", () => handler());
}
