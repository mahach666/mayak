// Chat view — DOM port of PromptView / ChatBubble / TypingDotsView from
// IslandViewContent.swift, rebuilt as the launcher (launcher spec §5): the question opens the
// chosen agent in the chosen project with the question in its field, never sent. "Claude API
// (answers here)" keeps the island chat as it was.

import { h, svg, clear } from "./dom";
import { ICONS } from "./icons";
import { Bridge, type ChatContext, type LauncherAgent } from "../core/bridge";
import { Sessions } from "../core/sessions";
import { Sound } from "../core/sound";
import { State, type ChatMessage } from "../core/state";
import {
  AGENT_LABEL, API_AGENT, NO_PROJECT, chooseAgent, defaultRoot, enterStep, keptRoot, launchFollowUp, projectChoices,
} from "./launcher-model";
import type { ViewActions, ViewHost } from "./views";

let nextId = 1;

/** How long the launcher's hint or error stays up before the island moves on. */
const NOTE_MS = 2400;

function bubble(message: ChatMessage): HTMLElement {
  if (message.role === "user") {
    return h(
      "div",
      { class: "chat-row user" },
      h("div", { class: "bubble", text: message.content }),
    );
  }
  return h("div", { class: "chat-row" }, h("div", { class: "reply", text: message.content }));
}

function typingDots(): HTMLElement {
  return h(
    "div",
    { class: "chat-row" },
    h("div", { class: "typing" }, h("i"), h("i"), h("i")),
  );
}

/** The coloured chip showing what the question is about (a dropped file), with its ✕ (spec §5). */
function contextChip(label: string, onRemove: () => void): HTMLElement {
  const remove = h("button", { class: "chip-x", title: "Remove", onclick: () => onRemove() }, svg(ICONS.xmark, 9));
  const chip = h("div", { class: "chip" }, h("i", { class: "chip-dot" }), h("span", { text: label }), remove);
  requestAnimationFrame(() => chip.classList.add("settled"));
  return chip;
}

