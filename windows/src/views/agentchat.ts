// The Agents tab: a chat run by the user's own agent CLIs (src-tauri/src/chat), with no API key.
// A conversation is a tab of this view; the log shows what the CLI says (its answer as it is
// written, the tools it runs) and every permission request it asks, with its own Allow and Deny.
// Rust keeps the conversation: the view reads it again when told it changed (`agent-chat-changed`)
// and when it is shown. A request is answered only by a click on its button (agentChatPermission);
// nothing here has a timeout or a default.
//
// Off with the rest of the feature (core/features.ts CLI_CHAT_ENABLED): views.ts builds it only then.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { Bridge, onAgentChatChanged } from "../core/bridge";
import { Sessions } from "../core/sessions";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import { t } from "../core/i18n";
import { clickCounts, type CardBinding } from "../island/answers";
import { defaultRoot, enterStep, keptRoot, projectChoices } from "./launcher-model";
import {
  agentCommand, canSend, chooseAgentKind, folderName, isBusy, keepMode, keepModel, loginCommand, messageSig, modeKey, normalizeError,
  reconcileActive, stateColor, tabTitle,
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
};

/** The conversation the island was told to show (a request is waiting in it), read when the view shows. */
let wanted: number | null = null;

/** island/agent-chat.ts: the conversation to open on when the next shown view is this one. */
export function showAgentChat(conv: number): void {
  wanted = conv;
  State.notify();
}

/** The agent a new chat starts with, remembered while the app runs. */
let lastAgent: AgentKind | null = null;

/** A mode a newer Rust offers that this page has no word for shows as its id. */
function modeLabel(mode: string): string {
  const key = modeKey(mode);
  return key ? t(key) : mode;
}

