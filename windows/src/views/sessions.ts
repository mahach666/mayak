// Agent sessions views: the project-grouped list ("sessions"), the read-only
// waiting card ("waiting") and the details card of any session ("session"),
// plus the pieces the finished / error cards share with them. Everything shown
// comes from the Rust snapshot (spec §10). The only value computed here is the
// elapsed time, rewritten once a second while a view is really on screen and
// never through State.notify(), so the frame loop stays asleep.
// The project headers also carry their git state, read by Rust on request (core/git-state.ts).

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { washRGBA, type Wash } from "../core/layout";
import { Sessions, splitIdle } from "../core/sessions";
import { State, type CardSubject } from "../core/state";
import { Bridge, type ContextFill, type Host, type ProjectView, type RecentSession, type SessionMenuItem, type SessionView, type SilentReason } from "../core/bridge";
import { menuKeyAction, menuPlace, stepItem } from "../core/session-menu";
import { fileLabel, rowDiff, turnLine } from "./turn-summary";
import {
  closedRecent, copyCommand, findRecent, noteAfterResume, offersResumeCopy, recentDetail, recentEndWord, recentKey, resumeCommand, type RecentNote,
} from "../core/recent";
import { percentTone } from "./usage-format";
import { contextPercent, contextTip, formatTokens } from "./context-tip";
import { SVG_ICONS } from "./app-icon-art";
import { hostIconId } from "./host-icon";
import { t, currentLanguage, type Key } from "../core/i18n";
import { clockTime, nearAutoCompact } from "../core/context-alerts";
import { keepSelection, selectionAfterPointer } from "../core/island-keys";
import type { ViewActions, ViewHost } from "./views";
import { gitChip, gitTooltip, idleMarks, projectsToRefresh, type GitState } from "../core/git-state";
import { cardButtons, type SessionCardView } from "../core/card-buttons";

// ── Labels ────────────────────────────────────────────────────────────────────

/** The host's short name: app names as they are, "Other" translated. */
export function hostLabel(host: Host): string {
  switch (host) {
    case "claudeDesktop":
      return t("host.desktop");
    case "cursor":
      return "Cursor";
    case "vsCode":
      return "VS Code";
    case "codexDesktop":
      return t("host.codexApp");
    case "antigravityIde":
      return "Antigravity";
    case "antigravityCli":
      return "agy";
    case "windowsTerminal":
      return "Windows Terminal";
    case "powerShell":
      return "PowerShell";
    case "commandPrompt":
      return t("host.commandPrompt");
    case "gitBash":
      return "Git Bash";
    case "wezTerm":
      return "WezTerm";
    case "alacritty":
      return "Alacritty";
    case "tabby":
      return "Tabby";
    case "hyper":
      return "Hyper";
    case "conEmu":
      return "ConEmu";
    case "warp":
      return "Warp";
    case "unknown":
      return t("host.other");
  }
}

const AGENT_NAME = { claude: "Claude", codex: "Codex", antigravity: "Antigravity" } as const;
const AGENT_GLYPH = { claude: "C", codex: "X", antigravity: "A" } as const;

/** A row's badge: its host's app icon (ideas §41), or the agent's letter when the host has none. */
function agentBadge(s: SessionView): HTMLElement {
  const id = hostIconId(s.host);
  if (id !== null) return h("span", { class: "sess-agent sess-hosticon", title: AGENT_NAME[s.agent], html: SVG_ICONS[id] });
  return h("span", { class: "sess-agent", title: AGENT_NAME[s.agent], text: AGENT_GLYPH[s.agent] });
}

/** "Agent · Host", or the agent alone when the host is the agent's own app (Antigravity's IDE). */
export function agentHostLabel(agent: SessionView["agent"], host: Host): string {
  const name = AGENT_NAME[agent];
  const where = hostLabel(host);
  return where === name ? name : `${name} · ${where}`;
}

/** Spec §10 project dot (D60): FNV-1a 32 of the project key into a fixed palette. */
const PROJECT_PALETTE = [
  "#38BDF8", "#F29B38", "#7C5CFF", "#2EC4A0", "#EC4899", "#EAB308", "#FF5A4E", "#60A5FA",
];

export function colorForProject(projectKey: string): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < projectKey.length; i++) {
    hash ^= projectKey.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return PROJECT_PALETTE[hash % PROJECT_PALETTE.length];
}

/** "<60s" → "Ns"; "<60m" → "Nm"; else "Hh MMm" (in the island's language). */
export function formatElapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return t("unit.s", { n: s });
  const m = Math.floor(s / 60);
  if (m < 60) return t("unit.m", { n: m });
  return t("unit.hm", { h: Math.floor(m / 60), m: String(m % 60).padStart(2, "0") });
}

/** Texts for a Codex session that writes its rollout while no hook arrives (silent-codex spec §3.1, §4). */
export function silentText(reason: SilentReason): { short: string; help: string } {
  switch (reason) {
    case "hooksNotInstalled":
      return { short: t("silent.notInstalledShort"), help: t("silent.notInstalledHelp") };
    case "folderMissing":
      return { short: t("silent.folderShort"), help: t("silent.folderHelp") };
    case "hooksNotReaching":
      return { short: t("silent.reachShort"), help: t("silent.reachHelp") };
  }
}

