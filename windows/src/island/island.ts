// The island: DOM shell, sizing animation, Luch placement, mouse handling.
// Mirrors IslandRootView.swift + IslandWindowController.swift.

import { Tracked, Spring } from "../core/anim";
import { CHAT_ENABLED } from "../core/features";
import { t } from "../core/i18n";
import {
  Bridge, IS_TAURI, onDragDrop, onMedia, refuseDrops, type Alert, type DragDropPayload, type DroppedFile, type SessionsSnapshot,
} from "../core/bridge";
import {
  EXPANDED_CORNER, EXPANDED_W, NOTCH_W, PANEL_H, PANEL_W,
  ROUNDED_CORNER, VIEW_LAYOUTS, botGlowColor, botGlowOpacity, botPosition, chatPromptHeight,
  islandSize,
  type IslandMode, type IslandViewName,
} from "../core/layout";
import { ALERT_SOUND, Sessions, viewForAttention } from "../core/sessions";
import { Sound } from "../core/sound";
import { State, type ApprovalInfo } from "../core/state";
import { BotEngine, hexToRGB, type RGB } from "../luch/engine";
import { Greeting, type CompactLook } from "../luch/greeting";
import { createMiniBot, miniFire, pruneMiniBots, syncMiniBotStates, tickMiniBots } from "../luch/minibots";
import { UploadCanvas } from "../upload/canvas";
import { USC, UploadSeq } from "../upload/sequence";
import { buildHeader, buildViews, type ViewActions, type ViewHost } from "../views/views";
import { cardSubject } from "../views/sessions";
import { h, svg } from "../views/dom";
import { ICONS } from "../views/icons";
import { addFocusRecord, shouldOpenForAlert, type FocusRecord } from "./focus";
import { activityFor } from "./activity";
import {
  advanceRun, minutesLeft, msToNextMinute, normalizeConfig, startRun, stopRun, type PomodoroRun,
} from "./pomodoro";
import { IslandStateMachine } from "./fsm";
import { CHARACTER_FPS, cursorNeedsFrame, fireDelay, fireFps, tooSoon } from "./pace";
import { STRIP_ZONE_W, lineAt } from "./strip";
import { episodeStart, nextAlert, staleCard, type CardEpisode } from "./alert-queue";
import { clickCounts, type CardBinding } from "./answers";

/** Pomodoro phase colours (pomodoro spec §4): the strip's violet and green. */
const WORK_RGB: RGB = [167 / 255, 139 / 255, 250 / 255];
const BREAK_RGB: RGB = [52 / 255, 211 / 255, 153 / 255];

const BOT_OVERHANG = 40;
/** Same margin as the Rust hit test (src-tauri/src/island.rs). */
const HIT_MARGIN = 14;

/** The three views the drop sequence owns; leaving them stops the engine. */
const UPLOAD_VIEWS: ReadonlySet<IslandViewName> = new Set(["upload", "uploading", "choose"]);

/** Views where the user answers or types: a phase card is held over them (pomodoro spec §2.2). */
const VIEWS_IN_USE: ReadonlySet<IslandViewName> = new Set([
  "approval", "question", "waiting", "session", "settings", "prompt", "upload", "uploading", "choose",
]);

/** Seconds between the drop and the moment the progress bar starts filling. */
const PRE_PROGRESS = USC.T_PROG_START - USC.T_DROP;

const modeOrder = (m: IslandMode) => (m === "hidden" ? 0 : m === "compact" ? 1 : 2);

/** Compact counters: unacked waits, errors, finished turns, then working sessions. */
const COUNTERS = [
  ["waiting", "!", "amber"],
  ["error", "✕", "red"],
  ["finished", "✓", "green"],
  ["working", "•", "blue"],
] as const;

export class Island {
  readonly fsm = new IslandStateMachine();

  private root: HTMLElement;
  private islandEl!: HTMLElement;
  private clipEl!: HTMLElement;
  private contentEl!: HTMLElement;
  private viewsEl!: HTMLElement;
  private botCanvas!: HTMLCanvasElement;
  private botGlow!: HTMLElement;
  private greetingCanvas!: HTMLCanvasElement;
  private miniGrid!: HTMLElement;
  private counters!: HTMLElement;
  private focusMins!: HTMLElement;
  private countdown!: HTMLElement;
  private wakeStrip!: HTMLElement;
  /** Windows: the visible line of a hidden island (strip.rs drives it). */
  private stripLine!: HTMLElement;
  private nativeStrip = false;
  /** strip-fullscreen: an app covers the island's monitor, so the line is not shown. */
  private stripFullscreen = false;
  /** Compact island: hide to the strip, shown while the cursor is on the island. */
  private petitCollapse!: HTMLElement;
  /** main.ts hands the tray's setPaused here: the quiet button is that same pause (spec §5). */
  quietHandler: (() => void) | null = null;

  private header!: ViewHost;
  private views!: Map<IslandViewName, ViewHost>;
  /** Last value handed to each view's setActive. */
  private activeViews = new Map<IslandViewName, boolean>();
  private uploadCanvas!: UploadCanvas;

  private width = new Tracked(NOTCH_W);
  private height = new Tracked(0);
  private radius = new Tracked(ROUNDED_CORNER);
  private botCx = new Spring(46);
  private botCy = new Spring(16);
  private botSize = new Spring(10);

  private engine = new BotEngine();
  private greeting = new Greeting();

  private running = false;
  private lastFrame = 0;
  /** The pending wait of a fire-only loop (pace.ts): ensureRunning cuts it short. */
  private fireTimer: number | null = null;
  /** Only the character moved last frame: the next one keeps to CHARACTER_FPS (pace.ts). */
  private characterOnly = false;
  /** The compact mini grid is showing (syncDom). */
  private gridShown = false;
  /**
   * Set while the island narrows after a greeting that landed in the compact pose: Луч, the mini
   * grid and the counters keep the screen places the greeting drew them in, instead of riding the
   * shrinking edges, so the island's own take over from its last frame without a seam.
   */
  private landing = false;
  private dirty = true;
  private canvasPx = 0;

  // Rust starts the window at full size so the launch greeting has room.
  private collapsed = false;
  private collapseTimer: number | null = null;
  private wasInIsland = false;
  /** Last shape handed to Rust for the click-through test. */
  private pushedRect = { x: -1, y: -1, w: -1, h: -1 };
  private homeCollapseAt: number | null = null;
  /** homeCollapseAt the countdown animation runs for; null when none runs. */
  private countdownFor: number | null = null;
  /** Set while an agent alert opens the island: its sound is the batch's only one (spec §10). */
  private silentOpen = false;
  /** Set while launch({ greet: false }) opens the compact island: a reload makes no sound. */
  private quietLaunch = false;

  // Bot hover → love (IslandWindowController.botHoverIn)
  private botHovering = false;
  private botHoverTimer: number | null = null;
  private lastLoveTime = 0;
  private botHoverStart = { x: 0, y: 0 };

  private confusedRecovery: number | null = null;
  /** The reset after a failed upload's note: re-armed, never stacked, by the next failure. */
  private noteReset: number | null = null;
  private prevViewBeforeConfused: IslandViewName = "overview";
  private lastSyncedView: IslandViewName | null = null;

  /** Drop sequence bookkeeping: last tick played, and whether the ✓ has fired. */
  private uploadTens = 0;
  private uploadDone = false;
  /** A file drag entered the island since the mouse button last went up (onPointerReleased). */
  private fileDragSeen = false;
  /** The one timer of a pomodoro: the end of the current phase (spec §1). No tick while hidden. */
  private phaseTimer: number | null = null;
  /** The minutes label's tick: only while the island is visible and a pomodoro runs (spec §4). */
  private minuteTimer: number | null = null;

  constructor(root: HTMLElement) {
    this.root = root;
    this.build();
    this.wireFsm();
    this.wireInput();
    this.engine.onDizzy = () => this.handleDizzy();
    // Dev builds expose the engine so states and emotes can be driven from the devtools.
    if (import.meta.env.DEV) (window as unknown as { __luchEngine?: BotEngine }).__luchEngine = this.engine;
    this.greeting.onComplete = () => this.fsm.greetComplete();
    State.subscribe(() => {
      this.dirty = true;
      this.ensureRunning();
    });
  }

  // ── DOM ─────────────────────────────────────────────────────────────────────