export function buildAgentChat(actions: ViewActions, backend: AgentChatBackend = BRIDGE_BACKEND): ViewHost {
  void actions;
  const tabs = h("div", { class: "ac-tabs" });
  const agentSelect = h("select", { class: "launch-select", title: t("chat.agent") });
  const projectSelect = h("select", { class: "launch-select", title: t("chat.project") });
  const modelSelect = h("select", { class: "launch-select", title: t("agentchat.model") });
  const modeSelect = h("select", { class: "launch-select", title: t("agentchat.mode") });
  const opts = h("div", { class: "launch-row ac-opts" }, agentSelect, projectSelect, modelSelect, modeSelect);
  const log = h("div", { class: "chat-log ac-log" });
  const errorBox = h("div", { class: "ac-error ac-flash" });
  const input = h("textarea", { class: "chat-input", rows: 1, spellcheck: "false" });
  const action = h("button", { class: "send-btn", title: t("chat.send") }, svg(ICONS.arrowUp, 11));
  const bar = h("div", { class: "chat-bar" }, input, action);
  const body = h("div", { class: "ac-body" }, tabs, opts, log, errorBox, bar);
  const el = h("div", { class: "view" }, h("div", { class: "card chat-card" }, body));

  let agents: AgentInfo[] | null = null;
  let convos: ConvoSummary[] = [];
  /** The open conversation; null is a new chat (the pickers say what it will be). */
  let active: number | null = null;
  let current: Convo | null = null;
  const draft = { agent: null as AgentKind | null, root: null as string | null, model: "", mode: "" };
  let error: ChatError | null = null;
  let sending = false;
  let isOn = false;
  /** Enter went down in the field; the message goes on its release (enterStep). */
  let armed = false;
  /** When the buttons of each request were first drawn: a click right after is ignored (answers.ts). */
  const builtAt = new Map<string, number>();
  const rendered = new Map<number, { sig: string; el: HTMLElement }>();
  let renderedFor: number | null = null;
  /** The log must end at its last line the next time it has room (it was shown, or grew, without any). */
  let wantEnd = true;
  const typing = h("div", { class: "chat-row ac-tail" }, h("div", { class: "typing" }, h("i"), h("i"), h("i")));
  const statusLine = h("div", { class: "ac-note ac-status" });
  typing.append(statusLine);
  let tabsSig = "";
  let optsSig = "";

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
    const next = await backend.get(id, current?.id === id ? current.rev : null);
    if (next && active === id) current = next;
    changed();
  }

  async function refreshAll() {
    await Promise.all([refreshList(), agents === null ? refreshAgents() : Promise.resolve()]);
    if (wanted !== null) {
      if (convos.some((c) => c.id === wanted)) active = wanted;
      wanted = null;
    }
    await refreshCurrent();
  }

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
    changed();
    void refreshCurrent();
  }

  async function submit() {
    const text = input.value.trim();
    const kind = viewKind();
    if (!canSend(text, current?.state, kind !== null, sending)) return;
    error = null;
    sending = true;
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

  agentSelect.addEventListener("change", () => {
    const kind = agentSelect.value as AgentKind;
    if (active !== null || !agentOf(kind)?.found) return;
    draft.agent = kind;
    draft.model = keepModel(agentOf(kind), draft.model);
    draft.mode = keepMode(agentOf(kind), draft.mode);
    error = null;
    changed();
  });
  projectSelect.addEventListener("change", () => {
    draft.root = projectSelect.value === "" ? null : projectSelect.value;
    changed();
  });
  modelSelect.addEventListener("change", () => void choose(modelSelect.value, active === null ? draft.mode : current?.mode ?? draft.mode));
  modeSelect.addEventListener("change", () => void choose(active === null ? draft.model : current?.model ?? draft.model, modeSelect.value));
  action.addEventListener("click", () => (isBusy(current?.state) ? stop() : void submit()));

  function resize() {
    input.style.height = "18px";
    input.style.height = `${Math.min(input.scrollHeight, 54)}px`;
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
    e.stopPropagation(); // Escape closes the island, not the chat
  };
  input.addEventListener("keydown", onKey);
  input.addEventListener("keyup", onKey);
  action.addEventListener("keydown", onKey);
  action.addEventListener("keyup", onKey);

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

  function agentOptionText(a: AgentInfo): string {
    if (!a.found) return `${a.label} · ${t("agentchat.notInstalled")}`;
    if (a.login === "notLoggedIn") return `${a.label} · ${t("agentchat.signedOut")}`;
    return a.verified ? a.label : `${a.label} · ${t("agentchat.unverified")}`;
  }

  function syncTabs() {
    const sig = `${active}|${convos.map((c) => `${c.id}:${c.state}:${c.title}`).join(",")}`;
    if (sig === tabsSig) return;
    tabsSig = sig;
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

  function syncOptions(busy: boolean) {
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
      info?.models.length, info?.modes.join(","),
    ]);
    if (sig === optsSig) return;
    optsSig = sig;

    clear(agentSelect);
    for (const a of agents ?? []) {
      agentSelect.append(h("option", { value: a.kind, text: agentOptionText(a), disabled: !a.found }));
    }
    if (agents === null) agentSelect.append(h("option", { value: "", text: "…" }));
    agentSelect.value = kind ?? "";
    agentSelect.disabled = locked || agents === null;

    clear(projectSelect);
    projectSelect.append(h("option", { value: "", text: t("agentchat.home") }));
    for (const r of roots) projectSelect.append(h("option", { value: r.root, text: r.name, title: r.root }));
    const shown = locked ? current?.cwd ?? "" : draft.root ?? "";
    if (shown !== "" && !roots.some((r) => r.root === shown)) {
      projectSelect.append(h("option", { value: shown, text: folderName(shown), title: shown }));
    }
    projectSelect.value = shown;
    projectSelect.disabled = locked;

    clear(modelSelect);
    for (const m of info?.models ?? []) modelSelect.append(h("option", { value: m.id, text: m.label === "" ? t("agentchat.modelDefault") : m.label }));
    modelSelect.value = model;
    modelSelect.disabled = busy || !info;

    clear(modeSelect);
    for (const m of info?.modes ?? []) modeSelect.append(h("option", { value: m, text: modeLabel(m) }));
    modeSelect.value = mode;
    modeSelect.disabled = busy || !info;
  }

  function toolLine(m: Extract<ChatMsg, { kind: "tool" }>): HTMLElement {
    const mark = m.status === "running" ? "…" : m.status === "ok" ? "✓" : "✕";
    const line = h("div", { class: `ac-tool ${m.status}` },
      h("span", { class: "ac-tool-mark", text: mark }),
      h("span", { class: "ac-tool-name", text: m.name }),
      m.summary ? h("span", { class: "ac-tool-sum", text: m.summary }) : null);
    if (m.status === "failed" && m.output) line.append(h("div", { class: "ac-tool-out", text: m.output }));
    return line;
  }

  function permissionBlock(m: Extract<ChatMsg, { kind: "permission" }>): HTMLElement {
    if (!builtAt.has(m.requestId)) builtAt.set(m.requestId, performance.now());
    const bound: CardBinding = { requestId: m.requestId, builtAt: builtAt.get(m.requestId) ?? performance.now() };
    const block = h("div", { class: `ac-perm ${m.status}` },
      h("div", { class: "ac-perm-head" },
        dot("#F5A524", 7),
        h("b", { text: m.tool }),
        m.summary ? h("span", { class: "ac-perm-sum", text: m.summary }) : null),
      m.detail ? h("div", { class: "ac-perm-detail", text: m.detail }) : null);
    if (m.status === "pending") {
      block.append(h("div", { class: "ac-perm-row" },
        h("span", { class: "ac-perm-wait", text: t("agentchat.waiting") }),
        h("div", { class: "grow" }),
        h("button", { class: "btn secondary", onclick: () => void answer(m.requestId, false, bound) }, h("span", { text: t("approval.deny") })),
        h("button", { class: "btn primary", onclick: () => void answer(m.requestId, true, bound) }, h("span", { text: t("approval.allow") }))));
    } else {
      const word = m.status === "allowed" ? t("agentchat.allowed") : m.status === "denied" ? t("agentchat.denied") : t("agentchat.unanswered");
      block.append(h("div", { class: "ac-perm-done", text: word }));
    }
    return block;
  }

  function errorBlock(m: Extract<ChatMsg, { kind: "error" }>): HTMLElement {
    const block = h("div", { class: "ac-error" }, h("div", { text: m.text }));
    const kind = current?.agent;
    const cmd = m.code === "notLoggedIn" && kind ? loginCommand(kind) : null;
    if (cmd) block.append(h("div", { class: "ac-hint", text: t("agentchat.hint.login", { cmd }) }));
    return block;
  }

  function renderMessage(m: ChatMsg): HTMLElement {
    switch (m.kind) {
      case "user": return h("div", { class: "chat-row user" }, h("div", { class: "bubble", text: m.text }));
      case "assistant": return h("div", { class: "chat-row" }, h("div", { class: "reply", text: m.text }));
      case "tool": return toolLine(m);
      case "permission": return permissionBlock(m);
      case "error": return errorBlock(m);
      case "notice": {
        const text = m.code === "stopped" ? t("agentchat.stopped") : m.code === "apiKey" ? t("agentchat.apiKey", { source: m.text }) : m.text;
        return h("div", { class: "ac-note", text });
      }
    }
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
    const roomy = log.clientHeight >= 40;
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
        log.append(h("div", { class: "ac-empty", text: none ? t("agentchat.noAgent") : t("agentchat.empty") }));
      }
    } else if (log.dataset.empty) {
      delete log.dataset.empty;
      log.querySelector(".ac-empty")?.remove();
    }
    if (roomy && (nearBottom || freshRequest || wantEnd)) {
      log.scrollTop = log.scrollHeight;
      wantEnd = false;
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
    errorBox.style.display = error ? "" : "none";
    errorBox.textContent = error ? errorText(error) : "";
  }

  return {
    el,
    sync() {
      const busy = isBusy(current?.state);
      syncTabs();
      syncOptions(busy);
      syncLog();
      syncBar();
    },
    focus() {
      input.focus();
    },
    setActive(on) {
      isOn = on;
      armed = false;
      if (on) {
        wantEnd = true;
        void refreshAll();
      }
    },
  };
}
