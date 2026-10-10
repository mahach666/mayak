// Island open/close FSM — port of IslandStateMachine.swift.
// No DOM, no Tauri: it only reports transitions.

export type FsmState = "hidden" | "petit" | "home" | "coucou";

export class IslandStateMachine {
  state: FsmState = "hidden";

  onTransition: ((from: FsmState, to: FsmState) => void) | null = null;

  /** home → petit delay, seconds. */
  homeToPetitDelay = 15;
  /** petit → hidden delay, seconds. */
  petitToHiddenDelay = 60;
  /**
   * coucou → petit once the greeting animation ends (no hover). The greeting itself ends in the
   * compact pose and the island takes over from that frame, so there is nothing left to wait for.
   */
  greetAutoCollapseDelay = 0;
  /** coucou → petit while the mouse hovers the greeting. */
  greetHoverCollapseDelay = 10;
  /** An alert waiting for an answer stays open, even when the mouse leaves. */
  pinned = false;
  /**
   * The open island does not fold on its own while this is set: an answer is being typed on the
   * question card (spec §6). Unlike `pinned` it changes nothing else (Escape still folds).
   */
  heldOpen = false;
  /** An unacked agent wait keeps the compact island from hiding (snapshot.hold). */
  holdVisible = false;
  /**
   * The collapse button hid a compact island (spec §5): holdVisible does not hold it until the
   * next event (alert, reveal) or until the user opens the island.
   */
  collapsedByButton = false;
  /**
   * Hovering opens the island all the way instead of peeking (Settings → General → Open on hover,
   * off by default; ideas §34, from Coucou's IslandStateMachine.openOnHover).
   */
  openOnHover = false;
  /**
   * How long the pointer must rest on the hidden or compact island before a hover opens it,
   * seconds: a pass along the top of the screen, over the strip or the compact island, only wakes it.
   */
  hoverOpenDwell = 0.15;
  /** Grace period after the pointer leaves a hover-opened island, seconds. */
  hoverCloseDelay = 0.6;
  /** Open because of a hover, until the user clicks inside it. */
  get openedByHover(): boolean {
    return this.byHover;
  }
  /** When the scheduled home → petit fold is due (`now()` time, ms); null when none is scheduled. */
  get homeCollapseDueAt(): number | null {
    return this.homeCollapseDue;
  }
  /** The delay, seconds, a home → petit fold scheduled now would use. */
  get homeCollapseDelay(): number {
    return this.byHover ? this.hoverCloseDelay : this.homeToPetitDelay;
  }
  /** Clock for homeCollapseDueAt, ms (the island's performance.now). */
  now: () => number = () => performance.now();

  private byHover = false;
  private homeCollapseDue: number | null = null;
  private petitHide: number | null = null;
  private homeCollapse: number | null = null;
  private greetCollapse: number | null = null;
  private hoverOpen: number | null = null;

  // ── Inputs ──────────────────────────────────────────────────────────────────

  /**
   * The app starts on the greeting. `greet: false` (a language reload, batch spec §F) skips it and
   * goes straight to compact, where the greeting would have left the island.
   */
  launch({ greet = true }: { greet?: boolean } = {}) {
    this.collapsedByButton = false;
    this.cancelTimers();
    this.transition(greet ? "coucou" : "petit");
  }

  mouseEntered() {
    switch (this.state) {
      case "hidden":
        this.collapsedByButton = false;
        this.cancelTimers();
        // Armed first: the island reports a pointer already outside (mouseLeft) on the transition.
        this.scheduleHoverOpen();
        this.transition("petit");
        break;
      case "petit":
        this.clear("petitHide");
        this.scheduleHoverOpen();
        break;
      case "home":
        this.clear("homeCollapse");
        break;
      case "coucou":
        this.scheduleGreetCollapse(this.greetHoverCollapseDelay);
        break;
    }
  }

  mouseLeft() {
    switch (this.state) {
      case "hidden":
        break;
      case "petit":
        this.clear("hoverOpen");
        this.schedulePetitHide();
        break;
      case "home":
        this.scheduleHomeCollapse();
        break;
      case "coucou":
        this.clear("greetCollapse");
        this.transition("petit");
        break;
    }
  }

  click() {
    if (this.state !== "petit") return;
    this.collapsedByButton = false;
    this.cancelTimers();
    this.transition("home");
  }

  /** Greeting animation finished (T.end). Doesn't override a running hover timer. */
  greetComplete() {
    if (this.state !== "coucou") return;
    if (this.greetCollapse == null) this.scheduleGreetCollapse(this.greetAutoCollapseDelay);
  }

