// App state — mirror of AppState.swift (the parts the island needs).

import type { BotEmoteName, BotStateName, IslandMode, IslandViewName } from "./layout";
import type { EyeShape } from "../luch/engine";
import { Sessions, botStateName } from "./sessions";
import type { Alert, HookAgent, HookCardKind, MediaState, ProjectView, SessionView } from "./bridge";
import type { Question } from "../island/answers";
import type { FocusRecord } from "../island/focus";
import type { PomodoroRun } from "../island/pomodoro";
import { type OpenPage, viewOnOpen } from "./open-view";
import type { CustomApp } from "../views/apps-model";

export type AgentSource = "claudeCode" | "n8n";
export type PillBadge = "approval" | "finished" | "error";

export interface AgentTask {
  id: string;
  name: string;
  color: string;
  state: BotStateName;
  stepIndex: number;
  steps: string[];
  source: AgentSource;
  isIntegration: boolean;
  emote?: BotEmoteName | null;
  miniEye?: EyeShape | null;
  pillBadge?: PillBadge | null;
  sessionCwd?: string | null;
}

export interface ApprovalInfo {
  requestId: string;
  sessionId: string;
  tool: string;
  command: string;
  /** Observer key of the asking session (router annotation), null when unknown. */
  sessionKey: string | null;
  /** Caption of the card: project basename from the router, else the cwd basename. */
  projectName: string;
  /** A permission (the "approval" view) or a question (the "question" view). */
  kind: HookCardKind;
  agent: HookAgent;
  /** A Claude permission with a rule to keep: the card offers "Always". */
  canAlways: boolean;
  /** The question card's questions as received; [] for a permission. */
  questions: Question[];
}

/**
 * What an alert card shows when the snapshot cannot show State.alertKey: the
 * alert that opened the card (spec §9 sends it just before the snapshot that
 * contains it) or the row that was clicked (its ack can remove the session, §7).
 */
export type CardSubject =
  | { alert: Alert }
  | { session: SessionView; project: ProjectView };

export interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  content: string;
}

export type PromptContext =
  | { kind: "window"; appName: string; title: string; url?: string }
  | { kind: "file"; name: string; path?: string };

export interface ResultItem {
  label: string;
  detail: string;
  url?: string;
}

export interface SearchResult {
  title: string;
  items: ResultItem[];
  note?: string;
}

const task = (
  id: string, name: string, color: string, source: AgentSource,
): AgentTask => ({
  id, name, color, state: "idle", stepIndex: 0, steps: [], source, isIntegration: true,
});

/** AgentTask.integrationAgents — same ids, names and colours as macOS. */
export const INTEGRATION_AGENTS: AgentTask[] = [
  task("integration_resend", "Resend", "#22C55E", "n8n"),
  task("integration_n8n", "n8n", "#F29B38", "n8n"),
  task("integration_vercel", "Vercel", "#7C5CFF", "n8n"),
  task("integration_github", "GitHub", "#F4505E", "n8n"),
  task("integration_notion", "Notion", "#8C8C8C", "n8n"),
  task("integration_calcom", "Cal.com", "#C9956A", "n8n"),
  task("integration_stripe", "Stripe", "#0570DE", "n8n"),
];

export const TOGGLEABLE_INTEGRATION_IDS = [
  "integration_resend", "integration_n8n", "integration_vercel", "integration_github",
  "integration_notion", "integration_calcom", "integration_stripe",
];

/** What an integration poller last reported. */
export interface IntegrationInfo {
  data: Record<string, unknown>;
  error: string | null;
  loaded: boolean;
  configured: boolean;
}

