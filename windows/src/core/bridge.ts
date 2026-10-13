// Thin wrapper over the Tauri commands/events. Every call is a no-op when the
// page is opened in a plain browser, so the island can be iterated on with
// `npm run dev` alone.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Settings } from "./state";
import { t } from "./i18n";
import type { RustMessage } from "./message";
import type { GitState } from "./git-state";
import type { AgentInfo, Convo, ConvoSummary, OpenRequest, PermissionEvent } from "./agent-chat";

export type { RustMessage } from "./message";

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
  screen: {
    x: number; y: number; width: number; height: number; scale: number;
    /** The notch of a MacBook screen, `[width, height]` in logical px; null elsewhere. */
    notch: [number, number] | null;
  };
  version: string;
  hookPath: string;
  /** False where the OS has no global cursor (Wayland): see Island.followPageCursor. */
  cursorPoll: boolean;
  /** Windows: a hidden island is the visible strip driven by src-tauri/src/strip.rs. */
  nativeStrip: boolean;
  /** Quiet mode as Rust holds it: true only after a reload (a new language) while paused. */
  paused: boolean;
}

/** mcp_server `Status`: `error` is a text Rust built (msg.mcp.*), present only while it is on and not listening. */
export interface McpStatus { enabled: boolean; running: boolean; port: number; url: string; error: RustMessage | null }
export type McpCli = "claude" | "codex";
/** src-tauri/src/ao `Detection`: what this PC has of Agent Orchestrator, from its run file and PATH. `cli`: the `ao` found on PATH. */
export interface AoDetection { state: "running" | "notRunning" | "notInstalled" | "unsupported"; cli: string | null; port: number | null }
/** What «Copy command» did (lib.rs `McpCopy`): "copy" = put `text` on the clipboard from the page. */
export type McpCopy = { kind: "done" } | { kind: "copy"; text: string } | { kind: "failed" };

/** What an island card answers (pipe.rs `kind` on the `hook` event). */
export type HookCardKind = "permission" | "question";
/** Which agent waits for the card (pipe.rs `agent` on the `hook` event). */
export type HookAgent = "claude" | "codex";
export type ApprovalDecision = "allow" | "always" | "deny";
/**
 * Question text as the island got it → the chosen label, the chosen labels of a multi-select
 * (never an empty list), or `{ other }`, the text typed in "Other" (trimmed, 1 to 4000
 * characters; spec §6). Rust also refuses an object with no answer at all.
 */
export type QuestionAnswers = Record<string, string | [string, ...string[]] | { other: string }>;

