// Agent sessions views: the project-grouped list ("sessions"), the read-only
// waiting card ("waiting") and the details card of any session ("session"),
// plus the pieces the finished / error cards share with them. Everything shown
// comes from the Rust snapshot (spec §10). The only value computed here is the
// elapsed time, rewritten once a second while a view is really on screen and
// never through State.notify(), so the frame loop stays asleep.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { washRGBA, type Wash } from "../core/layout";
import { Sessions, splitIdle } from "../core/sessions";
import { State, type CardSubject } from "../core/state";
import type { ContextFill, Host, ProjectView, SessionView, SilentReason } from "../core/bridge";
import { percentTone } from "./usage-format";
import { contextPercent, contextTip, formatTokens } from "./context-tip";
import { SVG_ICONS } from "./app-icon-art";
import { hostIconId } from "./host-icon";
import { t, type Key } from "../core/i18n";
import { keepSelection, selectionAfterPointer } from "../core/island-keys";
import type { ViewActions, ViewHost } from "./views";

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
 * press on it is a press on the row. Its tooltip gives the numbers and the model (contextTip).
 */
function contextEl(c: ContextFill | undefined, model?: string): HTMLElement | null {
  if (!c) return null;
  const label = contextLabel(c);
  const el = h(
    "span",
    { class: label.tone === "dim" ? "sess-ctx" : `sess-ctx ${label.tone}`, title: contextTip(c, model, t) },
    label.text,
  );
  if (label.percent !== null) el.prepend(contextRing(label.percent));
  return el;
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

export function buildSessions(actions: ViewActions): ViewHost {
  const list = h("div", { class: "sess-list" });
  const el = h("div", { class: "view" }, h("div", { class: "card sess-card" }, list));
  let clocks: Clock[] = [];
  let renderedKey = "";
  const runClock = clockRunner(() => clocks);

  // Rows act on mousedown: a snapshot can rebuild the list between the press
  // and the release, which would swallow a click.
  const onPress = (fn: () => void) => (e: Event) => {
    if ((e as MouseEvent).button === 0) fn();
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
    if (dim) cls.push("dim");
    if (s.attention) cls.push("attn");
    return h(
      "div",
      { class: cls.join(" "), "data-key": s.key, onmousedown: onPress(() => actions.openSession(s.key)) },
      agentBadge(s),
      h("span", { class: "sess-host", text: hostLabel(s.host) }),
      h("span", { class: "sess-phase", text: phaseLabel(s) }),
      h("span", { class: "sess-detail", text: rowDetail(s) }),
      contextEl(s.context, s.model),
      time,
    );
  }

  function projectHeader(project: ProjectView): HTMLElement {
    const hosts = [...new Set(project.sessions.map((s) => s.host))].filter((x) => x !== "unknown");
    return h(
      "div",
      { class: "sess-project", title: project.root },
      dot(colorForProject(project.key), 7),
      h("span", { class: "sess-pname", text: project.name }),
      ...hosts.map((x) => h("span", { class: "sess-hostlabel", text: hostLabel(x) })),
      // Sub-project 2 puts the project's app windows here.
      h("span", { class: "sess-apps" }),
    );
  }

  function render() {
    clear(list);
    clocks = [];
    const projects = (Sessions.snapshot?.projects ?? []).filter((p) => p.sessions.length > 0);
    if (projects.length === 0) {
      list.append(h("div", { class: "sess-empty", text: t("sess.empty") }));
      return;
    }
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
  }

  /** The rows Enter can open, top to bottom (a pure silent row opens nothing). */
  function rowEls(): HTMLElement[] {
    return [...list.querySelectorAll<HTMLElement>(".sess-row[data-key]")];
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
      const key = `${Sessions.snapshot?.rev ?? -1}|${[...Sessions.expandedIdle].join("|")}`;
      if (key !== renderedKey) {
        renderedKey = key;
        render();
      }
      highlight();
    },
    listKeys: () => rowEls().map((r) => r.dataset.key ?? ""),
    setActive(on: boolean) {
      runClock(on);
      // The "+N idle" folds close again once the list is left.
      if (!on && Sessions.expandedIdle.size > 0) Sessions.expandedIdle.clear();
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
    button(t("common.ok"), "primary", () => actions.ackAlert()),
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

// ── Session card ──────────────────────────────────────────────────────────────

export function buildSession(actions: ViewActions): ViewHost {
  const who = h("div");
  const phase = h("div", { class: "title" });
  const activity = h("div", { class: "sub sess-line" });
  const outcome = h("div", { class: "sub sess-line" });
  const since = h("span", { class: "sess-waited" });
  const row = h(
    "div",
    { class: "actions sess-actions" },
    button(t("common.openArrow"), "primary", () => actions.openSessionTarget()),
    since,
  );
  const el = h("div", { class: "view" }, cardEl("soft", stackEl(who, phase, activity, outcome, row)));
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
        return;
      }
      const s = subject.session;
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
      clocks.push({ el: since, since: s.since, line: (time) => t("session.for", { time }) });
      tickClocks(clocks);
    },
    setActive: runClock,
  };
}
