// Dev harness: the Agents tab built with the real buildAgentChat() and styles, on a fake backend, in
// the states of the CLI chat: a new chat with the picker, a turn that is working, a request waiting
// for a click, the same after the click, a CLI that is not signed in, and an agent that is missing.
// Not part of the app bundle.

import "../src/style.css";
import { buildAgentChat, type AgentChatBackend } from "../src/views/agentchat";
import type { ViewActions } from "../src/views/views";
import { State } from "../src/core/state";
import type { AgentInfo, ChatMsg, Convo, ConvoSummary, OpenRequest } from "../src/core/agent-chat";

const AGENTS: AgentInfo[] = [
  { kind: "claudeCode", label: "Claude Code", found: true, verified: true, verifiedWith: "2.1.296",
    models: [{ id: "", label: "" }, { id: "fable", label: "Fable" }, { id: "opus", label: "Opus" }, { id: "sonnet", label: "Sonnet" }, { id: "haiku", label: "Haiku" }],
    modes: ["ask", "plan", "acceptEdits"], login: "ready" },
  { kind: "codex", label: "Codex", found: true, verified: true, verifiedWith: "0.162.1",
    models: [{ id: "", label: "" }, { id: "gpt-6.1-sol", label: "GPT-6.1-Sol" }], modes: ["readOnly", "workspaceWrite"], login: "ready" },
  { kind: "geminiCli", label: "Gemini CLI", found: true, verified: false, verifiedWith: null,
    models: [{ id: "", label: "" }, { id: "flash", label: "Flash" }], modes: ["ask", "plan"], login: "unknown" },
  { kind: "cursorAgent", label: "Cursor Agent", found: false, verified: false, verifiedWith: null,
    models: [{ id: "", label: "" }], modes: ["proposeOnly"], login: "unknown" },
];

let rev = 1;
function convo(id: number, over: Partial<Convo>, messages: ChatMsg[]): Convo {
  return {
    id, agent: "claudeCode", cwd: String.raw`C:\work\mayak`, model: "haiku", mode: "ask", title: "", session: "s", usedModel: null,
    state: "idle", status: null, loginHint: false, rev: rev++, messages, ...over,
  };
}

function backend(convos: Convo[], answered: { allowed?: boolean } = {}): AgentChatBackend {
  return {
    agents: async () => AGENTS,
    list: async () => convos.map((c): ConvoSummary => ({ id: c.id, agent: c.agent, cwd: c.cwd, title: c.title, state: c.state, rev: c.rev })),
    get: async (id) => convos.find((c) => c.id === id) ?? null,
    open: async (r: OpenRequest) => {
      void r;
      return 99;
    },
    set: async () => {},
    send: async () => {},
    permission: async (_id, allow) => {
      answered.allowed = allow;
      return true;
    },
    cancel: async () => {},
    close: async () => {},
    onChanged: () => {},
  };
}

const actions = new Proxy({}, { get: () => () => {} }) as ViewActions;
const root = document.getElementById("panels")!;

async function panel(title: string, convos: Convo[], open: number | null) {
  const wrap = document.createElement("div");
  wrap.className = "panel";
  const h2 = document.createElement("h2");
  h2.textContent = title;
  const stage = document.createElement("div");
  stage.className = "stage";
  wrap.append(h2, stage);
  root.append(wrap);
  const host = buildAgentChat(actions, backend(convos));
  host.el.classList.add("on");
  stage.append(host.el);
  State.subscribe(() => host.sync());
  host.setActive?.(true);
  // The view opens on a new chat; the second tab is the conversation of this panel.
  await new Promise((r) => setTimeout(r, 80));
  if (open !== null) host.el.querySelectorAll<HTMLElement>(".ac-chip")[open]?.click();
  await new Promise((r) => setTimeout(r, 80));
  host.sync();
}

await panel("A new chat: the picker (Cursor Agent is not installed, Gemini CLI is unverified)", [], null);

await panel("Working: a tool ran, the answer is being written, the CLI is retrying", [
  convo(1, { title: "Why does the build fail?", state: "working", status: "Reconnecting... 2/5 (unexpected status 401)" }, [
    { id: 1, kind: "user", text: "Why does the build fail on CI but not on my machine?" },
    { id: 2, kind: "tool", toolId: "t1", name: "Bash", summary: "git log --oneline -5", status: "ok", output: null },
    { id: 3, kind: "tool", toolId: "t2", name: "Read", summary: String.raw`C:\work\mayak\.github\workflows\ci.yml`, status: "running", output: null },
    { id: 4, kind: "assistant", text: "The workflow pins Node 20, but package.json asks for 22.6+, so the checks that strip types fail there." },
  ]),
], 0);

await panel("Waiting for a click: the CLI asks to run a command (Allow / Deny are the only way to answer)", [
  convo(2, { title: "Write the probe file", state: "waiting" }, [
    { id: 1, kind: "user", text: "Use the Bash tool to run exactly: cmd /c echo hello > probe_out.txt" },
    { id: 2, kind: "tool", toolId: "t1", name: "Bash", summary: "cmd /c echo hello > probe_out.txt", status: "running", output: null },
    { id: 3, kind: "permission", requestId: "2.1", tool: "Bash", summary: "cmd /c echo hello > probe_out.txt",
      detail: "Run the requested cmd echo redirect to probe_out.txt", toolUseId: "t1", status: "pending" },
  ]),
], 0);

await panel("After the clicks: allowed, then denied; stopped by the user; a model notice", [
  convo(3, { title: "Two commands", state: "idle" }, [
    { id: 1, kind: "user", text: "Run two commands." },
    { id: 2, kind: "permission", requestId: "3.1", tool: "Bash", summary: "cmd /c echo real > real_probe.txt", detail: null, toolUseId: null, status: "allowed" },
    { id: 3, kind: "tool", toolId: "t1", name: "Bash", summary: "cmd /c echo real > real_probe.txt", status: "ok", output: null },
    { id: 4, kind: "permission", requestId: "3.2", tool: "Bash", summary: "cmd /c echo no > denied_probe.txt", detail: null, toolUseId: null, status: "denied" },
    { id: 5, kind: "tool", toolId: "t2", name: "Bash", summary: "cmd /c echo no > denied_probe.txt", status: "failed", output: "The user denied this in Mayak." },
    { id: 6, kind: "notice", code: "apiKey", text: "ANTHROPIC_API_KEY" },
    { id: 7, kind: "notice", code: "stopped", text: "Stopped." },
  ]),
], 0);

await panel("Not signed in (the CLI's own message, then how to sign in); three chats open", [
  convo(4, { title: "Hello", state: "failed", loginHint: true, agent: "codex", mode: "readOnly", model: "" }, [
    { id: 1, kind: "user", text: "Hello" },
    { id: 2, kind: "error", code: "notLoggedIn", text: "unexpected status 401 Unauthorized: Missing bearer or basic authentication in header" },
  ]),
  convo(5, { title: "Refactor the parser", state: "working" }, []),
  convo(6, { title: "Review the diff", state: "waiting" }, []),
], 0);