  private build() {
    const actions: ViewActions = {
      setView: (v) => this.setView(v),
      collapse: () => this.collapse(),
      forceCollapse: () => {
        State.isPinned = false;
        this.fsm.pinned = false;
        this.fsm.forceCollapse();
      },
      toggleQuiet: () => this.quietHandler?.(),
      togglePomodoro: () => this.togglePomodoro(),
      checkPomodoro: () => this.checkPomodoro(),
      setFocus: (id) => {
        State.setFocus(id);
        Sound.play("blip");
      },
      // The ↗ button — same targets as openAgentTarget() on macOS.
      openTarget: () => {
        const task = State.focusTask;
        if (!task) return;
        const urls: Record<string, string> = {
          integration_resend: "https://resend.com/emails",
          integration_vercel: "https://vercel.com/dashboard",
          integration_github: "https://github.com",
          integration_stripe: "https://dashboard.stripe.com/payments",
          integration_notion: "https://notion.so",
          integration_calcom: "https://app.cal.com/bookings",
        };
        if (task.id === "integration_n8n") void Bridge.openN8n();
        else if (urls[task.id]) void Bridge.openUrl(urls[task.id]);
      },
      openUrl: (url) => {
        if (url) void Bridge.openUrl(url);
      },
      decide: (bound, d) => {
        const req = this.clickedCard(bound, `decide ${d}`);
        if (!req || req.kind !== "permission") return;
        Sound.play(d === "deny" ? "blip" : "approve");
        void Bridge.approvalDecision(req.requestId, d);
        this.closeCard();
      },
      inTerminal: (bound) => {
        const req = this.clickedCard(bound, "in terminal");
        if (!req) return;
        Sound.play("blip");
        void Bridge.approvalDecline(req.requestId);
        this.closeCard();
      },
      answer: (bound, answers) => {
        const req = this.clickedCard(bound, "answer");
        if (!req || req.kind !== "question") return;
        Sound.play("approve");
        const sent = Bridge.questionAnswer(req.requestId, answers);
        this.closeCard();
        void sent.then((ok) => {
          if (ok) return;
          // Rust refused the answers, so the request still waits: release it, the agent asks
          // in its own window, and the island says why its click did nothing.
          void Bridge.approvalDecline(req.requestId);
          if (State.mode !== "expanded" || State.pendingApproval != null) return;
          State.noteMessage = t("question.refused");
          Sound.play("error");
          this.setView("note");
        });
      },
      holdOpen: (on) => {
        if (this.fsm.heldOpen === on) return;
        this.fsm.setHeldOpen(on);
        void Bridge.log(`question hold ${on ? "on" : "off"}`);
        if (on) {
          this.homeCollapseAt = null;
        } else if (this.fsm.state === "home" && !this.wasInIsland && !State.isPinned) {
          // Let go with the cursor off the island: it folds after autoCloseInterval, as on leaving it.
          this.armAutoClose();
          this.homeCollapseAt = performance.now() + State.settings.autoCloseInterval * 1000;
        }
        this.updateCountdown(performance.now());
      },
      toggleSound: () => {
        State.settings.soundEnabled = !State.settings.soundEnabled;
        Sound.setEnabled(State.settings.soundEnabled);
        void Bridge.saveSettings(State.settings);
        State.notify();
      },
      // While the slider moves: the sound follows at once, nothing is written.
      setVolume: (v) => {
        State.settings.soundVolume = v;
        Sound.setVolume(v);
      },
      // When the slider is released: one save.
      saveVolume: () => {
        void Bridge.saveSettings(State.settings);
        State.notify();
      },
      setAutoClose: (s) => {
        State.settings.autoCloseInterval = s;
        this.setAutoCloseDelay(s);
        void Bridge.saveSettings(State.settings);
        State.notify();
      },
      openSettingsWindow: () => void Bridge.openSettingsWindow(),
      blip: () => Sound.play("blip"),
      openSession: (key) => {
        // The row as it was before this click decides the card, and stays on it
        // when the ack removes the session (spec §7: Idle Codex, no process).
        const found = Sessions.find(key);
        State.alertFallback = found;
        void Bridge.sessionAck(key);
        State.alertKey = key;
        State.alertSince = found ? episodeStart(found.session) : null;
        Sound.play("blip");
        const s = found?.session;
        if (s?.phase.kind === "waiting") this.setView("waiting");
        else if (s?.attention === "error") this.setView("error");
        else if (s?.attention === "finished") this.setView("finished");
        else this.setView("session");
      },
      ackAlert: () => {
        if (State.alertKey) void Bridge.sessionAck(State.alertKey);
        this.collapse();
      },
      pageAlert: (step) => {
        const list = Sessions.snapshot?.attention ?? [];
        if (list.length === 0) return;
        const idx = State.alertKey ? list.indexOf(State.alertKey) : -1;
        const next =
          idx < 0 ? (step === 1 ? 0 : list.length - 1) : (idx + step + list.length) % list.length;
        const session = Sessions.find(list[next])?.session;
        const kind = session?.attention;
        if (!session || !kind) return;
        State.alertKey = list[next];
        State.alertSince = episodeStart(session);
        Sound.play("blip");
        this.setView(viewForAttention(kind));
      },
      openSessionTarget: () => {
        const key = State.alertKey;
        if (!key) return;
        // The open looks the session up first: the ack may remove it (Idle Codex, no process).
        // `sessionOpen` never rejects (a failed call is null), so the fold always follows.
        void Bridge.sessionOpen(key).then(() => {
          void Bridge.sessionAck(key);
          this.collapse();
        });
      },
      toggleIdleFold: (projectKey) => {
        if (Sessions.expandedIdle.has(projectKey)) Sessions.expandedIdle.delete(projectKey);
        else Sessions.expandedIdle.add(projectKey);
        Sound.play("blip");
        State.notify();
        this.animateGeometry(false);
      },
    };

    this.wakeStrip = h("div", { id: "wake-strip" });
    this.stripLine = h("div", { id: "strip-line", class: "rest" });
    this.botGlow = h("div", { id: "bot-glow" });
    this.botCanvas = h("canvas", { id: "bot-canvas" });
    this.greetingCanvas = h("canvas", { id: "greeting-canvas" });
    this.miniGrid = h("div", { id: "mini-grid" });
    this.counters = h("div", { id: "counters" });
    this.focusMins = h("div", { id: "focus-mins" });
    this.countdown = h("div", { id: "countdown" });
    this.petitCollapse = h(
      "button",
      {
        id: "petit-collapse",
        title: t("island.hide"),
        onmousedown: (e: Event) => {
          // The compact island opens on mousedown: this press must not reach it.
          e.stopPropagation();
          Sound.resume();
          this.fsm.forceCollapse();
        },
      },
      svg(ICONS.chevronUp, 9, { stroke: 2.4 }),
    );

    this.header = buildHeader(actions);
    this.views = buildViews(actions, () => this.animateGeometry(false));
    this.viewsEl = h("div", { id: "views" });
    for (const v of this.views.values()) this.viewsEl.append(v.el);
    this.contentEl = h("div", { id: "content" }, this.header.el, this.viewsEl);

    // The drop sequence draws the card, the bar and its own Mochi. It sits under
    // the header, which stays visible on top of it exactly as on macOS.
    this.uploadCanvas = new UploadCanvas({
      ask: () => {
        State.promptContext = State.droppedFile
          ? { kind: "file", name: State.droppedFile.name, path: State.droppedFile.path }
          : null;
        this.setView("prompt");
      },
      cancel: () => this.setView(State.defaultView()),
    });

    this.clipEl = h(
      "div",
      { id: "island-clip" },
      this.greetingCanvas,
      this.uploadCanvas.el,
      this.contentEl,
    );
    this.islandEl = h(
      "div",
      { id: "island" },
      this.clipEl,
      this.botGlow,
      this.botCanvas,
      this.miniGrid,
      this.counters,
      this.focusMins,
      this.countdown,
      this.petitCollapse,
    );

    const dpr = Math.min(2, window.devicePixelRatio || 1);
    this.greetingCanvas.width = Math.round(EXPANDED_W * dpr);
    this.greetingCanvas.height = Math.round(150 * dpr);
    this.greetingCanvas.style.width = `${EXPANDED_W}px`;
    this.greetingCanvas.style.height = "150px";

    this.root.append(this.wakeStrip, this.stripLine, this.islandEl);
    this.applyGeometry();
  }

  // ── FSM ─────────────────────────────────────────────────────────────────────

  private wireFsm() {
    this.fsm.homeToPetitDelay = State.settings.autoCloseInterval;
    this.fsm.onTransition = (from, to) => {
      switch (to) {
        case "hidden":
          // Pause (quiet mode) during the greeting: its timed sounds must not play on.
          if (from === "coucou") this.greeting.interrupt();
          this.setMode("hidden");
          if (from === "coucou") State.view = State.defaultView();
          break;
        case "petit":
          if (from === "coucou") {
            // Before interrupt(): it tells whether the greeting's last frame was the compact pose.
            this.landing = this.greeting.landed;
            this.greeting.interrupt();
            if (this.landing) {
              this.engine.takeOverFromGreeting();
              // the greeting drew the minis at full opacity: no fade-in from nothing
              this.miniGrid.style.transition = "none";
            }
          } else if (from === "hidden" && !this.quietLaunch) Sound.play("peek");
          this.setMode("compact");
          if (from === "coucou" || this.quietLaunch) State.view = State.defaultView();
          if (!this.wasInIsland) this.fsm.mouseLeft();
          break;
        case "home":
          // An alert that cuts the greeting short also stops its timed sounds.
          if (from === "coucou") this.greeting.interrupt();
          this.expand(State.openView());
          if (!this.wasInIsland) this.fsm.mouseLeft();
          break;
        case "coucou":
          this.expand("greeting");
          this.greeting.start();
          break;
      }
      State.notify();
    };
  }

