// Island geometry — ported from IslandTypes.swift + IslandWindowController.islandSize
// + IslandRootView.botPosition. All values are logical pixels, identical to the
// macOS app's points.

export type IslandMode = "hidden" | "compact" | "expanded";

export type IslandViewName =
  | "overview"
  | "empty"
  | "approval"
  | "question"
  | "error"
  | "finished"
  | "confused"
  | "upload"
  | "uploading"
  | "choose"
  | "mail"
  | "prompt"
  | "searching"
  | "result"
  | "note"
  | "settings"
  | "greeting"
  | "sessions"
  | "waiting"
  | "session"
  | "focus-phase"
  | "stats"
  | "focus"
  | "apps";

export type BotStateName =
  | "idle"
  | "working"
  | "thinking"
  | "searching"
  | "approval"
  | "question"
  | "error"
  | "finished"
  | "ratelimit"
  | "sleeping"
  | "dizzy";

export type BotEmoteName = "love" | "surprised" | "proud" | "wink" | "yawn" | "happy" | "annoyed";

export type AgentLayoutMode = "none" | "grid" | "pills" | "column";

export interface ViewLayout {
  height: number;
  botX: number;
  botY: number | null; // null = auto-centred
  botDiameter: number;
  agentMode: AgentLayoutMode;
}

// The window is a fixed 720×560 like the macOS panel; the island is drawn inside it, glued to
// the top edge and horizontally centred. Only the question card goes past 320 (answers spec §6).
export const PANEL_W = 720;
export const PANEL_H = 560;

// No notch on a PC: these are the hidden/compact sizes from docs/SPEC.md.
export const NOTCH_W = 184;
export const NOTCH_H = 32;
export const COMPACT_W = 288; // NOTCH_W + 104
export const EXPANDED_W = 640;

export const ROUNDED_CORNER = 14; // hidden / compact
export const EXPANDED_CORNER = 22;

/** Invisible hover strip that wakes the island when hidden. */
export const WAKE_STRIP_W = 240;
export const WAKE_STRIP_H = 6;

export const VIEW_LAYOUTS: Record<IslandViewName, ViewLayout> = {
  overview: { height: 160, botX: 68, botY: null, botDiameter: 58, agentMode: "pills" },
  empty: { height: 160, botX: 70, botY: null, botDiameter: 62, agentMode: "none" },
  approval: { height: 160, botX: 62, botY: null, botDiameter: 56, agentMode: "column" },
  // The shortest question card; questionHeight follows its content. A small Luch stays at the
  // top left, by the asking line, however tall the card grows.
  question: { height: 170, botX: 46, botY: 78, botDiameter: 40, agentMode: "column" },
  error: { height: 160, botX: 62, botY: null, botDiameter: 58, agentMode: "column" },
  finished: { height: 160, botX: 62, botY: null, botDiameter: 58, agentMode: "column" },
  confused: { height: 160, botX: 76, botY: null, botDiameter: 66, agentMode: "column" },
  upload: { height: 176, botX: 140, botY: 104, botDiameter: 62, agentMode: "column" },
  // botY 103 = bar top (42 + 58) + 3, so the dot really rides the bar. The Swift
  // layout says 118 while its own comment says 103; the comment matches the spec.
  uploading: { height: 176, botX: 46, botY: 103, botDiameter: 20, agentMode: "none" },
  choose: { height: 176, botX: 60, botY: 101, botDiameter: 52, agentMode: "column" },
  mail: { height: 240, botX: 56, botY: null, botDiameter: 46, agentMode: "column" },
  prompt: { height: 160, botX: 52, botY: null, botDiameter: 44, agentMode: "column" },
  searching: { height: 160, botX: 52, botY: null, botDiameter: 44, agentMode: "column" },
  result: { height: 160, botX: 52, botY: null, botDiameter: 44, agentMode: "column" },
  note: { height: 160, botX: 60, botY: null, botDiameter: 50, agentMode: "column" },
  settings: { height: 160, botX: 54, botY: null, botDiameter: 46, agentMode: "none" },
  greeting: { height: 150, botX: 320, botY: 90, botDiameter: 0, agentMode: "none" },
  // The sessions list takes the whole card, so Luch is not drawn there.
  sessions: { height: 160, botX: 320, botY: 90, botDiameter: 0, agentMode: "none" },
  waiting: { height: 160, botX: 62, botY: null, botDiameter: 56, agentMode: "column" },
  session: { height: 160, botX: 62, botY: null, botDiameter: 58, agentMode: "column" },
  // The card a pomodoro phase change opens (pomodoro spec §2.2): the same place as the finished card.
  "focus-phase": { height: 160, botX: 62, botY: null, botDiameter: 58, agentMode: "column" },
  // Token statistics (night spec §C): the chart and tables take the whole card, like the list.
  stats: { height: 260, botX: 320, botY: 90, botDiameter: 0, agentMode: "none" },
  // The Focus tab (pomodoro spec §3): no Luch, like Stats.
  focus: { height: 260, botX: 320, botY: 90, botDiameter: 0, agentMode: "none" },
  // The Apps tab: a grid of tiles, no Luch.
  apps: { height: 200, botX: 320, botY: 90, botDiameter: 0, agentMode: "none" },
};

