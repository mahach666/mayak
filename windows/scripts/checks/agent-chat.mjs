// core/agent-chat.ts: the pure choices of the CLI chat view (src-tauri/src/chat), and the contract
// between its wire types and the dictionaries and the feature flag.
// Run: node --experimental-strip-types agent-chat.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import {
  AGENT_KINDS, agentCommand, canSend, chooseAgentKind, folderName, isBusy, keepMode, keepModel, loginCommand, messageSig, modeKey,
  normalizeError, pendingPermissions, reconcileActive, stateColor, tabTitle,
} from "../../src/core/agent-chat.ts";
import { CLICK_GUARD_MS, clickCounts } from "../../src/island/answers.ts";
import { en } from "../../src/core/i18n/en.ts";
import { VIEW_LAYOUTS } from "../../src/core/layout.ts";

const info = (kind, over = {}) => ({
  kind, label: kind, found: true, verified: true, verifiedWith: "1", models: [{ id: "", label: "" }, { id: "haiku", label: "Haiku" }],
  modes: ["ask", "plan"], login: "ready", ...over,
});

// The agent to start with: the saved one while installed, else the first installed, else none.
const agents = [info("claudeCode", { found: false }), info("codex"), info("geminiCli", { verified: false })];
assert.equal(chooseAgentKind(agents, "geminiCli"), "geminiCli");
assert.equal(chooseAgentKind(agents, "claudeCode"), "codex", "a saved agent that is gone gives way");
assert.equal(chooseAgentKind(agents, null), "codex");
assert.equal(chooseAgentKind([info("codex", { found: false })], "codex"), null);
assert.equal(chooseAgentKind([], null), null);

// Changing the agent keeps the model and mode only when the new agent has them.
assert.equal(keepModel(info("codex"), "haiku"), "haiku");
assert.equal(keepModel(info("codex"), "opus"), "");
assert.equal(keepModel(undefined, "haiku"), "");
assert.equal(keepMode(info("codex"), "plan"), "plan");
assert.equal(keepMode(info("codex"), "readOnly"), "ask", "the new agent's first mode");
assert.equal(keepMode(undefined, "plan"), "");

// The open conversation survives a list that still has it, and gives way to a new chat when it is gone.
const list = [{ id: 1 }, { id: 4 }];
assert.equal(reconcileActive(list, 4), 4);
assert.equal(reconcileActive(list, 9), null);
assert.equal(reconcileActive(list, null), null);

// A message goes only with text, an agent, no turn running and no call under way.
assert.equal(canSend("hi", "idle", true, false), true);
assert.equal(canSend("hi", undefined, true, false), true, "a new chat has no state yet");
assert.equal(canSend("hi", "failed", true, false), true, "a failed chat can be tried again");
assert.equal(canSend("  \n", "idle", true, false), false);
assert.equal(canSend("hi", "working", true, false), false);
assert.equal(canSend("hi", "waiting", true, false), false);
assert.equal(canSend("hi", "idle", false, false), false);
assert.equal(canSend("hi", "idle", true, true), false);
assert.equal(isBusy("working") && isBusy("waiting"), true);
assert.equal(isBusy("idle") || isBusy("failed") || isBusy(undefined), false);

// Errors come as Rust's {code, message}, an Error, or a string.
assert.deepEqual(normalizeError({ code: "cliMissing", message: "no claude" }), { code: "cliMissing", message: "no claude" });
assert.deepEqual(normalizeError(new Error("boom")), { code: "internal", message: "boom" });
assert.deepEqual(normalizeError("text"), { code: "internal", message: "text" });
assert.equal(normalizeError(null).code, "internal");
assert.equal(normalizeError({ code: 3 }).code, "internal");

// Names: the command each CLI is found by, and how to sign in (none known for Gemini CLI).
assert.deepEqual(AGENT_KINDS.map(agentCommand), ["claude", "codex", "gemini", "cursor-agent"]);
assert.equal(loginCommand("claudeCode"), "claude auth login");
assert.equal(loginCommand("codex"), "codex login");
assert.equal(loginCommand("cursorAgent"), "cursor-agent login");
assert.equal(loginCommand("geminiCli"), null);

