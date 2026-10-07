// Thin wrapper over the Tauri commands/events. Every call is a no-op when the
// page is opened in a plain browser, so the island can be iterated on with
// `npm run dev` alone.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
  /** Windows: a hidden island is the visible strip driven by src-tauri/src/strip.rs. */
  nativeStrip: boolean;
}

export const Bridge = {
  boot: () => call<BootInfo>("boot"),

  saveSettings: (settings: Settings) => call<void>("save_settings", { settings }),

  /**
   * Hidden: shrink the window to the strip zone, where strip.rs runs the visible strip (Windows),
   * or to the invisible 240×6 wake strip (Linux). Otherwise back to full.
   */
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
  /** The home view opened: Rust reads Claude Code's cached limits again; the result comes with the next snapshot. */
  usageRefresh: () => call<void>("usage_refresh"),

  // ── Launcher (prompt view, launcher spec §3-§4) ───────────────────────────
  /** The installed agents in the spec's order; "claudeApi" last when the Anthropic key is saved. */
  launcherTargets: () => call<LauncherAgent[]>("launcher_targets"),
  /** Opens the agent with the question in its field, never sends it; resolves when the launch is done. */
  launcherOpen: (agent: LauncherAgent, prompt: string, root: string | null, files: string[]) =>
    call<LaunchOutcome>("launcher_open", { agent, prompt, root, files }),

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
  /** The models the saved Anthropic key can use, newest first; throws without a key or on an API error. */
  chatModels: () => callOrThrow<{ id: string; displayName: string }[]>("chat_models"),
  /** Copies a file into the inbox by its path. Island drops go through onDragDrop / `ingest_bytes`. */
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

  // ── Global shortcut (src-tauri/src/hotkey.rs) ─────────────────────────────
  /** Current state of the shortcut; every later change arrives as `hotkey-status`. */
  hotkeyStatus: () => call<HotkeyStatus>("hotkey_status"),
  /** Settings is recording a shortcut: release the current one meanwhile, so the keys reach the page. */
  hotkeyPause: (on: boolean) => call<void>("hotkey_pause", { on }),
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

/** A target of the prompt view (launcher spec §3); wire names of Rust `launcher::AgentId`. */
export type LauncherAgent =
  | "claudeDesktop" | "claudeCursor" | "claudeVsCode"
  | "codexDesktop" | "codexCursor" | "codexVsCode"
  | "claudeApi";
/** What a launch gave (launcher spec §4); `reason` is Mayak's own text. */
export type LaunchOutcome =
  | { kind: "opened" }
  | { kind: "openedPasteYourself" }
  | { kind: "failed"; reason: string };

/** What Settings shows under the global shortcut (hotkey spec §3.1). */
export interface HotkeyStatus {
  combo: string;
  state: "off" | "active" | "taken" | "invalid" | "unsupported";
  error: string | null;
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
/** How full a session's context is (context-fill spec §3); `window` only when the agent writes it (Codex). */
export interface ContextFill { used: number; window?: number }
/** One window of a plan limit (usage-home spec §3); `resetsAtMs` only when the agent wrote a valid time. */
/** `id`: "5h", "7d" or "7d:<model>" (a model's weekly window, e.g. "7d:opus"). */
export interface ClaudeUsageWindow { id: string; percent: number; resetsAtMs?: number }
export interface CodexUsageWindow { minutes: number; percent: number; resetsAtMs?: number }
/** The home page's limits; a side is null until its source gave a value. */
export interface Usage {
  claude: { fetchedAtMs: number; windows: ClaudeUsageWindow[] } | null;
  codex: { seenAtMs: number; windows: CodexUsageWindow[] } | null;
}
export interface SessionView {
  key: string; agent: "claude" | "codex"; host: Host; title: string | null;
  phase: Phase; since: number; activity: string | null; detail: string | null;
  outcome: OutcomeView; attention: AttentionKind; acked: boolean;
  silent?: SilentReason;
  context?: ContextFill;
}
export interface ProjectView { key: string; name: string; root: string; sessions: SessionView[] }
export interface SessionsSnapshot {
  rev: number; now: number; projects: ProjectView[]; attention: string[];
  counts: { waiting: number; error: number; finished: number; working: number };
  hold: boolean; aggregate: BotState;
  sources: { claude: Health; codex: Health; codexHooks: CodexHooksState };
  usage: Usage;
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
  | { name: "pointer-released"; payload: null }
  | { name: "strip-cursor"; payload: { x: number; y: number } | null }
  | { name: "strip-touch"; payload: null }
  | { name: "strip-fullscreen"; payload: { on: boolean } }
  | { name: "hotkey"; payload: null }
  | { name: "hotkey-status"; payload: HotkeyStatus }
  | { name: "tray"; payload: string }
  | { name: "hook"; payload: Record<string, unknown> }
  | { name: "screen-changed"; payload: null }
  | { name: "sessions"; payload: SessionsSnapshot }
  | { name: "session-alert"; payload: Alert[] };

export interface DragDropPayload {
  type: "enter" | "drop" | "leave";
  /**
   * On drop: the first dropped item. `ingest` copies it into the inbox when called, and rejects
   * with the note to show for a folder, something that is not a file, or a file over the limit.
   */
  file?: { name: string; ingest: () => Promise<DroppedFile> };
}

/** Same limit as files::MAX_DROP_BYTES: bigger files are not worth pushing through the IPC. */
const MAX_DROP_BYTES = 200 * 1024 * 1024;

/**
 * Files dragged onto the island, as a plain HTML5 drag and drop. Only reaches us while the window
 * takes the mouse (the cursor poll accepts a held button over the panel).
 *
 * Tauri's native drop (dragDropEnabled) never fires on Windows: the windows a drag crosses belong
 * to the WebView2 process, and OLE stops at WebView2's own drop target there. WebView2 delivers
 * the drop to the page instead, with the file's contents and no path, so the contents go to Rust,
 * which writes them into the inbox like any other drop.
 */
export async function onDragDrop(handler: (e: DragDropPayload) => void) {
  if (!IS_TAURI) return () => {};
  const hasFiles = (e: DragEvent) => !!e.dataTransfer?.types.includes("Files");
  // dragenter / dragleave fire for every element crossed: count them to know when the drag
  // really enters and leaves the page.
  let depth = 0;

  const enter = (e: DragEvent) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    if (depth++ === 0) handler({ type: "enter" });
  };
  const over = (e: DragEvent) => {
    if (!hasFiles(e)) return;
    e.preventDefault(); // without it the page refuses the drop
    e.dataTransfer!.dropEffect = "copy";
  };
  const leave = (e: DragEvent) => {
    if (!hasFiles(e) || depth === 0) return;
    if (--depth === 0) handler({ type: "leave" });
  };
  const drop = (e: DragEvent) => {
    if (!hasFiles(e)) return;
    // Always: an unhandled file drop would navigate the webview to the file.
    e.preventDefault();
    depth = 0;
    handler({ type: "drop", file: droppedItem(e.dataTransfer!) });
  };

  window.addEventListener("dragenter", enter);
  window.addEventListener("dragover", over);
  window.addEventListener("dragleave", leave);
  window.addEventListener("drop", drop);
  return () => {
    window.removeEventListener("dragenter", enter);
    window.removeEventListener("dragover", over);
    window.removeEventListener("dragleave", leave);
    window.removeEventListener("drop", drop);
  };
}

/**
 * The first dropped item, read during the drop event (the entry is gone after it). Only the first
 * file is used, as with the native drop.
 */
function droppedItem(data: DataTransfer): NonNullable<DragDropPayload["file"]> {
  const file = data.files[0];
  const entry = data.items[0]?.webkitGetAsEntry?.() ?? null;
  if (!file) return refused("file", "Only files can be dropped.");
  if (entry?.isDirectory) return refused(file.name, "Folders can't be dropped yet.");
  if (file.size > MAX_DROP_BYTES) return refused(file.name, "That file is too big to drop (200 MB max).");
  return {
    name: file.name,
    ingest: async () => {
      let bytes: ArrayBuffer;
      try {
        bytes = await file.arrayBuffer();
      } catch {
        // A folder the entry did not reveal, or a file that went away meanwhile.
        throw new Error("That file can't be read.");
      }
      return invoke<DroppedFile>("ingest_bytes", bytes, {
        headers: { "x-file-name": encodeURIComponent(file.name) },
      });
    },
  };
}

function refused(name: string, note: string): NonNullable<DragDropPayload["file"]> {
  return { name, ingest: () => Promise.reject(new Error(note)) };
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