// The upload views above are only the fallback geometry. Once a file is actually
// dropped the whole sequence — Mochi included — is drawn by src/upload, which
// owns its own constants (USC) straight from UploadSequenceEngine.swift.

/** Chat view grows with the conversation — IslandContainer.chatPromptHeight. */
export function chatPromptHeight(messageCount: number): number {
  return Math.min(300, 240 + messageCount * 40);
}

/** Sessions list grows with its 24 px lines, then scrolls (spec §10). */
export function sessionsHeight(rows: number): number {
  return Math.min(300, 72 + 24 * rows);
}

/**
 * The player row (night spec §B): a footer under the card of every main tab, which grows by it while
 * something plays. 8 px above the 44 px row; #content's 10 px bottom padding stays under it.
 */
export const PLAYER_ROW_H = 52;

/** The main tabs (the header's pages, core/open-view.ts OPEN_PAGES): the views with the player footer. */
export const PLAYER_VIEWS: ReadonlySet<IslandViewName> = new Set<IslandViewName>(["sessions", "overview", "stats", "focus", "apps"]);

/** The player footer shows on this view (a main tab) when something plays; never on cards or the greeting. */
export function showsPlayer(view: IslandViewName): boolean {
  return PLAYER_VIEWS.has(view);
}

/** What the island adds around a view's card: 8 px top inset, the 34 px header, 10 px bottom (style.css #content). */
export const CARD_CHROME_H = 52;

/**
 * The question card is as tall as its content (`contentH`, the card's natural height at full
 * width, 0 before it is measured), at least its layout height and at most the panel; beyond
 * that its options list scrolls (spec §6).
 */
export function questionHeight(contentH: number): number {
  const wanted = Math.ceil(Math.max(0, contentH)) + CARD_CHROME_H;
  return Math.min(PANEL_H, Math.max(VIEW_LAYOUTS.question.height, wanted));
}

export function islandSize(
  mode: IslandMode,
  view: IslandViewName,
  chatCount = 0,
  sessionRows = 0,
  playerRow = false,
  questionContentH = 0,
): { w: number; h: number } {
  switch (mode) {
    case "hidden":
      // No notch to hide inside on a PC: the island retracts to zero height and
      // slides into the top edge of the screen instead of sitting there as a bar.
      return { w: NOTCH_W, h: 0 };
    case "compact":
      return { w: COMPACT_W, h: NOTCH_H };
    case "expanded": {
      const h =
        view === "prompt" ? chatPromptHeight(chatCount)
        : view === "sessions" ? sessionsHeight(sessionRows)
        : view === "question" ? questionHeight(questionContentH)
        : VIEW_LAYOUTS[view].height;
      // The card keeps its height; the player footer is added under it.
      return { w: EXPANDED_W, h: h + (playerRow && showsPlayer(view) ? PLAYER_ROW_H : 0) };
    }
  }
}

