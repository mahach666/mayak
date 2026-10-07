// The Focus tab (pomodoro spec §3): the countdown, Start/Stop and the pomodoro settings.
// Its 1 s clock runs only while the tab is open (setActive); the run itself lives in the island.

import { h } from "./dom";
import { Bridge } from "../core/bridge";
import { State } from "../core/state";
import { POMODORO_CHOICES, clock, normalizeConfig, phaseLabel, phaseMs } from "../island/pomodoro";
import { t } from "../core/i18n";
import type { ViewActions, ViewHost } from "./views";

type LengthKey = "pomodoroWork" | "pomodoroShort" | "pomodoroLong" | "pomodoroCycles";

function save() {
  void Bridge.saveSettings(State.settings);
  State.notify();
}

export function buildFocusTab(actions: ViewActions): ViewHost {
  const time = h("div", { class: "focus-tab-clock" });
  const label = h("div", { class: "focus-tab-phase" });
  const startBtn = h("button", { class: "btn primary", onclick: () => actions.togglePomodoro() });

  /** One row of segment buttons for a length (the island's `.seg`, like Auto-close). */
  function lengthRow(text: string, key: LengthKey, choices: readonly number[], unit: string) {
    const buttons = choices.map((v) =>
      h("button", {
        text: `${v}${unit}`,
        onclick: () => {
          State.settings[key] = v;
          save();
        },
      }),
    );
    const row = h(
      "div",
      { class: "settings-row focus-tab-row" },
      h("span", { class: "focus-tab-label", text, title: text }),
      h("div", { class: "seg" }, ...buttons),
    );
    return { row, sync: (current: number) => buttons.forEach((b, i) => b.classList.toggle("on", choices[i] === current)) };
  }

  const work = lengthRow(t("focus.work"), "pomodoroWork", POMODORO_CHOICES.work, "");
  const short = lengthRow(t("pomodoro.short"), "pomodoroShort", POMODORO_CHOICES.short, "");
  const long = lengthRow(t("pomodoro.long"), "pomodoroLong", POMODORO_CHOICES.long, "");
  const cycles = lengthRow(t("focus.longAfter"), "pomodoroCycles", POMODORO_CHOICES.cycles, "");

  const muteSwitch = h("button", {
    class: "switch",
    onclick: () => {
      State.settings.pomodoroMute = !State.settings.pomodoroMute;
      save();
    },
  });
  const waitsSwitch = h("button", {
    class: "switch",
    onclick: () => {
      if (!State.settings.pomodoroMute) return;
      State.settings.focusLetWaitsThrough = !State.settings.focusLetWaitsThrough;
      save();
    },
  });
  const waitsRow = h(
    "div",
    { class: "settings-row focus-tab-row" },
    waitsSwitch,
    h("span", { text: t("focus.letWaits") }),
  );

  const el = h(
    "div",
    { class: "view" },
    h(
      "div",
      { class: "card focus-tab-card" },
      h(
        "div",
        { class: "focus-tab" },
        h("div", { class: "focus-tab-left" }, time, label, startBtn),
        h(
          "div",
          { class: "focus-tab-right" },
          work.row,
          short.row,
          long.row,
          cycles.row,
          h("div", { class: "settings-row focus-tab-row" }, muteSwitch, h("span", { text: t("focus.mute") })),
          waitsRow,
        ),
      ),
    ),
  );

  function render() {
    const cfg = normalizeConfig(State.settings);
    const run = State.pomodoro;
    const now = Date.now();
    time.textContent = run ? clock(run.endsAt, now) : clock(phaseMs("work", cfg), 0);
    label.textContent = run ? phaseLabel(run.phase, run.cycle, cfg, t) : t("focus.notRunning");
    time.classList.toggle("break", run != null && run.phase !== "work");
    startBtn.textContent = t(run ? "focus.stop" : "focus.start");
    startBtn.className = run ? "btn secondary" : "btn primary";
    work.sync(cfg.work);
    short.sync(cfg.short);
    long.sync(cfg.long);
    cycles.sync(cfg.cycles);
    muteSwitch.classList.toggle("on", State.settings.pomodoroMute);
    waitsSwitch.classList.toggle("on", State.settings.focusLetWaitsThrough);
    waitsRow.classList.toggle("disabled", !State.settings.pomodoroMute);
  }

  let ticker: number | null = null;

  return {
    el,
    sync: render,
    setActive(on: boolean) {
      if (ticker != null) window.clearInterval(ticker);
      ticker = null;
      if (!on) return;
      render();
      ticker = window.setInterval(() => {
        actions.checkPomodoro();
        render();
      }, 1000);
    },
  };
}
