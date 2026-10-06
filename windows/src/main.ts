// Entry point: boot the bridge, wire the island, start the greeting.

import "./style.css";
import { Bridge, IS_TAURI, onEvent } from "./core/bridge";
import { Sound } from "./core/sound";
import { State, type Settings } from "./core/state";
import { Island } from "./island/island";
import { registerHookHandlers } from "./island/hooks";
import { registerIntegrationHandlers, refreshConfigured } from "./island/integrations";
import { registerObserver } from "./island/observer";

async function main() {
  const root = document.getElementById("root");
  if (!root) return;

  void Sound.preload();

  const island = new Island(root);

  const boot = await Bridge.boot();
  if (boot) {
    State.settings = { ...State.settings, ...boot.settings };
  }
  island.applySettings();
  State.loadIntegrationTasks();
  if (boot && !boot.cursorPoll) island.followPageCursor();
  if (boot?.nativeStrip) island.enableNativeStrip();

  await onEvent<{ x: number; y: number }>("cursor", ({ x, y }) => island.onCursor(x, y));
  await onEvent<null>("strip-touch", () => island.onStripTouch());
  await onEvent<{ x: number; y: number } | null>("strip-cursor", (p) => island.onStripCursor(p));
  await onEvent<{ on: boolean }>("strip-fullscreen", ({ on }) => island.onStripFullscreen(on));

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
    }
  });

  await onEvent<null>("screen-changed", () => void Bridge.reposition());

  // The settings window writes preferences; apply them here without a restart.
  await onEvent<Settings>("settings-changed", (s) => {
    State.settings = { ...State.settings, ...s };
    island.applySettings();
    State.loadIntegrationTasks();
    void refreshConfigured();
  });

  registerHookHandlers(island);
  registerIntegrationHandlers(island);
  // Before launch, so the greeting already collapses onto the sessions view.
  await registerObserver(island);

  island.launch();

  // In a plain browser there is no wake strip behind the cursor: make the whole
  // page wake the island so the visuals can be checked with `npm run dev`.
  if (!IS_TAURI) {
    document.addEventListener("click", () => Sound.resume(), { once: true });
  }
}

void main();
