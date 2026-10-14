// Dev harness: the Agents tab built with the real buildAgentChat() and styles, on a fake backend, in
// the states of the CLI chat: a new chat, a turn that is working, answers in Markdown, a request
// waiting for a click, the same after the click, a CLI that is not signed in, a plan limit, the model
// panel open, and an agent that is missing. Luch is drawn by the island in the app; here a disc of
// the colour of its state stands in its place, and its state is named in the title. Not part of the
// app bundle.

import "../src/style.css";
import { buildAgentChat, showAgentChat, type AgentChatBackend } from "../src/views/agentchat";
import type { ViewActions } from "../src/views/views";
import { State } from "../src/core/state";
import { botGlowColor } from "../src/core/layout";
import type { AgentInfo, ChatMsg, Convo, ConvoSummary, OpenRequest } from "../src/core/agent-chat";

const AGENTS: AgentInfo[] = [
  { kind: "claudeCode", label: "Claude Code", found: true, verified: true, verifiedWith: "2.1.296",
    models: [{ id: "", label: "" }, { id: "fable", label: "Fable 5.1" }, { id: "opus", label: "Opus 5.5" }, { id: "sonnet", label: "Sonnet 5.5" }, { id: "haiku", label: "Haiku 5.5" }],
    modes: ["ask", "plan"], login: "ready" },
  { kind: "codex", label: "Codex", found: true, verified: true, verifiedWith: "0.162.1",
    models: [{ id: "", label: "" }, { id: "gpt-6.1-sol", label: "GPT-6.1-Sol" }, { id: "gpt-5.5", label: "GPT-5.5" }, { id: "gpt-5.4-mini", label: "GPT-5.4-Mini" }],
    modes: ["readOnly", "workspaceWrite"], login: "ready" },
  { kind: "geminiCli", label: "Gemini CLI", found: true, verified: false, verifiedWith: null,
    models: [{ id: "", label: "" }, { id: "flash", label: "Flash" }], modes: ["ask", "plan"], login: "unknown" },
  { kind: "cursorAgent", label: "Cursor Agent", found: false, verified: false, verifiedWith: null,
    models: [{ id: "", label: "" }], modes: ["proposeOnly"], login: "unknown" },
];

const NONE: AgentInfo[] = AGENTS.map((a) => ({ ...a, found: false }));

let rev = 1;
function convo(id: number, over: Partial<Convo>, messages: ChatMsg[]): Convo {
  return {
    id, agent: "claudeCode", cwd: String.raw`C:\work\mayak`, model: "haiku", mode: "ask", title: "", session: "s", usedModel: null,
    state: "idle", status: null, loginHint: false, turnStarted: null, turnEnded: null, contextTokens: null, contextWindow: null,
    limits: [], rev: rev++, messages, ...over,
  };
}

function backend(convos: Convo[], agents: AgentInfo[] = AGENTS): AgentChatBackend {
  return {
    agents: async () => agents,
    list: async () => convos.map((c): ConvoSummary => ({ id: c.id, agent: c.agent, cwd: c.cwd, title: c.title, state: c.state, rev: c.rev })),
    get: async (id) => convos.find((c) => c.id === id) ?? null,
    open: async (r: OpenRequest) => {
      void r;
      return 99;
    },
    set: async () => {},
    send: async () => {},
    permission: async () => true,
    cancel: async () => {},
    close: async () => {},
    onChanged: () => {},
    openUrl: (url) => console.log("open", url),
  };
}

const actions = new Proxy({}, { get: () => () => {} }) as ViewActions;
const root = document.getElementById("panels")!;
const params = new URLSearchParams(location.search);

interface PanelOptions {
  /** The index of the chip to click once drawn (a conversation of this panel); null leaves the new chat. */
  open: number | null;
  /** Open the list of a composer chip. */
  menu?: "agent" | "mode" | "model";
  agents?: AgentInfo[];
  /** The island is narrow: the stage is this wide (px) instead of the island's 620. */
  width?: number;
  /**
   * Once drawn, a request comes in this conversation (as island/agent-chat.ts tells the view), with this
   * typed first. `behindCard`: a hook's card is on the island meanwhile (the view is not on screen), and
   * the view comes back when the card is answered (resumeChatRequest's island.alert).
   */
  request?: { conv: number; typed: string; behindCard?: boolean };
}

