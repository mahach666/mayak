// Island views — DOM ports of IslandViewContent.swift. Paddings, font sizes,
// colours and wording are copied from the Swift views so both platforms read
// identically.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { Sessions } from "../core/sessions";
import { State, type AgentTask } from "../core/state";
import { CHAT_ENABLED } from "../core/features";
import { t } from "../core/i18n";
import type { ApprovalDecision } from "../core/bridge";
import { washRGBA, type IslandViewName, type Wash } from "../core/layout";
import {
  answersOnClick, buildAnswers, canAnswer, togglePick, type Answers, type Question,
} from "../island/answers";
import { createMiniBot, pruneMiniBots } from "../mochi/minibots";
import { buildPrompt } from "./chat";
import { buildChoose, buildUpload, buildUploading } from "./upload";
import { renderIntegrationCard, type IntegrationCardHooks } from "./integrations";
import { buildHome } from "./usage";
import { buildStats } from "./stats";
import { buildFocusTab } from "./focus";
import { buildApps } from "./apps";
import {
  focusCountsLine, focusProjectsLine, focusSummary,
} from "../island/focus";
import {
  buildPager, buildSession, buildSessions, buildWaiting, cardSubject, outcomeSummary, silentText, subjectWho,
} from "./sessions";

export interface ViewActions {
  setView(v: IslandViewName): void;
  collapse(): void;
  /** Collapse button: home → petit, petit → strip (fsm.forceCollapse). */
  forceCollapse(): void;
  /** Quiet button: the tray's Pause, without hiding the island. */
  toggleQuiet(): void;
  /** The Focus tab's Start/Stop and the tray's Focus: start a pomodoro, or stop the running one. */
  togglePomodoro(): void;
  /** A tick of the Focus tab: move on if the phase is over (a timer late after sleep). */
  checkPomodoro(): void;
  setFocus(id: string): void;
  /** The ↗ button: opens whatever the focused pill points at. */
  openTarget(): void;
  openUrl(url: string): void;
  /** A permission card's Allow / Always / Deny. */
  decide(d: ApprovalDecision): void;
  /** "In terminal" on a permission or question card: releases the agent, no decision. */
  inTerminal(): void;
  /** A question card's answers (labels only, as received). */
  answer(answers: Answers): void;
  toggleSound(): void;
  /** The slider moves: applies the volume, does not save. */
  setVolume(v: number): void;
  /** The slider is released: saves the volume. */
  saveVolume(): void;
  setAutoClose(seconds: number): void;
  openSettingsWindow(): void;
  blip(): void;
  /** Sessions row: ack it, then open the card that fits its state before the click. */
  openSession(key: string): void;
  /** "OK" on a card: ack State.alertKey and collapse. */
  ackAlert(): void;
  /** Pager over snapshot.attention. */
  pageAlert(step: 1 | -1): void;
  /** "↗ Open": the project of State.alertKey in its host editor. */
  openSessionTarget(): void;
  toggleIdleFold(projectKey: string): void;
}

export interface ViewHost {
  el: HTMLElement;
  sync(): void;
  /** Called when the view becomes active, for views with a text field. */
  focus?(): void;
  /** Called every frame while the view is on screen. */
  tick?(nowMs: number): void;
  /** Expanded and shown (true) or not (false); called only when that changes. */
  setActive?(on: boolean): void;
}

// ── Shared pieces ─────────────────────────────────────────────────────────────

function card(wash: Wash, ...children: (Node | string)[]): HTMLElement {
  const el = h("div", { class: wash ? "card wash" : "card" }, ...children);
  if (wash) el.style.setProperty("--wash", washRGBA(wash));
  return el;
}

function btn(
  label: string,
  kind: "primary" | "secondary",
  onClick: () => void,
  kbd?: string,
): HTMLElement {
  return h(
    "button",
    { class: `btn ${kind}`, onclick: onClick },
    h("span", { text: label }),
    kbd ? h("span", { class: "kbd", text: kbd }) : null,
  );
}

function stack(padLeft: number, padRight: number, ...children: Node[]): HTMLElement {
  const el = h("div", { class: "stack" }, ...children);
  el.style.padding = `4px ${padRight}px 4px ${padLeft}px`;
  return el;
}

// ── Header ────────────────────────────────────────────────────────────────────