export { formatTokens };

export type ContextTone = "dim" | "warn" | "full";

/**
 * The context label of a row (context-fill spec §4). With a window: floor(used / window × 100),
 * at most 100, computed on integers (0.29 * 100 is 28.999… in floating point); grey below 80,
 * "warn" from 80, "full" from 95. Without one: the token count, grey, no bar.
 */
export function contextLabel(c: ContextFill): { text: string; percent: number | null; tone: ContextTone } {
  const percent = contextPercent(c);
  if (percent !== null) {
    const tone: ContextTone = percentTone(percent);
    return { text: `${percent}%`, percent, tone };
  }
  return { text: formatTokens(c.used), percent: null, tone: "dim" };
}

const OUTCOME_WORD: Record<NonNullable<SessionView["outcome"]>["kind"], Key> = {
  finished: "phase.finished",
  error: "phase.error",
  interrupted: "phase.interrupted",
};

/** Short phase word of a row. A label from Rust is shown as it is. */
export function phaseLabel(s: SessionView): string {
  switch (s.phase.kind) {
    case "working":
      return t("phase.working");
    case "waiting":
      if (s.phase.wait === "permission") return t("phase.needsPermission");
      if (s.phase.wait === "input") return t("phase.asks");
      return s.phase.label || t("phase.waiting");
    case "idle":
      return t(s.outcome ? OUTCOME_WORD[s.outcome.kind] : "phase.idle");
    case "unknown":
      return s.phase.label || t("phase.unknown");
  }
}

/** Colour family of the phase word. */
function tone(s: SessionView): "amber" | "red" | "green" | "blue" | "dim" {
  if (s.phase.kind === "waiting") return "amber";
  if (s.phase.kind === "working") return "blue";
  if (s.phase.kind === "idle" && s.outcome?.kind === "error") return "red";
  if (s.phase.kind === "idle" && s.outcome?.kind === "finished") return "green";
  return "dim";
}

/** What the row's detail column says. */
function rowDetail(s: SessionView): string {
  switch (s.phase.kind) {
    case "waiting":
      return s.detail ?? s.title ?? "";
    case "working":
      return s.activity ?? s.detail ?? s.title ?? "";
    case "idle":
      return s.outcome?.summary || s.title || "";
    case "unknown":
      return s.title ?? "";
  }
}

/**
 * The row's context label, placed before its time; null without a context. It has no handler: a
 * press on it is a press on the row. Its tooltip gives the numbers and the model (contextTip), then
 * `extra` (the row's own tooltip, which the label's hides).
 */
function contextEl(c: ContextFill | undefined, model?: string, extra?: string | null): HTMLElement | null {
  if (!c) return null;
  const label = contextLabel(c);
  const tip = contextTip(c, model, t);
  const el = h(
    "span",
    { class: label.tone === "dim" ? "sess-ctx" : `sess-ctx ${label.tone}`, title: extra ? `${tip}
${extra}` : tip },
    label.text,
  );
  if (label.percent !== null) el.prepend(contextRing(label.percent));
  return el;
}

/** "compacted 14:02" before the context label, after a compaction (ideas §6); null without one. */
function compactedEl(s: SessionView): HTMLElement | null {
  if (!s.compacted) return null;
  return h("span", { class: "sess-ctx", text: t("ctx.compacted", { time: clockTime(s.compacted.at, currentLanguage()) }) });
}

/**
 * The row's tooltip, only for what the context labels shorten (ideas §6): "context 87%, auto-compaction
 * soon" at 85 % and up, and the full compaction mark "compacted 14:02 · 967k→27k". Null when neither.
 */
function contextTitle(s: SessionView): string | null {
  const lines: string[] = [];
  if (s.context && nearAutoCompact(s.context)) lines.push(t("ctx.nearAuto", { percent: contextPercent(s.context) ?? 0 }));
  const c = s.compacted;
  if (c) {
    const time = clockTime(c.at, currentLanguage());
    lines.push(c.post !== undefined
      ? t("ctx.compactedFull", { time, pre: formatTokens(c.pre), post: formatTokens(c.post) })
      : t("ctx.compacted", { time }));
  }
  return lines.length > 0 ? lines.join("\n") : null;
}

/** The "+N −M" of a row whose finished turn changed files (ideas §3); null otherwise. Never clickable. */
function diffEl(s: SessionView): HTMLElement | null {
  if (s.phase.kind !== "idle") return null;
  const text = rowDiff(s.turn);
  return text ? h("span", { class: "sess-diff", text }) : null;
}

const RING_R = 4.5;
const RING_C = 2 * Math.PI * RING_R;

/** A 12 px ring filled clockwise from the top to `percent`, in the label's colour. Also the home page's limits (views/usage.ts). */
export function contextRing(percent: number): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const ring = document.createElementNS(ns, "svg");
  ring.setAttribute("class", "sess-ctx-ring");
  ring.setAttribute("viewBox", "0 0 12 12");
  const track = document.createElementNS(ns, "circle");
  const fill = document.createElementNS(ns, "circle");
  for (const c of [track, fill]) {
    c.setAttribute("cx", "6");
    c.setAttribute("cy", "6");
    c.setAttribute("r", String(RING_R));
  }
  track.setAttribute("class", "track");
  fill.setAttribute("class", "fill");
  fill.setAttribute("stroke-dasharray", `${(RING_C * percent) / 100} ${RING_C}`);
  // At 0 % a round cap would still draw a dot: the track alone.
  ring.append(track);
  if (percent > 0) ring.append(fill);
  return ring;
}