async function panel(title: string, convos: Convo[], options: PanelOptions) {
  const wrap = document.createElement("div");
  wrap.className = "panel";
  const h2 = document.createElement("h2");
  const stage = document.createElement("div");
  stage.className = "stage";
  const stageW = options.width ?? Number(params.get("w") ?? 0);
  const stageH = Number(params.get("h") ?? 0);
  if (stageW) stage.style.width = `${stageW}px`;
  if (stageH) stage.style.height = `${stageH}px`;
  const luch = document.createElement("div");
  luch.className = "fake-luch";
  stage.append(luch);
  wrap.append(h2, stage);
  root.append(wrap);
  const host = buildAgentChat(actions, backend(convos, options.agents));
  host.el.classList.add("on");
  stage.append(host.el);
  const draw = () => {
    host.sync();
    const state = State.stateOverride ?? "idle";
    luch.style.background = botGlowColor(state);
    h2.textContent = `${title}  ·  Luch: ${state}`;
  };
  State.subscribe(draw);
  host.setActive?.(true);
  await new Promise((r) => setTimeout(r, 80));
  if (options.open !== null) host.el.querySelectorAll<HTMLElement>(".ac-chip")[options.open]?.click();
  await new Promise((r) => setTimeout(r, 80));
  if (options.menu) host.el.querySelector<HTMLElement>(`.ac-${options.menu}`)?.click();
  if (options.request) {
    const field = host.el.querySelector<HTMLTextAreaElement>(".ac-composer textarea");
    const shown = () => host.el.querySelector(".ac-chip.on .ac-chip-title")?.textContent ?? "";
    if (field) field.value = options.request.typed;
    if (options.request.behindCard) host.setActive?.(false);
    showAgentChat(options.request.conv);
    await new Promise((r) => setTimeout(r, 80));
    wrap.dataset.whileCard = shown();
    if (options.request.behindCard) host.setActive?.(true);
    await new Promise((r) => setTimeout(r, 120));
    wrap.dataset.after = shown();
  }
  await new Promise((r) => setTimeout(r, 40));
  draw();
}

/** What Claude Code's rate_limit_event reported in a real run (2026-10-14), the reset times moved to now. */
const PLAN = [
  { kind: "five_hour", used: 0.05, resetsAt: Math.floor(Date.now() / 1000) + 2 * 3600 + 14 * 60 },
  { kind: "seven_day", used: 0.85, resetsAt: Math.floor(Date.now() / 1000) + 3 * 86_400 },
];

const RICH = [
  "The CI fails because the workflow pins **Node 20**, but `package.json` asks for *22.6+*.",
  "",
  "## What to change",
  "",
  "1. Bump the version in `.github/workflows/ci.yml`",
  "2. Re-run the checks:",
  "   - `npm run check`",
  "   - `cargo test --workspace`",
  "",
  "```yaml",
  "- uses: actions/setup-node@v4",
  "  with:",
  "    node-version: 22",
  "```",
  "",
  "> The type-stripping flag needs Node 22.6 or newer.",
  "",
  "| File | Change |",
  "|---|---|",
  "| ci.yml | node 22 |",
  "| package.json | engines |",
  "",
  "See [the Node docs](https://nodejs.org/api/typescript.html) or https://nodejs.org.",
].join("\n");