export function buildHeader(actions: ViewActions): ViewHost {
  const tabHome = h(
    "button",
    { class: "tab", title: t("tab.overview"), onclick: () => go("overview") },
    svg(ICONS.house, 13),
  );
  const tabSessions = h("button", { class: "tab", title: t("tab.sessions"), onclick: () => go("sessions") }, svg(ICONS.list, 13));
  const tabStats = h("button", { class: "tab", title: t("tab.stats"), onclick: () => go("stats") }, svg(ICONS.bars, 13));
  const tabFocus = h("button", { class: "tab", title: t("tab.focus"), onclick: () => go("focus") }, svg(ICONS.target, 13));
  const tabApps = h("button", { class: "tab", title: t("tab.apps"), onclick: () => go("apps") }, svg(ICONS.grid, 13));
  const tabChat = h("button", { class: "tab", title: t("tab.ask"), onclick: () => go("prompt") }, svg(ICONS.bubble, 13));
  const tabDrop = h("button", { class: "tab", title: t("tab.drop"), onclick: () => go("upload") }, svg(ICONS.plus, 13));

  const gearBtn = h("button", { title: t("header.settings"), onclick: () => go("settings") }, svg(ICONS.gear, 14));
  const soundBtn = h("button", { title: t("header.mute"), onclick: () => actions.toggleSound() }, svg(ICONS.speakerOn, 14));
  const quietBtn = h("button", { title: t("header.quiet"), onclick: () => actions.toggleQuiet() }, svg(ICONS.bell, 14));
  const collapseBtn = h(
    "button",
    { title: t("header.collapse"), onclick: () => actions.forceCollapse() },
    svg(ICONS.chevronUp, 12, { stroke: 2.4 }),
  );

  function go(v: IslandViewName) {
    actions.blip();
    actions.setView(v);
  }

  const el = h(
    "div",
    { id: "header" },
    h("div", { class: "tabs" }, tabSessions, tabHome, tabStats, tabFocus, tabApps, ...(CHAT_ENABLED ? [tabChat, tabDrop] : [])),
    h("div", { class: "header-actions" }, gearBtn, soundBtn, quietBtn, collapseBtn),
  );

  return {
    el,
    sync() {
      const v = State.view;
      tabSessions.classList.toggle("on", v === "sessions" || v === "waiting" || v === "session");
      tabHome.classList.toggle("on", v === "overview" || v === "empty");
      tabStats.classList.toggle("on", v === "stats");
      tabFocus.classList.toggle("on", v === "focus");
      tabApps.classList.toggle("on", v === "apps");
      tabChat.classList.toggle("on", v === "prompt");
      tabDrop.classList.toggle("on", v === "upload");
      gearBtn.classList.toggle("on", v === "settings");
      clear(gearBtn);
      gearBtn.append(svg(v === "settings" ? ICONS.gearFill : ICONS.gear, 14));
      clear(soundBtn);
      soundBtn.append(svg(State.settings.soundEnabled ? ICONS.speakerOn : ICONS.speakerOff, 14));
      clear(quietBtn);
      quietBtn.append(svg(State.paused ? ICONS.bellSlash : ICONS.bell, 14));
      quietBtn.title = t(State.paused ? "header.quietOn" : "header.quiet");
      el.style.opacity = v === "confused" ? "0" : "1";
    },
  };
}

// ── Overview ──────────────────────────────────────────────────────────────────

/**
 * The integrations home inherited from Coucou (integration card and pills). Not registered since
 * the home page shows the plan limits (usage-home spec §1.1, §4); kept unchanged with
 * renderIntegrationCard and the pills.
 */