// ── Shared pieces ─────────────────────────────────────────────────────────────

/** Elapsed-time text nodes of one view, rewritten by its 1 s clock. */
interface Clock {
  el: HTMLElement;
  since: number;
  /** The line around the elapsed time ("waiting 5m"); the time alone without it. */
  line?: (time: string) => string;
}

function tickClocks(clocks: Clock[]) {
  const now = Date.now();
  for (const c of clocks) {
    const time = formatElapsed(now - c.since);
    c.el.textContent = c.line ? c.line(time) : time;
  }
}

/** One 1 s interval per view, alive only while the view is active (D57). */
function clockRunner(clocks: () => Clock[]): (on: boolean) => void {
  let timer: number | null = null;
  return (on) => {
    if (on && timer == null) {
      tickClocks(clocks());
      timer = window.setInterval(() => tickClocks(clocks()), 1000);
    } else if (!on && timer != null) {
      window.clearInterval(timer);
      timer = null;
    }
  };
}

function cardEl(wash: Wash, ...children: Node[]): HTMLElement {
  const el = h("div", { class: wash ? "card wash" : "card" }, ...children);
  if (wash) el.style.setProperty("--wash", washRGBA(wash));
  return el;
}

function stackEl(...children: Node[]): HTMLElement {
  const el = h("div", { class: "stack" }, ...children);
  el.style.padding = "4px 16px 4px 116px";
  return el;
}

function button(label: string, kind: "primary" | "secondary", onClick: () => void): HTMLElement {
  return h("button", { class: `btn ${kind}`, onclick: onClick }, h("span", { text: label }));
}

/**
 * A session card's «↗ Open» and «OK» (core/card-buttons.ts), the first one primary. Open raises the
 * session's terminal or editor and never answers anything; OK acknowledges the alert.
 */
export function sessionCardButtons(view: SessionCardView, actions: ViewActions): HTMLElement[] {
  return cardButtons(view).map((b, i) => {
    const kind = i === 0 ? "primary" : "secondary";
    return b === "open"
      ? button(t("common.openArrow"), kind, () => actions.openSessionTarget())
      : button(t("common.ok"), kind, () => actions.ackAlert());
  });
}

/** Coloured project dot, project name, "Agent · Host", then an optional verb. */
export function sessionWho(
  found: { session: SessionView; project: ProjectView },
  verb?: string,
): HTMLElement {
  const { session, project } = found;
  return h(
    "div",
    { class: "who-row" },
    dot(colorForProject(project.key), 8),
    h("span", { class: "n", text: project.name }),
    h("span", { text: agentHostLabel(session.agent, session.host) }),
    verb ? h("span", { class: "who-verb", text: verb }) : null,
  );
}

/**
 * What the card of State.alertKey shows: the alert that opened it until the next
 * snapshot is in (spec §9 sends the alert first), else the live row, else the
 * row that was clicked when its ack removed the session (spec §7).
 */
export function cardSubject(): CardSubject | null {
  const key = State.alertKey;
  if (!key) return null;
  const fallback = State.alertFallback;
  if (fallback && "alert" in fallback && fallback.alert.key === key) return fallback;
  const found = Sessions.find(key);
  if (found) return found;
  if (fallback && "session" in fallback && fallback.session.key === key) return fallback;
  return null;
}

/** sessionWho for a card subject. An alert names the project but has no project key: no dot. */
export function subjectWho(subject: CardSubject, verb?: string): HTMLElement {
  if (!("alert" in subject)) return sessionWho(subject, verb);
  const a = subject.alert;
  return h(
    "div",
    { class: "who-row" },
    h("span", { class: "n", text: a.project }),
    h("span", { text: agentHostLabel(a.agent, a.host) }),
    verb ? h("span", { class: "who-verb", text: verb }) : null,
  );
}

/**
 * Summary of a finished / error turn: the row's outcome, or the alert's body
 * (Alert.detail carries the summary / error summary for those kinds).
 */
export function outcomeSummary(subject: CardSubject | null, kind: "finished" | "error"): string {
  if (!subject) return "";
  if ("alert" in subject) return subject.alert.kind === kind ? subject.alert.detail ?? "" : "";
  const o = subject.session.outcome;
  return o?.kind === kind ? o.summary : "";
}

/**
 * The files line of a card (ideas §3 «Итог хода»): "Files: 4 · +120 −34 · 6m", then one chip per
 * changed file ("lib.rs +12 −3", its project-relative path as the tooltip) that opens it in the
 * editor at its first change. Scrolls sideways when the chips do not fit. Hidden without a summary.
 */
