// Island views — DOM ports of IslandViewContent.swift. Paddings, font sizes,
// colours and wording are copied from the Swift views so both platforms read
// identically.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { Sessions } from "../core/sessions";
import { State, type AgentTask } from "../core/state";
import { CHAT_ENABLED, CLI_CHAT_ENABLED } from "../core/features";
import { isOpenPage } from "../core/open-view";
import { t } from "../core/i18n";
import { Bridge, type ApprovalDecision } from "../core/bridge";
import { EXPANDED_W, washRGBA, type IslandViewName, type Wash } from "../core/layout";
import {
  answersOnClick, buildAnswers, canAnswer, clickCounts, otherText, OTHER_MAX_CHARS, questionAnswered, togglePick,
  type Answers, type CardBinding, type Question,
} from "../island/answers";
import { createMiniBot, pruneMiniBots } from "../luch/minibots";
import { buildPrompt } from "./chat";
import { buildAgentChat } from "./agentchat";
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
  buildPager, buildResumeCopy, buildSession, buildSessions, buildTurnFiles, buildWaiting, cardAoChip, cardSubject,
  outcomeSummary, sessionCardButtons, silentText, subjectWho,
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
  /**
   * A permission card's Allow / Always / Deny. `bound` is the card the buttons were built for:
   * a click for another card, or within CLICK_GUARD_MS of the build, is ignored (answers.ts clickCounts).
   */
  decide(bound: CardBinding, d: ApprovalDecision): void;
  /** "In terminal" on a permission or question card: releases the agent, no decision. Bound as decide. */
  inTerminal(bound: CardBinding): void;
  /** A question card's answers (labels as received, or an "Other" text). Bound as decide. */
  answer(bound: CardBinding, answers: Answers): void;
  /**
   * The question card holds the open island (true) while an "Other" answer is being typed, or
   * lets it fold on its own again (false): auto-close and its countdown wait (spec §6).
   */
  holdOpen(on: boolean): void;
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
  /** "↗ Open": the session `key` (default State.alertKey) in its host editor; a row's «↗» passes its own key. */
  openSessionTarget(key?: string): void;
  /** A file chip of a turn summary: opens that file in an editor; false when nothing could open it. */
  openTurnFile(key: string, index: number, path: string): Promise<boolean>;
  toggleIdleFold(projectKey: string): void;
  /** The sessions list's «Recent» fold (ideas §7): open it (and read the recent sessions) or close it. */
  toggleRecent(): void;
  /** The sessions list's Agent Orchestrator fold (core/ao.ts): open it or close it. */
  toggleAo(): void;
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
  /** The session list: the keys of the rows ↑ ↓ walk through, top to bottom. */
  listKeys?(): string[];
  /** The session list: Enter on a row that is not a session (a «Recent» line); false leaves it to the host switch. */
  activate?(key: string): boolean;
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
  const tabAgents = h("button", { class: "tab", title: t("tab.agents"), onclick: () => go("agentchat") }, svg(ICONS.terminal, 13));

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
    // "Open on click: Last opened" follows the tabs the user chooses, nothing else.
    if (isOpenPage(v)) State.lastOpenedPage = v;
    actions.setView(v);
  }

  const el = h(
    "div",
    { id: "header" },
    h("div", { class: "tabs" }, tabSessions, tabHome, tabStats, tabFocus, tabApps, ...(CHAT_ENABLED ? [tabChat, tabDrop] : []), ...(CLI_CHAT_ENABLED ? [tabAgents] : [])),
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
      tabAgents.classList.toggle("on", v === "agentchat");
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
 * The integrations home inherited from Coucou (integration card and pills). Not registered: the
 * home page (usage.ts buildHome) shows the plan limits and renderIntegrationCard's cards as tiles;
 * kept unchanged with the pills.
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
        // Its Agent Orchestrator session, when AO (pill on) lists it.
        cardAoChip(card),
      ));
      // The whole point of approving here rather than in the terminal: this line
      // is the command, the file path or the URL being authorised, not just the
      // name of the tool asking.
      code.textContent = card?.command || card?.tool || "…";
      // Built once per request. Rebuilding them between a mouse-down and a mouse-up
      // would swallow the click. "Always" only where Claude sent a rule to keep;
      // Codex has no such answer (spec §1.6).
      // No permission card pending (released, answered): no button to press.
      if (!card || card.kind !== "permission") {
        rowKey = "";
        clear(row);
        return;
      }
      if (rowKey === card.requestId) return;
      rowKey = card.requestId;
      clear(row);
      // The buttons answer this card only, and not in their first 400 ms on screen.
      const bound: CardBinding = { requestId: card.requestId, builtAt: performance.now() };
      row.append(
        btn(t("approval.deny"), "secondary", () => actions.decide(bound, "deny")),
        btn(t("approval.allow"), "primary", () => actions.decide(bound, "allow")),
      );
      if (card.canAlways) row.append(btn(t("approval.always"), "secondary", () => actions.decide(bound, "always")));
      row.append(h("div", { class: "grow" }), btn(t("approval.inTerminal"), "secondary", () => actions.inTerminal(bound)));
    },
  };
}