export function buildOverview(actions: ViewActions): ViewHost {
  const leftBody = h("div", { class: "left-body" });
  const jump = h(
    "button",
    { class: "icon-btn jump", title: t("common.open"), onclick: () => actions.openTarget() },
    svg(ICONS.arrowUpRight, 8),
  );
  const left = card(null, leftBody, jump);
  const pills = h("div", { class: "pills" });
  const right = card(null, pills);

  const el = h("div", { class: "view overview" },
    h("div", { class: "left" }, left),
    h("div", { class: "right" }, right),
  );

  let pillIds = "";
  let detailOpen = false;
  let lastFocus: string | null = null;
  let cardKey = "";

  const hooks: IntegrationCardHooks = {
    get detailOpen() {
      return detailOpen;
    },
    openDetail() {
      detailOpen = true;
      cardKey = "";
      State.notify();
    },
    closeDetail() {
      detailOpen = false;
      cardKey = "";
      State.notify();
    },
    openSettings: () => actions.openSettingsWindow(),
  };

  return {
    el,
    sync() {
      const task = State.focusTask;
      if (task?.id !== lastFocus) {
        lastFocus = task?.id ?? null;
        detailOpen = false;
        cardKey = "";
      }

      // Every pill shows its own card, exactly like IntegrationCardView. Agent
      // sessions have their own view ("sessions").
      if (task) {
        const info = State.integrations[task.id];
        const key = [
          task.id, detailOpen, task.state, task.steps.join("|"),
          info?.loaded, info?.error, info?.configured,
          JSON.stringify(info?.data ?? {}),
        ].join("~");
        if (key !== cardKey) {
          cardKey = key;
          clear(leftBody);
          leftBody.append(renderIntegrationCard(task, hooks));
        }
      }

      jump.style.display = detailOpen ? "none" : "";

      const others = State.otherTasks.slice(0, 4);
      const pillKey = others.map((t) => `${t.id}:${t.pillBadge ?? ""}`).join("|");
      if (pillKey !== pillIds) {
        pillIds = pillKey;
        clear(pills);
        for (const t of others) pills.append(buildPill(t, actions));
        pruneMiniBots();
      }
    },
  };
}

function buildPill(task: AgentTask, actions: ViewActions): HTMLElement {
  const label = task.name;
  const canvas = createMiniBot(task, 24);
  const pill = h(
    "div",
    { class: "pill", onclick: () => actions.setFocus(task.id) },
    canvas,
    h("span", { class: "lbl", text: label }),
  );
  pill.style.borderColor = `${task.color}24`;
  pill.addEventListener("mouseenter", () => {
    pill.style.background = `${task.color}2e`;
    pill.style.borderColor = `${task.color}8c`;
    pill.style.boxShadow = `0 2px 10px ${task.color}59`;
    (pill.querySelector(".lbl") as HTMLElement).style.color = lighten(task.color, 0.3);
  });
  pill.addEventListener("mouseleave", () => {
    pill.style.background = "";
    pill.style.borderColor = `${task.color}24`;
    pill.style.boxShadow = "";
    (pill.querySelector(".lbl") as HTMLElement).style.color = "";
  });

  if (task.pillBadge) {
    const colors = { approval: "#F5A524", finished: "#22C55E", error: "#F4505E" } as const;
    const icons = { approval: ICONS.bang, finished: ICONS.check, error: ICONS.xmark } as const;
    const inner = h("i", { style: `background:${colors[task.pillBadge]}` }, svg(icons[task.pillBadge], 6, { stroke: task.pillBadge === "finished" ? 3 : 0 }));
    const badge = h("div", { class: "pill-badge" }, inner);
    badge.style.boxShadow = `0 0 4px ${colors[task.pillBadge]}99`;
    pill.append(badge);
  }
  return pill;
}

function lighten(hex: string, amount: number): string {
  const v = parseInt(hex.replace("#", ""), 16);
  const c = [(v >> 16) & 255, (v >> 8) & 255, v & 255].map((x) =>
    Math.min(255, Math.round(x + amount * 255)),
  );
  return `rgb(${c[0]},${c[1]},${c[2]})`;
}

// ── Empty ─────────────────────────────────────────────────────────────────────

function buildEmpty(actions: ViewActions): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 118px;flex-direction:row;align-items:center;gap:16px" },
    h(
      "div",
      { style: "display:flex;flex-direction:column;gap:5px" },
      h("div", { class: "title", text: t("empty.title") }),
      ...(CHAT_ENABLED ? [h("div", { class: "sub", text: t("empty.sub") })] : []),
    ),
    h("div", { class: "grow" }),
    ...(CHAT_ENABLED ? [btn(t("empty.ask"), "primary", () => actions.setView("prompt"))] : []),
  );
  return { el: h("div", { class: "view" }, card(null, body)), sync() {} };
}

// ── Approval ──────────────────────────────────────────────────────────────────