export function buildTurnFiles(actions: ViewActions): { el: HTMLElement; sync(found: { session: SessionView; project: ProjectView } | null): void } {
  const summary = h("span", { class: "sess-turn" });
  const el = h("div", { class: "sess-files" }, summary);
  el.addEventListener("wheel", (e) => {
    if (e.deltaY === 0 || el.scrollWidth <= el.clientWidth) return;
    el.scrollLeft += e.deltaY;
    e.preventDefault();
  }, { passive: false });
  let shown = "";
  return {
    el,
    sync(found) {
      const turn = found?.session.turn;
      const line = turnLine(turn, t, formatElapsed);
      el.style.display = line ? "" : "none";
      // Rebuilt only when the summary changes, so a pending "no editor" note and the scroll stay.
      const sig = found && turn ? `${found.session.key}|${line}|${turn.files.map((f) => f.path).join("|")}` : "";
      if (sig === shown) return;
      shown = sig;
      while (el.lastChild !== summary) el.lastChild?.remove();
      el.scrollLeft = 0;
      summary.textContent = line;
      if (!found || !turn) return;
      const key = found.session.key;
      turn.files.forEach((f, index) => {
        const label = fileLabel(f.path, found.project.root);
        el.append(h(
          "button",
          {
            class: "sess-file",
            title: label.relative,
            onclick: () => {
              void actions.openTurnFile(key, index, f.path).then((ok) => {
                if (!ok) summary.textContent = `${line} · ${t("turn.noEditor")}`;
              });
            },
          },
          h("span", { text: label.name }),
          f.added > 0 ? h("span", { class: "add", text: `+${f.added}` }) : null,
          f.removed > 0 ? h("span", { class: "del", text: `\u2212${f.removed}` }) : null,
        ));
      });
    },
  };
}

/** ‹ n/m › over snapshot.attention (unacked episodes only). */
export function buildPager(actions: ViewActions): { el: HTMLElement; sync(): void } {
  const label = h("span", { class: "pager-label" });
  const el = h(
    "div",
    { class: "pager" },
    h("button", { class: "pager-btn", title: t("common.previous"), onclick: () => actions.pageAlert(-1) },
      svg(ICONS.chevronLeft, 9, { stroke: 2.4 })),
    label,
    h("button", { class: "pager-btn", title: t("common.next"), onclick: () => actions.pageAlert(1) },
      svg(ICONS.chevronRight, 9, { stroke: 2.4 })),
  );
  return {
    el,
    sync() {
      const list = Sessions.snapshot?.attention ?? [];
      const idx = State.alertKey ? list.indexOf(State.alertKey) : -1;
      const others = idx >= 0 ? list.length - 1 : list.length;
      el.style.display = others > 0 ? "" : "none";
      label.textContent = idx >= 0 ? `${idx + 1}/${list.length}` : t("pager.more", { n: list.length });
    },
  };
}

// ── Sessions list ─────────────────────────────────────────────────────────────

const MENU_LABEL: Record<SessionMenuItem, Key> = {
  copyReply: "sessMenu.copyReply",
  copyPath: "sessMenu.copyPath",
  copyResume: "sessMenu.copyResume",
  openFolder: "sessMenu.openFolder",
};

/** How long a menu item shows "Copied" or its failure before the menu closes. */
const MENU_FEEDBACK_MS = 900;