export const Bridge = {
  boot: () => call<BootInfo>("boot"),

  saveSettings: (settings: Settings) => call<void>("save_settings", { settings }),

  /**
   * Hidden: shrink the window to the strip zone, where strip.rs runs the visible strip (Windows),
   * or to the invisible 240×6 wake strip (Linux). Otherwise back to full.
   */
  setCollapsed: (collapsed: boolean) => call<void>("set_collapsed", { collapsed }),
  /** Whether the hidden island shows its line (Windows; a Mac screen without a notch). */
  stripVisible: () => call<boolean>("strip_visible"),

  /**
   * Pushes the island shape in window coordinates. Rust flips click-through from
   * its own cursor poll, so the flag is never a frame behind a click.
   */
  setIslandRect: (x: number, y: number, width: number, height: number) =>
    call<void>("set_island_rect", { x, y, width, height }),

  /** Give the window keyboard focus (chat field) and take it away again. */
  focusWindow: (focused: boolean) => call<void>("focus_window", { focused }),
  /** The hotkey opened the island: remember the foreground window, then take the keyboard (Windows). */
  hotkeyTakeFocus: () => call<void>("hotkey_take_focus"),
  /** Give the hotkey's keyboard back; the remembered window gets it if the island still holds it and opened nothing. */
  hotkeyGiveFocus: (openedSomething: boolean, keepActivating: boolean) =>
    call<void>("hotkey_give_focus", { openedSomething, keepActivating }),

  reposition: () => call<void>("reposition"),

  openUrl: (url: string) => call<void>("open_url", { url }),

  /** Which of these sounds have a file of the user's own in the sounds folder. */
  customSounds: (names: readonly string[]) => call<string[]>("custom_sounds", { names }),
  /** The user's file for one sound, as raw bytes. */
  customSound: (name: string) => call<ArrayBuffer>("custom_sound", { name }),
  /** Settings → Open sounds folder: %APPDATA%\Mayak\sounds, created if needed. */
  revealSoundsFolder: () => call<void>("reveal_sounds_folder"),
  /** Tells the island to read the sounds folder again ("sounds-changed"). */
  reloadSounds: () => call<void>("reload_sounds"),

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
  /** The row menu (idea #8): the items this session offers, in order; null outside Mayak. */
  sessionMenu: (key: string) => call<SessionMenuItem[]>("session_menu", { key }),
  /** A row menu item: Rust copies or opens the folder itself (a copy outside Windows comes back as text). */
  sessionMenuRun: (key: string, item: SessionMenuItem) => call<SessionMenuDone>("session_menu_run", { key, item }),
  /** Opens the `index`th file of the session's last turn in Cursor or VS Code at its first change; false when it could not or that file is no longer `path`. */
  sessionFileOpen: (key: string, index: number, path: string) => call<boolean>("session_file_open", { key, index, path }),
  /** Git state of these projects (git_state.rs), keyed by project key; a project with no entry shows none. */
  gitStates: (keys: string[]) => call<Record<string, GitState>>("git_states", { keys }),
  /** The resume command of a Claude or Codex session on the list onto the clipboard; false when nothing was copied. */
  sessionResumeCopy: (key: string) => call<boolean>("session_resume_copy", { key }),
  /** «Recent» opened: the sessions of the last 24 hours that are not listed, newest first. */
  recentSessions: () => call<RecentSession[]>("recent_sessions"),
  /** A recent session's resume command in a new console in its folder; "copied" when the CLI is not on PATH. */
  recentResume: (r: RecentSession) => call<ResumeOutcome>("recent_resume", { agent: r.agent, id: r.id, cwd: r.cwd }),
  /** A recent session's resume command onto the clipboard. */
  recentCopy: (r: RecentSession) => call<boolean>("recent_copy", { agent: r.agent, id: r.id, cwd: r.cwd }),
  /** The home view opened: Rust reads Claude Code's cached limits again; the result comes with the next snapshot. */
  usageRefresh: () => call<void>("usage_refresh"),
  /** The home page's minute tick: the OpenRouter, DeepSeek and Moonshot balances whose key is saved, when stale. */
  balancesRefresh: () => call<void>("balances_refresh"),

  /** The Stats tab (night spec §C): counts tokens from the agents' files; null on a failure. */
  statsQuery: (days: 7 | 30) => call<StatsReport>("stats_query", { days }),

  // ── Launcher (prompt view, launcher spec §3-§4) ───────────────────────────
  /** The installed agents in the spec's order; "claudeApi" last when the Anthropic key is saved. */
  launcherTargets: () => call<LauncherAgent[]>("launcher_targets"),
  /** Opens the agent with the question in its field, never sends it; resolves when the launch is done. */
  launcherOpen: (agent: LauncherAgent, prompt: string, root: string | null, files: string[]) =>
    call<LaunchOutcome>("launcher_open", { agent, prompt, root, files }),

  // ── Apps tab (installed IDEs and agent apps) ──────────────────────────────
  /** The tiles: installed apps not hidden and the user's apps, in the user's order. */
  appsList: () => call<AppTile[]>("apps_list"),
  /** Every built-in app, installed or not (Settings names the hidden ones). */
  appsBuiltin: () => call<AppEntry[]>("apps_builtin"),
  /** Opens or raises the app; false for an app that is not installed or a user app whose file is gone. */
  appOpen: (id: string) => call<boolean>("app_open", { id }),
  /** The "+" tile: the Open dialog for an exe or a shortcut; resolves when it is closed. */
  appPick: (title: string, filter: string) => call<PickOutcome>("app_pick", { title, filter }),
  /** A user app's own icon: RGBA bytes of a square; null when it has none. */
  appIcon: (id: string) => call<ArrayBuffer>("app_icon", { id }),

  // ── Hook installers (Codex hooks.json, Claude settings.json) ─────────────────
  hooksStatus: (target: HookTarget = "claude") => call<HookStatus>("hooks_status", { target }),
  /** Diff to show before anything is written. `install: false` previews removal. Rejects with a RustMessage. */
  hooksPreview: (install: boolean, target: HookTarget = "claude") =>
    callOrThrow<HookPreview>("hooks_preview", { target, install }),
  /**
   * Writes the target's config file — only ever after an explicit click, and only
   * when the file still matches the preview the user looked at. Rejects with a RustMessage.
   */
  hooksApply: (install: boolean, fingerprint: string, target: HookTarget = "claude") =>
    callOrThrow<string>("hooks_apply", { target, install, fingerprint }),
  /**
   * Diff of switching Codex answers from the island on or off (spec §3). Rejects with a
   * RustMessage, also while Mayak's Codex hooks are not installed.
   */
  codexAnswersPreview: (on: boolean) => callOrThrow<HookPreview>("codex_answers_preview", { on }),
  /** Writes the diff codexAnswersPreview showed, only after an explicit click. Rejects with a RustMessage. */
  codexAnswersApply: (on: boolean, fingerprint: string) =>
    callOrThrow<string>("codex_answers_apply", { on, fingerprint }),

  /** A permission card's button. "always" for Codex, or without `canAlways`, is a plain allow. */
  approvalDecision: (requestId: string, decision: ApprovalDecision) =>
    call<void>("approval_decision", { requestId, decision }),
  /**
   * A question card's answers. True when Rust took them: sent to the agent, or the request had
   * already ended (a late click changes nothing). False when Rust refused them (no answer, an
   * empty list, a value that is not a label or an Other text) or outside Mayak: the request keeps waiting.
   */
  questionAnswer: async (requestId: string, answers: QuestionAnswers): Promise<boolean> => {
    try {
      await callOrThrow<void>("question_answer", { requestId, answers });
      return true;
    } catch (err) {
      console.error("[mayak] question_answer refused", err);
      return false;
    }
  },
  /** "The card is up" — until this lands the relay only waits a moment. */
  approvalAck: (requestId: string) => call<void>("approval_ack", { requestId }),
  /** "Nobody can act on this" or the card is gone — the agent asks in its own window right away. */
  approvalDecline: (requestId: string) => call<void>("approval_decline", { requestId }),
  /** The island booted: every request a page before held is released, its agent asks itself. */
  approvalsReleaseAll: () => call<void>("approvals_release_all"),

  // ── Chat, files, secrets ──────────────────────────────────────────────────
  /** One chat turn. The API key and any file bytes never leave Rust. */
  chatSend: (query: string, context: ChatContext | null) =>
    callOrThrow<{ text: string }>("chat_send", { query, context }),
  chatReset: () => call<void>("chat_reset"),
  /** The models the saved Anthropic key can use, newest first; throws without a key or on an API error. */
  chatModels: () => callOrThrow<{ id: string; displayName: string }[]>("chat_models"),
  /** Only ever tells you whether a key exists — never its value. */
  secretPresent: (key: string) => call<boolean>("secret_present", { key }),
  secretSet: (key: string, value: string) => callOrThrow<void>("secret_set", { key, value }),
  secretClear: (key: string) => callOrThrow<void>("secret_clear", { key }),

  // ── CLI chat (src-tauri/src/chat): the user's own agent CLIs, no API key ────
  /** The CLIs, found or not, with login state, models and modes. `refresh` skips Rust's 30 s memory. */
  agentChatAgents: (refresh: boolean) => call<AgentInfo[]>("agent_chat_agents", { refresh }),
  agentChatList: () => call<ConvoSummary[]>("agent_chat_list"),
  /** The conversation; null when it is gone or `since` is its current revision (nothing new). */
  agentChatGet: (conv: number, since: number | null) => call<Convo | null>("agent_chat_get", { conv, since }),
  /** Rejects with a ChatError (agent-chat.ts normalizeError). */
  agentChatOpen: (request: OpenRequest) => callOrThrow<number>("agent_chat_open", { request }),
  agentChatSet: (conv: number, model: string, mode: string) => callOrThrow<void>("agent_chat_set", { conv, model, mode }),
  agentChatSend: (conv: number, text: string) => callOrThrow<void>("agent_chat_send", { conv, text }),
  /** Allow or Deny on a request of the chat; only its buttons call this. False: it was not pending any more. */
  agentChatPermission: (requestId: string, allow: boolean) => call<boolean>("agent_chat_permission", { requestId, allow }),
  agentChatCancel: (conv: number) => call<void>("agent_chat_cancel", { conv }),
  agentChatClose: (conv: number) => call<void>("agent_chat_close", { conv }),

  // ── Mayak's MCP server (src-tauri/src/mcp_server) ─────────────────────────
  /** Whether the server is on, listening, and why not when it is on but is not. */
  mcpStatus: () => call<McpStatus>("mcp_status"),
  /** Settings' switch: saves it and starts or stops the server; the status after. */
  mcpSetEnabled: (enabled: boolean) => call<McpStatus>("mcp_set_enabled", { enabled }),
  /** «Copy command»: Rust puts the line that adds the server to the user's Claude Code or Codex on the clipboard (the token is in it). */
  mcpCopyCommand: (cli: McpCli) => call<McpCopy>("mcp_copy_command", { cli }),
  /** «New token»: every agent holding the old one is refused from now on. Throws when it cannot be saved. */
  mcpNewToken: () => callOrThrow<McpStatus>("mcp_new_token"),

  // ── Integrations ──────────────────────────────────────────────────────────
  refreshIntegration: (id: string) => call<void>("refresh_integration", { id }),
  /** Settings → Integrations → Agent Orchestrator: is it installed and running, without asking its daemon anything. */
  aoDetect: () => call<AoDetection>("ao_detect"),
  /** Opens the configured n8n instance in the browser. */
  openN8n: () => call<void>("open_n8n"),
  /** A link of the GitLab card; Rust opens it only when it is https on the configured GitLab host. */
  openGitlab: (url: string) => call<boolean>("open_gitlab", { url }),
  /** A link of the GitHub card; Rust opens it only when it is https on github.com. */
  openGithub: (url: string) => call<boolean>("open_github", { url }),
  /** The GitLab address as Rust will use it, or null when it is not a valid https host. */
  gitlabCheckUrl: (url: string) => call<string | null>("gitlab_check_url", { url }),

  // ── Connect (src-tauri/src/oauth.rs, device flow) ─────────────────────────
  /** The code to show and the page to enter it on; throws a StartError (read it with settings/oauth-view startFailure). The end comes as `oauth`. */
  oauthStart: (service: OAuthService) => callOrThrow<DeviceStart>("oauth_start", { service }),
  /** GitLab's «new personal access token» page, prefilled (name Mayak, scope read_api), on the configured https host only. */
  openGitlabTokenPage: () => call<boolean>("open_gitlab_token_page"),
  oauthCancel: (service: OAuthService) => call<void>("oauth_cancel", { service }),
  /** Deletes the token and what goes with it from the Credential Manager. */
  oauthDisconnect: (service: OAuthService) => callOrThrow<void>("oauth_disconnect", { service }),
  oauthStatus: (service: OAuthService) => call<ConnState>("oauth_status", { service }),
  /** GitHub's Connect: GitHub App or OAuth app, whether the app can be installed, how the token was had. */
  githubInfo: () => call<GithubInfo>("github_info"),
  /** The installation page of Mayak's GitHub App on github.com; false while its slug is not set. */
  openGithubInstall: () => call<boolean>("open_github_install"),

  /** Launch at login as Windows has it: the saved flag follows it first (src-tauri/src/autostart.rs). */
  autostartStatus: () => call<AutostartStatus>("autostart_status"),

  /** Tray → Pause. Stops the integration pollers, not just the island. */
  setPaused: (paused: boolean) => call<void>("set_paused", { paused }),
  /** Keeps the tray's Focus check in step with the island's focus mode (night spec §A). */
  setFocusCheck: (on: boolean) => call<void>("set_focus_check", { on }),
  /** The tray's five texts in the current language (batch spec §C): Rust keeps no dictionary. */
  trayTexts: (texts: TrayTexts) => call<void>("tray_texts", { texts }),

  // ── Global shortcut (src-tauri/src/hotkey.rs) ─────────────────────────────
  /** Current state of the shortcut; every later change arrives as `hotkey-status`. */
  hotkeyStatus: () => call<HotkeyStatus>("hotkey_status"),
  /** The same for the «next waiting session» shortcut; later changes arrive as `hotkey-next-status`. */
  hotkeyNextStatus: () => call<HotkeyStatus>("hotkey_next_status"),
  /** Settings is recording a shortcut: release both shortcuts meanwhile, so the keys reach the page. */
  hotkeyPause: (on: boolean) => call<void>("hotkey_pause", { on }),

  // ── Player (src-tauri/src/media.rs, night spec §B) ────────────────────────
  /** What is playing now, null when nothing is; every later change arrives as `media`. */
  mediaState: () => call<MediaState | null>("media_state"),
  /** The player buttons: handed to the media thread, never waited for. */
  mediaToggle: () => call<void>("media_toggle"),
  mediaNext: () => call<void>("media_next"),
  mediaPrev: () => call<void>("media_prev"),
  /** The progress bar: `positionMs` into the track; the new place comes back as `media`. */
  mediaSeek: (positionMs: number) => call<void>("media_seek", { positionMs: Math.max(0, Math.round(positionMs)) }),
  mediaShuffle: (on: boolean) => call<void>("media_shuffle", { on }),
  mediaRepeat: (mode: MediaRepeat) => call<void>("media_repeat", { mode }),
};