export function buildPrompt(actions: ViewActions, onHeightChange: () => void): ViewHost {
  const chipRow = h("div", { class: "chip-row" });
  const log = h("div", { class: "chat-log" });
  const input = h("textarea", {
    class: "chat-input",
    rows: 1,
    placeholder: "Ask me anything…",
    spellcheck: "false",
  });
  const send = h("button", { class: "send-btn", title: "Send" }, svg(ICONS.arrowUp, 11));
  const bar = h("div", { class: "chat-bar" }, input, send);
  const agentSelect = h("select", { class: "launch-select", title: "Agent" });
  const projectSelect = h("select", { class: "launch-select", title: "Project" });
  const launchRow = h("div", { class: "launch-row" }, agentSelect, projectSelect);
  const chatCard = h(
    "div",
    { class: "card wash chat-card launch" },
    h("div", { class: "chat-body" }, chipRow, log, bar, launchRow),
  );
  chatCard.style.setProperty("--wash", "rgba(99,102,241,0.5)");
  const el = h("div", { class: "view" }, chatCard);
  agentSelect.append(h("option", { value: "", text: "…" }));

  let sending = false;
  let launching = false;
  let renderedCount = -1;
  /** Agents found when the view opened; null until the answer comes. */
  let targets: LauncherAgent[] | null = null;
  let agent: LauncherAgent | null = null;
  /** The project of this opening of the view; undefined until the first draw picks the default. */
  let chosenRoot: string | null | undefined;
  let root: string | null = null;
  /** Enter went down in the field; the question goes on its release (enterStep). */
  let armed = false;
  /** A failed launch's note is up: the chosen project survives the view hiding and coming back. */
  let keepChoice = false;

  function setAgent(next: LauncherAgent | null) {
    agent = next;
    State.launcherApi = next === API_AGENT;
    agentSelect.value = next ?? "";
  }

  function rebuildAgents() {
    const list = targets ?? [];
    clear(agentSelect);
    if (list.length === 0) agentSelect.append(h("option", { value: "", text: "No agent found" }));
    for (const id of list) agentSelect.append(h("option", { value: id, text: AGENT_LABEL[id] }));
    setAgent(chooseAgent(list, State.settings.launcherAgent));
  }

  /** "Claude API (answers here)": the island chat, unchanged. */
  async function submitApi() {
    const query = input.value.trim();
    if (!query || sending) return;
    input.value = "";
    sending = true;
    Sound.play("send");

    State.chatHistory.push({ id: nextId++, role: "user", content: query });
    State.stateOverride = "thinking";
    State.notify();
    onHeightChange();

    const file = State.droppedFile;
    const context: ChatContext | null =
      State.chatHistory.length === 1 && file ? { kind: "file", name: file.name, path: file.path } : null;

    try {
      const reply = await Bridge.chatSend(query, context);
      State.chatHistory.push({ id: nextId++, role: "assistant", content: reply.text });
      State.stateOverride = null;
      Sound.play("finish");
    } catch (err) {
      State.stateOverride = null;
      State.noteMessage = String(err).replace(/^Error:\s*/, "");
      State.view = "note";
      Sound.play("error");
    } finally {
      sending = false;
      State.notify();
      onHeightChange();
      input.focus();
    }
  }

  /** Any other agent: Rust opens it with the question in its field (spec §4). */
  async function launch(target: LauncherAgent) {
    const prompt = input.value.trim();
    if (!prompt || launching) return;
    launching = true;
    Sound.play("send");
    State.stateOverride = "thinking";
    State.notify();
    const files = State.droppedFile ? [State.droppedFile.path] : [];
    const outcome = await Bridge.launcherOpen(target, prompt, root, files);
    launching = false;
    State.stateOverride = null;
    const next = launchFollowUp(outcome, target);
    if (next.clear) {
      input.value = "";
      State.droppedFile = null;
      State.promptContext = null;
    }
    if (next.note === null) {
      State.notify();
      actions.collapse();
      return;
    }
    State.noteMessage = next.note;
    if (next.then === "prompt") Sound.play("error");
    keepChoice = next.then === "prompt";
    actions.setView("note");
    window.setTimeout(() => {
      if (State.view !== "note") {
        // The island moved on without coming back to the prompt: this opening is over.
        if (keepChoice) {
          keepChoice = false;
          chosenRoot = undefined;
        }
        return;
      }
      if (next.then === "collapse") actions.collapse();
      else actions.setView("prompt");
    }, NOTE_MS);
  }

  function submit() {
    if (agent === API_AGENT) void submitApi();
    else if (agent) void launch(agent);
  }

  send.addEventListener("click", () => submit());
  const onKey = (e: KeyboardEvent) => {
    const step = enterStep(
      {
        type: e.type === "keyup" ? "keyup" : "keydown",
        key: e.key,
        shiftKey: e.shiftKey,
        repeat: e.repeat,
        isComposing: e.isComposing,
      },
      armed,
    );
    armed = step.armed;
    if (step.prevent) e.preventDefault();
    if (step.send) submit();
    e.stopPropagation(); // Escape closes the island, not the chat
  };
  input.addEventListener("keydown", onKey);
  input.addEventListener("keyup", onKey);
  send.addEventListener("keydown", onKey);
  send.addEventListener("keyup", onKey);
  agentSelect.addEventListener("change", () => {
    const picked = (targets ?? []).find((t) => t === agentSelect.value);
    if (!picked) return;
    setAgent(picked);
    State.settings.launcherAgent = picked;
    void Bridge.saveSettings(State.settings);
    State.notify();
    onHeightChange();
  });
  projectSelect.addEventListener("change", () => {
    chosenRoot = projectSelect.value || null;
    State.notify();
  });

  return {
    el,
    sync() {
      const file = State.droppedFile;
      const wantChip = file?.name ?? "";
      if (chipRow.dataset.label !== wantChip) {
        chipRow.dataset.label = wantChip;
        clear(chipRow);
        if (wantChip) {
          chipRow.append(contextChip(wantChip, () => {
            State.droppedFile = null;
            State.promptContext = null;
            State.notify();
          }));
        }
      }

      const api = agent === API_AGENT;
      chatCard.classList.toggle("launch", !api);
      projectSelect.hidden = api;

      const snapshot = Sessions.snapshot;
      const choices = projectChoices(snapshot);
      const key = choices.map((c) => c.root).join("\n");
      if (projectSelect.dataset.key !== key) {
        projectSelect.dataset.key = key;
        clear(projectSelect);
        projectSelect.append(h("option", { value: "", text: NO_PROJECT }));
        for (const c of choices) projectSelect.append(h("option", { value: c.root, text: c.name, title: c.root }));
      }
      if (chosenRoot === undefined) chosenRoot = defaultRoot(snapshot);
      root = keptRoot(chosenRoot, choices);
      projectSelect.value = root ?? "";

      const thinking = State.stateOverride === "thinking";
      const count = api ? State.chatHistory.length + (thinking ? 0.5 : 0) : -1;
      if (count !== renderedCount) {
        renderedCount = count;
        clear(log);
        if (api) {
          for (const m of State.chatHistory) log.append(bubble(m));
          if (thinking) log.append(typingDots());
          log.scrollTop = log.scrollHeight;
        }
      }

      input.placeholder = api && State.chatHistory.length > 0 ? "Continue…" : "Ask me anything…";
      input.disabled = sending || launching;
      send.disabled = agent === null || sending || launching;
      agentSelect.disabled = launching || (targets ?? []).length === 0;
      projectSelect.disabled = launching;
    },
    focus() {
      input.focus();
      input.select();
    },
    setActive(on) {
      armed = false;
      if (!on) {
        if (!keepChoice) chosenRoot = undefined;
        return;
      }
      keepChoice = false;
      // Read each time the tab opens (spec §3): what is installed may have changed.
      void Bridge.launcherTargets().then((list) => {
        targets = list ?? [];
        rebuildAgents();
        State.notify();
        onHeightChange();
      });
    },
  };
}