export function buildSessions(actions: ViewActions): ViewHost {
  const list = h("div", { class: "sess-list" });
  const card = h("div", { class: "card sess-card" }, list);
  const el = h("div", { class: "view" }, card);
  let clocks: Clock[] = [];
  let renderedKey = "";
  let active = false;
  const runClock = clockRunner(() => clocks);

  // ── Row menu (idea #8): right-click on a row, or the menu key / Shift+F10 on the highlighted one.

  let menu: HTMLElement | null = null;
  let menuItems: HTMLElement[] = [];
  let menuIndex = -1;
  /** The session the menu is for, from the request until it closes (the items come from Rust). */
  let menuKey: string | null = null;
  let menuRequest = 0;
  let menuTimer: number | undefined;
  /** An item runs: a second press does nothing until it settles. */
  let menuBusy = false;

  function closeMenu() {
    menuRequest++;
    window.clearTimeout(menuTimer);
    menuTimer = undefined;
    menuBusy = false;
    menu?.remove();
    menu = null;
    menuItems = [];
    menuIndex = -1;
    menuKey = null;
  }

  function showMenuIndex() {
    menuItems.forEach((b, i) => b.classList.toggle("selected", i === menuIndex));
  }

  /** The menu of `key`, its corner at `point` (client coordinates); `keyboard` highlights its first item. */
  function openMenu(key: string, point: { x: number; y: number }, keyboard: boolean) {
    closeMenu();
    menuKey = key;
    const mine = menuRequest;
    void Bridge.sessionMenu(key).then((items) => {
      if (mine !== menuRequest || !active || !items || items.length === 0) {
        if (mine === menuRequest) closeMenu();
        return;
      }
      menuItems = items.map((item) => {
        const b = h("button", {
          class: "apps-menu-item",
          role: "menuitem",
          text: t(MENU_LABEL[item]),
          onclick: (e: Event) => {
            e.stopPropagation();
            runMenuItem(item, b);
          },
        });
        b.addEventListener("mouseenter", () => {
          menuIndex = menuItems.indexOf(b);
          showMenuIndex();
        });
        return b;
      });
      menu = h("div", { class: "apps-menu sess-menu", role: "menu" }, ...menuItems);
      card.append(menu);
      const area = card.getBoundingClientRect();
      const place = menuPlace(
        { x: point.x - area.left, y: point.y - area.top },
        { w: menu.offsetWidth, h: menu.offsetHeight },
        { w: area.width, h: area.height },
      );
      menu.style.left = `${place.left}px`;
      menu.style.top = `${place.top}px`;
      menuIndex = keyboard ? 0 : -1;
      showMenuIndex();
    });
  }

  /** Runs one item; a copy says "Copied" (or why not) on the item for a moment, then the menu closes. */
  function runMenuItem(item: SessionMenuItem, button: HTMLElement) {
    const key = menuKey;
    if (!key || menuBusy) return;
    menuBusy = true;
    const mine = menuRequest;
    const settle = (ok: boolean) => {
      if (mine !== menuRequest) return;
      if (ok && item === "openFolder") {
        closeMenu();
        return;
      }
      button.textContent = t(ok ? "sessMenu.copied" : item === "openFolder" ? "sessMenu.folderFailed" : "sessMenu.copyFailed");
      menuTimer = window.setTimeout(closeMenu, MENU_FEEDBACK_MS);
    };
    void Bridge.sessionMenuRun(key, item).then((done) => {
      if (mine !== menuRequest) return;
      if (done?.kind === "copy") {
        navigator.clipboard.writeText(done.text).then(() => settle(true), () => settle(false));
      } else {
        settle(done?.kind === "done");
      }
    });
  }

  // Git state of each project (ideas #5), asked only while the list is on screen: when it comes on
  // screen, then for a project where a turn ended or that appeared (core/git-state.ts).
  const gitStates = new Map<string, GitState>();
  const gitEls = new Map<string, HTMLElement>();
  let gitActive = false;
  let gitMarks: Map<string, Set<string>> | null = null;
  let gitRev = -1;

  function paintGit(el: HTMLElement, state: GitState | undefined) {
    clear(el);
    if (!state) {
      el.style.display = "none";
      el.removeAttribute("title");
      return;
    }
    const chip = gitChip(state, t);
    el.style.display = "";
    el.title = gitTooltip(state, t);
    el.append(h("span", { class: "sess-git-ref", text: chip.ref }));
    if (chip.mark !== "clean") el.append(h("i", { class: `sess-git-mark ${chip.mark}` }));
    if (chip.sync) el.append(h("span", { text: chip.sync }));
    if (chip.op) el.append(h("span", { class: "sess-git-op", text: chip.op }));
  }

  function refreshGit(keys: string[]) {
    if (keys.length === 0) return;
    void Bridge.gitStates(keys).then((states) => {
      if (!states) return;
      for (const key of keys) {
        const state = states[key];
        if (state) gitStates.set(key, state);
        else gitStates.delete(key);
        const el = gitEls.get(key);
        if (el) paintGit(el, state);
      }
    });
  }

  // Any press outside the menu closes it; a right-click on another row then opens that one's.
  el.addEventListener("pointerdown", (e) => {
    if (menu && !menu.contains(e.target as Node)) closeMenu();
  });
  list.addEventListener("scroll", () => {
    if (menuKey) closeMenu();
  });

  // The menu's keys, before the island's own (Esc must close the menu, not fold the island).
  window.addEventListener(
    "keydown",
    (e) => {
      if (!active || State.mode !== "expanded" || State.view !== "sessions") return;
      const action = menuKeyAction(e, menuKey !== null);
      if (!action) return;
      switch (action.kind) {
        case "open": {
          const key = State.sessionSelection;
          const row = key ? rowEls().find((r) => r.dataset.key === key) : undefined;
          if (!key || !row) return;
          const r = row.getBoundingClientRect();
          openMenu(key, { x: r.left + 24, y: r.bottom }, true);
          break;
        }
        case "move":
          menuIndex = stepItem(menuIndex, menuItems.length, action.delta);
          showMenuIndex();
          break;
        case "run":
          if (menuIndex >= 0 && menuItems[menuIndex]) menuItems[menuIndex].click();
          break;
        case "close":
          closeMenu();
          break;
      }
      e.preventDefault();
      e.stopPropagation();
    },
    true,
  );

  // The context-menu key also sends a contextmenu event to the page: no browser menu over ours.
  window.addEventListener("contextmenu", (e) => {
    if (active && menuKey !== null && !(e.target instanceof Element && e.target.closest(".sess-row"))) e.preventDefault();
  });
  function shownProjects() {
    return (Sessions.snapshot?.projects ?? []).filter((p) => p.sessions.length > 0);
  }

  // Rows act on mousedown: a snapshot can rebuild the list between the press
  // and the release, which would swallow a click.
  const onPress = (fn: () => void) => (e: Event) => {
    if ((e as MouseEvent).button === 0) fn();
  };

  /** Right-click (or a two-finger tap) on a row: its menu at the pointer. */
  const onMenu = (key: string) => (e: Event) => {
    const m = e as MouseEvent;
    m.preventDefault();
    openMenu(key, { x: m.clientX, y: m.clientY }, false);
  };

  function row(s: SessionView, dim: boolean): HTMLElement {
    if (s.silent) {
      // A silent Codex session: muted, no clock. A pure `silent:<id>` row is no real session and opens
      // nothing; a known session (`codex:<id>`) whose turn ran without hooks keeps its click.
      const pure = s.key.startsWith("silent:");
      const cls = ["sess-row", `tone-${tone(s)}`, s.agent, "dim", "silent"];
      if (pure) cls.push("pure");
      return h(
        "div",
        {
          class: cls.join(" "),
          "data-key": pure ? undefined : s.key,
          onmousedown: pure ? undefined : onPress(() => actions.openSession(s.key)),
          oncontextmenu: pure ? undefined : onMenu(s.key),
        },
        agentBadge(s),
        h("span", { class: "sess-host", text: hostLabel(s.host) }),
        h("span", { class: "sess-phase", text: t("sess.noHooks") }),
        h("span", { class: "sess-detail", text: silentText(s.silent).short }),
        contextEl(s.context),
      );
    }
    const time = h("span", { class: "sess-time", text: formatElapsed(Date.now() - s.since) });
    clocks.push({ el: time, since: s.since });
    const cls = ["sess-row", `tone-${tone(s)}`, s.agent];
    const title = contextTitle(s);
    if (dim) cls.push("dim");
    if (s.attention) cls.push("attn");
    return h(
      "div",
      {
        class: cls.join(" "),
        "data-key": s.key,
        title: title ?? undefined,
        onmousedown: onPress(() => actions.openSession(s.key)),
        oncontextmenu: onMenu(s.key),
      },
      agentBadge(s),
      h("span", { class: "sess-host", text: hostLabel(s.host) }),
      h("span", { class: "sess-phase", text: phaseLabel(s) }),
      h("span", { class: "sess-detail", text: rowDetail(s) }),
      diffEl(s),
      compactedEl(s),
      contextEl(s.context, s.model, title),
      time,
    );
  }

  function projectHeader(project: ProjectView): HTMLElement {
    const hosts = [...new Set(project.sessions.map((s) => s.host))].filter((x) => x !== "unknown");
    const git = h("span", { class: "sess-git" });
    gitEls.set(project.key, git);
    paintGit(git, gitStates.get(project.key));
    return h(
      "div",
      { class: "sess-project", title: project.root },
      dot(colorForProject(project.key), 7),
      h("span", { class: "sess-pname", text: project.name }),
      ...hosts.map((x) => h("span", { class: "sess-hostlabel", text: hostLabel(x) })),
      git,
      // Sub-project 2 puts the project's app windows here.
      h("span", { class: "sess-apps" }),
    );
  }

  function render() {
    clear(list);
    clocks = [];
    gitEls.clear();
    const projects = shownProjects();
    if (projects.length === 0) list.append(h("div", { class: "sess-empty", text: t("sess.empty") }));
    for (const project of projects) {
      list.append(projectHeader(project));
      const { live, idle } = splitIdle(project);
      for (const s of live) list.append(row(s, false));
      if (Sessions.folded(project.key, idle.length)) {
        list.append(h("div", {
          class: "sess-fold",
          text: t("sess.idleFold", { n: idle.length }),
          onmousedown: onPress(() => actions.toggleIdleFold(project.key)),
        }));
      } else {
        for (const s of idle) list.append(row(s, true));
      }
    }
    renderRecent();
  }

  // ── «Recent» (ideas §7) ──

  /** The fold line's ↑ ↓ key: Enter opens or closes it. */
  const RECENT_FOLD_KEY = "recent:fold";

  const NOTE_TEXT: Record<RecentNote, (r: RecentSession) => string> = {
    copied: () => t("recent.copied"),
    noCli: (r) => t("recent.noCli", { cli: r.agent }),
    failed: () => t("recent.failed"),
  };

  function setNote(r: RecentSession, kind: RecentNote | null) {
    const recent = Sessions.recent;
    recent.note = kind ? { key: recentKey(r), kind } : null;
    recent.rev++;
    State.notify();
  }

  /** Resume: a console in the session's folder; the island folds once it started. */
  function resume(r: RecentSession) {
    actions.blip();
    void Bridge.recentResume(r).then((outcome) => {
      const note = noteAfterResume(outcome);
      if (note) setNote(r, note);
      else actions.collapse();
    });
  }

  function copy(r: RecentSession) {
    actions.blip();
    void Bridge.recentCopy(r).then((ok) => setNote(r, ok ? "copied" : "failed"));
  }

  function miniButton(label: string, title: string, fn: () => void): HTMLElement {
    return h("button", { class: "sess-mini", title, onmousedown: onPress(fn) }, label);
  }

  function recentRow(r: RecentSession): HTMLElement {
    const key = recentKey(r);
    const word = recentEndWord(r.end);
    const note = Sessions.recent.note?.key === key ? Sessions.recent.note.kind : null;
    const command = resumeCommand(r.agent, r.id);
    return h(
      "div",
      { class: `sess-row recent tone-${word?.tone ?? "dim"} ${r.agent}`, "data-key": key, title: r.cwd },
      h("span", { class: "sess-agent", title: AGENT_NAME[r.agent], text: AGENT_GLYPH[r.agent] }),
      h("span", { class: "sess-host", text: r.project }),
      word ? h("span", { class: "sess-phase", text: t(word.key) }) : null,
      h("span", { class: "sess-detail", text: note ? NOTE_TEXT[note](r) : recentDetail(r), title: r.last ?? "" }),
      h("span", { class: "sess-time", text: formatElapsed(Date.now() - r.at) }),
      miniButton(t("recent.resume"), command, () => resume(r)),
      miniButton(t("recent.copy"), `PowerShell: ${copyCommand(r.agent, r.id, r.cwd)}`, () => copy(r)),
    );
  }

  function renderRecent() {
    const recent = Sessions.recent;
    const chevron = svg(ICONS.chevronRight, 9, { stroke: 2.4 });
    chevron.classList.add("sess-recent-chev");
    list.append(h(
      "div",
      {
        class: recent.open ? "sess-fold sess-recent open" : "sess-fold sess-recent",
        "data-key": RECENT_FOLD_KEY,
        onmousedown: onPress(() => actions.toggleRecent()),
      },
      chevron,
      h("span", { text: t("recent.fold") }),
    ));
    if (!recent.open) return;
    if (recent.list === null) list.append(h("div", { class: "sess-empty", text: "…" }));
    else if (recent.list.length === 0) list.append(h("div", { class: "sess-empty", text: t("recent.none") }));
    else for (const r of recent.list) list.append(recentRow(r));
  }

  /** Enter on a recent row resumes it; false for any other key (a session row opens its host). */
  function activate(key: string): boolean {
    if (!key.startsWith("recent:")) return false;
    if (key === RECENT_FOLD_KEY) {
      actions.toggleRecent();
      return true;
    }
    const r = findRecent(Sessions.recent.list, key);
    if (r) resume(r);
    return true;
  }

  /** The rows Enter can open, top to bottom (a pure silent row opens nothing). */
  function rowEls(): HTMLElement[] {
    return [...list.querySelectorAll<HTMLElement>(".sess-row[data-key], .sess-recent[data-key]")];
  }

  let shownSelection: string | null = null;

  /** The ↑ ↓ highlight: kept by session key across rebuilds, dropped when its row is gone. */
  function highlight() {
    const rows = rowEls();
    const keys = rows.map((r) => r.dataset.key ?? "");
    State.sessionSelection = keepSelection(keys, State.sessionSelection);
    const at = State.sessionSelection == null ? -1 : keys.indexOf(State.sessionSelection);
    rows.forEach((r, i) => r.classList.toggle("selected", i === at));
    if (at >= 0 && State.sessionSelection !== shownSelection) rows[at].scrollIntoView({ block: "nearest" });
    shownSelection = State.sessionSelection;
  }

  // The mouse takes over from the keyboard: a real move over the list clears the ↑ ↓ highlight.
  list.addEventListener("mousemove", (e) => {
    if (State.sessionSelection == null) return;
    State.sessionSelection = selectionAfterPointer(State.sessionSelection, e.movementX, e.movementY);
    if (State.sessionSelection == null) highlight();
  });

  return {
    el,
    sync() {
      const key = `${Sessions.snapshot?.rev ?? -1}|${[...Sessions.expandedIdle].join("|")}|${Sessions.recent.rev}`;
      if (key !== renderedKey) {
        renderedKey = key;
        render();
        // The menu stays over a rebuilt list while its session is still there.
        if (menuKey && !rowEls().some((r) => r.dataset.key === menuKey)) closeMenu();
      }
      highlight();
      const rev = Sessions.snapshot?.rev ?? -1;
      if (gitActive && gitMarks && rev !== gitRev) {
        gitRev = rev;
        const next = idleMarks(shownProjects());
        refreshGit(projectsToRefresh(gitMarks, next));
        gitMarks = next;
      }
    },
    listKeys: () => rowEls().map((r) => r.dataset.key ?? ""),
    activate,
    setActive(on: boolean) {
      active = on;
      if (!on) closeMenu();
      runClock(on);
      gitActive = on;
      if (on) {
        const projects = shownProjects();
        gitMarks = idleMarks(projects);
        gitRev = Sessions.snapshot?.rev ?? -1;
        refreshGit(projects.map((p) => p.key));
      } else {
        gitMarks = null;
      }
      // The "+N idle" folds close again once the list is left.
      if (!on && Sessions.expandedIdle.size > 0) Sessions.expandedIdle.clear();
      // So does «Recent»: it is read again the next time it is opened.
      if (!on && Sessions.recent.open) Sessions.recent = { ...closedRecent(), rev: Sessions.recent.rev + 1 };
    },
  };
}