/** Token statistics of a period; mirrors Rust `stats::StatsReport`. */
export interface StatsReport {
  /** Every day of the period (`YYYY-MM-DD`, local), oldest first, today last. */
  days: { day: string; total: number }[];
  /** Largest first; `name` is the last component of the root, "Other" for no folder. */
  projects: { project: string; name: string; total: number }[];
  /** (agent, model) pairs, largest first. */
  agents: { agent: "claude" | "codex"; model: string; input: number; output: number; cacheRead: number; cacheWrite: number }[];
  total: { input: number; output: number; cacheRead: number; cacheWrite: number; requests: number; total: number };
  scannedFiles: number;
  fromCache: number;
}

/** The player row (the footer of the main tabs); mirrors Rust `media::MediaState`. */
export interface MediaState {
  title: string;
  artist: string;
  /** Display name of the playing app ("Spotify", "Chrome"…). */
  app: string;
  playing: boolean;
  canNext: boolean;
  canPrev: boolean;
  /** Album art as a data URL (spec §B), null without one. */
  cover: string | null;
  /** Where the track is; null when the player reports no timeline (the row has no bar). */
  timeline: MediaTimeline | null;
  /** Shuffle on or off; null when the player does not offer it (no button). */
  shuffle: boolean | null;
  /** The repeat mode; null when the player does not offer it (no button). */
  repeat: MediaRepeat | null;
}