function buildApproval(actions: ViewActions): ViewHost {
  const who = h("div");
  const code = h("div", { class: "code" });
  const row = h("div", { class: "actions" });
  const el = h("div", { class: "view" }, card("amber", stack(116, 16, who, code, row)));
  let rowKey = "";
  return {
    el,
    sync() {
      clear(who);
      // The router names the project of the asking session; no project key comes
      // with it, so there is no project dot here.
      const card = State.pendingApproval;
      who.append(h("div", { class: "who-row" },
        h("span", { class: "n", text: card?.projectName ?? (card?.agent === "codex" ? "Codex" : "Claude Code") }),
        h("span", { text: t("phase.needsPermission") }),
      ));
      // The whole point of approving here rather than in the terminal: this line
      // is the command, the file path or the URL being authorised, not just the
      // name of the tool asking.
      code.textContent = card?.command || card?.tool || "…";
      // Built once per request. Rebuilding them between a mouse-down and a mouse-up
      // would swallow the click. "Always" only where Claude sent a rule to keep;
      // Codex has no such answer (spec §1.6).
      const key = card?.requestId ?? "";
      if (!card || rowKey === key) return;
      rowKey = key;
      clear(row);
      row.append(
        btn(t("approval.deny"), "secondary", () => actions.decide("deny"), "N"),
        btn(t("approval.allow"), "primary", () => actions.decide("allow"), "Y"),
      );
      if (card.canAlways) row.append(btn(t("approval.always"), "secondary", () => actions.decide("always")));
      row.append(h("div", { class: "grow" }), btn(t("approval.inTerminal"), "secondary", () => actions.inTerminal()));
    },
  };
}

// ── Question ──────────────────────────────────────────────────────────────────

/**
 * A Claude Code question (AskUserQuestion, spec §2.3): each question with its header and its
 * options. One single-select question is answered by the click on an option; otherwise each
 * question takes its pick (ticks for a multi-select) and Answer sends them all. No free text:
 * that is "In terminal". The hovered option's description shows in the bottom row.
 */
function buildQuestion(actions: ViewActions): ViewHost {
  const who = h("div");
  const list = h("div", { class: "q-list" });
  const hint = h("div", { class: "q-hint" });
  const answerBtn = btn(t("question.answer"), "primary", () => submit()) as HTMLButtonElement;
  answerBtn.classList.add("q-answer");
  const row = h("div", { class: "actions sess-actions" },
    hint,
    btn(t("approval.inTerminal"), "secondary", () => actions.inTerminal()),
    answerBtn,
  );
  const el = h("div", { class: "view" }, card("cyan", stack(116, 16, who, list, row)));

  let builtFor = "";
  let questions: Question[] = [];
  let picks: number[][] = [];
  let optionEls: HTMLButtonElement[][] = [];

  function syncPicks() {
    optionEls.forEach((els, qi) => els.forEach((b, oi) => b.classList.toggle("on", picks[qi].includes(oi))));
    answerBtn.disabled = !canAnswer(questions, picks);
  }

  function submit() {
    if (canAnswer(questions, picks)) actions.answer(buildAnswers(questions, picks));
  }

  function pick(qi: number, oi: number) {
    picks[qi] = togglePick(questions[qi], picks[qi], oi);
    if (answersOnClick(questions)) {
      submit();
      return;
    }
    syncPicks();
  }

  function build() {
    clear(list);
    hint.textContent = "";
    optionEls = questions.map((q, qi) => {
      const opts = q.options.map((o, oi) => h("button", {
        class: q.multiSelect ? "q-opt multi" : "q-opt",
        title: o.description || o.label,
        onclick: () => pick(qi, oi),
        onmouseenter: () => { hint.textContent = o.description; },
        onmouseleave: () => { hint.textContent = ""; },
      }, h("span", { text: o.label })));
      list.append(h("div", { class: "q-block" },
        h("div", { class: "q-head" },
          q.header ? h("span", { class: "q-chip", text: q.header }) : null,
          h("span", { class: "q-text", text: q.question, title: q.question }),
        ),
        h("div", { class: "q-opts" }, ...opts),
      ));
      return opts;
    });
    answerBtn.style.display = answersOnClick(questions) ? "none" : "";
    list.scrollTop = 0;
    syncPicks();
  }

  return {
    el,
    sync() {
      const card = State.pendingApproval;
      clear(who);
      who.append(h("div", { class: "who-row" },
        h("span", { class: "n", text: card?.projectName ?? "Claude Code" }),
        h("span", { text: t("phase.asks") }),
      ));
      // Built once per request: a rebuild between a mouse-down and a mouse-up would swallow the click.
      if (!card || card.kind !== "question" || builtFor === card.requestId) return;
      builtFor = card.requestId;
      questions = card.questions;
      picks = questions.map(() => []);
      build();
    },
  };
}