// ── Waiting card ──────────────────────────────────────────────────────────────

interface WaitFacts {
  wait: "permission" | "input" | "other";
  text: string | null;
  since: number;
  verb: string;
}

/** The waiting card's content, from the row or from the alert; null when nothing waits. */
function waitFacts(subject: CardSubject): WaitFacts | null {
  if ("alert" in subject) {
    const a = subject.alert;
    if (a.kind !== "permission" && a.kind !== "input") return null;
    const verb = t(a.kind === "permission" ? "phase.needsPermission" : "phase.asks");
    return { wait: a.kind, text: a.detail ?? a.title, since: a.since, verb };
  }
  const s = subject.session;
  if (s.phase.kind !== "waiting") return null;
  return { wait: s.phase.wait, text: s.detail ?? s.title, since: s.since, verb: phaseLabel(s) };
}

export function buildWaiting(actions: ViewActions): ViewHost {
  const who = h("div");
  const target = h("div", { class: "code" });
  const question = h("div", { class: "title sess-clamp" });
  const waited = h("span", { class: "sess-waited" });
  const pager = buildPager(actions);
  const row = h(
    "div",
    { class: "actions sess-actions" },
    ...sessionCardButtons("waiting", actions),
    waited,
    h("div", { class: "grow" }),
    pager.el,
  );
  const el = h("div", { class: "view" }, cardEl("amber", stackEl(who, target, question, row)));
  let clocks: Clock[] = [];
  const runClock = clockRunner(() => clocks);

  return {
    el,
    sync() {
      const subject = cardSubject();
      const w = subject ? waitFacts(subject) : null;
      clear(who);
      clocks = [];
      waited.textContent = "";
      target.style.display = "none";
      question.style.display = "";
      if (!subject) {
        question.textContent = "…";
      } else if (!w) {
        who.append(subjectWho(subject));
        question.textContent = t("wait.notWaiting");
      } else {
        who.append(subjectWho(subject, w.verb));
        if (w.wait === "permission") {
          // Read-only: the answer is given in the agent itself.
          target.style.display = "";
          target.textContent = w.text ?? "…";
          question.style.display = "none";
        } else {
          question.textContent = w.text ?? t("wait.needsInput");
        }
        clocks.push({ el: waited, since: w.since, line: (time) => t("wait.waitingFor", { time }) });
        tickClocks(clocks);
      }
      pager.sync();
    },
    setActive: runClock,
  };
}