/** Mirrors Rust `media::Timeline`: `positionMs` was true at `updatedAt` (Unix ms). */
export interface MediaTimeline {
  positionMs: number;
  durationMs: number;
  updatedAt: number;
  /** The player takes a new position. */
  seekable: boolean;
}

/** Mirrors Rust `media::Repeat`: off, the whole list, or the current track. */
export type MediaRepeat = "off" | "all" | "one";

export type OAuthService = "github" | "gitlab";
/** What Settings shows for a service: no token, a token, or a refresh token the service refused. */
export type ConnState = "none" | "connected" | "reconnect";
/** Meant for display: the code the user enters and the page to enter it on (https on the service's host). */
export interface DeviceStart {
  userCode: string;
  verificationUri: string;
  expiresIn: number;
}
/** Mirrors Rust `GithubInfo` (lib.rs): `app` Connect signs in with a GitHub App, `installable` its page can be opened, `mode` how the token was had. */
export interface GithubInfo {
  app: boolean;
  installable: boolean;
  mode: GithubMode;
}
/** oauth::github_mode: Mayak's GitHub App, the OAuth app, a pasted token, or none. */
export type GithubMode = "app" | "oauth" | "token" | "none";
/** Why Connect could not start (oauth.rs StartError): a problem code, and whether GitLab's token page is offered instead. */
export interface StartError {
  code: string;
  fallback: boolean;
}
/** How a device flow ended, or a Disconnect; `fallback` offers GitLab's token page. */
export interface OAuthResult {
  service: OAuthService;
  state: "connected" | "expired" | "denied" | "notAllowed" | "invalidScope" | "error" | "disconnected";
  fallback?: boolean;
}

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