export interface BotPlacement {
  cx: number;
  cy: number;
  diameter: number;
  opacity: number;
}

/** IslandRootView.botPosition — cy is measured from the island's top edge. */
export function botPosition(
  mode: IslandMode,
  view: IslandViewName,
  islandH: number,
  uploadProgress = 0,
): BotPlacement {
  switch (mode) {
    case "hidden":
      return { cx: 46, cy: 16, diameter: 6, opacity: 0 };
    case "compact":
      return { cx: 40, cy: 16, diameter: 20, opacity: 1 };
    case "expanded": {
      const layout = VIEW_LAYOUTS[view];
      if (view === "uploading") {
        return {
          cx: 36 + uploadProgress * 526,
          cy: layout.botY ?? 103,
          diameter: layout.botDiameter,
          opacity: 1,
        };
      }
      if (layout.botY != null) {
        return { cx: layout.botX, cy: layout.botY, diameter: layout.botDiameter, opacity: 1 };
      }
      // Centre of the fixed 84 pt card (8 pt top inset + 34 pt header → content at y = 42)
      const headerBottom = 42;
      const cardH = 84;
      const cy = headerBottom + (islandH - headerBottom - cardH) / 2 + cardH / 2;
      return { cx: layout.botX, cy, diameter: layout.botDiameter, opacity: 1 };
    }
  }
}

export function botGlowColor(s: BotStateName): string {
  switch (s) {
    case "working":
      return "#3B9EFF";
    case "thinking":
      return "#A78BFA";
    case "searching":
      return "#6366F1";
    case "approval":
      return "#F5A524";
    case "error":
      return "#F4505E";
    case "finished":
      return "#34D399";
    case "ratelimit":
      return "#F59E0B";
    default:
      return "#FFFFFF";
  }
}

export function botGlowOpacity(s: BotStateName): number {
  switch (s) {
    case "idle":
    case "sleeping":
      return 0.15;
    case "dizzy":
      return 0;
    default:
      return 0.65;
  }
}

// Project colours (IslandConst.projectColors)
const PROJECT_COLORS: Record<string, string> = {
  korus: "#FF5A4E",
  "sbe hub": "#2EC4A0",
  "morning ai brief": "#F29B38",
  "publication ig": "#7C5CFF",
  "ig post": "#7C5CFF",
  "louisraille.fr": "#38BDF8",
  louisraille: "#38BDF8",
  "notch buddy": "#EC4899",
  "notch-buddy": "#EC4899",
  notchbuddy: "#EC4899",
};

const FALLBACK_COLORS = ["#22C55E", "#EAB308", "#60A5FA", "#E879F9"];

export function colorForProject(name: string): string {
  const key = name.toLowerCase().trim();
  const exact = PROJECT_COLORS[key];
  if (exact) return exact;
  for (const [k, c] of Object.entries(PROJECT_COLORS)) {
    if (key.startsWith(k) || key.includes(k)) return c;
  }
  let hash = 0;
  for (let i = 0; i < name.length; i++) hash = (hash * 31 + name.charCodeAt(i)) | 0;
  return FALLBACK_COLORS[Math.abs(hash) % FALLBACK_COLORS.length];
}

// Card wash colours (CardBackground.washColor)
export type Wash = "red" | "green" | "pink" | "amber" | "cyan" | "indigo" | "soft" | null;

export function washRGBA(wash: Wash): string {
  switch (wash) {
    case "red":
      return "rgba(244,80,94,0.55)";
    case "green":
      return "rgba(52,211,153,0.5)";
    case "pink":
      return "rgba(244,114,182,0.55)";
    case "amber":
      return "rgba(245,165,36,0.42)";
    case "cyan":
      return "rgba(34,211,238,0.38)";
    case "indigo":
      return "rgba(99,102,241,0.5)";
    case "soft":
      return "rgba(255,255,255,0.08)";
    default:
      return "rgba(0,0,0,0)";
  }
}
