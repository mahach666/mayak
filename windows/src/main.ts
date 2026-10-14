// Entry point: boot the bridge, wire the island, start the greeting.

import "./style.css";
import { Bridge, IS_TAURI, onEvent } from "./core/bridge";
import { setNotch } from "./core/layout";
import { Sound } from "./core/sound";
import { State, type Settings } from "./core/state";
import { setLanguage, t } from "./core/i18n";
import { Island } from "./island/island";
import { declineAllApprovals, registerHookHandlers, releaseOpenCard } from "./island/hooks";
import { registerAgentChatHandlers } from "./island/agent-chat";
import { registerIntegrationHandlers, refreshConfigured } from "./island/integrations";
import { registerObserver } from "./island/observer";
import { SAVED_RUN_KEY, decodeReloadEntry, encodeReloadEntry, type ReloadEntry } from "./island/pomodoro";

/**
 * What a language reload kept (spec §C, §F), taken once: the key is removed as it is read, so a
 * later real launch greets again. Where sessionStorage cannot be used, nothing is kept and the
 * page starts as a launch.
 */
function takeReloadEntry(): ReloadEntry | null {
  try {
    const raw = sessionStorage.getItem(SAVED_RUN_KEY);
    sessionStorage.removeItem(SAVED_RUN_KEY);
    return decodeReloadEntry(raw);
  } catch (err) {
    console.error("[mayak] could not read the kept pomodoro", err);
    return null;
  }
}

/** Just before a language reload: the marker, the running pomodoro and its phase card. */
function keepForReload() {
  try {
    sessionStorage.setItem(SAVED_RUN_KEY, encodeReloadEntry({
      run: State.pomodoro,
      phaseCard: State.phaseCard,
      phaseCardPending: State.phaseCardPending,
    }));
  } catch (err) {
    console.error("[mayak] could not keep the pomodoro", err);
  }
}

async function main() {
  const root = document.getElementById("root");
  if (!root) return;

  void Sound.preload();

  const boot = await Bridge.boot();
  // Before any card can be shown (registerHookHandlers, below): a page reload, whatever its
  // cause, left no card of the page before on screen, so nothing it held may keep its agent
  // waiting until the 590 s limit.
  await Bridge.approvalsReleaseAll();
  const kept = takeReloadEntry();
  const savedRun = kept?.run ?? null;
  if (boot) {
    State.settings = { ...State.settings, ...boot.settings };
    // A reloaded page has no pomodoro unless one was kept for a language reload (below), so the
    // tray check must not stay on.
    if (!savedRun) {
      void Bridge.setFocusCheck(false);
      // Nor may Rust still hold the run of the page before.
      void Bridge.pomodoroState(null);
    }
  }
  // A reloaded page starts expanded-sized, like a fresh app; the window may still be collapsed from before the reload.
  await Bridge.setCollapsed(false);
  // Before the island is built: its views take their texts when they are made.
  setLanguage(State.settings.language);
  // Rust keeps no dictionary: the tray takes its texts from here, at every load (spec §C).
  void Bridge.trayTexts({
    open: t("tray.open"),
    settings: t("tray.settings"),
    focus: t("tray.focus"),
    pause: t("tray.pause"),
    quit: t("tray.quit"),
    waiting: t("tray.waiting"),
    minutes: t("tray.minutes"),
    justNow: t("tray.justNow"),
  });

  // Before the island is built: its compact size wraps around the real notch.
  if (boot?.screen.notch) setNotch(boot.screen.notch[0], boot.screen.notch[1]);
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
  await onEvent<null>("hotkey-next", () => island.onNextHotkey());

  /**
   * Pause has to reach Rust too, or the pollers keep calling out. Pause beats
   * hold (spec §10): pausing drops it, and lifting the pause restores it from
   * the last snapshot before anything reveals the island.
   */
  const setPaused = (on: boolean) => {
    if (State.paused === on) return;
    State.paused = on;
    void Bridge.setPaused(on);
    // Quiet mode releases an open permission or question card (spec §2.1), from the tray or
    // from the island's quiet button, which leaves the island open.
    if (on) releaseOpenCard(island);
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
        island.alert(State.openView());
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

  await onEvent<null>("screen-changed", async () => {
    await Bridge.reposition();
    // A Mac measures the notch of the new screen as the window lands on it.
    window.setTimeout(async () => island.setNativeStrip((await Bridge.stripVisible()) ?? false), 300);
  });

  // Settings → Reload sounds: read the sounds folder again, and let them hear it.
  await onEvent<null>("sounds-changed", () => {
    Sound.reload().then(
      () => Sound.play("pop"),
      (err) => void Bridge.log(`sounds: reload failed: ${String(err)}`),
    );
  });

  // The settings window writes preferences; apply them here without a restart.
  await onEvent<Settings>("settings-changed", async (s) => {
    // The views took their texts when they were built: a new language reloads the page (spec §C).
    if (s.language !== undefined && s.language !== State.settings.language) {
      // A permission or question card would not survive the reload: hand it back to the agent
      // first, so it is not held up until the relay gives up. The card is forgotten before the
      // await, so a fold meanwhile does not release it a second time.
      await declineAllApprovals();
      // The reload is no launch: main() takes back the marker, the running pomodoro and its card.
      keepForReload();
      location.reload();
      return;
    }
    State.settings = { ...State.settings, ...s };
    island.applySettings();
    State.loadIntegrationTasks();
    void refreshConfigured();
  });

  registerHookHandlers(island);
  registerAgentChatHandlers(island);
  registerIntegrationHandlers(island);
  // Before launch, so the greeting already collapses onto the sessions view.
  await registerObserver(island);

  // Just before launch: a kept phase that ended meanwhile moves on once the island is launched,
  // and its card joins the kept one. A held card shows through releaseHeldCard as the island opens.
  if (kept?.phaseCard) {
    State.phaseCard = kept.phaseCard;
    State.phaseCardPending = kept.phaseCardPending;
  }
  if (savedRun) island.resumePomodoro(savedRun);
  // A language reload is no launch: no greeting (spec §F).
  island.launch({ greet: !kept?.reloaded });

  // In a plain browser there is no wake strip behind the cursor: make the whole
  // page wake the island so the visuals can be checked with `npm run dev`.
  if (!IS_TAURI) {
    document.addEventListener("click", () => Sound.resume(), { once: true });
  }
}

void main();