/** A built-in app; wire names of Rust `apps::AppEntry`. */
export interface AppEntry {
  id: string;
  name: string;
}

/** One tile of the Apps tab; wire names of Rust `apps::AppTile`. */
export interface AppTile extends AppEntry {
  /** Added by the user: Rename and Remove; a built-in one has Hide. */
  custom: boolean;
  /** A user app whose file is gone: dimmed, does not open. */
  missing: boolean;
  /** A user app's exe file name (no folder), for its styled icon. */
  exe: string | null;
}

/** What the "+" tile's dialog gave; wire names of Rust `apps::PickOutcome`. `path` is user data. */
export type PickOutcome =
  | { kind: "picked"; name: string; path: string }
  | { kind: "cancelled" }
  | { kind: "notAnApp" }
  | { kind: "failed" };

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

/** The tray's items in the current language; wire names of Rust `tray::TrayTexts`. */
export interface TrayTexts {
  open: string;
  settings: string;
  focus: string;
  pause: string;
  quit: string;
  /** The tooltip while sessions wait: "{n} waiting", "{m} min" and "just now" (Rust `tray::WaitTexts`). */
  waiting: string;
  minutes: string;
  justNow: string;
}

/** Launch at login: the real state, and what failed, if anything, for the line under the toggle. */
export interface AutostartStatus {
  enabled: boolean;
  error: "read" | "write" | null;
}