  /**
   * `greet: false` after a language reload (batch spec §F): the page reloaded, the app did not
   * start, so the island comes back compact with no greeting and no sound, as the greeting leaves
   * it; in quiet mode it goes on hidden. A held phase card shows as the compact island opens.
   */
  launch({ greet = true }: { greet?: boolean } = {}) {
    if (greet) {
      this.fsm.launch();
      return;
    }
    this.quietLaunch = true;
    try {
      this.fsm.launch({ greet: false });
    } finally {
      this.quietLaunch = false;
    }
    if (State.paused) this.fsm.forceHidden();
  }

  // ── Mode / view ─────────────────────────────────────────────────────────────

  private setMode(mode: IslandMode) {
    const prev = State.mode;
    if (mode === prev) return;
    State.mode = mode;
    // Leaving the strip: it comes back at rest the next time the island hides.
    if (prev === "hidden") this.onStripCursor(null);
    if (mode === "expanded" && !this.silentOpen) Sound.play("open");
    if (prev === "expanded") {
      Sound.play("close");
      State.isPinned = false;
      void Bridge.focusWindow(false);
    }
    if (mode !== "expanded") {
      this.engine.resetMorph();
      // Nothing can be seen of the sequence once the island is shut, and leaving
      // it running would keep the frame loop awake — the island must cost
      // nothing while hidden.
      UploadSeq.deactivate();
    }
    // The pomodoro: a phase that ended while hidden (or asleep) moves on as the island shows,
    // and the minute tick runs only while it is visible (spec §2.5, §4). The check is deferred:
    // a phase change opens its card, and that must not re-enter setMode halfway through. A card
    // held while the island was hidden or over a view in use is released in the same callback.
    if (mode !== "hidden") {
      window.setTimeout(() => {
        this.checkPomodoro();
        this.releaseHeldCard();
      }, 0);
    }
    this.refreshActivity();
    this.syncMinuteTick();
    this.updateWindowCollapsed();
    this.animateGeometry(modeOrder(mode) < modeOrder(prev));
    State.notify();
  }

  /** True while the drop sequence owns the island body. */
  private get uploadActive(): boolean {
    return State.mode === "expanded" && UploadSeq.isActive && UPLOAD_VIEWS.has(State.view);
  }

  /** Navigating out of the drop flow ends the sequence, as on macOS. */
  private stopSequenceIfLeaving(view: IslandViewName) {
    if (UploadSeq.isActive && !UPLOAD_VIEWS.has(view)) UploadSeq.deactivate();
  }

  expand(view: IslandViewName) {
    this.stopSequenceIfLeaving(view);
    State.view = view;
    if (State.mode !== "expanded") this.setMode("expanded");
    else this.animateGeometry(false);
    State.lastActivity = performance.now();
    this.homeCollapseAt = null;
    this.updateCountdown(performance.now());
    State.notify();
  }

  setView(view: IslandViewName) {
    const prev = State.view;
    this.stopSequenceIfLeaving(view);
    if (State.mode !== "expanded") {
      this.fsm.forceHome();
      State.view = view;
      this.animateGeometry(false);
      State.notify();
    } else {
      const grew = VIEW_LAYOUTS[view].height >= VIEW_LAYOUTS[State.view].height;
      State.view = view;
      State.lastActivity = performance.now();
      this.animateGeometry(!grew);
      State.notify();
    }
    // Leaving a view may free a held phase card; deferred so it never re-enters setView mid-call.
    if (view !== prev) window.setTimeout(() => this.releaseHeldCard(), 0);
  }

  collapse() {
    State.isPinned = false;
    this.fsm.pinned = false;
    // Drive the state machine rather than the mode: setting the mode behind its
    // back left it thinking the island was still open, and a click on the compact
    // island then did nothing — the island could never be reopened.
    this.fsm.forcePetit();
  }

  /**
   * Alert from the hook server: open on this view. Pinned alerts never auto-close.
   * `silent`: the caller has played the alert's own sound, so opening plays no "open".
   */
  alert(view: IslandViewName, opts: { silent?: boolean } = {}) {
    this.fsm.pinned = State.isPinned;
    this.silentOpen = opts.silent ?? false;
    try {
      this.fsm.forceHome();
      this.expand(view);
    } finally {
      this.silentOpen = false;
    }
  }

  /**
   * After alert(): the open island folds after autoCloseInterval unless the cursor is on it. An
   * island that was already open got no new fold timer from forceHome (it only cancels the
   * timers), so a card on it would wait for the cursor to come and go before it could fold.
   */
  armAutoClose() {
    if (this.fsm.state === "home" && !this.wasInIsland) this.fsm.mouseLeft();
  }

  /** Shows the compact island for a non-alert event; never during a muted work interval (pomodoro spec §2.3). */
  reveal() {
    if (State.muting) return;
    this.fsm.reveal();
  }

  // ── Pomodoro (pomodoro spec) ────────────────────────────────────────────────

  /** The Focus tab's Start/Stop and the tray's Focus: start a pomodoro, or stop the running one. */
  togglePomodoro() {
    Sound.resume();
    if (State.pomodoro) {
      this.stopPomodoro();
      return;
    }
    // A new run starts with no card of an earlier one.
    State.phaseCard = null;
    State.phaseCardPending = false;
    State.pomodoro = startRun(normalizeConfig(State.settings), Date.now());
    this.schedulePhaseEnd();
    void Bridge.setFocusCheck(true);
    Sound.play("blip");
    State.notify();
    this.refreshActivity();
    this.syncMinuteTick();
  }

  /**
   * Spec §2.4: a stop shows what the current work interval held, plus what a still-pending card
   * held, as one "Focus stopped" card; else nothing, and no card of the run outlives it.
   */
  private stopPomodoro() {
    const run = State.pomodoro;
    if (!run) return;
    this.clearPhaseTimer();
    State.pomodoro = null;
    void Bridge.setFocusCheck(false);
    const held = stopRun(run);
    const pending = State.phaseCardPending ? State.phaseCard?.records ?? [] : [];
    // queuePhaseCard adds the pending card's records itself: only the run's records go in.
    if (pending.length + held.length > 0) this.queuePhaseCard(t("pomodoro.stopped"), held);
    else {
      State.phaseCard = null;
      State.phaseCardPending = false;
    }
    State.notify();
    this.refreshActivity();
    this.syncMinuteTick();
  }

  /**
   * A run kept across a language reload (main.ts, spec §C) goes on where it was: its phase timer
   * is armed again, so a phase that ended meanwhile moves on through checkPomodoro as usual.
   */
  resumePomodoro(run: PomodoroRun) {
    State.pomodoro = run;
    this.schedulePhaseEnd();
    void Bridge.setFocusCheck(true);
    State.notify();
    this.refreshActivity();
    this.syncMinuteTick();
  }

  /** Moves on when the current phase is over, else waits for its end again (spec §2.5). */
  checkPomodoro() {
    const run = State.pomodoro;
    if (!run) return;
    if (Date.now() >= run.endsAt) this.advancePhase();
    else this.schedulePhaseEnd();
  }

  private advancePhase() {
    const run = State.pomodoro;
    if (!run) return;
    const next = advanceRun(run, normalizeConfig(State.settings), Date.now(), t);
    State.pomodoro = next.run;
    this.schedulePhaseEnd();
    this.queuePhaseCard(next.title, next.held);
    State.notify();
    this.refreshActivity();
    this.syncMinuteTick();
  }

  /**
   * Luch's activity (spec §A, §4): headphones and notes while music plays, a laptop, a coffee or
   * sleep for the pomodoro phase, and the minutes in the Luch pill while expanded (in the compact
   * label while compact, none while hidden). One frame is drawn; only notes and sleep keep it going.
   */
  private refreshActivity() {
    const run = State.pomodoro;
    const now = Date.now();
    const look = activityFor(run?.phase ?? null, State.media?.playing === true);
    const bare = !look.headphones && !look.notes && look.prop == null && !look.sleep;
    this.engine.activity = run || !bare
      ? {
          ...look,
          minutes: run && State.mode === "expanded" ? Math.max(1, minutesLeft(run.endsAt, now)) : null,
          minutesColor: run?.phase === "work" ? WORK_RGB : BREAK_RGB,
        }
      : null;
    const compact = run != null && State.mode === "compact";
    this.focusMins.textContent = compact ? String(Math.max(1, minutesLeft(run.endsAt, now))) : "";
    this.focusMins.style.color = run?.phase === "work" ? "rgb(167, 139, 250)" : "rgb(52, 211, 153)";
    if (State.mode !== "hidden") this.ensureRunning();
  }

