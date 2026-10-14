// The Agents tab: a chat run by the user's own agent CLIs (src-tauri/src/chat), with no API key.
// A conversation is a tab of this view; the log shows what the CLI says (its answer as Markdown, the
// tools it runs as small chips) and every permission request it asks, as a card with its own Allow
// and Deny. Luch sits on the card's left, as in the island's other chats, and shows the run: it
// thinks while the CLI answers, works while a tool runs, waits for a click on a request.
// The composer holds the field and, on its bottom row, what the next message runs with: the agent
// (fixed once the chat has started), the access mode, the model, and the round send / stop button.
// Rust keeps the conversation: the view reads it again when told it changed (`agent-chat-changed`)
// and when it is shown. A request is answered only by a click on its button (agentChatPermission);
// nothing here has a timeout or a default.
//
// Off with the rest of the feature (core/features.ts CLI_CHAT_ENABLED): views.ts builds it only then.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { renderMarkdown } from "./markdown-view";
import { resetsText, usagePercent } from "./usage-format";
import { Bridge, onAgentChatChanged } from "../core/bridge";
import { Sessions } from "../core/sessions";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import { t } from "../core/i18n";
import { washRGBA, type BotStateName, type Wash } from "../core/layout";
import { clickCounts, type CardBinding } from "../island/answers";
import { defaultRoot, enterStep, keptRoot, projectChoices } from "./launcher-model";
import {
  agentCommand, canSend, chatBotState, chooseAgentKind, clockText, compactTokens, folderName, isBusy, keepMode, keepModel, loginCommand,
  messageSig, modeAccent, modeHintKey, modeKey, modeShortKey, normalizeError, reconcileActive, sideStatus, stateColor, tabTitle, takesWanted, toolLabel,
  turnTools,
  type AgentInfo, type AgentKind, type ChatError, type ChatMsg, type Convo, type ConvoSummary, type OpenRequest,
} from "../core/agent-chat";
import type { ViewActions, ViewHost } from "./views";

/** What the view asks of Rust; the tests and a dev preview pass their own. */
export interface AgentChatBackend {
  agents(refresh: boolean): Promise<AgentInfo[] | null>;
  list(): Promise<ConvoSummary[] | null>;
  get(conv: number, since: number | null): Promise<Convo | null>;
  open(request: OpenRequest): Promise<number>;
  set(conv: number, model: string, mode: string): Promise<void>;
  send(conv: number, text: string): Promise<void>;
  permission(requestId: string, allow: boolean): Promise<boolean | null>;
  cancel(conv: number): Promise<unknown>;
  close(conv: number): Promise<unknown>;
  onChanged(handler: (ids: number[]) => void): void;
  /** A link in an answer: Rust opens web addresses and nothing else. */
  openUrl(url: string): void;
}

const BRIDGE_BACKEND: AgentChatBackend = {
  agents: (refresh) => Bridge.agentChatAgents(refresh),
  list: () => Bridge.agentChatList(),
  get: (conv, since) => Bridge.agentChatGet(conv, since),
  open: (request) => Bridge.agentChatOpen(request),
  set: (conv, model, mode) => Bridge.agentChatSet(conv, model, mode),
  send: (conv, text) => Bridge.agentChatSend(conv, text),
  permission: (requestId, allow) => Bridge.agentChatPermission(requestId, allow),
  cancel: (conv) => Bridge.agentChatCancel(conv),
  close: (conv) => Bridge.agentChatClose(conv),
  onChanged: (handler) => void onAgentChatChanged(handler),
  openUrl: (url) => void Bridge.openUrl(url),
};

/**
 * The conversation the island was told to show (a request is waiting in it): read when the view
 * shows, or at once by the view when it is already on screen (`takeWanted`).
 */
let wanted: number | null = null;
let takeWanted: (() => void) | null = null;

/** island/agent-chat.ts and the session list: the conversation to show, now or when the view next shows. */
export function showAgentChat(conv: number): void {
  wanted = conv;
  takeWanted?.();
  State.notify();
}

/** The least height, in px, the log needs to be scrolled: a collapsed or still opening island has less. */
const LOG_ROOM = 40;

/** How long Luch looks pleased after a turn that ended well. */
const FINISHED_MS = 4000;

/** The field's height, in px: an empty chat gets a roomy box, a running one a single line; it grows to the max. */
const FIELD_ROOMY = 64;
const FIELD_LINE = 20;
const FIELD_MAX = 110;

/** The agent a new chat starts with, remembered while the app runs. */
let lastAgent: AgentKind | null = null;

/** A mode a newer Rust offers that this page has no word for shows as its id. */
function modeLabel(mode: string): string {
  const key = modeKey(mode);
  return key ? t(key) : mode;
}

function modeShortLabel(mode: string): string {
  const key = modeShortKey(mode);
  return key ? t(key) : mode;
}