/** What Settings shows under the global shortcut (hotkey spec §3.1). */
export interface HotkeyStatus {
  combo: string;
  state: "off" | "active" | "taken" | "invalid" | "unsupported";
  error: RustMessage | null;
}

// ── Sessions snapshot — mirrors the Rust wire types (spec §10) ────────────────

export type Host =
  | "claudeDesktop" | "cursor" | "vsCode" | "codexDesktop" | "antigravityIde" | "antigravityCli"
  // the terminal a session runs in (Rust observe/terminal.rs)
  | "windowsTerminal" | "powerShell" | "commandPrompt" | "gitBash" | "wezTerm" | "alacritty" | "tabby"
  | "hyper" | "conEmu" | "warp"
  // macOS terminals
  | "macTerminal" | "iTerm" | "ghostty" | "kitty"
  | "unknown";
export type Phase =
  | { kind: "working" }
  | { kind: "waiting"; wait: "permission" | "input" | "other"; label: string }
  | { kind: "idle" }
  | { kind: "unknown"; label: string };
export type OutcomeView = null | { kind: "finished" | "error" | "interrupted"; summary: string; at: number };
export type AttentionKind = null | "permission" | "input" | "error" | "finished";
export type BotState = "approval" | "question" | "error" | "working" | "finished" | "idle";
export type Health = "ok" | "missing" | { error: RustMessage } | { degraded: { unreadable: number } };
export type CodexHooksState =
  | "not_installed" | "installed_unseen" | { receiving: { hosts: Host[]; since: number } };
export type SilentReason = "hooksNotInstalled" | "folderMissing" | "hooksNotReaching";
/** How full a session's context is (context-fill spec §3); `window` when Codex writes it or Claude's model is known. */
export interface ContextFill { used: number; window?: number }
/** The last compaction of a Claude session's context (model.rs `Compacted`): when it ended, the tokens before and after. */
export interface Compacted { at: number; pre: number; post?: number }
/** A file the last turn changed (ideas §3): absolute path, line counts, first changed line. */
export interface TurnFileView { path: string; added: number; removed: number; line: number }
/**
 * What the last turn did (ideas §3), once it has an outcome: at most 50 files (`fileCount` is the
 * real number), totals over all of them (a lower bound when `partial`), `ms` its length when known.
 */
