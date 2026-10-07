// Entry point: boot the bridge, wire the island, start the greeting.

import "./style.css";
import { Bridge, IS_TAURI, onEvent } from "./core/bridge";
import { Sound } from "./core/sound";
import { State, type Settings } from "./core/state";
import { setLanguage, t } from "./core/i18n";
import { Island } from "./island/island";
import { registerHookHandlers } from "./island/hooks";
import { registerIntegrationHandlers, refreshConfigured } from "./island/integrations";
import { registerObserver } from "./island/observer";
import { SAVED_RUN_KEY, decodeSavedRun, encodeSavedRun, type PomodoroRun } from "./island/pomodoro";

/**
 * The pomodoro kept for a language reload (spec §C), taken once: the key is removed as it is
 * read. Where sessionStorage cannot be used, the run is simply not kept.
 */
function takeSavedRun(): PomodoroRun | null {
  try {
    const raw = sessionStorage.getItem(SAVED_RUN_KEY);
    sessionStorage.removeItem(SAVED_RUN_KEY);
    return decodeSavedRun(raw);
  } catch (err) {
    console.error("[mayak] could not read the kept pomodoro", err);
    return null;
  }
}

function keepRun(run: PomodoroRun) {
  try {
    sessionStorage.setItem(SAVED_RUN_KEY, encodeSavedRun(run));
  } catch (err) {
    console.error("[mayak] could not keep the pomodoro", err);
  }
}

async function main() {
  const root = document.getElementById("root");
  if (!root) return;

  void Sound.preload();

  const boot = await Bridge.boot();
  const savedRun = takeSavedRun();
  if (boot) {
    State.settings = { ...State.settings, ...boot.settings };
    // A reloaded page has no pomodoro unless one was kept for a language reload (below), so the
    // tray check must not stay on.
    if (!savedRun) void Bridge.setFocusCheck(false);
  }
  // Before the island is built: its views take their texts when they are made.
  setLanguage(State.settings.language);
  // Rust keeps no dictionary: the tray takes its texts from here, at every load (spec §C).
  void Bridge.trayTexts({
    open: t("tray.open"),
    settings: t("tray.settings"),
    focus: t("tray.focus"),
    pause: t("tray.pause"),
    quit: t("tray.quit"),
  });

  const island = new Island(root);
  island.applySettings();
  State.loadIntegrationTasks();
  if (boot && !boot.cursorPoll) island.followPageCursor();
  if (boot?.nativeStrip) island.enableNativeStrip();

  await onEvent<{ x: number; y: number }>("cursor", ({ x, y }) => island.onCursor(x, y));
  await onEvent<null>("pointer-released", () => island.onPointerReleased());
  await onEvent<null>("strip-touch", () => island.onStripTouch());
  await onEvent<{ x: number; y: number } | null>("strip-cursor", (p) => island.onStripCursor(p));
  await onEvent<{ on: boolean }>("strip-fullscreen", ({ on }) => island.onStripFullscreen(on));
  await onEvent<null>("hotkey", () => island.onHotkey());

  /**
   * Pause has to reach Rust too, or the pollers keep calling out. Pause beats
   * hold (spec §10): pausing drops it, and lifting the pause restores it from
   * the last snapshot before anything reveals the island.
   */
  const setPaused = (on: boolean) => {
    if (State.paused === on) return;
    State.paused = on;
    void Bridge.setPaused(on);
    island.syncHold();
    State.notify();
  };
  // A language reload keeps quiet mode: Rust still holds it, and the tray still shows it.
  if (boot?.paused) setPaused(true);

  // The island's quiet button is the tray's Pause, except that it leaves the island open (spec §5).
  island.quietHandler = () => {
    setPaused(!State.paused);
    Sound.play("blip");
  };

  await onEvent<string>("tray", (what) => {
    switch (what) {
      case "settings":
        setPaused(false);
        island.alert("settings");
        break;
      case "open":
        setPaused(false);
        island.alert(State.defaultView());
        break;
      case "pause":
        setPaused(!State.paused);
        if (State.paused) island.fsm.forceHidden();
        else island.reveal();
        break;
      // Same as the Focus tab's Start/Stop: starts a pomodoro, or stops it (pomodoro spec §4).
      case "focus":
        island.togglePomodoro();
        break;
    }
  });

  await onEvent<null>("screen-changed", () => void Bridge.reposition());

  // The settings window writes preferences; apply them here without a restart.
  await onEvent<Settings>("settings-changed", async (s) => {
    // The views took their texts when they were built: a new language reloads the page (spec §C).
    if (s.language !== undefined && s.language !== State.settings.language) {
      // A legacy Allow/Deny card would not survive the reload: hand it back to the terminal
      // first, so Claude Code is not held up until the relay gives up.
      const approval = State.pendingApproval;
      if (approval) await Bridge.approvalDecline(approval.requestId);
      // The running pomodoro goes on after the reload (main() takes it back).
      if (State.pomodoro) keepRun(State.pomodoro);
      location.reload();
      return;
    }
    State.settings = { ...State.settings, ...s };
    island.applySettings();
    State.loadIntegrationTasks();
    void refreshConfigured();
  });

  registerHookHandlers(island);
  registerIntegrationHandlers(island);
  // Before launch, so the greeting already collapses onto the sessions view.
  await registerObserver(island);

  // Just before launch: a kept phase that ended meanwhile moves on once the island is launched.
  if (savedRun) island.resumePomodoro(savedRun);
  island.launch();

  // In a plain browser there is no wake strip behind the cursor: make the whole
  // page wake the island so the visuals can be checked with `npm run dev`.
  if (!IS_TAURI) {
    document.addEventListener("click", () => Sound.resume(), { once: true });
  }
}

void main();