  /** (Re)arms the minute tick for the visible island; stops it while hidden or off. */
  private syncMinuteTick() {
    if (this.minuteTimer != null) window.clearTimeout(this.minuteTimer);
    this.minuteTimer = null;
    const run = State.pomodoro;
    if (!run || State.mode === "hidden") return;
    this.minuteTimer = window.setTimeout(() => {
      this.minuteTimer = null;
      this.checkPomodoro();
      this.refreshActivity();
      this.syncMinuteTick();
    }, Math.max(0, msToNextMinute(run.endsAt, Date.now())));
  }

  private schedulePhaseEnd() {
    this.clearPhaseTimer();
    const run = State.pomodoro;
    if (!run) return;
    this.phaseTimer = window.setTimeout(() => {
      this.phaseTimer = null;
      this.checkPomodoro();
    }, Math.max(0, run.endsAt - Date.now()));
  }

  private clearPhaseTimer() {
    if (this.phaseTimer != null) window.clearTimeout(this.phaseTimer);
    this.phaseTimer = null;
  }

  /**
   * Spec §2.2: the card for a phase change or a stop. Quiet mode drops it (no card, no sound).
   * While a permission or question card is up, or a view the user is using is open, it is held; a later card
   * replaces its title and adds its records. Over a view in use the phase change is heard at
   * once. Otherwise flushPhaseCard opens it now; a held card waits for releaseHeldCard.
   */
  private queuePhaseCard(title: string, held: FocusRecord[]) {
    if (State.paused) {
      State.phaseCard = null;
      State.phaseCardPending = false;
      return;
    }
    const earlier = State.phaseCardPending ? State.phaseCard?.records ?? [] : [];
    State.phaseCard = { title, records: [...earlier, ...held] };
    State.phaseCardPending = true;
    if (State.pendingApproval != null) return;
    if (this.viewInUse()) {
      Sound.play("finish");
      return;
    }
    this.flushPhaseCard();
  }

  /**
   * Pomodoro spec §2.2 as ruled 2026-10-07 (revised after the branch review): never over a view
   * the user is using. Only an expanded island on a view where the user answers or types is in
   * use; every other view (sessions, stats, focus, the home pages, the cards…) is not.
   */
  private viewInUse(): boolean {
    if (State.mode !== "expanded") return false;
    return VIEWS_IN_USE.has(State.view);
  }

  /**
   * Shows a held phase card when it may; true when it opened the card. It stays held while a
   * permission or question card is up or a view is in use, and while the island is hidden (it
   * shows at the next open); a held card with no records is dropped. Called deferred from setView
   * and setMode, and after a card's button closes it (closeCard, the hooks.ts time limit).
   */
  releaseHeldCard(): boolean {
    if (!State.phaseCardPending) return false;
    if (State.pendingApproval != null || this.viewInUse()) return false;
    if ((State.phaseCard?.records.length ?? 0) === 0) {
      State.phaseCard = null;
      State.phaseCardPending = false;
      return false;
    }
    if (State.mode === "hidden") return false;
    return this.flushPhaseCard();
  }

  /**
   * The only place a phase card opens: shows the pending card with the finished sound. False when
   * none is pending, or when quiet mode drops the card (no card, no sound). queuePhaseCard calls it
   * for a fresh card that may open at once, releaseHeldCard for a held one.
   */
  flushPhaseCard(): boolean {
    if (!State.phaseCardPending) return false;
    State.phaseCardPending = false;
    if (State.paused) {
      State.phaseCard = null;
      return false;
    }
    Sound.play("finish");
    this.alert("focus-phase", { silent: true });
    return true;
  }

  /** snapshot.hold reaches the FSM only while the tray pause is off: pause beats hold (spec §10). */
  syncHold(hold = Sessions.snapshot?.hold ?? false) {
    this.fsm.setHoldVisible(hold && !State.paused);
  }

  /**
   * The pending card a click on a permission or question card's button is for: the card the
   * button was built for, still pending, CLICK_GUARD_MS after its buttons were built. Null (the
   * click is ignored) for a click on a card that changed meanwhile, or the second click of a
   * double click that would otherwise answer the next card.
   */
  private clickedCard(bound: CardBinding, what: string): ApprovalInfo | null {
    const req = State.pendingApproval;
    if (!clickCounts(bound, req?.requestId, performance.now())) {
      void Bridge.log(`${what} ignored req=${bound.requestId} pending=${req?.requestId ?? "none"}`);
      return null;
    }
    void Bridge.log(`${what} req=${bound.requestId}`);
    return req;
  }

  /**
   * A button of the permission or question card answered or released it: the card goes, the
   * island stays open. A phase card that came while the card was up shows now: the card's view is
   * still in use here, so the default view opens and its deferred release shows the phase card.
   */
  closeCard() {
    State.pendingApproval = null;
    State.isPinned = false;
    this.fsm.pinned = false;
    if (!this.releaseHeldCard()) this.setView(State.defaultView());
  }

  /** A sessions snapshot passed the rev guard (island/observer.ts). */
  onSnapshot(s: SessionsSnapshot) {
    this.syncHold(s.hold);
    // Spec §9 sends an alert batch just before the snapshot that contains it, so
    // from this snapshot on the cards read the session again, not the alert.
    if (State.alertFallback && "alert" in State.alertFallback) State.alertFallback = null;
    this.moveOnFromStaleCard(s);
    State.notify();
    if (State.mode === "expanded" && State.view === "sessions") this.animateGeometry(false);
  }

  /**
   * Spec §2.6: the session behind the card on screen went on (answered in the agent's own
   * window, a new turn): the card gives way, silently, to the most important other unanswered
   * alert, or the island folds. Only the session cards; a view the user opened is left alone, and
   * so is a permission or question card, which ends only by the user (a click, "In terminal"),
   * the island leaving it, pause, a language reload or its 590 s limit.
   */
  private moveOnFromStaleCard(s: SessionsSnapshot) {
    if (State.mode !== "expanded") return;
    const card = this.cardEpisode();
    if (!card || !staleCard(card, s)) return;
    const letWaits = State.settings.focusLetWaitsThrough;
    const prefs = State.settings;
    const next = nextAlert(s, card, (kind) =>
      (kind === "permission" || kind === "input" ? prefs.alertWaiting : kind === "error" ? prefs.alertError : prefs.alertFinished)
      && shouldOpenForAlert(kind, { muting: State.muting, paused: State.paused, letWaits }));
    void Bridge.log(`stale card ${card.kind} -> ${next ? next.kind : "fold"}`);
    if (!next) {
      this.collapse();
      return;
    }
    State.alertKey = next.key;
    State.alertFallback = Sessions.find(next.key);
    State.alertSince = next.since;
    this.setView(viewForAttention(next.kind));
  }

  /**
   * The episode the session card on screen stands for; null on any other view (the permission and
   * question cards included), or when it is not known.
   */
  private cardEpisode(): CardEpisode | null {
    const view = State.view;
    if (view !== "waiting" && view !== "finished" && view !== "error") return null;
    const key = State.alertKey;
    const since = State.alertSince;
    return key != null && since != null ? { key, kind: view, since } : null;
  }