export interface Settings {
  soundEnabled: boolean;
  soundVolume: number;
  autoCloseInterval: number;
  absenceInterval: number;
  activeIntegrations: string[];
  screen: "primary" | "cursor";
  autostart: boolean;
  /** Claude model used by the chat. */
  model: string;
  /** Alert kinds, switched separately; Rust defaults them to true for older files. */
  alertWaiting: boolean;
  alertFinished: boolean;
  alertError: boolean;
  /** Global shortcut that shows and hides the island, e.g. "Alt+M". Rust defaults it for older files. */
  hotkey: string;
  /** The shortcut's on/off switch; off keeps `hotkey` for when it is turned on again. */
  hotkeyEnabled: boolean;
  /** The prompt view's last chosen agent (a LauncherAgent name), "" until one is chosen. */
  launcherAgent: string;
  /** Unused since the pomodoro replaced the one-shot focus; kept so older files round-trip. */
  focusMinutes: number;
  /** During a muted pomodoro work interval, permission and question waits still open the island. */
  focusLetWaitsThrough: boolean;
  /** Pomodoro lengths in minutes and the work intervals before a long break (pomodoro spec §3). */
  pomodoroWork: number;
  pomodoroShort: number;
  pomodoroLong: number;
  pomodoroCycles: number;
  /** Agent alerts are held back during work intervals (pomodoro spec §2.3). */
  pomodoroMute: boolean;
  /** The player row in the footer of the main tabs (night spec §B). Rust defaults it to true for older files. */
  showMedia: boolean;
  /** UI language (batch spec §C): "system" follows the OS, else one of core/i18n LANGS. */
  language: string;
  /** The page the island opens on when the user opens it: an OpenOnClick, "sessions" by default. */
  openOnClick: string;
  /** The home page reads Codex's limits from the Codex CLI when the held ones are stale. Rust defaults it to true for older files. */
  codexFreshLimits: boolean;
  /** Luch's face and a sound when a Claude or Codex window crosses 80 % and 100 % (core/usage-alerts.ts). Rust defaults it to true for older files. */
  usageAlerts: boolean;
  /** The Apps tab's order, built-in and user app ids (views/apps-model.ts). Rust defaults the three to [] for older files. */
  appOrder: string[];
  /** Built-in apps hidden from the Apps tab; Settings shows them again. */
  hiddenApps: string[];
  /** Apps the user added to the Apps tab. */
  customApps: CustomApp[];
}

export const DEFAULT_SETTINGS: Settings = {
  soundEnabled: true,
  soundVolume: 0.12,
  autoCloseInterval: 15,
  absenceInterval: 180,
  activeIntegrations: [
    "integration_resend", "integration_n8n", "integration_vercel", "integration_github",
  ],
  screen: "primary",
  autostart: false,
  model: "claude-opus-5",
  alertWaiting: true,
  alertFinished: true,
  alertError: true,
  hotkey: "Alt+M",
  hotkeyEnabled: true,
  launcherAgent: "",
  focusMinutes: 25,
  focusLetWaitsThrough: true,
  pomodoroWork: 25,
  pomodoroShort: 5,
  pomodoroLong: 15,
  pomodoroCycles: 4,
  pomodoroMute: true,
  showMedia: true,
  language: "system",
  openOnClick: "sessions",
  codexFreshLimits: true,
  usageAlerts: true,
  appOrder: [],
  hiddenApps: [],
  customApps: [],
};

type Listener = () => void;

class AppState {
  mode: IslandMode = "hidden";
  view: IslandViewName = "overview";
  /**
   * The last page the user opened with a header tab (Sessions, Overview, Stats, Focus, Apps), for
   * "Open on click: Last opened". Kept in memory: a tab click costs no settings write and no
   * broadcast to the other windows, and a restart starting on the sessions is the natural reset.
   */
  lastOpenedPage: OpenPage | null = null;
  /** The session row highlighted with ↑ ↓ (core/island-keys.ts); null when none. Cleared on close and page change. */
  sessionSelection: string | null = null;

  tasks: AgentTask[] = [];
  focusId: string | null = null;

  stateOverride: BotStateName | null = null;

  /** Cursor in logical screen pixels, origin top-left (like AppState.mousePosition). */
  mouse = { x: 0, y: 0 };
  /** Cursor relative to the island's top-left corner. */
  mouseInIsland = { x: 0, y: 0 };

  isPinned = false;
  paused = false;
  /** The running pomodoro (pomodoro spec §2), null while off. Island.togglePomodoro owns it. */
  pomodoro: PomodoroRun | null = null;
  /** The card a phase change opens (spec §2.2): its title and what the work interval held. */
  phaseCard: { title: string; records: FocusRecord[] } | null = null;
  /** A phase card waits for an Allow/Deny card to close (spec §2.2). */
  phaseCardPending = false;

  uploadProgress = 0;
  uploadDuration = 2.4;
  fileDragOver = false;