// ── Copy resume (ideas §7) ────────────────────────────────────────────────────

/**
 * «Copy resume» on the session and error cards: the session's resume command, in its folder, onto
 * the clipboard (Rust builds it from the snapshot). Shown for a Claude or Codex subject whose turn
 * stopped short (core/recent.ts offersResumeCopy); after a press it says whether it copied, until
 * the card shows another session.
 */
export function buildResumeCopy(actions: ViewActions): { el: HTMLElement; sync(subject: CardSubject | null, outcome: string | null): void } {
  const label = h("span", { text: t("resume.copy") });
  let key: string | null = null;
  let pressed: { key: string; ok: boolean } | null = null;
  const el = h("button", {
    class: "btn secondary",
    onclick: () => {
      const at = key;
      if (!at) return;
      actions.blip();
      void Bridge.sessionResumeCopy(at).then((ok) => {
        pressed = { key: at, ok: ok === true };
        if (key === at) label.textContent = t(ok ? "recent.copied" : "recent.failed");
      });
    },
  }, label);
  return {
    el,
    sync(subject, outcome) {
      const who = subject ? ("alert" in subject ? subject.alert : subject.session) : null;
      // Only while the session is on the list: Rust takes its id and folder from the snapshot.
      key = who && offersResumeCopy(who.agent, outcome) && Sessions.find(who.key) ? who.key : null;
      el.style.display = key ? "" : "none";
      if (pressed && pressed.key !== key) pressed = null;
      label.textContent = pressed ? t(pressed.ok ? "recent.copied" : "recent.failed") : t("resume.copy");
    },
  };
}