// Every mode this page names has a word in the dictionary.
for (const mode of ["ask", "plan", "readOnly", "workspaceWrite", "proposeOnly"]) {
  const key = modeKey(mode);
  assert.ok(key && key in en, `no dictionary key for the mode ${mode}`);
}
assert.equal(modeKey("somethingNew"), null);
assert.equal(modeKey("constructor"), null, "inherited names are not modes");

// The modes the Rust adapters list are exactly the ones above (no mode without a word).
const rustDir = fileURLToPath(new URL("../../src-tauri/src/chat/", import.meta.url));
const offered = new Set();
for (const file of ["claude_code.rs", "codex.rs", "gemini.rs", "cursor.rs"]) {
  const text = readFileSync(rustDir + file, "utf8");
  const m = text.match(/fn modes\(&self\) -> &'static \[&'static str\] \{[\s\S]*?&\[([^\]]*)\]/);
  assert.ok(m, `${file} has no modes()`);
  for (const id of m[1].matchAll(/"([A-Za-z]+)"/g)) offered.add(id[1]);
}
assert.deepEqual([...offered].sort(), ["ask", "plan", "proposeOnly", "readOnly", "workspaceWrite"]);
for (const id of offered) assert.ok(modeKey(id), `the adapters offer ${id}, the page has no word for it`);
// And never a mode in which the CLI approves something itself (no request, so no click).
for (const never of ["bypassPermissions", "dontAsk", "acceptEdits", "yolo", "auto_edit", "danger-full-access", "force"]) assert.ok(!offered.has(never), never);

// A state's dot, a tab's title, a folder's name.
assert.equal(stateColor("working"), "#3B9EFF");
assert.equal(stateColor("waiting"), "#F5A524");
assert.equal(stateColor("failed"), "#F4505E");
assert.equal(new Set(["idle", "working", "waiting", "failed"].map(stateColor)).size, 4);
assert.equal(tabTitle({ title: "  fix the build \n" }, "New chat"), "fix the build");
assert.equal(tabTitle({ title: " " }, "New chat"), "New chat");
assert.equal(folderName("C:\\work\\app\\"), "app");
assert.equal(folderName("/home/me/app"), "app");
assert.equal(folderName("C:\\"), "C:");

// A line is drawn again only when what shows in it changed.
const tool = (status, summary = "ls", output = null) => ({ id: 3, kind: "tool", toolId: "t", name: "Bash", summary, status, output });
assert.equal(messageSig(tool("running")), messageSig(tool("running")));
assert.notEqual(messageSig(tool("running")), messageSig(tool("ok")));
assert.notEqual(messageSig(tool("failed", "ls", "x")), messageSig(tool("failed", "ls", "y")));
const text = (s) => ({ id: 1, kind: "assistant", text: s });
assert.notEqual(messageSig(text("Hel")), messageSig(text("Hello")));
assert.equal(messageSig(text("Hello")), messageSig(text("Hello")));
const perm = (status) => ({ id: 2, kind: "permission", requestId: "1.1", tool: "Bash", summary: "ls", detail: null, toolUseId: null, status });
assert.notEqual(messageSig(perm("pending")), messageSig(perm("allowed")));
assert.deepEqual(pendingPermissions({ messages: [perm("allowed"), perm("pending"), tool("ok")] }).map((m) => m.status), ["pending"]);
assert.deepEqual(pendingPermissions(null), []);

// The buttons of a request ignore a click made right after they were drawn (a double click must not
// answer the next request), and a click for another request.
const bound = { requestId: "1.1", builtAt: 1000 };
assert.equal(clickCounts(bound, "1.1", 1000 + CLICK_GUARD_MS - 1), false);
assert.equal(clickCounts(bound, "1.1", 1000 + CLICK_GUARD_MS), true);
assert.equal(clickCounts(bound, "1.2", 5000), false);

// The view exists in the layout, and the feature is off in this build.
assert.ok(VIEW_LAYOUTS.agentchat.height >= 260, "the chat needs room");
const features = readFileSync(fileURLToPath(new URL("../../src/core/features.ts", import.meta.url)), "utf8");
assert.match(features, /export const CLI_CHAT_ENABLED = false;/, "the CLI chat stays off until it is enabled on purpose");

console.log("agent-chat-check: all passed");