export interface TurnView {
  files: TurnFileView[]; fileCount: number; added: number; removed: number; partial: boolean; ms?: number;
}
/** One window of a plan limit (usage-home spec §3); `resetsAtMs` only when the agent wrote a valid time. */
/** `id`: "5h", "7d" or "7d:<model>" (a model's weekly window, e.g. "7d:opus"). */
export interface ClaudeUsageWindow { id: string; percent: number; resetsAtMs?: number }
export interface CodexUsageWindow { minutes: number; percent: number; resetsAtMs?: number }
/**
 * An OpenRouter key's credits in US dollars (openrouter.rs): `limit` and `limitRemaining` null for a
 * key without a limit, `limitReset` ("daily", "weekly", "monthly") null for one that never resets.
 */
export interface OpenRouterUsage {
  fetchedAtMs: number;
  limit: number | null;
  limitRemaining: number | null;
  limitReset: string | null;
  usage: number;
  usageDaily: number;
  usageWeekly: number;
  usageMonthly: number;
}
/** The home page's limits; a side is null until its source gave a value. */
export interface Usage {
  claude: { fetchedAtMs: number; windows: ClaudeUsageWindow[] } | null;
  codex: { seenAtMs: number; windows: CodexUsageWindow[] } | null;
  /** Only while an OpenRouter key is saved and was read. */
  openrouter: OpenRouterUsage | null;
  /** Why the last read of the OpenRouter key failed (the Settings line); null when nothing failed. */
  openrouterProblem?: KeyProblem | null;
  /** Only while a DeepSeek key is saved and was read (absent from an older snapshot). */
  deepseek?: DeepSeekBalance | null;
  deepseekProblem?: KeyProblem | null;
  /** Only while a Moonshot (Kimi) key is saved and was read (absent from an older snapshot). */
  moonshot?: MoonshotBalance | null;
  moonshotProblem?: KeyProblem | null;
}
/** Why the last read of a provider key gave nothing (key_reads.rs): refused (401/403), or not reached. */
export type KeyProblem = "rejected" | "unreachable";
export type OpenRouterProblem = KeyProblem;
/** One currency of a DeepSeek balance (deepseek.rs): "CNY" or "USD". */
export interface CurrencyBalance { currency: string; total: number; granted: number; toppedUp: number }
/** A DeepSeek key's balance; `isAvailable` false when it is too low for API calls. */
export interface DeepSeekBalance { fetchedAtMs: number; isAvailable: boolean; balances: CurrencyBalance[] }
/** A Moonshot key's balance (moonshot.rs): "USD" on api.moonshot.ai, "CNY" on api.moonshot.cn; `cash` may be below 0. */
export interface MoonshotBalance { fetchedAtMs: number; currency: string; available: number; voucher: number; cash: number }
/** Who a session row belongs to (observe/model.rs `Source`). */
export type SessionAgent = "claude" | "codex" | "antigravity";
export interface SessionView {
  key: string; agent: SessionAgent; host: Host; title: string | null;
  phase: Phase; since: number; activity: string | null; detail: string | null;
  outcome: OutcomeView; attention: AttentionKind; acked: boolean;
  silent?: SilentReason;
  context?: ContextFill;
  /** Claude: the model id of the last answer, once known. */
  model?: string;
  compacted?: Compacted;
  turn?: TurnView;
}
export interface ProjectView { key: string; name: string; root: string; sessions: SessionView[] }
/** Row menu items (src-tauri/src/session_menu.rs `MenuItem`). */
export type SessionMenuItem = "copyReply" | "copyPath" | "copyResume" | "openFolder";
/** What a row menu item did (lib.rs `MenuDone`): "copy" = put `text` on the clipboard from the page. */
export type SessionMenuDone = { kind: "done" } | { kind: "copy"; text: string } | { kind: "failed" };
export interface SessionsSnapshot {
  rev: number; now: number; projects: ProjectView[]; attention: string[];
  counts: { waiting: number; error: number; finished: number; working: number };
  hold: boolean; aggregate: BotState;
  sources: { claude: Health; codex: Health; codexHooks: CodexHooksState };
  usage: Usage;
}
/** The agent whose CLI resumes a session (resume.rs `ResumeAgent`). */
export type ResumeAgent = "claude" | "codex";
/** How a recent session's last turn ended (resume.rs `RecentEnd`); "unfinished": closed mid-turn. */
export type RecentEnd = "finished" | "error" | "interrupted" | "unfinished";
/** A session of the last 24 hours that is no longer listed (resume.rs `RecentSession`). */
export interface RecentSession {
  agent: ResumeAgent; id: string; cwd: string; project: string;
  title: string | null; last: string | null; end: RecentEnd | null; at: number;
}
/** What «Resume» did (resume.rs `ResumeOutcome`). */
export type ResumeOutcome = "started" | "copied" | "failed";
export interface Alert {
  key: string; kind: "permission" | "input" | "error" | "finished"; rank: number;
  agent: SessionAgent; host: Host; project: string; title: string | null;
  detail: string | null; text: string; since: number;
}