const only = params.get("only");
const panels: [string, () => Promise<void>][] = [
  ["new", () => panel("A new chat: nothing running yet", [], { open: null })],
  ["none", () => panel("No agent CLI installed", [], { open: null, agents: NONE })],
  ["menu-mode", () => panel("A new chat: the access mode list", [], { open: null, menu: "mode" })],
  ["menu-model", () => panel("A new chat: the model list", [], { open: null, menu: "model" })],
  ["menu-agent", () => panel("A new chat: the agent list", [], { open: null, menu: "agent" })],
  ["working", () => panel("Working: tools ran, one runs, the CLI is retrying", [
    convo(1, { title: "Why does the build fail?", state: "working", status: "Reconnecting... 2/5 (unexpected status 401)",
      turnStarted: Date.now() - 42_000, contextTokens: 26_155, contextWindow: 1_000_000, limits: PLAN }, [
      { id: 1, kind: "user", text: "Why does the build fail on CI but not on my machine?" },
      { id: 2, kind: "tool", toolId: "t1", name: "Bash", summary: "git log --oneline -5", status: "ok", output: null },
      { id: 3, kind: "tool", toolId: "t3", name: "mcp__mayak__list_sessions", summary: "", status: "ok", output: null },
      { id: 4, kind: "tool", toolId: "t2", name: "Read", summary: String.raw`C:\work\mayak\.github\workflows\ci.yml`, status: "running", output: null },
    ]),
  ], { open: 0 })],
  ["thinking", () => panel("Thinking: the answer is being written, with the agent's own line (set_status)", [
    convo(7, { title: "Why does the build fail?", state: "working", agentNote: "Comparing the Node versions of CI and package.json" }, [
      { id: 1, kind: "user", text: "Why does the build fail on CI but not on my machine?" },
      { id: 2, kind: "tool", toolId: "t1", name: "Bash", summary: "git log --oneline -5", status: "ok", output: null },
      { id: 4, kind: "assistant", text: "The workflow pins **Node 20**, but `package.json` asks for 22.6+, so" },
    ]),
  ], { open: 0 })],
  ["answer", () => panel("An answer in Markdown (finished)", [
    convo(8, { title: "Why does the build fail?", state: "idle", turnStarted: Date.now() - 75_000, turnEnded: Date.now() - 63_000,
      contextTokens: 48_900, contextWindow: 200_000, limits: PLAN }, [
      { id: 1, kind: "user", text: "Why does the build fail on CI but not on my machine?" },
      { id: 2, kind: "assistant", text: RICH },
    ]),
  ], { open: 0 })],
  ["perm", () => panel("Waiting for a click: the CLI asks to run a command", [
    convo(2, { title: "Write the probe file", state: "waiting", turnStarted: Date.now() - 9_000, contextTokens: 31_020, contextWindow: 1_000_000, limits: PLAN }, [
      { id: 1, kind: "user", text: "Use the Bash tool to run exactly: cmd /c echo hello > probe_out.txt" },
      { id: 2, kind: "tool", toolId: "t1", name: "Bash", summary: "cmd /c echo hello > probe_out.txt", status: "running", output: null },
      { id: 3, kind: "permission", requestId: "2.1", tool: "Bash", summary: "cmd /c echo hello > probe_out.txt",
        detail: "Run the requested cmd echo redirect to probe_out.txt", toolUseId: "t1", status: "pending" },
    ]),
  ], { open: 0 })],
  ["ask", () => panel("ask_user: the agent waits for an answer on the island", [
    convo(9, { title: "Pick a colour", state: "working" }, [
      { id: 1, kind: "user", text: "Ask me red or blue through the Mayak MCP server." },
      { id: 2, kind: "tool", toolId: "t1", name: "mcp__mayak__ask_user", summary: "Red or blue?", status: "running", output: null },
    ]),
  ], { open: 0 })],
  ["done", () => panel("After the clicks: allowed, then denied; stopped by the user", [
    convo(3, { title: "Two commands", state: "idle" }, [
      { id: 1, kind: "user", text: "Run two commands." },
      { id: 2, kind: "permission", requestId: "3.1", tool: "Bash", summary: "cmd /c echo real > real_probe.txt", detail: null, toolUseId: null, status: "allowed" },
      { id: 3, kind: "tool", toolId: "t1", name: "Bash", summary: "cmd /c echo real > real_probe.txt", status: "ok", output: null },
      { id: 4, kind: "permission", requestId: "3.2", tool: "Bash", summary: "cmd /c echo no > denied_probe.txt", detail: null, toolUseId: null, status: "denied" },
      { id: 5, kind: "tool", toolId: "t2", name: "Bash", summary: "cmd /c echo no > denied_probe.txt", status: "failed", output: "The user denied this in Mayak." },
      { id: 6, kind: "notice", code: "apiKey", text: "ANTHROPIC_API_KEY" },
      { id: 7, kind: "notice", code: "stopped", text: "Stopped." },
    ]),
  ], { open: 0 })],
  ["request", () => panel("A request in another chat while this one is open: the view goes to it", [
    convo(21, { title: "Refactor the parser", state: "idle" }, [{ id: 1, kind: "user", text: "Refactor the parser" }, { id: 2, kind: "assistant", text: "Done." }]),
    convo(22, { title: "Write the probe file", state: "waiting" }, [
      { id: 1, kind: "user", text: "Use the Bash tool to run exactly: cmd /c echo hello > probe_out.txt" },
      { id: 2, kind: "permission", requestId: "22.1", tool: "Bash", summary: "cmd /c echo hello > probe_out.txt", detail: null, toolUseId: null, status: "pending" },
    ]),
  ], { open: 0, request: { conv: 22, typed: "" } })],
  ["request-typing", () => panel("The same while a message is being typed: the view goes there, the typed text stays with its own chat", [
    convo(23, { title: "Refactor the parser", state: "idle" }, [{ id: 1, kind: "user", text: "Refactor the parser" }, { id: 2, kind: "assistant", text: "Done." }]),
    convo(24, { title: "Write the probe file", state: "waiting" }, [
      { id: 1, kind: "user", text: "Use the Bash tool to run exactly: cmd /c echo hello > probe_out.txt" },
      { id: 2, kind: "permission", requestId: "24.1", tool: "Bash", summary: "cmd /c echo hello > probe_out.txt", detail: null, toolUseId: null, status: "pending" },
    ]),
  ], { open: 0, request: { conv: 24, typed: "half a message for the parser chat" } })],
  ["request-behind-card", () => panel("A request kept off by a hook's card: the view goes to it when the card is answered", [
    convo(25, { title: "Refactor the parser", state: "idle" }, [{ id: 1, kind: "user", text: "Refactor the parser" }, { id: 2, kind: "assistant", text: "Done." }]),
    convo(26, { title: "Write the probe file", state: "waiting" }, [
      { id: 1, kind: "user", text: "Use the Bash tool to run exactly: cmd /c echo hello > probe_out.txt" },
      { id: 2, kind: "permission", requestId: "26.1", tool: "Bash", summary: "cmd /c echo hello > probe_out.txt", detail: null, toolUseId: null, status: "pending" },
    ]),
  ], { open: 0, request: { conv: 26, typed: "typed before the card came", behindCard: true } })],
  ["login", () => panel("Not signed in; three chats open", [
    convo(4, { title: "Hello", state: "failed", loginHint: true, agent: "codex", mode: "readOnly", model: "" }, [
      { id: 1, kind: "user", text: "Hello" },
      { id: 2, kind: "error", code: "notLoggedIn", text: "unexpected status 401 Unauthorized: Missing bearer or basic authentication in header" },
    ]),
    convo(5, { title: "Refactor the parser", state: "working" }, []),
    convo(6, { title: "Review the diff", state: "waiting" }, []),
  ], { open: 0 })],
  ["limit", () => panel("The plan limit is used up", [
    convo(10, { title: "Hello", state: "failed", agent: "codex", mode: "readOnly", model: "", turnStarted: Date.now() - 20_000, turnEnded: Date.now() - 15_000 }, [
      { id: 1, kind: "user", text: "Reply with exactly: pong" },
      { id: 2, kind: "error", code: "rateLimit", text: "You’ve hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), visit https://chatgpt.com/settings/usage to purchase more credits or try again at 1:23 AM." },
    ]),
  ], { open: 0 })],
  ["error", () => panel("A failed turn", [
    convo(11, { title: "Hello", state: "failed" }, [
      { id: 1, kind: "user", text: "Hello" },
      { id: 2, kind: "error", code: "failed", text: "The 'definitely-not-a-model' model is not supported when using Codex with a ChatGPT account." },
    ]),
  ], { open: 0 })],
  ["finished", () => panel("A turn that just ended well (Luch is pleased for a moment in the app)", [
    convo(12, { title: "pong", state: "idle" }, [
      { id: 1, kind: "user", text: "Reply with exactly: pong" },
      { id: 2, kind: "assistant", text: "pong" },
    ]),
  ], { open: 0 })],
];

for (const [name, run] of panels) {
  if (only === null || only.split(",").includes(name)) await run();
}