// ── Question ──────────────────────────────────────────────────────────────────

/**
 * A Claude Code question (AskUserQuestion, spec §2.3, §6), read like Claude's own dialog: one
 * question per page ("Question i of N", Back, Next), its full text in bold, its options as
 * full-width rows (bold label, dim description, number badge; ticks for a multi-select) and an
 * "Other" field under them. One single-select question is answered by the click on an option;
 * otherwise Answer, on the last page, sends every question at once, and so does Enter in "Other"
 * (Next before the last page). Whichever was touched last answers a question: the typed text
 * (in place of the picks) or a clicked option (the text stays in the field, inactive). The
 * card is as tall as its content up to the panel; beyond that only the options list scrolls
 * (`relayout`). The island takes the keyboard only while "Other" has the focus, and does not
 * fold on its own while "Other" has the focus or any page holds a typed text (`holdOpen`).
 */
function buildQuestion(actions: ViewActions, relayout: () => void): ViewHost {
  const who = h("div", { class: "q-who" });
  const pageLabel = h("span", { class: "q-page" });
  const chip = h("span", { class: "q-chip" });
  const text = h("div", { class: "q-text" });
  const list = h("div", { class: "q-list" });
  const otherInput = h("input", {
    class: "q-other-input",
    type: "text",
    maxlength: OTHER_MAX_CHARS,
    placeholder: t("question.otherPlaceholder"),
    "aria-label": t("question.other"),
    spellcheck: "false",
    autocomplete: "off",
  }) as HTMLInputElement;
  const otherNum = h("span", { class: "q-num" });
  const otherRow = h("label", { class: "q-row q-other" }, h("span", { class: "q-other-field" }, otherInput), otherNum);
  const backBtn = btn(t("question.back"), "secondary", () => back()) as HTMLButtonElement;
  const nextBtn = btn(t("question.next"), "primary", () => next()) as HTMLButtonElement;
  const answerBtn = btn(t("question.answer"), "primary", () => submit()) as HTMLButtonElement;
  nextBtn.classList.add("q-answer");
  answerBtn.classList.add("q-answer");
  const row = h("div", { class: "actions sess-actions q-actions" },
    btn(t("approval.inTerminal"), "secondary", () => { if (bound) actions.inTerminal(bound); }),
    h("div", { class: "grow" }),
    backBtn,
    nextBtn,
    answerBtn,
  );
  const stack = h("div", { class: "stack q-stack" },
    h("div", { class: "q-top" }, who, pageLabel),
    h("div", { class: "q-head" }, chip, text),
    list,
    otherRow,
    row,
  );
  const el = h("div", { class: "view" }, card("cyan", stack));

  let builtFor = "";
  /**
   * The card and page the controls were last built for: every button, row and Enter of the view
   * answers that card only, and not in the first CLICK_GUARD_MS after a page is built (a double
   * click on Next must not land on the Answer that takes its place).
   */
  let bound: CardBinding | null = null;
  let questions: Question[] = [];
  let picks: number[][] = [];
  let others: string[] = [];
  /** Per question: its typed text, not its picks, is the answer (it was edited after the last click). */
  let otherOn: boolean[] = [];
  let pageIdx = 0;
  let rowEls: HTMLButtonElement[] = [];
  /** focusWindow(true) was asked and not yet given back. */
  let keyboard = false;
  /** Expanded and on screen (setActive). */
  let active = false;
  /** holdOpen(true) was asked and not yet let go. */
  let holding = false;

  /** Holds the island open while an answer is being typed: the field has the focus, or a page has text. */
  function syncHold() {
    const want = active && bound != null && (keyboard || others.some((x) => x.trim() !== ""));
    if (want === holding) return;
    holding = want;
    actions.holdOpen(want);
  }

  function takeKeyboard() {
    if (keyboard || !bound) return;
    keyboard = true;
    void Bridge.focusWindow(true);
    syncHold();
  }

  function giveKeyboard() {
    if (!keyboard) return;
    keyboard = false;
    void Bridge.focusWindow(false);
    syncHold();
  }

  /** What answers each question: the typed text where it is the answer, else "" (the picks). */
  const activeOthers = () => others.map((x, i) => (otherOn[i] ? x : ""));

  /** The card ends or leaves the screen: "Other" lets go of the keyboard. */
  function dropKeyboard() {
    if (document.activeElement === otherInput) otherInput.blur();
    giveKeyboard();
  }

  /** A click or Enter counts: on this card, still pending, CLICK_GUARD_MS after the page was built. */
  function counts(): boolean {
    return bound != null && clickCounts(bound, State.pendingApproval?.requestId, performance.now());
  }

  const isLast = () => pageIdx === questions.length - 1;

  function syncControls() {
    const q = questions[pageIdx];
    if (!q) return;
    const typed = otherText(activeOthers()[pageIdx]) !== null;
    rowEls.forEach((b, oi) => b.classList.toggle("on", !typed && picks[pageIdx].includes(oi)));
    otherRow.classList.toggle("on", typed);
    // A text kept in the field while a clicked option answers: shown, not the answer.
    otherRow.classList.toggle("idle", !typed && others[pageIdx].trim() !== "");
    nextBtn.disabled = !questionAnswered(q, picks[pageIdx], activeOthers()[pageIdx]);
    answerBtn.disabled = !canAnswer(questions, picks, activeOthers());
  }

  function submit() {
    const typed = activeOthers();
    if (!bound || !canAnswer(questions, picks, typed)) return;
    actions.answer(bound, buildAnswers(questions, picks, typed));
  }

  function back() {
    if (pageIdx === 0 || !counts()) return;
    showPage(pageIdx - 1);
  }

  function next() {
    if (isLast() || !counts()) return;
    if (!questionAnswered(questions[pageIdx], picks[pageIdx], activeOthers()[pageIdx])) return;
    showPage(pageIdx + 1);
  }

  function pick(oi: number) {
    // A tick from a double click on the card before, or for a card no longer pending, is ignored too.
    if (!counts()) return;
    // The latest choice wins: an option clicked after typing answers in place of the text, which
    // stays in the field (spec §6). The picks start from where they were before the typing.
    picks[pageIdx] = togglePick(questions[pageIdx], picks[pageIdx], oi);
    otherOn[pageIdx] = false;
    if (answersOnClick(questions)) {
      submit();
      return;
    }
    syncControls();
  }

  /**
   * The card's natural height at full width: its paddings, its rows (fractional heights, with
   * their margins) and gaps, the options list at its full scroll height, and the border. Null
   * while the island is still opening (the texts would wrap more than they will).
   */
  function naturalHeight(): number | null {
    const island = stack.closest<HTMLElement>("#island");
    if (!island || Math.abs(island.offsetWidth - EXPANDED_W) > 1) return null;
    const cs = getComputedStyle(stack);
    const kids = [...stack.children].filter((k): k is HTMLElement => k instanceof HTMLElement && k.style.display !== "none");
    let total = parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom) + (parseFloat(cs.rowGap) || 0) * (kids.length - 1);
    for (const k of kids) {
      const ks = getComputedStyle(k);
      total += (k === list ? list.scrollHeight : k.getBoundingClientRect().height) + parseFloat(ks.marginTop) + parseFloat(ks.marginBottom);
    }
    const cardEl = stack.parentElement;
    if (cardEl) {
      const ccs = getComputedStyle(cardEl);
      total += parseFloat(ccs.borderTopWidth) + parseFloat(ccs.borderBottomWidth);
    }
    return total;
  }

  /** The card follows its content: on a page build, and as the island opens. */
  function fit() {
    if (!builtFor) return;
    const natural = naturalHeight();
    if (natural === null || Math.abs(natural - State.questionContentH) < 1) return;
    State.questionContentH = natural;
    relayout();
  }
  new ResizeObserver(() => fit()).observe(list);

  function showPage(i: number) {
    pageIdx = i;
    const q = questions[i];
    const many = questions.length > 1;
    pageLabel.textContent = many ? t("question.page", { i: i + 1, n: questions.length }) : "";
    pageLabel.style.display = many ? "" : "none";
    chip.textContent = q.header;
    chip.style.display = q.header ? "" : "none";
    text.textContent = q.question;
    text.scrollTop = 0;
    clear(list);
    rowEls = q.options.map((o, oi) => {
      const b = h("button", { class: "q-row", onclick: () => pick(oi) },
        q.multiSelect ? h("span", { class: "q-tick" }, svg(ICONS.check, 9, { stroke: 3 })) : null,
        h("span", { class: "q-body" },
          h("span", { class: "q-label", text: o.label }),
          o.description ? h("span", { class: "q-desc", text: o.description }) : null,
        ),
        h("span", { class: "q-num", text: String(oi + 1) }),
      );
      list.append(b);
      return b;
    });
    otherNum.textContent = String(q.options.length + 1);
    otherInput.value = others[i];
    list.scrollTop = 0;
    backBtn.style.display = many && i > 0 ? "" : "none";
    nextBtn.style.display = many && !isLast() ? "" : "none";
    answerBtn.style.display = isLast() ? "" : "none";
    syncControls();
    if (bound) bound = { requestId: bound.requestId, builtAt: performance.now() };
    fit();
  }

  otherInput.addEventListener("mousedown", () => takeKeyboard());
  otherInput.addEventListener("focus", () => takeKeyboard());
  otherInput.addEventListener("blur", () => giveKeyboard());
  otherInput.addEventListener("input", () => {
    if (!bound) return;
    // Editing the text makes it the answer (spec §6); the picks are kept, not sent.
    others[pageIdx] = otherInput.value;
    otherOn[pageIdx] = true;
    syncControls();
    syncHold();
  });
  otherInput.addEventListener("keydown", (e: KeyboardEvent) => {
    // An IME composing (zh, ja) uses Enter to commit its text, not to answer.
    if (e.key !== "Enter" || e.isComposing || e.keyCode === 229 || e.repeat) return;
    e.preventDefault();
    if (!isLast()) {
      next();
    } else if (counts()) {
      submit();
    }
  });

  return {
    el,
    sync() {
      const card = State.pendingApproval;
      clear(who);
      who.append(h("div", { class: "who-row" },
        h("span", { class: "n", text: card?.projectName ?? "Claude Code" }),
        h("span", { text: t("phase.asks") }),
        cardAoChip(card),
      ));
      // No question card pending (released, answered): no option, field or button to use.
      if (!card || card.kind !== "question") {
        dropKeyboard();
        builtFor = "";
        bound = null;
        questions = [];
        picks = [];
        others = [];
        otherOn = [];
        rowEls = [];
        pageIdx = 0;
        clear(list);
        otherInput.value = "";
        stack.style.display = "none";
        State.questionContentH = 0;
        syncHold();
        return;
      }
      // Built once per request: a rebuild between a mouse-down and a mouse-up would swallow the click.
      if (builtFor === card.requestId) return;
      dropKeyboard();
      stack.style.display = "";
      builtFor = card.requestId;
      questions = card.questions;
      picks = questions.map(() => []);
      others = questions.map(() => "");
      otherOn = questions.map(() => false);
      bound = { requestId: card.requestId, builtAt: performance.now() };
      syncHold();
      // A card before may have been taller: start from the layout height, then measure this one.
      State.questionContentH = 0;
      relayout();
      showPage(0);
    },
    setActive(on) {
      active = on;
      if (!on) dropKeyboard();
      syncHold();
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
  const resume = buildResumeCopy(actions);
  const pager = buildPager(actions);
  const row = h("div", { class: "actions sess-actions" },
    ...sessionCardButtons("error", actions),
    resume.el,
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
      resume.sync(subject, "error");
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
  const files = buildTurnFiles(actions);
  const pager = buildPager(actions);
  const row = h("div", { class: "actions sess-actions" },
    ...sessionCardButtons("finished", actions),
    h("div", { class: "grow" }),
    pager.el,
  );
  const el = h("div", { class: "view" }, card("green", stack(116, 16, who, title, files.el, note, row)));
  return {
    el,
    sync() {
      const subject = cardSubject();
      clear(who);
      if (subject) who.append(subjectWho(subject, t("phase.finished")));
      title.textContent = outcomeSummary(subject, "finished") || t("finished.fallback");
      // The summary comes with the live row; the alert that opened the card has none.
      files.sync(subject && !("alert" in subject) ? subject : null);
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
  map.set("overview", buildHome(actions));
  map.set("empty", buildEmpty(actions));
  map.set("approval", buildApproval(actions));
  map.set("question", buildQuestion(actions, onChatHeightChange));
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
  // The CLI chat exists only while its flag is on: no view is built, nothing listens, nothing runs.
  if (CLI_CHAT_ENABLED) map.set("agentchat", buildAgentChat(actions));
  return map;
}