// ── Hook installers ───────────────────────────────────────────────────────────

export type HookTarget = "claude" | "codex" | "antigravity";
export interface HookStatus {
  target: HookTarget; installed: boolean; outdated: boolean; settingsPath: string;
  hookPath: string; hookReady: boolean; notes: RustMessage[];
  /** Codex only (hooks.rs `Answers`): whether the island answers Codex permissions. */
  answers?: CodexAnswers;
}
/** "notInstalled": Mayak's Codex PermissionRequest group is not there, the integration comes first. */
export type CodexAnswers = "on" | "off" | "notInstalled";
export interface HookPreview {
  /** Hand `fingerprint` back to hooksApply so only the reviewed diff is ever written. */
  diff: string; backup: string; settingsPath: string; fingerprint: string; notes: RustMessage[];
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
  | { name: "hotkey-next"; payload: null }
  | { name: "hotkey-next-status"; payload: HotkeyStatus }
  | { name: "tray"; payload: string }
  | { name: "hook"; payload: Record<string, unknown> }
  | { name: "screen-changed"; payload: null }
  | { name: "sessions"; payload: SessionsSnapshot }
  | { name: "session-alert"; payload: Alert[] }
  | { name: "media"; payload: MediaState | null };

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
 * While drops are off (core/features.ts): a file dragged over the island shows the "no drop" cursor,
 * and a file dropped anyway is ignored. Without this the webview would navigate to the file.
 */
export function refuseDrops() {
  const refuse = (e: DragEvent) => {
    if (!e.dataTransfer?.types.includes("Files")) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "none";
  };
  window.addEventListener("dragover", refuse);
  window.addEventListener("drop", refuse);
}

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
  if (!file) return refused(t("upload.file"), t("drop.onlyFiles"));
  if (entry?.isDirectory) return refused(file.name, t("drop.noFolders"));
  if (file.size > MAX_DROP_BYTES) return refused(file.name, t("drop.tooBig"));
  return {
    name: file.name,
    ingest: async () => {
      let bytes: ArrayBuffer;
      try {
        bytes = await file.arrayBuffer();
      } catch {
        // A folder the entry did not reveal, or a file that went away meanwhile.
        throw new Error(t("drop.unreadable"));
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

/** What is playing changed (emit_to "island", only on a change); null when nothing is. */
export function onMedia(handler: (s: MediaState | null) => void): Promise<() => void> {
  return onEvent<MediaState | null>("media", handler);
}

/** These CLI chat conversations changed (chat/mod.rs; only while the island is shown). */
export function onAgentChatChanged(handler: (ids: number[]) => void): Promise<() => void> {
  return onEvent<number[]>("agent-chat-changed", handler);
}

/** A request of the CLI chat waits for a click (always sent, hidden or not). */
export function onAgentChatPermission(handler: (e: PermissionEvent) => void): Promise<() => void> {
  return onEvent<PermissionEvent>("agent-chat-permission", handler);
}

/** A plan-limit window has just reset while the island is visible (observe/mod.rs run_store). */
export function onUsageRollover(handler: () => void): Promise<() => void> {
  return onEvent<null>("usage-rollover", () => handler());
}

/** A new silent Codex row appeared: the island shows itself once, with no sound and no card. */
export function onSessionSilent(handler: () => void): Promise<() => void> {
  return onEvent<null>("session-silent", () => handler());
}