  /** Non-alert work event: show compact from hidden. */
  reveal() {
    if (this.state !== "hidden") return;
    this.collapsedByButton = false;
    this.cancelTimers();
    this.transition("petit");
    this.schedulePetitHide();
  }

  /**
   * The user clicked inside the island: a hover-opened island now stays like any open island
   * (normal auto-close) instead of folding once the pointer leaves.
   */
  userInteracted() {
    if (!this.byHover) return;
    this.byHover = false;
    // The short countdown already running becomes the normal one.
    if (this.state === "home" && this.homeCollapse != null) this.scheduleHomeCollapse();
  }

  /** Alert or explicit request: open straight to expanded. */
  forceHome() {
    this.byHover = false;
    this.collapsedByButton = false;
    this.cancelTimers();
    this.transition("home");
  }

  /// Explicit close (OK button, Escape, an alert being answered).
  forcePetit() {
    this.byHover = false;
    this.collapsedByButton = false;
    this.cancelTimers();
    this.transition("petit");
  }

  forceHidden() {
    this.byHover = false;
    this.cancelTimers();
    this.transition("hidden");
  }

  /** Collapse button (spec §5): home → petit, petit → hidden; elsewhere nothing. */
  forceCollapse() {
    this.byHover = false;
    if (this.state === "home") {
      this.cancelTimers();
      this.transition("petit");
    } else if (this.state === "petit") {
      this.cancelTimers();
      this.collapsedByButton = true;
      this.transition("hidden");
    }
  }

  /**
   * Called with every sessions snapshot and on every pause change (Island.syncHold):
   * always off while paused, so pause (forceHidden) still wins.
   */
  setHoldVisible(on: boolean) {
    if (this.holdVisible === on) return;
    this.holdVisible = on;
    // Released while compact: the island may hide again (0 % CPU when hidden).
    if (!on && this.state === "petit") this.schedulePetitHide();
  }

  /** Holds the open island (`heldOpen`); a fold already scheduled is cancelled. */
  setHeldOpen(on: boolean) {
    this.heldOpen = on;
    if (on) this.clear("homeCollapse");
  }

  // ── Timers ──────────────────────────────────────────────────────────────────

  private schedulePetitHide() {
    this.clear("petitHide");
    this.petitHide = window.setTimeout(() => {
      this.petitHide = null;
      if (this.holdVisible && !this.collapsedByButton) return;
      if (this.state === "petit") this.transition("hidden");
    }, this.petitToHiddenDelay * 1000);
  }

  /** A new home → petit delay, in seconds: a collapse already scheduled starts over with it. */
  setHomeToPetitDelay(seconds: number) {
    if (seconds === this.homeToPetitDelay) return;
    this.homeToPetitDelay = seconds;
    if (this.homeCollapse != null) this.scheduleHomeCollapse();
  }

  private scheduleHomeCollapse() {
    this.clear("homeCollapse");
    if (this.pinned || this.heldOpen) return;
    const ms = this.homeCollapseDelay * 1000;
    this.homeCollapseDue = this.now() + ms;
    this.homeCollapse = window.setTimeout(() => {
      this.homeCollapse = null;
      this.homeCollapseDue = null;
      if (this.state === "home") {
        this.byHover = false;
        this.transition("petit");
      }
    }, ms);
  }

  /** Open on hover: the pointer resting on the hidden or compact island opens it after hoverOpenDwell. */
  private scheduleHoverOpen() {
    this.clear("hoverOpen");
    if (!this.openOnHover || this.pinned) return;
    this.hoverOpen = window.setTimeout(() => {
      this.hoverOpen = null;
      if (this.state !== "petit" || this.pinned || !this.openOnHover) return;
      this.cancelTimers();
      this.collapsedByButton = false;
      this.byHover = true;
      this.transition("home");
    }, this.hoverOpenDwell * 1000);
  }

  private scheduleGreetCollapse(delay: number) {
    this.clear("greetCollapse");
    this.greetCollapse = window.setTimeout(() => {
      this.greetCollapse = null;
      if (this.state === "coucou") this.transition("petit");
    }, delay * 1000);
  }

  private clear(which: "petitHide" | "homeCollapse" | "greetCollapse" | "hoverOpen") {
    const id = this[which];
    if (id != null) window.clearTimeout(id);
    this[which] = null;
    if (which === "homeCollapse") this.homeCollapseDue = null;
  }

  cancelTimers() {
    this.clear("petitHide");
    this.clear("homeCollapse");
    this.clear("greetCollapse");
    this.clear("hoverOpen");
  }

  private transition(next: FsmState) {
    if (next === this.state) return;
    const from = this.state;
    this.state = next;
    this.onTransition?.(from, next);
  }
}