// ── Session card ──────────────────────────────────────────────────────────────

export function buildSession(actions: ViewActions): ViewHost {
  const who = h("div");
  const phase = h("div", { class: "title" });
  const activity = h("div", { class: "sub sess-line" });
  const outcome = h("div", { class: "sub sess-line" });
  const files = buildTurnFiles(actions);
  const since = h("span", { class: "sess-waited" });
  const resume = buildResumeCopy(actions);
  const row = h(
    "div",
    { class: "actions sess-actions" },
    ...sessionCardButtons("session", actions),
    resume.el,
    since,
  );
  const el = h("div", { class: "view" }, cardEl("soft", stackEl(who, phase, activity, outcome, files.el, row)));
  let clocks: Clock[] = [];
  const runClock = clockRunner(() => clocks);

  return {
    el,
    sync() {
      const subject = cardSubject();
      clear(who);
      clocks = [];
      since.textContent = "";
      // Opened from a row; an alert subject belongs to the waiting / finished / error cards.
      if (!subject || "alert" in subject) {
        phase.textContent = "…";
        activity.textContent = "";
        outcome.style.display = "none";
        files.sync(null);
        resume.sync(null, null);
        return;
      }
      const s = subject.session;
      resume.sync(subject, s.outcome?.kind ?? null);
      who.append(sessionWho(subject));
      const word = phaseLabel(s);
      phase.textContent = word.charAt(0).toUpperCase() + word.slice(1);
      activity.textContent = s.activity ?? s.detail ?? s.title ?? "";
      const o = s.outcome;
      outcome.style.display = o ? "" : "none";
      if (o) {
        const head = t(o.kind === "finished" ? "outcome.finished" : o.kind === "error" ? "outcome.error" : "outcome.interrupted");
        outcome.textContent = o.summary ? t("outcome.line", { head, summary: o.summary }) : head;
      }
      files.sync(subject);
      clocks.push({ el: since, since: s.since, line: (time) => t("session.for", { time }) });
      tickClocks(clocks);
    },
    setActive: runClock,
  };
}