  /**
   * One batch of model alerts (spec §10). Kinds turned off in the settings never
   * reach here (Rust drops them); pause and a muted work interval are applied here. One sound per batch,
   * only when it opens or replaces a card or lands on the sessions list. During a muted work
   * interval (pomodoro spec §2.3), what it holds back is recorded for the phase card, paused or not.
   */
  handleAlerts(alerts: Alert[]) {
    // A live permission or question card already stands for that session's wait.
    const approvalKey = State.pendingApproval?.sessionKey ?? null;
    const fresh = alerts.filter(
      (a) => !((a.kind === "permission" || a.kind === "input") && approvalKey !== null && a.key === approvalKey),
    );
    const letWaits = State.settings.focusLetWaitsThrough;
    const run = State.pomodoro;
    if (run && State.muting) {
      const at = Date.now();
      for (const a of fresh) {
        if (!shouldOpenForAlert(a.kind, { muting: true, paused: false, letWaits })) {
          addFocusRecord(run.records, { key: a.key, project: a.project, kind: a.kind, at });
        }
      }
    }
    const batch = fresh.filter(
      (a) => shouldOpenForAlert(a.kind, { muting: State.muting, paused: State.paused, letWaits }),
    );
    if (batch.length === 0) return;
    // Most important = smallest rank; ties keep the batch order.
    const top = batch.reduce((best, a) => (a.rank < best.rank ? a : best));
    const view = State.view;

    // Hidden, compact, or the launch greeting: none of them is a view the user chose.
    if (State.mode !== "expanded" || view === "greeting") {
      Sound.play(ALERT_SOUND[top.kind]);
      State.alertKey = top.key;
      State.alertFallback = { alert: top };
      State.alertSince = top.since;
      this.alert(viewForAttention(top.kind), { silent: true });
    } else if ((view === "waiting" || view === "finished" || view === "error") && top.rank < this.cardRank(view)) {
      Sound.play(ALERT_SOUND[top.kind]);
      State.alertKey = top.key;
      State.alertFallback = { alert: top };
      State.alertSince = top.since;
      this.setView(viewForAttention(top.kind));
    } else if (view === "sessions") {
      // The rows re-sort from the snapshot that follows; the view stays.
      Sound.play(ALERT_SOUND[top.kind]);
      State.notify();
    } else {
      // A card that is not outranked, the approval card and every other view keep
      // the screen without a sound: only the counters and the pager move.
      State.notify();
    }
  }

  /**
   * A new silent Codex row (no hook reached Mayak for its turn): show the island on the
   * sessions list so the muted row is seen. No sound, no card, no alertFallback. Already
   * visible (compact or expanded), paused or in a muted work interval: nothing. The island then closes by its own rules.
   */
  handleSilentWake() {
    if (State.paused || State.muting || State.mode !== "hidden") return;
    this.alert("sessions", { silent: true });
    // The silent row may sit below the fold (silent-only projects come last): bring it into view.
    requestAnimationFrame(() => requestAnimationFrame(() => {
      const list = document.querySelector<HTMLElement>(".sess-list");
      const row = list?.querySelector<HTMLElement>(".sess-row.silent");
      if (!list || !row) return;
      const below = row.getBoundingClientRect().bottom - list.getBoundingClientRect().bottom;
      if (below > 0) list.scrollTop += below;
    }));
  }

  /** Rank of the alert card on screen, on the same 0..3 scale as Alert.rank. */
  private cardRank(view: "waiting" | "finished" | "error"): number {
    if (view === "error") return 2;
    if (view === "finished") return 3;
    const subject = cardSubject();
    if (subject && "alert" in subject) return subject.alert.rank;
    const phase = subject?.session.phase;
    return phase?.kind === "waiting" && phase.wait === "permission" ? 0 : 1;
  }

  // ── File drop ───────────────────────────────────────────────────────────────

  private onDragDrop(e: DragDropPayload) {
    void Bridge.log(`drag ${e.type}${e.type === "drop" ? ` ${e.file ? 1 : 0} file(s)` : ""}`);
    if (State.paused) return;
    switch (e.type) {
      case "enter": {
        if (State.fileDragOver) return;
        State.fileDragOver = true;
        this.fileDragSeen = true;
        this.engine.animateMorph(1);
        // enterZone must run before the island expands, so the sequence is
        // already active by the time the view becomes `upload`.
        UploadSeq.enterZone(State.mouseInIsland.x, State.mouseInIsland.y);
        this.alert("upload");
        // From the compact island (a drag that woke the strip), opening passes through the home
        // view, and leaving the drop views on the way switches the sequence off: back on.
        if (!UploadSeq.isActive) UploadSeq.enterZone(State.mouseInIsland.x, State.mouseInIsland.y);
        break;
      }
      case "leave": {
        if (!State.fileDragOver) return;
        State.fileDragOver = false;
        this.engine.animateMorph(0);
        // The island deliberately stays open: the drag session is still alive.
        UploadSeq.exitZone();
        State.notify();
        break;
      }
      case "drop": {
        State.fileDragOver = false;
        if (!e.file) {
          this.engine.animateMorph(0);
          this.setView(State.defaultView());
          return;
        }
        this.swallow(e.file.name, e.file.ingest);
        break;
      }
    }
  }

  /**
   * pointer-released (cursor poll): the left button went up. A file drag that had left the island
   * without a drop ended somewhere else — HTML5 has no event for that — so the drop view closes
   * instead of waiting for a file that is not coming.
   */
  onPointerReleased() {
    // Only the release that ends a file drag: a plain click on the "+" tab opens the same view
    // and must leave it open.
    const wasFileDrag = this.fileDragSeen;
    this.fileDragSeen = false;
    if (!wasFileDrag || State.fileDragOver || State.view !== "upload" || UploadSeq.dropped) return;
    void Bridge.log("drag ended outside the island");
    this.engine.animateMorph(0);
    this.setView(State.defaultView());
  }

  /**
   * Mochi eats the file. Nothing here waits on the file system: the copy into
   * the inbox runs in the background and fills the path in when it lands, so a
   * slow disk can never stall the animation — same as FileDropHandler on macOS.
   */
  private swallow(name: string, ingest: () => Promise<DroppedFile>) {
    // No path until the copy lands: the launcher refuses anything that is not in the inbox.
    State.droppedFile = { name, path: "" };
    State.promptContext = { kind: "file", name, path: "" };
    State.chatHistory = [];
    void Bridge.chatReset();

    UploadSeq.performDrop(State.uploadDuration);
    this.uploadTens = 0;
    this.uploadDone = false;

    this.engine.gulp();
    Sound.play("approve");
    this.engine.triggerEmote("happy");
    this.engine.animateMorph(0);

    State.uploadProgress = 0;
    this.setView("uploading");
    this.ensureRunning();

    void ingest()
      .then((file) => {
        // The user may have removed the file (or dropped another) while it was copied.
        if (State.droppedFile?.name !== name) return;
        State.droppedFile = { name: file.name, path: file.path };
        State.promptContext = { kind: "file", name: file.name, path: file.path };
        State.notify();
      })
      .catch((err) => {
        UploadSeq.deactivate();
        State.droppedFile = null;
        State.promptContext = null;
        State.noteMessage = String(err).replace(/^Error:\s*/, "");
        this.engine.animateMorph(0);
        this.setView("note");
        Sound.play("error");
        // Only while this note is still what the open island shows: a card that arrived since, or a
        // collapse, must not be replaced (and the island reopened) by a stale reset.
        if (this.noteReset != null) window.clearTimeout(this.noteReset);
        this.noteReset = window.setTimeout(() => {
          this.noteReset = null;
          if (State.view === "note" && State.mode === "expanded") this.setView(State.defaultView());
        }, 2400);
      });
  }

  /**
   * Sounds and view changes hung off the canvas timeline: a `tick` every 10 %,
   * the ✓ chime when the bar completes, then `choose` once Mochi has grown back.
   */
  private stepSequence() {
    const since = UploadSeq.sinceDrop();
    if (since == null) return;
    const dur = State.uploadDuration;
    const p = Math.max(0, Math.min(1, (since - PRE_PROGRESS) / dur));

    const tens = Math.floor(p * 10);
    if (tens > this.uploadTens && tens < 10) {
      this.uploadTens = tens;
      Sound.play("tick");
    }

    if (!this.uploadDone && since >= PRE_PROGRESS + dur) {
      this.uploadDone = true;
      Sound.play("approve");
      this.engine.triggerEmote("happy");
    }
    // The extra second is the grow-back, after which the choose card is up.
    if (since >= PRE_PROGRESS + dur + 1 && State.view === "uploading") {
      this.setView("choose");
    }
  }

  // ── Geometry ────────────────────────────────────────────────────────────────

  private targetSize(): { w: number; h: number; r: number } {
    const { w, h } = islandSize(
      State.mode, State.view, State.promptCount, Sessions.rowCount(), State.media != null, State.questionContentH,
    );
    const r = State.mode === "expanded" ? EXPANDED_CORNER : ROUNDED_CORNER;
    return { w, h, r };
  }

  private animateGeometry(shrinking: boolean) {
    const { w, h, r } = this.targetSize();
    if (shrinking) {
      this.width.curveTowards(w);
      this.height.curveTowards(h);
      this.radius.curveTowards(r);
    } else {
      this.width.springTo(w);
      this.height.springTo(h);
      this.radius.springTo(r);
    }
    this.ensureRunning();
  }