/** What each mode shows in its pill, besides its colour. */
function modeIcon(mode: string): string {
  switch (mode) {
    case "plan": return ICONS.list;
    case "readOnly": return ICONS.doc;
    case "workspaceWrite": return ICONS.pencil;
    case "proposeOnly": return ICONS.star;
    default: return ICONS.bubble;
  }
}

/** The card's glow under the text, by what Luch shows (the capture's indigo wash while it thinks). */
const WASH: Record<BotStateName, Wash> = {
  idle: "indigo", working: "cyan", thinking: "indigo", searching: "indigo", approval: "amber", question: "amber",
  error: "red", finished: "green", ratelimit: "amber", sleeping: "indigo", dizzy: "indigo",
};

type MenuKind = "agent" | "mode" | "model";

const chevron = (size = 9): SVGSVGElement => {
  const el = svg(ICONS.chevronDown, size, { stroke: 2.6 });
  el.classList.add("ac-chev");
  return el;
};

export function buildAgentChat(actions: ViewActions, backend: AgentChatBackend = BRIDGE_BACKEND): ViewHost {
  void actions;
  const hooks = {
    openUrl: (url: string) => backend.openUrl(url),
    copy: (text: string) => navigator.clipboard.writeText(text),
  };

  // The header: the chats of the open conversations, one chip each with a plus for a new one, and the
  // project. The project's list is a native one laid transparent over its chip: it opens with the full names.
  const tabs = h("div", { class: "ac-tabs" });
  const projectSelect = h("select", { class: "ac-sel", title: t("chat.project") });
  const projectText = h("span", { class: "ac-pill-text" });
  const projectPill = h("div", { class: "ac-pill ac-proj" }, svg(ICONS.folder, 12), projectText, chevron(), projectSelect);
  const head = h("div", { class: "ac-head" }, tabs, projectPill);

  const log = h("div", { class: "chat-log ac-log" });
  const errorBox = h("div", { class: "ac-flash" });

  // The composer: the field, and under it what the next message runs with.
  const input = h("textarea", { class: "chat-input", rows: 1, spellcheck: "false" });
  const agentChip = h("button", { class: "ac-cb ac-agent", title: t("chat.agent") });
  const modeChip = h("button", { class: "ac-cb ac-mode", title: t("agentchat.mode") });
  const modelChip = h("button", { class: "ac-cb ac-model", title: t("agentchat.model") });
  const action = h("button", { class: "send-btn", title: t("chat.send") }, svg(ICONS.arrowUp, 11));
  const row = h("div", { class: "ac-crow" }, agentChip, modeChip, modelChip, h("div", { class: "grow" }), action);
  const composer = h("div", { class: "ac-composer" }, input, row);
  // The list a chip opens, over the log.
  const menu = h("div", { class: "ac-menu" });
  menu.hidden = true;

  // The column under Luch: what the agent is doing and for how long, the tools of the turn, and what
  // the CLI said of the conversation's size and of the plan's windows. Only what the CLI reports.
  const sideLine = h("div", { class: "ac-side-status" });
  const sideTime = h("div", { class: "ac-side-time" });
  const sideTools = h("div", { class: "ac-side-tools", title: t("agentchat.side.tools") });
  const sideMeters = h("div", { class: "ac-side-meters" });
  const side = h("div", { class: "ac-side" }, sideLine, sideTime, sideTools, sideMeters);

  const body = h("div", { class: "ac-body" }, side, head, log, errorBox, composer, menu);
  const card = h("div", { class: "card wash chat-card ac-card" }, body);
  const el = h("div", { class: "view" }, card);

  let agents: AgentInfo[] | null = null;
  let convos: ConvoSummary[] = [];
  /** The open conversation; null is a new chat (the chips say what it will be). */
  let active: number | null = null;
  let current: Convo | null = null;
  const draft = { agent: null as AgentKind | null, root: null as string | null, model: "", mode: "" };
  let error: ChatError | null = null;
  let sending = false;
  let isOn = false;
  /** The chip whose list is open. */
  let menuOpen: MenuKind | null = null;
  /** Enter went down in the field; the message goes on its release (enterStep). */
  let armed = false;
  /** When the buttons of each request were first drawn: a click right after is ignored (answers.ts). */
  const builtAt = new Map<string, number>();
  const rendered = new Map<number, { sig: string; el: HTMLElement }>();
  let renderedFor: number | null = null;
  /** The log must end at its last line the next time it has room (it was shown, or grew, without any). */
  let wantEnd = true;
  /** Until when Luch looks pleased about a turn that just ended (performance.now()). */
  let finishedUntil = 0;
  /** This view put its state on Luch (State.stateOverride) and takes it off again. */
  let botMine = false;
  /** The column's clock while a turn runs: one interval, only while the view is on screen. */
  let tick: number | null = null;
  let metersSig = "";
  // The island opens with a transition: lines that arrived while it was collapsed are drawn when the log
  // still has no room, and nothing else would scroll it to its end once it has.
  new ResizeObserver(() => {
    if (wantEnd && log.clientHeight >= LOG_ROOM) {
      log.scrollTop = log.scrollHeight;
      wantEnd = false;
    }
  }).observe(log);
  const typing = h("div", { class: "chat-row ac-tail" }, h("div", { class: "typing" }, h("i"), h("i"), h("i")));
  const statusLine = h("div", { class: "ac-note ac-status" });
  typing.append(statusLine);
  let tabsSig = "";
  let ctxSig = "";
  let menuSig = "";

  const agentOf = (kind: AgentKind | null | undefined): AgentInfo | undefined => agents?.find((a) => a.kind === kind);
  const viewKind = (): AgentKind | null =>
    active !== null ? current?.agent ?? convos.find((c) => c.id === active)?.agent ?? null : draft.agent;
  const viewInfo = (): AgentInfo | undefined => agentOf(viewKind());
  const changed = () => State.notify();

  // ── Reading Rust ──

  async function refreshAgents(refresh = false) {
    const list = await backend.agents(refresh);
    if (list) {
      agents = list;
      if (draft.agent === null || !agentOf(draft.agent)?.found) {
        draft.agent = chooseAgentKind(list, lastAgent);
        const info = agentOf(draft.agent);
        draft.model = keepModel(info, draft.model);
        draft.mode = keepMode(info, draft.mode);
      }
    }
    changed();
  }

  async function refreshList() {
    const list = await backend.list();
    if (!list) return;
    convos = list;
    const kept = reconcileActive(list, active);
    if (kept !== active) {
      active = kept;
      current = null;
    }
    changed();
  }

  async function refreshCurrent() {
    if (active === null) return;
    const id = active;
    const before = current?.id === id ? current.state : null;
    const next = await backend.get(id, current?.id === id ? current.rev : null);
    if (next && active === id) {
      // A turn that went on and ended well: Luch is pleased for a moment, then settles.
      if ((before === "working" || before === "waiting") && next.state === "idle" && next.messages.some((m) => m.kind === "assistant")) {
        finishedUntil = performance.now() + FINISHED_MS;
        window.setTimeout(() => {
          if (isOn) changed();
        }, FINISHED_MS + 50);
      }
      current = next;
    }
    changed();
  }

  async function refreshAll() {
    await Promise.all([refreshList(), agents === null ? refreshAgents() : Promise.resolve()]);
    // The conversation the island was told to show. A message being typed keeps the view where it is
    // (Enter must not send it to another chat): the other chat's tab shows its request, and the view
    // goes there at a later show with an empty field. A conversation that is gone is forgotten.
    if (wanted !== null && !convos.some((c) => c.id === wanted)) wanted = null;
    if (wanted === active) wanted = null;
    if (wanted !== null && takesWanted(wanted, active, input.value)) {
      active = wanted;
      current = null;
      error = null;
      closeMenu();
      wanted = null;
    }
    await refreshCurrent();
  }

  // A request in another chat while this view is on screen: the view goes to it, unless a message
  // is being typed here (it would then be sent to the other chat); the request's tab shows it meanwhile.
  takeWanted = () => {
    if (!isOn || !takesWanted(wanted, active, input.value)) return;
    void refreshAll();
  };

  // While the view is not shown Rust does not even send these; one that comes anyway waits for the next show.
  backend.onChanged((ids) => {
    if (!isOn) return;
    void refreshList();
    if (active !== null && ids.includes(active)) void refreshCurrent();
  });

  // ── The user's moves ──

  function select(id: number | null) {
    if (active === id) return;
    active = id;
    current = null;
    error = null;
    closeMenu();
    changed();
    void refreshCurrent();
    input.focus();
  }

  async function submit() {
    const text = input.value.trim();
    const kind = viewKind();
    if (!canSend(text, current?.state, kind !== null, sending)) return;
    error = null;
    sending = true;
    closeMenu();
    changed();
    try {
      let id = active;
      if (id === null && kind !== null) {
        id = await backend.open({ agent: kind, cwd: draft.root ?? "", model: draft.model, mode: draft.mode, resume: null });
        lastAgent = kind;
        active = id;
        current = null;
        await refreshList();
      }
      if (id !== null) {
        await backend.send(id, text);
        input.value = "";
        resize();
        Sound.play("send");
      }
    } catch (err) {
      error = normalizeError(err);
      Sound.play("error");
    } finally {
      sending = false;
    }
    await refreshCurrent();
    await refreshList();
  }

  function stop() {
    if (active === null) return;
    void backend.cancel(active);
  }

  async function closeChat(id: number) {
    await backend.close(id);
    if (active === id) {
      active = null;
      current = null;
    }
    await refreshList();
  }

  async function answer(requestId: string, allow: boolean, bound: CardBinding) {
    if (!clickCounts(bound, requestId, performance.now())) return;
    await backend.permission(requestId, allow);
    await refreshCurrent();
  }

  async function choose(model: string, mode: string) {
    if (active === null) {
      draft.model = model;
      draft.mode = mode;
      changed();
      return;
    }
    try {
      await backend.set(active, model, mode);
      error = null;
    } catch (err) {
      error = normalizeError(err);
    }
    await refreshCurrent();
  }

  function pickAgent(kind: AgentKind) {
    if (active !== null || !agentOf(kind)?.found) return;
    draft.agent = kind;
    draft.model = keepModel(agentOf(kind), draft.model);
    draft.mode = keepMode(agentOf(kind), draft.mode);
    error = null;
  }

  projectSelect.addEventListener("change", () => {
    draft.root = projectSelect.value === "" ? null : projectSelect.value;
    changed();
    input.focus();
  });
  action.addEventListener("click", () => (isBusy(current?.state) ? stop() : void submit()));

  function resize() {
    input.style.height = "0px";
    const min = composer.classList.contains("roomy") ? FIELD_ROOMY : FIELD_LINE;
    input.style.height = `${Math.min(Math.max(input.scrollHeight, min), FIELD_MAX)}px`;
  }
  input.addEventListener("input", () => {
    resize();
    changed();
  });
  const onKey = (e: KeyboardEvent) => {
    const step = enterStep(
      { type: e.type === "keyup" ? "keyup" : "keydown", key: e.key, shiftKey: e.shiftKey, repeat: e.repeat, isComposing: e.isComposing },
      armed,
    );
    armed = step.armed;
    if (step.prevent) e.preventDefault();
    if (step.send) void submit();
    // Every key but Escape stays in the field; Escape goes on to the island, which folds (an open list takes it first).
    if (e.key !== "Escape") e.stopPropagation();
  };
  input.addEventListener("keydown", onKey);
  input.addEventListener("keyup", onKey);
  action.addEventListener("keydown", onKey);
  action.addEventListener("keyup", onKey);
  // A press anywhere on the composer's blank part is a press in the field.
  composer.addEventListener("pointerdown", (e) => {
    if (e.target === composer || e.target === row) {
      e.preventDefault();
      input.focus();
    }
  });

  // ── The lists the chips open ──

  const chips: Record<MenuKind, HTMLElement> = { agent: agentChip, mode: modeChip, model: modelChip };

  function openMenu(kind: MenuKind) {
    if (isBusy(current?.state) || !viewInfo() || (kind === "agent" && active !== null)) return;
    menuOpen = menuOpen === kind ? null : kind;
    menuSig = "";
    changed();
  }
  function closeMenu() {
    if (menuOpen === null) return;
    menuOpen = null;
    changed();
  }
  for (const kind of Object.keys(chips) as MenuKind[]) chips[kind].addEventListener("click", () => openMenu(kind));
  // Esc closes the list and nothing else: the island folds on the next Esc (island.ts).
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || menuOpen === null) return;
    e.preventDefault();
    closeMenu();
    input.focus();
  }, true);
  // A press anywhere else lets it go.
  document.addEventListener("pointerdown", (e) => {
    if (menuOpen === null) return;
    const target = e.target as Node | null;
    if (target && (menu.contains(target) || Object.values(chips).some((c) => c.contains(target)))) return;
    closeMenu();
  }, true);

  // ── Drawing ──

  function errorText(e: ChatError): string {
    const info = viewInfo();
    switch (e.code) {
      case "cliMissing": {
        const kind = viewKind();
        return info && kind ? t("agentchat.error.cliMissing", { agent: info.label, cmd: agentCommand(kind) }) : e.message;
      }
      case "badFolder": return t("agentchat.error.folder");
      case "busy": return t("agentchat.error.busy");
      case "badPrompt":
      case "promptTooLong":
      case "promptNotPassable": return t("agentchat.error.prompt");
      default: return e.message;
    }
  }

  /** What an agent says of itself in the list: not installed, not signed in, or never run here. */
  function agentNote(a: AgentInfo): string {
    if (!a.found) return t("agentchat.notInstalled");
    if (a.login === "notLoggedIn") return t("agentchat.signedOut");
    return a.verified ? "" : t("agentchat.unverified");
  }

  function syncTabs() {
    const sig = `${active}|${convos.map((c) => `${c.id}:${c.state}:${c.title}`).join(",")}`;
    if (sig === tabsSig) return;
    tabsSig = sig;
    tabs.hidden = convos.length === 0;
    head.classList.toggle("solo", convos.length === 0);
    clear(tabs);
    for (const c of convos) {
      const close = h("button", { class: "chip-x", title: t("agentchat.closeChat"), onclick: (e: Event) => {
        e.stopPropagation();
        void closeChat(c.id);
      } }, svg(ICONS.xmark, 9));
      tabs.append(
        h("div", { class: `ac-chip${c.id === active ? " on" : ""}`, onclick: () => select(c.id) },
          dot(stateColor(c.state), 7),
          h("span", { class: "ac-chip-title", text: tabTitle(c, t("agentchat.new")) }),
          close),
      );
    }
    tabs.append(h("div", { class: `ac-chip ac-plus${active === null ? " on" : ""}`, title: t("agentchat.new"), onclick: () => select(null) }, svg(ICONS.plus, 10)));
  }

  /** The project chip and the three chips of the composer. */
  function syncContext(busy: boolean) {
    const kind = viewKind();
    const locked = active !== null;
    const info = viewInfo();
    const roots = projectChoices(Sessions.snapshot);
    const chosenRoot = locked ? current?.cwd ?? null : keptRoot(draft.root, roots) ?? (draft.root === null ? defaultRoot(Sessions.snapshot) : null);
    if (!locked && draft.root === null && chosenRoot !== null && keptRoot(chosenRoot, roots) !== null) draft.root = chosenRoot;
    const model = locked ? current?.model ?? "" : draft.model;
    const mode = locked ? current?.mode ?? "" : draft.mode;
    const sig = JSON.stringify([
      agents?.map((a) => [a.kind, a.found, a.login, a.verified]), kind, locked, chosenRoot, roots.map((r) => r.root), model, mode, busy,
      info?.models.map((m) => m.label),
    ]);
    if (sig === ctxSig) return;
    ctxSig = sig;

    clear(projectSelect);
    projectSelect.append(h("option", { value: "", text: t("agentchat.home") }));
    for (const r of roots) projectSelect.append(h("option", { value: r.root, text: r.name, title: r.root }));
    const shown = locked ? current?.cwd ?? "" : draft.root ?? "";
    if (shown !== "" && !roots.some((r) => r.root === shown)) {
      projectSelect.append(h("option", { value: shown, text: folderName(shown), title: shown }));
    }
    projectSelect.value = shown;
    projectSelect.disabled = locked;
    projectPill.classList.toggle("locked", locked);
    projectPill.title = shown === "" ? "" : shown;
    projectText.textContent = shown === "" ? t("agentchat.home") : roots.find((r) => r.root === shown)?.name ?? folderName(shown);

    // The agent: a dot (green ready, amber signed out, violet never run here), its name, and a lock once the chat has started.
    agentChip.replaceChildren(
      dot(!info || !info.found ? "#6B7079" : info.login === "notLoggedIn" ? "#F5A524" : info.verified ? "#34D399" : "#A78BFA", 7),
      h("span", { class: "ac-cb-text", text: info?.label ?? (agents === null ? "…" : t("chat.noAgent")) }),
      locked ? svg(ICONS.lock, 11, { stroke: 2 }) : chevron());
    agentChip.title = locked ? t("agentchat.locked") : t("chat.agent");
    agentChip.disabled = !info || locked || busy;
    agentChip.classList.toggle("locked", locked);
    agentChip.classList.toggle("bare", !info);

    // The access mode, a coloured pill with its own mark.
    const accent = modeAccent(mode);
    modeChip.style.setProperty("--mc", accent.color);
    modeChip.style.setProperty("--mb", accent.ground);
    modeChip.replaceChildren(svg(modeIcon(mode), 12), h("span", { class: "ac-cb-text", text: modeShortLabel(mode) }), chevron());
    modeChip.title = `${t("agentchat.mode")}: ${modeLabel(mode)}`;
    modeChip.disabled = busy || !info;
    modeChip.hidden = !info || mode === "";

    // The model, as text.
    const choice = info?.models.find((m) => m.id === model);
    modelChip.replaceChildren(
      h("span", { class: "ac-cb-text", text: choice ? choice.label || t("agentchat.modelDefault") : model || t("agentchat.modelDefault") }),
      chevron());
    modelChip.disabled = busy || !info;
    modelChip.hidden = !info;
  }

  /** The open list: drawn when it opens or what it shows changes, and set above the composer. */
  function syncMenu() {
    menu.hidden = menuOpen === null;
    for (const [kind, chip] of Object.entries(chips)) chip.classList.toggle("open", menuOpen === kind);
    if (menuOpen === null) return;
    const info = viewInfo();
    const locked = active !== null;
    const model = locked ? current?.model ?? "" : draft.model;
    const mode = locked ? current?.mode ?? "" : draft.mode;
    const anchor = chips[menuOpen].getBoundingClientRect();
    const frame = body.getBoundingClientRect();
    const composerTop = composer.getBoundingClientRect().top;
    menu.style.left = `${Math.max(0, anchor.left - frame.left)}px`;
    menu.style.bottom = `${frame.bottom - composerTop + 6}px`;
    menu.style.maxHeight = `${Math.max(110, composerTop - frame.top - 14)}px`;
    const sig = JSON.stringify([menuOpen, agents?.map((a) => [a.kind, a.found, a.login, a.verified]), viewKind(), info?.models.map((m) => m.id + m.label), info?.modes, model, mode]);
    if (sig === menuSig || !info) return;
    menuSig = sig;
    const row = (label: string, hint: string | null, on: boolean, off: boolean, pick: () => void, accent?: string) =>
      h("button", { class: `ac-opt${on ? " on" : ""}`, disabled: off, onclick: pick },
        h("span", { class: "ac-opt-mark", style: accent ? `color:${accent}` : "" }, on ? svg(ICONS.check, 11, { stroke: 2.6 }) : null),
        h("span", { class: "ac-opt-text" }, h("span", { class: "ac-opt-label", text: label }), hint ? h("span", { class: "ac-opt-hint", text: hint }) : null));
    const rows: HTMLElement[] = [];
    if (menuOpen === "agent") {
      for (const a of agents ?? []) {
        rows.push(row(a.label, agentNote(a) || null, a.kind === viewKind(), !a.found, () => {
          pickAgent(a.kind);
          closeMenu();
          input.focus();
        }));
      }
    } else if (menuOpen === "mode") {
      for (const m of info.modes) {
        const hint = modeHintKey(m);
        rows.push(row(modeLabel(m), hint ? t(hint) : null, m === mode, false, () => {
          void choose(model, m);
          closeMenu();
          input.focus();
        }, modeAccent(m).color));
      }
    } else {
      for (const m of info.models) {
        rows.push(row(m.label === "" ? t("agentchat.modelDefault") : m.label, null, m.id === model, false, () => {
          void choose(m.id, mode);
          closeMenu();
          input.focus();
        }));
      }
    }
    menu.replaceChildren(h("div", { class: "ac-menu-title", text: menuOpen === "agent" ? t("chat.agent") : menuOpen === "mode" ? t("agentchat.mode") : t("agentchat.model") }), h("div", { class: "ac-menu-list" }, ...rows));
  }

  function toolLine(m: Extract<ChatMsg, { kind: "tool" }>): HTMLElement {
    const mark = m.status === "running"
      ? h("i", { class: "ac-tool-run" })
      : m.status === "ok" ? svg(ICONS.check, 10, { stroke: 2.8 }) : svg(ICONS.xmark, 8);
    const line = h("div", { class: `ac-tool ${m.status}` },
      h("span", { class: "ac-tool-mark" }, mark),
      h("span", { class: "ac-tool-name", text: toolLabel(m.name) }),
      m.summary ? h("span", { class: "ac-tool-sum", text: m.summary, title: m.summary }) : null);
    if (m.status === "failed" && m.output) return h("div", { class: "ac-tool-group" }, line, h("div", { class: "ac-tool-out", text: m.output }));
    return line;
  }

  function permissionBlock(m: Extract<ChatMsg, { kind: "permission" }>): HTMLElement {
    if (m.status !== "pending") {
      const word = m.status === "allowed" ? t("agentchat.allowed") : m.status === "denied" ? t("agentchat.denied") : t("agentchat.unanswered");
      return h("div", { class: `ac-tool ac-perm-done ${m.status}` },
        h("span", { class: "ac-tool-mark" }, m.status === "allowed" ? svg(ICONS.check, 10, { stroke: 2.8 }) : svg(ICONS.xmark, 8)),
        h("span", { class: "ac-tool-name", text: toolLabel(m.tool) }),
        h("span", { class: "ac-tool-sum", text: word }));
    }
    if (!builtAt.has(m.requestId)) builtAt.set(m.requestId, performance.now());
    const bound: CardBinding = { requestId: m.requestId, builtAt: builtAt.get(m.requestId) ?? performance.now() };
    return h("div", { class: "ac-perm" },
      h("div", { class: "ac-perm-head" },
        dot("#F5A524", 8),
        h("b", { text: toolLabel(m.tool) }),
        h("span", { class: "ac-perm-wait", text: t("agentchat.waiting") })),
      m.summary ? h("div", { class: "ac-perm-cmd", text: m.summary }) : null,
      m.detail ? h("div", { class: "ac-perm-detail", text: m.detail }) : null,
      h("div", { class: "ac-perm-row" },
        h("div", { class: "grow" }),
        h("button", { class: "btn secondary", onclick: () => void answer(m.requestId, false, bound) }, h("span", { text: t("approval.deny") })),
        h("button", { class: "btn primary", onclick: () => void answer(m.requestId, true, bound) }, h("span", { text: t("approval.allow") }))));
  }

  function errorBlock(m: Extract<ChatMsg, { kind: "error" }>): HTMLElement {
    const limit = m.code === "rateLimit";
    const block = h("div", { class: `ac-errc${limit ? " limit" : ""}` },
      h("span", { class: "ac-errc-icon" }, svg(limit ? ICONS.timer : ICONS.bang, 13)),
      h("div", { class: "ac-errc-body" }, h("div", { class: "ac-errc-text", text: m.text })));
    const kind = current?.agent;
    const cmd = m.code === "notLoggedIn" && kind ? loginCommand(kind) : null;
    if (cmd) block.querySelector(".ac-errc-body")?.append(h("div", { class: "ac-hint", text: t("agentchat.hint.login", { cmd }) }));
    return block;
  }

  function renderMessage(m: ChatMsg): HTMLElement {
    switch (m.kind) {
      case "user": return h("div", { class: "chat-row user" }, h("div", { class: "bubble", text: m.text }));
      case "assistant": return h("div", { class: "chat-row" }, h("div", { class: "ac-reply" }, renderMarkdown(m.text, hooks)));
      case "tool": return toolLine(m);
      case "permission": return permissionBlock(m);
      case "error": return errorBlock(m);
      case "notice": {
        const text = m.code === "stopped" ? t("agentchat.stopped") : m.code === "apiKey" ? t("agentchat.apiKey", { source: m.text }) : m.text;
        return h("div", { class: "ac-note", text });
      }
    }
  }

  function emptyBlock(none: boolean): HTMLElement {
    return h("div", { class: "ac-empty" },
      h("div", { class: "ac-empty-title", text: none ? t("agentchat.noAgentTitle") : t("agentchat.emptyTitle") }),
      h("div", { class: "ac-empty-hint", text: none ? t("agentchat.noAgent") : t("agentchat.empty") }));
  }

  function syncLog() {
    const id = active !== null && current?.id === active ? active : null;
    if (renderedFor !== id) {
      renderedFor = id;
      rendered.clear();
      clear(log);
      wantEnd = true;
    }
    const msgs = id === null ? [] : current?.messages ?? [];
    // A collapsed island (or a view not shown) has no room for the log: what is added meanwhile, a
    // request waiting for its click above all, must still be in view when the island opens.
    const roomy = log.clientHeight >= LOG_ROOM;
    const nearBottom = roomy && log.scrollHeight - log.scrollTop - log.clientHeight < 28;
    if (!roomy) wantEnd = true;
    let freshRequest = false;
    const ids = new Set(msgs.map((m) => m.id));
    for (const [mid, r] of rendered) {
      if (!ids.has(mid)) {
        r.el.remove();
        rendered.delete(mid);
      }
    }
    let prev: Element | null = null;
    for (const m of msgs) {
      const sig = messageSig(m);
      let r = rendered.get(m.id);
      if (!r) {
        r = { sig, el: renderMessage(m) };
        rendered.set(m.id, r);
        if (m.kind === "permission" && m.status === "pending") freshRequest = true;
      } else if (r.sig !== sig) {
        const next = renderMessage(m);
        r.el.replaceWith(next);
        r = { sig, el: next };
        rendered.set(m.id, r);
      }
      const want: ChildNode | null = prev ? prev.nextSibling : log.firstChild;
      if (r.el !== want) log.insertBefore(r.el, want);
      prev = r.el;
    }
    const working = current?.state === "working";
    if (working) {
      statusLine.textContent = current?.status ?? "";
      statusLine.style.display = current?.status ? "" : "none";
      log.append(typing);
    } else if (typing.parentNode === log) {
      typing.remove();
    }
    if (msgs.length === 0) {
      const none = agents !== null && chooseAgentKind(agents, null) === null;
      if (log.dataset.empty !== (none ? "none" : "empty")) {
        log.dataset.empty = none ? "none" : "empty";
        clear(log);
        rendered.clear();
        log.append(emptyBlock(none));
      }
    } else if (log.dataset.empty) {
      delete log.dataset.empty;
      log.querySelector(".ac-empty")?.remove();
    }
    if (roomy && (nearBottom || freshRequest || wantEnd)) {
      log.scrollTop = log.scrollHeight;
      wantEnd = false;
    }
    // A chat with nothing said yet gets the roomy field, a running one a line.
    const wantRoomy = msgs.length === 0;
    if (composer.classList.contains("roomy") !== wantRoomy) {
      composer.classList.toggle("roomy", wantRoomy);
      resize();
    }
  }

  function syncBar() {
    const kind = viewKind();
    const info = viewInfo();
    const busy = isBusy(current?.state);
    if (busy) {
      action.title = t("agentchat.stop");
      if (action.dataset.mode !== "stop") {
        action.dataset.mode = "stop";
        clear(action);
        action.append(svg(ICONS.stop, 12));
      }
      action.disabled = false;
    } else {
      action.title = t("chat.send");
      if (action.dataset.mode !== "send") {
        action.dataset.mode = "send";
        clear(action);
        action.append(svg(ICONS.arrowUp, 11));
      }
      action.disabled = !canSend(input.value, current?.state, kind !== null, sending);
    }
    input.placeholder = info ? t("agentchat.placeholder", { agent: info.label }) : "…";
    input.disabled = kind === null;
    errorBox.hidden = !error;
    errorBox.textContent = error ? errorText(error) : "";
  }

  /** The open conversation as last read, or null (a new chat, or one not read yet). */
  const shownConvo = (): Convo | null => (active !== null && current?.id === active ? current : null);

  function syncSideTime(c: Convo | null) {
    const started = c?.turnStarted ?? null;
    const text = started === null ? ""
      : isBusy(c?.state) ? clockText(Date.now() - started)
      : c?.turnEnded != null ? t("agentchat.side.took", { time: clockText(c.turnEnded - started) })
      : "";
    sideTime.textContent = text;
    sideTime.hidden = text === "";
  }

  function meter(label: string, value: string, percent: number | null, title: string): HTMLElement {
    const bar = h("div", { class: "ac-meter-bar" });
    if (percent !== null) {
      const p = usagePercent(percent);
      bar.append(h("i", { class: `tone-${p.tone}`, style: `width:${Math.max(2, p.ring)}%` }));
    }
    return h("div", { class: "ac-meter", title },
      h("div", { class: "ac-meter-label", text: label }), bar, h("div", { class: "ac-meter-value", text: value }));
  }

  function drawMeters(c: Convo | null) {
    const list: HTMLElement[] = [];
    if (c?.contextTokens != null) {
      const used = c.contextTokens;
      const size = c.contextWindow;
      if (size) {
        const percent = (used / size) * 100;
        list.push(meter(t("agentchat.side.context"), `${compactTokens(used)} · ${usagePercent(percent).text}`, percent,
          t("agentchat.side.contextOf", { used: used.toLocaleString(), window: size.toLocaleString() })));
      } else {
        list.push(meter(t("agentchat.side.context"), compactTokens(used), null, t("agentchat.side.contextSent", { used: used.toLocaleString() })));
      }
    }
    for (const w of c?.limits ?? []) {
      const label = w.kind === "five_hour" ? t("unit.h", { n: 5 }) : w.kind === "seven_day" ? t("unit.d", { n: 7 }) : w.kind;
      const percent = w.used * 100;
      list.push(meter(label, usagePercent(percent).text, percent, w.resetsAt ? resetsText(w.resetsAt * 1000, Date.now(), t) : ""));
    }
    sideMeters.replaceChildren(...list);
  }

  /** The column under Luch, in the tone of the state Luch shows. */
  function syncSide(state: BotStateName) {
    const shown = shownConvo();
    const status = sideStatus(shown);
    sideLine.textContent = t(status.key, status.vars);
    sideLine.dataset.tone = state;
    syncSideTime(shown);
    const tools = turnTools(shown);
    sideTools.hidden = tools === 0;
    if (tools > 0 && sideTools.dataset.n !== String(tools)) {
      sideTools.dataset.n = String(tools);
      sideTools.replaceChildren(svg(ICONS.terminal, 11), h("span", { text: String(tools) }));
    }
    // The reset times in the tooltips are read again at most once a minute.
    const sig = JSON.stringify([shown?.id, shown?.contextTokens, shown?.contextWindow, shown?.limits, Math.floor(Date.now() / 60_000)]);
    if (sig !== metersSig) {
      metersSig = sig;
      drawMeters(shown);
    }
    // The clock ticks only while the view is on screen and a turn runs.
    const wantTick = isOn && isBusy(shown?.state) && shown?.turnStarted != null;
    if (wantTick && tick === null) {
      tick = window.setInterval(() => syncSideTime(shownConvo()), 1000);
    } else if (!wantTick && tick !== null) {
      window.clearInterval(tick);
      tick = null;
    }
  }

  /** Luch shows the chat while this view is on screen; the card's glow follows it. */
  function syncBot(): BotStateName {
    const shown = shownConvo();
    const state = chatBotState(shown, performance.now() < finishedUntil);
    if (!isOn) {
      if (botMine) {
        State.stateOverride = null;
        botMine = false;
      }
      return state;
    }
    State.stateOverride = state;
    botMine = true;
    card.style.setProperty("--wash", washRGBA(WASH[state]));
    return state;
  }

  return {
    el,
    sync() {
      const busy = isBusy(current?.state);
      if (busy && menuOpen !== null) menuOpen = null;
      syncTabs();
      syncContext(busy);
      syncLog();
      syncMenu();
      syncBar();
      syncSide(syncBot());
    },
    focus() {
      input.focus();
    },
    setActive(on) {
      isOn = on;
      armed = false;
      if (!on) {
        menuOpen = null;
        syncSide(syncBot());
      } else {
        wantEnd = true;
        void refreshAll();
      }
    },
  };
}