// ── Error ─────────────────────────────────────────────────────────────────────

/** Muted line on a Finished/Error card whose live Codex session ran its turn without hooks (silent-codex spec §4). */
function syncSilentNote(note: HTMLElement, subject: ReturnType<typeof cardSubject>) {
  const reason = subject && "session" in subject ? subject.session.silent : undefined;
  note.textContent = reason ? t("card.noHooks", { reason: silentText(reason).short }) : "";
  note.style.display = reason ? "" : "none";
}

/** Agent session that ended its turn on an error (State.alertKey). */
function buildError(actions: ViewActions): ViewHost {
  const who = h("div");
  const title = h("div", { class: "title", text: t("error.title") });
  const detail = h("div", { class: "detail sess-clamp" });
  const note = h("div", { class: "sub" });
  const pager = buildPager(actions);
  const row = h("div", { class: "actions sess-actions" },
    btn(t("common.openArrow"), "primary", () => actions.openSessionTarget()),
    btn(t("common.ok"), "secondary", () => actions.ackAlert()),
    h("div", { class: "grow" }),
    pager.el,
  );
  const el = h("div", { class: "view" }, card("red", stack(116, 16, who, title, detail, note, row)));
  return {
    el,
    sync() {
      const subject = cardSubject();
      clear(who);
      if (subject) who.append(subjectWho(subject, t("phase.error")));
      detail.textContent = outcomeSummary(subject, "error") || t("error.noDetail");
      syncSilentNote(note, subject);
      pager.sync();
    },
  };
}

// ── Finished ──────────────────────────────────────────────────────────────────

/** Agent session that finished its turn (State.alertKey). */
function buildFinished(actions: ViewActions): ViewHost {
  const who = h("div");
  const title = h("div", { class: "title sess-clamp" });
  const note = h("div", { class: "sub" });
  const pager = buildPager(actions);
  const row = h("div", { class: "actions sess-actions" },
    btn(t("common.openArrow"), "primary", () => actions.openSessionTarget()),
    btn(t("common.ok"), "secondary", () => actions.ackAlert()),
    h("div", { class: "grow" }),
    pager.el,
  );
  const el = h("div", { class: "view" }, card("green", stack(116, 16, who, title, note, row)));
  return {
    el,
    sync() {
      const subject = cardSubject();
      clear(who);
      if (subject) who.append(subjectWho(subject, t("phase.finished")));
      title.textContent = outcomeSummary(subject, "finished") || t("finished.fallback");
      syncSilentNote(note, subject);
      pager.sync();
    },
  };
}

// ── Pomodoro phase ────────────────────────────────────────────────────────────

/** A phase change or a stop (State.phaseCard, pomodoro spec §2.2), with what work held back. */
function buildFocusPhase(actions: ViewActions): ViewHost {
  const title = h("div", { class: "title" });
  const counts = h("div", { class: "sub" });
  const projects = h("div", { class: "focus-projects sess-clamp" });
  const row = h("div", { class: "actions sess-actions" }, btn(t("common.ok"), "primary", () => actions.collapse()));
  const el = h("div", { class: "view" }, card("indigo", stack(116, 16, title, counts, projects, row)));
  return {
    el,
    sync() {
      const shown = State.phaseCard;
      const records = shown?.records ?? [];
      const summary = focusSummary(records);
      title.textContent = shown?.title ?? "";
      counts.textContent = focusCountsLine(summary.counts, t);
      counts.style.display = records.length > 0 ? "" : "none";
      projects.textContent = focusProjectsLine(summary.projects, t);
      projects.style.display = records.length > 0 ? "" : "none";
    },
  };
}

// ── Confused ──────────────────────────────────────────────────────────────────

function buildConfused(): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 128px" },
    h("div", { class: "title", text: t("confused.title") }),
    h("div", { class: "sub", text: t("confused.sub") }),
  );
  return { el: h("div", { class: "view" }, card("pink", body)), sync() {} };
}

// ── Note ──────────────────────────────────────────────────────────────────────

function buildNote(): ViewHost {
  const title = h("div", { class: "title" });
  const el = h("div", { class: "view" }, card(null, h("div", { class: "stack", style: "padding:0 18px 0 98px" }, title)));
  return {
    el,
    sync() {
      title.textContent = State.noteMessage ?? "";
    },
  };
}

// ── In-island settings ────────────────────────────────────────────────────────