  private applyGeometry() {
    const w = this.width.value;
    // The spring overshoots its target a little: never past the bottom of the window (a 560 px
    // question card would lose its rounded corners for a few frames).
    const hh = Math.min(PANEL_H, this.height.value);
    const r = this.radius.value;
    this.islandEl.style.width = `${w}px`;
    this.islandEl.style.height = `${hh}px`;
    this.islandEl.style.borderRadius = `0 0 ${r}px ${r}px`;
    this.islandEl.style.transform = `translateX(-50%)`;
    // These follow the island as it resizes, so they belong here rather than in
    // the state-driven DOM sync.
    if (this.landing && !this.width.animating && !this.height.animating) {
      this.landing = false;
      this.miniGrid.style.transition = "";
      this.counters.style.right = "";
    }
    // While landing, the grid and the counters sit where they will be once the island has narrowed,
    // measured from its centre; otherwise they follow its right edge and middle as before.
    const end = this.landing ? this.targetSize() : { w, h: hh };
    const shift = (w - end.w) / 2;
    this.miniGrid.style.left = `${shift + end.w - 40 - 14.5}px`;
    this.miniGrid.style.top = `${end.h / 2 - 14.5}px`;
    if (this.landing) this.counters.style.right = `${16 + shift}px`;
    this.greetingCanvas.style.left = `${(w - EXPANDED_W) / 2}px`;
    this.uploadCanvas.el.style.left = `${(w - EXPANDED_W) / 2}px`;

    const rect = { x: (PANEL_W - w) / 2, y: 0, w, h: hh };
    const p = this.pushedRect;
    if (Math.abs(p.x - rect.x) > 0.5 || Math.abs(p.w - rect.w) > 0.5 || Math.abs(p.h - rect.h) > 0.5) {
      this.pushedRect = rect;
      void Bridge.setIslandRect(rect.x, rect.y, rect.w, rect.h);
    }
  }

  /** Island rect in window coordinates (origin top-left of the 720×560 window). */
  private islandRect(): { x: number; y: number; w: number; h: number } {
    const w = this.width.value;
    const hh = Math.min(PANEL_H, this.height.value);
    return { x: (PANEL_W - w) / 2, y: 0, w, h: hh };
  }

  // ── Window collapse (hidden → tiny wake strip, zero polling) ────────────────

  private updateWindowCollapsed() {
    if (this.collapseTimer != null) {
      window.clearTimeout(this.collapseTimer);
      this.collapseTimer = null;
    }
    if (State.mode === "hidden") {
      // Let the island finish retracting, then drop the window to the strip zone: from there
      // the OS delivers no cursor events, so nothing polls at all.
      this.collapseTimer = window.setTimeout(() => {
        this.collapseTimer = null;
        if (State.mode !== "hidden") return;
        this.collapsed = true;
        this.syncStrip();
        void Bridge.setCollapsed(true);
      }, 420);
    } else if (this.collapsed) {
      // Grow the window back before the island animates open.
      this.collapsed = false;
      this.syncStrip();
      void Bridge.setCollapsed(false);
    }
  }

  // ── Input ───────────────────────────────────────────────────────────────────