  promptContext: PromptContext | null = null;
  droppedFile: { name: string; path: string } | null = null;
  noteMessage: string | null = null;
  searchResult: SearchResult | null = null;
  chatHistory: ChatMessage[] = [];
  /** The prompt view's agent is "Claude API": the view is the island chat (launcher spec §5). */
  launcherApi = false;
  /** The permission or question card (one at a time), null when none is up. */
  pendingApproval: ApprovalInfo | null = null;
  /** The question card's natural content height at full width, 0 until measured: the card follows it (layout.ts questionHeight). */
  questionContentH = 0;
  /** Session shown by the waiting / finished / error / session cards. */
  alertKey: string | null = null;
  /** What those cards fall back on (views/sessions.ts cardSubject). */
  alertFallback: CardSubject | null = null;
  /** Episode start (Alert.since) of what those cards show, null when unknown (spec §2.6). */
  alertSince: number | null = null;

  integrations: Record<string, IntegrationInfo> = {};

  /** What is playing (night spec §B), null when nothing is; the player row (views/usage.ts buildPlayer) owns it. */
  media: MediaState | null = null;

  lastActivity = performance.now();

  settings: Settings = { ...DEFAULT_SETTINGS };

  private listeners = new Set<Listener>();

  subscribe(fn: Listener): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Marks the UI dirty; the island re-renders on the next frame. */
  notify() {
    for (const fn of this.listeners) fn();
  }

  /** A pomodoro runs (not the focused pill, which is focusTask). */
  get pomodoroOn(): boolean {
    return this.pomodoro != null;
  }

  /** Agent alerts are held back: a work interval with Mute on (spec §2.3). */
  get muting(): boolean {
    return this.pomodoro?.phase === "work" && this.settings.pomodoroMute;
  }

  get focusTask(): AgentTask | null {
    return this.tasks.find((t) => t.id === this.focusId) ?? this.tasks[0] ?? null;
  }

  /** Luch shows the sessions aggregate while there are sessions (spec §10). */
  get effectiveState(): BotStateName {
    const snap = Sessions.hasSessions ? Sessions.snapshot : null;
    return this.stateOverride ?? (snap ? botStateName(snap.aggregate) : this.focusTask?.state ?? "idle");
  }

  get otherTasks(): AgentTask[] {
    return this.tasks.filter((t) => t.id !== this.focusId);
  }

  /** Messages the prompt view's height counts: the chat's while it answers here, none while it launches. */
  get promptCount(): number {
    return this.launcherApi ? this.chatHistory.length : 0;
  }

  setFocus(id: string) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    this.focusId = id;
    t.pillBadge = null;
    this.notify();
  }

  updateTask(id: string, state: BotStateName) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.state = state;
    this.notify();
  }

  appendStep(id: string, step: string) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.steps.push(step);
    if (t.steps.length > 20) t.steps.shift();
    t.stepIndex = t.steps.length - 1;
    this.notify();
  }

  setPillBadge(id: string, badge: PillBadge | null) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.pillBadge = badge;
    this.notify();
  }

  /** loadIntegrationTasks() — opt-in integrations (max 4), in their declared order. */
  loadIntegrationTasks() {
    for (const proto of INTEGRATION_AGENTS) {
      const shouldLoad = this.settings.activeIntegrations.includes(proto.id);
      const idx = this.tasks.findIndex((t) => t.id === proto.id);
      if (shouldLoad && idx < 0) this.tasks.push({ ...proto, steps: [] });
      if (!shouldLoad && idx >= 0) this.tasks.splice(idx, 1);
    }
    // Keep the declared order so pills never shuffle.
    const order = INTEGRATION_AGENTS.map((t) => t.id);
    this.tasks.sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id));
    // Focus falls back to the first pill, or to nothing when no pill is on.
    if (!this.tasks.some((t) => t.id === this.focusId)) this.focusId = this.tasks[0]?.id ?? null;
    this.notify();
  }

  toggleIntegration(id: string) {
    const active = this.settings.activeIntegrations;
    if (active.includes(id)) {
      this.settings.activeIntegrations = active.filter((x) => x !== id);
    } else {
      if (active.length >= 4) return;
      this.settings.activeIntegrations = [...active, id];
    }
    this.loadIntegrationTasks();
  }

  /** Agent sessions first (spec §10), then the home page with the plan limits (usage-home spec §4). */
  defaultView(): IslandViewName {
    return Sessions.hasRows ? "sessions" : "overview";
  }

  /**
   * The page a user-initiated open shows (click on the compact island, tray "Open", the shortcut):
   * Settings → General → "Open on click". Alerts and cards open their own card, not this.
   * `hasSessions`: the shortcut has always opened the session list, even an empty one.
   */
  openView(hasSessions: boolean = Sessions.hasRows): IslandViewName {
    return viewOnOpen(this.settings.openOnClick, this.lastOpenedPage, hasSessions);
  }
}

export const State = new AppState();