function buildSettings(actions: ViewActions): ViewHost {
  const soundSwitch = h("button", { class: "switch", onclick: () => actions.toggleSound() });
  const volume = h("input", {
    type: "range", min: "0", max: "0.2", step: "0.005",
    oninput: (e: Event) => actions.setVolume(Number((e.target as HTMLInputElement).value)),
    onchange: () => actions.saveVolume(),
  }) as HTMLInputElement;
  const autoLabel = h("span", {});
  const segButtons = [10, 15, 30].map((s) =>
    h("button", { onclick: () => actions.setAutoClose(s) }, t("unit.s", { n: s })),
  );
  const claudeBadge = h("span", { class: "status-badge" });
  const apiBadge = h("span", { class: "status-badge" });

  const rows = h(
    "div",
    { class: "settings-rows" },
    h("div", { class: "settings-row" }, soundSwitch, h("span", { text: t("settings.sound") }), volume),
    h(
      "div",
      { class: "settings-row" },
      svg(ICONS.timer, 12),
      autoLabel,
      h("div", { class: "seg" }, ...segButtons),
    ),
    h(
      "div",
      { class: "settings-row", style: "gap:14px" },
      claudeBadge,
      apiBadge,
      h("div", { class: "grow" }),
      h("button", {
        class: "link-btn",
        style: "color:#8e939c;font-size:11.5px",
        text: t("common.settingsMore"),
        onclick: () => actions.openSettingsWindow(),
      }),
    ),
  );

  const el = h("div", { class: "view" },
    card(null, h("div", { class: "stack", style: "padding:14px 16px 14px 84px" }, rows)));

  return {
    el,
    sync() {
      const s = State.settings;
      soundSwitch.classList.toggle("on", s.soundEnabled);
      volume.value = String(s.soundVolume);
      volume.style.opacity = s.soundEnabled ? "1" : "0.4";
      autoLabel.textContent = t("settings.autoClose", { s: Math.round(s.autoCloseInterval) });
      segButtons.forEach((b, i) => b.classList.toggle("on", s.autoCloseInterval === [10, 15, 30][i]));
      clear(claudeBadge);
      // Green while the observer reads ~/.claude/sessions without trouble.
      claudeBadge.append(
        dot(Sessions.snapshot?.sources.claude === "ok" ? "#22C55E" : "#F4505E", 6),
        h("span", { text: "Claude Code" }),
      );
      clear(apiBadge);
      apiBadge.append(dot("#F4505E", 6), h("span", { text: "API" }));
    },
  };
}

// ── Placeholders filled in later stages ───────────────────────────────────────

function buildPlaceholder(title: string, sub: string): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 118px" },
    h("div", { class: "title", text: title }),
    h("div", { class: "sub", text: sub }),
  );
  return { el: h("div", { class: "view" }, card(null, body)), sync() {} };
}

// ── Registry ──────────────────────────────────────────────────────────────────

export function buildViews(
  actions: ViewActions,
  onChatHeightChange: () => void,
): Map<IslandViewName, ViewHost> {
  const map = new Map<IslandViewName, ViewHost>();
  // The player row changes the home page's height (night spec §B).
  map.set("overview", buildHome(onChatHeightChange));
  map.set("empty", buildEmpty(actions));
  map.set("approval", buildApproval(actions));
  map.set("question", buildQuestion(actions));
  map.set("error", buildError(actions));
  map.set("finished", buildFinished(actions));
  map.set("confused", buildConfused());
  map.set("note", buildNote());
  map.set("settings", buildSettings(actions));
  map.set("prompt", buildPrompt(actions, onChatHeightChange));
  map.set("upload", buildUpload());
  map.set("uploading", buildUploading());
  map.set("choose", buildChoose(actions));
  // Not in the Windows v1: sending a file by email, window attach + web result.
  map.set("mail", buildPlaceholder(t("placeholder.mail"), ""));
  map.set("searching", buildPlaceholder(t("placeholder.searching"), ""));
  map.set("result", buildPlaceholder(t("placeholder.result"), ""));
  // Agent sessions (views/sessions.ts).
  map.set("sessions", buildSessions(actions));
  map.set("waiting", buildWaiting(actions));
  map.set("session", buildSession(actions));
  map.set("focus-phase", buildFocusPhase(actions));
  map.set("stats", buildStats());
  map.set("focus", buildFocusTab(actions));
  map.set("apps", buildApps(actions));
  return map;
}