  private wireInput() {
    // Linux: the 240×6 wake strip is the only thing the OS can hit while the island is hidden.
    // Windows: the hidden window passes every click; strip.rs's catcher and mouse hook wake the
    // island instead (strip-touch → onStripTouch).
    this.wakeStrip.addEventListener("mouseenter", () => {
      Sound.resume();
      if (State.mode === "hidden") this.fsm.mouseEntered();
    });

    this.islandEl.addEventListener("mousedown", (e) => {
      Sound.resume();
      State.lastActivity = performance.now();
      if (State.mode !== "expanded") {
        this.fsm.click();
        return;
      }
      if (this.isBotHit(e.clientX, e.clientY)) {
        this.cancelBotHover();
        this.engine.slap();
      }
    });

    window.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && State.mode === "expanded" && !State.isPinned) this.collapse();
      State.lastActivity = performance.now();
    });

    // usage.ts sets State.media from the media event and from the first media_state answer; the
    // refresh waits a tick so it reads what usage.ts stored.
    const refreshSoon = () => void window.setTimeout(() => this.refreshActivity(), 0);
    void onMedia(refreshSoon);
    void Bridge.mediaState().then(refreshSoon);

    if (CHAT_ENABLED) void onDragDrop((e) => this.onDragDrop(e));
    else refuseDrops();

    // Outside Tauri (plain browser) drive the cursor from DOM events so the
    // island can be inspected with `npm run dev`.
    if (!IS_TAURI) this.followPageCursor();
  }

  /**
   * Takes the cursor from the page's own mouse events instead of Rust's poll.
   * Used where the OS has no global cursor position (Wayland): the events only
   * fire while the pointer is over the island, so leaving the window is
   * reported as a cursor far away, which is what the poll would have said.
   */
  followPageCursor() {
    window.addEventListener("mousemove", (e) => this.onCursor(e.clientX, e.clientY));
    window.addEventListener("mouseout", (e) => {
      if (e.relatedTarget == null) this.onCursor(-10_000, -10_000);
    });
  }

  /**
   * strip.rs (Windows): the cursor touched the line, or its catcher window. Same as the old
   * wake strip's mouseenter: hidden → petit.
   */
  onStripTouch() {
    Sound.resume();
    if (State.mode === "hidden") this.fsm.mouseEntered();
  }

  /**
   * The global shortcut (src-tauri/src/hotkey.rs, spec §4). An open island folds into the strip,
   * pinned or not, and like the collapse button `holdVisible` does not hold it; hidden, compact
   * or greeting opens on the page chosen in Settings (the session list by default) with the usual sound. Quiet mode neither stops it
   * nor is lifted: it is an explicit action.
   */
  onHotkey() {
    Sound.resume();
    if (this.fsm.state === "home") {
      State.isPinned = false;
      this.fsm.pinned = false;
      this.fsm.collapsedByButton = true;
      this.fsm.forceHidden();
    } else {
      this.alert(State.openView(true));
    }
  }

  /** boot.nativeStrip: the hidden island shows the line; the old wake strip can no longer be hit. */
  enableNativeStrip() {
    this.nativeStrip = true;
    this.wakeStrip.style.display = "none";
    this.syncStrip();
  }

  /**
   * strip-cursor: the line follows a cursor near it; null puts it back with one 150 ms
   * transition. Only cursor events move it: no frame loop runs for the strip.
   */
  onStripCursor(p: { x: number; y: number } | null) {
    // A late point, after the strip ended (the hook still sends moves until strip::leave runs), would
    // leave the line stretched for the next hide; only null resets it outside `hidden`.
    if (!this.nativeStrip || (p != null && State.mode !== "hidden")) return;
    const box = lineAt(p);
    this.stripLine.classList.toggle("rest", p == null);
    this.stripLine.style.width = `${box.w}px`;
    this.stripLine.style.height = `${box.h}px`;
    this.stripLine.style.left = `calc(50% + ${box.x + box.w / 2 - STRIP_ZONE_W / 2}px)`;
  }

  /** strip-fullscreen: the line hides while an app covers the island's monitor. */
  onStripFullscreen(on: boolean) {
    this.stripFullscreen = on;
    this.syncStrip();
  }

  /**
   * Shown only once the window is the strip zone; dashed and paler while paused (quiet), a static
   * violet during pomodoro work and green during its breaks. Pause wins while both are on.
   */
  private syncStrip() {
    const on = this.nativeStrip && State.mode === "hidden" && this.collapsed && !this.stripFullscreen;
    this.stripLine.classList.toggle("on", on);
    this.stripLine.classList.toggle("quiet", State.paused);
    const run = State.paused ? null : State.pomodoro;
    this.stripLine.classList.toggle("focus", run?.phase === "work");
    this.stripLine.classList.toggle("focus-break", run != null && run.phase !== "work");
  }

  /** Cursor in window-logical coordinates. */
  onCursor(x: number, y: number) {
    State.mouse = { x, y };
    const rect = this.islandRect();
    State.mouseInIsland = { x: x - rect.x, y: y - rect.y };

    // Windows sends no cursor position with an OLE drag, so the drop sequence is
    // fed from the Win32 cursor poll instead — it runs throughout the drag.
    const dragging = UploadSeq.isActive && !UploadSeq.dropped;
    if (dragging) {
      UploadSeq.updateCursor(State.mouseInIsland.x, State.mouseInIsland.y);
    }

    const inIsland =
      x >= rect.x - HIT_MARGIN && x <= rect.x + rect.w + HIT_MARGIN &&
      y >= rect.y - HIT_MARGIN && y <= rect.y + rect.h + HIT_MARGIN;
    const crossed = inIsland !== this.wasInIsland;

    if (inIsland && !this.wasInIsland) {
      if (this.fsm.state === "coucou") this.greeting.hover();
      this.fsm.mouseEntered();
      this.homeCollapseAt = null;
      this.updateCountdown(performance.now());
    }
    if (!inIsland && this.wasInIsland) {
      this.fsm.mouseLeft();
      if (this.fsm.state === "home" && !State.isPinned && !this.fsm.heldOpen) {
        this.homeCollapseAt = performance.now() + State.settings.autoCloseInterval * 1000;
        this.updateCountdown(performance.now());
      }
    }
    this.wasInIsland = inIsland;
    this.petitCollapse.classList.toggle("shown", inIsland);

    // Bot hover → love
    const overBot = State.mode === "expanded" && State.stateOverride == null && this.isBotHit(x, y);
    const hover = overBot !== this.botHovering;
    if (overBot && !this.botHovering) this.botHoverIn(x, y);
    if (!overBot && this.botHovering) this.cancelBotHover();
    this.botHovering = overBot;
    if (this.botHovering) {
      const d = Math.hypot(x - this.botHoverStart.x, y - this.botHoverStart.y);
      if (d > 40) {
        this.botHoverStart = { x, y };
        this.scheduleLove();
      }
    }

    // Only what answers the cursor now draws at once; a fire-only loop otherwise keeps its pace and
    // its next frame reads the cursor (pace.ts cursorNeedsFrame).
    const bot = botPosition(State.mode, State.view, this.height.value, State.uploadProgress);
    const botShown = State.mode !== "hidden" && bot.opacity > 0 && bot.diameter > 0;
    const eyeLag = botShown ? this.engine.cursorLookLag(this.lookX(), this.lookY()) : 0;
    if (cursorNeedsFrame({ crossed, hover, dragging, eyeLag })) this.ensureRunning();
  }

  private isBotHit(x: number, y: number): boolean {
    const rect = this.islandRect();
    const cx = rect.x + this.botCx.value;
    const cy = rect.y + this.botCy.value;
    const radius = this.botSize.value / 2;
    return (x - cx) ** 2 + (y - cy) ** 2 <= radius * radius;
  }

  private botHoverIn(x: number, y: number) {
    if (performance.now() / 1000 - this.lastLoveTime < 6) return;
    this.botHoverStart = { x, y };
    this.engine.blink();
    this.engine.tgEs = 1.08;
    Sound.play("hover");
    this.scheduleLove();
  }

  private scheduleLove() {
    if (this.botHoverTimer != null) window.clearTimeout(this.botHoverTimer);
    this.botHoverTimer = window.setTimeout(() => {
      this.botHoverTimer = null;
      if (!this.botHovering || State.stateOverride != null) return;
      if (performance.now() / 1000 - this.lastLoveTime < 6) return;
      this.lastLoveTime = performance.now() / 1000;
      this.engine.triggerEmote("love");
      Sound.play("love");
    }, 1900);
  }

  private cancelBotHover() {
    if (this.botHoverTimer != null) window.clearTimeout(this.botHoverTimer);
    this.botHoverTimer = null;
    this.engine.tgEs = 1;
  }

  /** Three slaps → dizzy + confused view for 3.3 s, then back. */
  private handleDizzy() {
    this.prevViewBeforeConfused = State.view;
    State.stateOverride = "dizzy";
    this.engine.setState("dizzy");
    Sound.play("dizzy");
    this.alert("confused");
    if (this.confusedRecovery != null) window.clearTimeout(this.confusedRecovery);
    this.confusedRecovery = window.setTimeout(() => {
      this.confusedRecovery = null;
      State.stateOverride = null;
      this.engine.setState(State.effectiveState);
      if (State.view === "confused") {
        const fallback = State.defaultView();
        const prev = this.prevViewBeforeConfused;
        // Leaving a permission or question card for the confused view released it: going back
        // to that view would show a card nobody can answer any more.
        const deadCard = (prev === "approval" || prev === "question") && State.pendingApproval == null;
        this.setView(prev === "confused" || deadCard ? fallback : prev);
      }
      this.engine.triggerEmote("happy");
    }, 3300);
  }

  // ── Frame loop ──────────────────────────────────────────────────────────────

  ensureRunning() {
    if (this.running) {
      // A fire-only loop waiting out its 30 fps slot: run the next frame now instead (still one loop).
      if (this.fireTimer != null) {
        window.clearTimeout(this.fireTimer);
        this.fireTimer = null;
        requestAnimationFrame(this.frame);
      }
      return;
    }
    this.running = true;
    this.lastFrame = performance.now();
    requestAnimationFrame(this.frame);
  }

  private frame = (nowMs: number) => {
    // A fast panel's vsync between two character-only frames: wait for the next one (still one loop).
    // A state change (dirty) is drawn at once.
    if (this.characterOnly && !this.dirty && tooSoon(nowMs - this.lastFrame, CHARACTER_FPS)) {
      requestAnimationFrame(this.frame);
      return;
    }
    const dt = Math.min(0.05, (nowMs - this.lastFrame) / 1000);
    this.lastFrame = nowMs;

    this.width.step(dt, nowMs);
    this.height.step(dt, nowMs);
    this.radius.step(dt, nowMs);
    this.applyGeometry();

    if (this.dirty) {
      this.dirty = false;
      this.syncDom();
    }

    this.updateBotTargets();
    this.botCx.step(dt);
    this.botCy.step(dt);
    this.botSize.step(dt);

    const greetingActive = State.mode === "expanded" && State.view === "greeting";
    if (greetingActive) {
      const gctx = this.greetingCanvas.getContext("2d");
      if (gctx) {
        const dpr = Math.min(2, window.devicePixelRatio || 1);
        gctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        this.greeting.compact = this.compactLook();
        this.greeting.draw(gctx);
      }
    } else {
      // Kept running even while the drop canvas is up, so the island's own Luch
      // is already in the right place the moment the canvas fades out.
      this.drawBot(dt);
    }

    const uploadActive = this.uploadActive;
    if (uploadActive) this.uploadCanvas.draw(UploadSeq.frame(), nowMs / 1000);
    this.uploadCanvas.el.classList.toggle("on", uploadActive);
    this.viewsEl.classList.toggle("hidden-by-upload", uploadActive);

    const minisShown = tickMiniBots(dt, this.miniShown);
    this.views.get(State.view)?.tick?.(nowMs);
    if (UploadSeq.isActive) this.stepSequence();
    this.updateCountdown(nowMs);

    // Nothing is drawn while the island is hidden, so nothing may keep the loop
    // alive either. This used to read `... || this.engine.busy || State.mode !==
    // "hidden"`, and engine.busy is permanently true for any state with a
    // looping animation — breathing, ratelimit sweat, sleeping z's, the search
    // sweep — so a hidden island went on burning frames in exactly the states it
    // spends most of its life in. Geometry still has to finish retracting.
    // The same goes for a view where Луч is not drawn (diameter 0: sessions, stats, focus,
    // apps): its notes or sleep would otherwise keep the loop at full rate for nothing.
    const settling = this.width.animating || this.height.animating || this.radius.animating;
    const bot = botPosition(State.mode, State.view, this.height.value, State.uploadProgress);
    const botShown = bot.opacity > 0 && bot.diameter > 0;
    // The island itself (geometry, Луч's place, greeting, drop) runs at the panel's full rate; the
    // character alone (blinks, eyes, emotes) at about CHARACTER_FPS (pace.ts).
    const shellMoving = State.mode === "hidden"
      ? settling
      : settling || !this.botCx.settled || !this.botCy.settled || !this.botSize.settled ||
        greetingActive || UploadSeq.isActive;
    const moving = shellMoving || (State.mode !== "hidden" && botShown && this.engine.busy);
    this.characterOnly = moving && !shellMoving;
    // Луч's fire never stops; nothing else moving → at most 30 fps (spec §2, plan Global Constraints):
    // fireFps in pace.ts.
    const fireOnly = State.mode !== "hidden" && !moving && this.engine.fireAlive && (botShown || minisShown > 0);

    if (moving) {
      requestAnimationFrame(this.frame);
    } else if (fireOnly) {
      this.fireTimer = window.setTimeout(() => {
        this.fireTimer = null;
        requestAnimationFrame(this.frame);
      }, fireDelay(fireFps(State.mode === "compact"), performance.now() - nowMs));
      // Nothing plays while only the fire moves: let the audio context suspend (idempotent).
      Sound.idle();
    } else {
      this.running = false;
      Sound.idle();
    }
  };

  /** A mini is on screen in the active view of an expanded island, or in the compact grid while it shows. */
  private miniShown = (canvas: HTMLCanvasElement): boolean => {
    if (this.miniGrid.contains(canvas)) return this.gridShown;
    if (State.mode !== "expanded" || State.view === "greeting" || this.uploadActive) return false;
    return this.views.get(State.view)?.el.contains(canvas) === true;
  };

  private updateBotTargets() {
    const p = botPosition(State.mode, State.view, this.height.value, State.uploadProgress);
    if (this.landing) {
      // Held at its compact screen place (where the greeting landed it) while the island narrows.
      this.botCx.set(this.width.value / 2 + p.cx - this.targetSize().w / 2);
      this.botCy.set(p.cy);
      this.botSize.set(p.diameter / 0.6);
    }
    this.botCx.target = p.cx;
    this.botCy.target = p.cy;
    this.botSize.target = p.diameter / 0.6;

    const greetingActive = State.mode === "expanded" && State.view === "greeting";
    // The drop canvas draws its own Mochi; two of them would overlap.
    const visible = p.opacity > 0 && !greetingActive && !this.uploadActive;
    this.botCanvas.style.opacity = visible ? "1" : "0";

    if (State.mode === "expanded" && State.view !== "uploading" && !greetingActive && !this.uploadActive) {
      const d = p.diameter;
      const color = botGlowColor(State.effectiveState);
      this.botGlow.style.display = "block";
      this.botGlow.style.width = `${d * 2.2}px`;
      this.botGlow.style.height = `${d * 2.2}px`;
      this.botGlow.style.left = `${this.botCx.value - d * 1.1}px`;
      this.botGlow.style.top = `${this.botCy.value - d * 1.1}px`;
      this.botGlow.style.background = `radial-gradient(circle, ${color} 0%, transparent 62%)`;
      this.botGlow.style.opacity = String(botGlowOpacity(State.effectiveState));
    } else {
      this.botGlow.style.display = "none";
    }
  }

  private drawBot(dt: number) {
    const size = this.botSize.value;
    const w = Math.max(1, Math.round(size));
    const hCss = w + BOT_OVERHANG;
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    if (this.canvasPx !== w) {
      this.canvasPx = w;
      this.botCanvas.width = Math.round(w * dpr);
      this.botCanvas.height = Math.round(hCss * dpr);
      this.botCanvas.style.width = `${w}px`;
      this.botCanvas.style.height = `${hCss}px`;
    }
    this.botCanvas.style.left = `${this.botCx.value - w / 2}px`;
    this.botCanvas.style.top = `${this.botCy.value - BOT_OVERHANG / 2 - hCss / 2}px`;

    const ctx = this.botCanvas.getContext("2d");
    if (!ctx) return;

    this.syncBodyColor();
    this.engine.particleOverhang = BOT_OVERHANG;
    this.engine.lookX = this.lookX();
    this.engine.lookY = this.lookY();
    if (this.engine.morph > 0.3) {
      this.engine.slotHTarget = State.fileDragOver ? 0.2 : 0;
    } else {
      this.engine.slotHTarget = 0;
      if (this.engine.morph < 0.05) {
        this.engine.slotH = 0;
        this.engine.slotHVel = 0;
      }
    }
    this.engine.update(dt);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, hCss);
    // A view without Luch (diameter 0) still keeps the canvas at the 1 px floor of `w`: drawn
    // into that pixel, Luch showed as a stray dot. Once the shrink is below a pixel, it stays empty.
    if (this.botSize.target <= 0 && size < 1) return;
    this.engine.draw(ctx, w, hCss);
  }

  /**
   * With agent sessions Луч shows their aggregate, so he keeps his own colour rather than taking
   * the colour of whichever integration pill has the focus.
   */
  private syncBodyColor() {
    const focus = State.focusTask;
    this.engine.bodyColor =
      !Sessions.hasSessions && focus?.isIntegration ? hexToRGB(focus.color) : null;
  }

  /** What the compact island will show (syncDom's grid rule), for the greeting to land in. */
  private compactLook(): CompactLook {
    this.syncBodyColor();
    return {
      fire: this.engine.settledFire(),
      minis: Sessions.hasSessions ? [] : State.otherTasks.slice(0, 4).map(miniFire),
    };
  }

  /** BotCanvasView.lookX / lookY — tanh of the distance to the bot. */
  private lookX(): number {
    const rect = this.islandRect();
    const botScreenX = rect.x + this.botCx.value;
    return Math.tanh((State.mouse.x - botScreenX) / 260);
  }

  private lookY(): number {
    return -Math.tanh((State.mouse.y - this.botCy.value) / 200);
  }

  /**
   * The collapse countdown is one CSS animation, so it runs with no frames and no mouse: started
   * when a home collapse is scheduled, removed when it is cancelled. Called every frame and right
   * after the schedule changes; it touches the DOM only when the wanted countdown changes.
   */
  private updateCountdown(nowMs: number) {
    const at = State.mode !== "expanded" || State.isPinned || this.fsm.heldOpen ? null : this.homeCollapseAt;
    if (at === this.countdownFor) return;
    this.countdownFor = at;
    const el = this.countdown;
    el.style.animation = "none";
    if (at == null) return;
    const windowS = Math.min(10, State.settings.autoCloseInterval * 0.6);
    const startIn = (at - nowMs) / 1000 - windowS;
    void el.offsetWidth; // restart the animation from its first keyframe
    el.style.animation = `countdown-shrink ${windowS}s linear ${startIn}s forwards`;
  }

  // ── DOM sync ────────────────────────────────────────────────────────────────

  private syncDom() {
    const expanded = State.mode === "expanded";
    const greetingActive = expanded && State.view === "greeting";

    this.contentEl.style.opacity = expanded && !greetingActive ? "1" : "0";
    this.contentEl.style.pointerEvents = expanded && !greetingActive ? "auto" : "none";
    this.greetingCanvas.style.display = greetingActive ? "block" : "none";

    this.header.sync();
    this.syncStrip();
    for (const [name, view] of this.views) {
      const on = name === State.view;
      view.el.classList.toggle("on", on);
      if (on) view.sync();
      // Views with a 1 s clock run it only while expanded and on screen (D57).
      const active = expanded && on;
      if (view.setActive && (this.activeViews.get(name) ?? false) !== active) {
        this.activeViews.set(name, active);
        view.setActive(active);
      }
    }

    // The chat takes keyboard focus for as long as it is open. The question card's "Other" field
    // takes it only while it has the focus, and gives it back itself (views.ts buildQuestion).
    if (this.lastSyncedView !== State.view) {
      const wasChat = this.lastSyncedView === "prompt";
      this.lastSyncedView = State.view;
      if (State.view === "prompt") {
        void Bridge.focusWindow(true);
        window.setTimeout(() => this.views.get("prompt")?.focus?.(), 120);
      } else if (wasChat) {
        void Bridge.focusWindow(false);
      }
    }

    // Compact: session counters when agents are tracked, else the mini grid.
    const compact = State.mode === "compact";
    this.petitCollapse.style.display = compact ? "grid" : "none";
    const showCounters = compact && Sessions.hasSessions;
    const showGrid = compact && !showCounters;
    this.gridShown = showGrid;
    this.miniGrid.style.opacity = showGrid ? "1" : "0";
    this.counters.style.opacity = showCounters ? "1" : "0";
    if (showCounters) this.syncCounters();
    if (showGrid) {
      const others = State.otherTasks.slice(0, 4);
      const key = others.map((t) => t.id).join("|");
      if (this.miniGrid.dataset.key !== key) {
        this.miniGrid.dataset.key = key;
        this.miniGrid.replaceChildren();
        for (const t of others) {
          this.miniGrid.append(createMiniBot(t, 13));
        }
        pruneMiniBots();
      }
    }

    syncMiniBotStates(State.tasks);
    this.engine.setState(State.effectiveState);
  }

  /** `!n` `✕n` `✓n` `•n`, non-zero values only (D34). */
  private syncCounters() {
    const c = Sessions.snapshot?.counts;
    if (!c) return;
    const key = COUNTERS.map(([k]) => c[k]).join("|");
    if (this.counters.dataset.key === key) return;
    this.counters.dataset.key = key;
    this.counters.replaceChildren(
      ...COUNTERS.filter(([k]) => c[k] > 0).map(([k, glyph, tone]) =>
        h("span", { class: `cnt ${tone}`, text: `${glyph}${c[k]}` }),
      ),
    );
  }

  /** Applies settings coming from Rust at boot. */
  applySettings() {
    Sound.setEnabled(State.settings.soundEnabled);
    Sound.setVolume(State.settings.soundVolume);
    this.setAutoCloseDelay(State.settings.autoCloseInterval);
    State.notify();
  }

  /** A new auto-close delay: a collapse countdown already running starts over with it. */
  private setAutoCloseDelay(seconds: number) {
    // Every settings save comes back here: only a new delay restarts the countdown.
    if (seconds === this.fsm.homeToPetitDelay) return;
    this.fsm.setHomeToPetitDelay(seconds);
    if (this.homeCollapseAt == null) return;
    const now = performance.now();
    this.homeCollapseAt = now + seconds * 1000;
    this.updateCountdown(now);
  }

  get panelSize() {
    return { w: PANEL_W, h: PANEL_H };
  }

  get chatHeight() {
    return chatPromptHeight(State.promptCount);
  }
}
