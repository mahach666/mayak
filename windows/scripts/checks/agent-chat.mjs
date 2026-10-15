// core/agent-chat.ts: the pure choices of the CLI chat view (src-tauri/src/chat), and the contract
// between its wire types and the dictionaries and the feature flag.
// Run: node --experimental-strip-types agent-chat.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import {
  AGENT_KINDS, HOME_FOLDER, MAX_ATTACHMENTS, agentCommand, canSend, chatBotState, chooseAgentKind, clockText, compactTokens, modeAccent, modeHintKey, modeShortKey,
  sideStatus, swapDraft, toolLabel, turnTools, folderName, isBusy, keepMode, keepModel, loginCommand, messageSig, modeKey,
  normalizeError, swapAttached, withAttachments, pendingPermissions, reconcileActive, stateColor, tabTitle, agentNoteText,
} from "../../src/core/agent-chat.ts";
import { CLICK_GUARD_MS, clickCounts } from "../../src/island/answers.ts";
import { en } from "../../src/core/i18n/en.ts";
import { de } from "../../src/core/i18n/de.ts";
import { es } from "../../src/core/i18n/es.ts";
import { fr } from "../../src/core/i18n/fr.ts";
import { ja } from "../../src/core/i18n/ja.ts";
import { ru } from "../../src/core/i18n/ru.ts";
import { zh } from "../../src/core/i18n/zh.ts";
const dicts = { en, de, es, fr, ja, ru, zh };
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

// What Luch shows of the open chat.
const chat = (state, messages = []) => ({ state, messages });
const toolMsg = (name, status) => ({ id: 1, kind: "tool", toolId: "t", name, summary: "", status, output: null });
const errMsg = (code) => ({ id: 2, kind: "error", code, text: "x" });
assert.equal(chatBotState(null, false), "idle");
assert.equal(chatBotState(chat("idle"), false), "idle");
assert.equal(chatBotState(chat("idle"), true), "finished", "a turn that just ended well");
assert.equal(chatBotState(chat("working"), true), "thinking", "no tool runs: the answer is being written");
assert.equal(chatBotState(chat("working", [toolMsg("Bash", "ok")]), false), "thinking", "a tool that ended is not running");
assert.equal(chatBotState(chat("working", [toolMsg("Bash", "running")]), false), "working");
assert.equal(chatBotState(chat("working", [toolMsg("mcp__mayak__ask_user", "running")]), false), "question");
assert.equal(chatBotState(chat("waiting"), false), "approval");
assert.equal(chatBotState(chat("working", [perm("pending")]), false), "approval", "a request waits for a click");
assert.equal(chatBotState(chat("failed", [errMsg("failed")]), false), "error");
assert.equal(chatBotState(chat("failed", [errMsg("notLoggedIn")]), true), "error");
assert.equal(chatBotState(chat("failed", [errMsg("rateLimit")]), false), "ratelimit");
assert.equal(chatBotState(chat("failed", [errMsg("rateLimit"), errMsg("failed")]), false), "error", "the last failure counts");
assert.equal(toolLabel("mcp__mayak__ask_user"), "mayak · ask_user");
assert.equal(toolLabel("mcp__srv__a__b"), "srv · a__b");
assert.equal(toolLabel("Bash"), "Bash");
// Every access mode the adapters offer has its word and its sentence in the dictionary.
for (const mode of ["ask", "plan", "readOnly", "workspaceWrite", "proposeOnly"]) {
  assert.ok(modeKey(mode) in en && modeHintKey(mode) in en, mode);
}
assert.equal(modeHintKey("somethingNew"), null);
// The composer's pill: a one-word name and a colour for each mode; the widest access is not the calm colour.
for (const mode of ["ask", "plan", "readOnly", "workspaceWrite", "proposeOnly"]) {
  assert.ok(modeShortKey(mode) in en, mode);
  assert.ok(en[modeShortKey(mode)].length <= 12, `${mode}: the pill's word is short`);
}
assert.equal(modeShortKey("somethingNew"), null);
assert.notEqual(modeAccent("ask").color, modeAccent("workspaceWrite").color);
assert.equal(new Set(["ask", "plan", "readOnly", "workspaceWrite", "proposeOnly"].map((m) => modeAccent(m).color)).size, 5, "five modes, five colours");
assert.equal(modeAccent("somethingNew").color, "#B0B5BE", "an unknown mode is neutral");

// The column under Luch: the tools of the turn under way (or the last), its clock, the token counts,
// and the line that says what the agent is doing.
const user = (id) => ({ id, kind: "user", text: "x" });
assert.equal(turnTools(null), 0);
assert.equal(turnTools({ messages: [user(1), toolMsg("Bash", "ok"), toolMsg("Read", "running")] }), 2);
assert.equal(turnTools({ messages: [toolMsg("Bash", "ok"), user(2), toolMsg("Read", "ok"), text("done")] }), 1, "only since the last message");
assert.equal(turnTools({ messages: [toolMsg("Bash", "ok"), user(2)] }), 0, "a new turn starts at 0");
assert.equal(clockText(0), "0:00");
assert.equal(clockText(7_400), "0:07");
assert.equal(clockText(12 * 60_000 + 5_000), "12:05");
assert.equal(clockText(3_723_000), "1:02:03");
assert.equal(clockText(-5), "0:00");
assert.equal(compactTokens(950), "950");
assert.equal(compactTokens(26_155), "26k");
assert.equal(compactTokens(1_240_000), "1.2M");
assert.equal(compactTokens(12_600_000), "13M");
const side = (state, messages = [], turnEnded = null) => sideStatus({ state, messages, turnEnded });
assert.deepEqual(sideStatus(null), { key: "agentchat.side.ready" });
assert.deepEqual(side("idle"), { key: "agentchat.side.ready" }, "a chat with no turn yet");
assert.deepEqual(side("idle", [user(1), text("pong")], 5), { key: "agentchat.side.done" });
assert.deepEqual(side("idle", [user(1), { id: 3, kind: "notice", code: "stopped", text: "Stopped." }], 5), { key: "agentchat.side.stopped" });
assert.deepEqual(side("working"), { key: "agentchat.side.thinking" });
assert.deepEqual(side("working", [toolMsg("Bash", "ok")]), { key: "agentchat.side.thinking" }, "a tool that ended is not running");
assert.deepEqual(side("working", [toolMsg("mcp__mayak__list_sessions", "running")]), { key: "agentchat.side.running", vars: { tool: "mayak · list_sessions" } });
assert.deepEqual(side("working", [toolMsg("mcp__mayak__ask_user", "running")]), { key: "agentchat.side.asking" });
assert.deepEqual(side("waiting"), { key: "agentchat.side.waiting" });
assert.deepEqual(side("working", [perm("pending")]), { key: "agentchat.side.waiting" });
assert.deepEqual(side("failed", [errMsg("failed")]), { key: "agentchat.side.failed" });
assert.deepEqual(side("failed", [errMsg("notLoggedIn")]), { key: "agentchat.side.signedOut" });
assert.deepEqual(side("failed", [errMsg("rateLimit")]), { key: "agentchat.side.limit" });
for (const key of ["ready", "thinking", "running", "waiting", "asking", "done", "stopped", "failed", "limit", "signedOut", "took", "tools", "context", "contextOf", "contextSent"]) {
  assert.ok(`agentchat.side.${key}` in en, key);
}

// The field keeps what was typed for each chat when the view goes to another one (a request in
// another chat takes it there at once): a message is only ever sent to the chat it was written in.
const drafts = new Map();
assert.equal(swapDraft(drafts, 1, 2, "half a message for 1"), "", "chat 2 had nothing typed");
assert.equal(swapDraft(drafts, 2, null, "  "), "", "blank is not kept");
assert.equal(drafts.has(2), false);
assert.equal(swapDraft(drafts, null, 1, "a new chat's idea"), "half a message for 1", "back to chat 1: its text");
assert.equal(drafts.has(1), false, "taken back out");
assert.equal(swapDraft(drafts, 1, null, ""), "a new chat's idea", "and the new chat's");
assert.equal(drafts.size, 0);

// The view exists in the layout, and the feature is on in this build (the API chat stays off).
assert.ok(VIEW_LAYOUTS.agentchat.height >= 440, "the chat needs room for its log");
assert.ok(VIEW_LAYOUTS.agentchat.height <= 560 - 60, "and stays well inside the island's window (core/layout.ts PANEL_H)");
assert.ok(VIEW_LAYOUTS.agentchat.botY !== null && VIEW_LAYOUTS.agentchat.botY < 120, "Luch at the top of its column, the run's column under it");
const features = readFileSync(fileURLToPath(new URL("../../src/core/features.ts", import.meta.url)), "utf8");
assert.match(features, /export const CLI_CHAT_ENABLED = true;/, "the CLI chat is on in this build");
assert.match(features, /export const CHAT_ENABLED = false;/, "the API chat and the file drop stay off");

// agentNoteText: the agent's own line under Luch (the MCP server's set_status), only while its turn runs.
assert.equal(agentNoteText(null), "");
assert.equal(agentNoteText({ state: "working", agentNote: "  Running the tests " }), "Running the tests");
assert.equal(agentNoteText({ state: "waiting", agentNote: "Asking" }), "Asking");
for (const state of ["idle", "failed"]) assert.equal(agentNoteText({ state, agentNote: "stale" }), "", state);
assert.equal(agentNoteText({ state: "working", agentNote: null }), "");
assert.equal(agentNoteText({ state: "working" }), "", "an older Rust sends no agentNote");

// Dropped files: the message is the typed text and then the inbox paths, and a drop alone is a message.
assert.equal(withAttachments("  look at this ", []), "look at this", "no files: just the text");
assert.equal(
  withAttachments("what is wrong?", [{ name: "a.png", path: "C:\\inbox\\a.png" }]),
  "what is wrong?\n\nAttached file (read it from this path):\n- C:\\inbox\\a.png",
);
assert.equal(
  withAttachments("", [{ name: "a", path: "/i/a" }, { name: "b c", path: "/i/b c" }]),
  "Attached files (read them from these paths):\n- /i/a\n- /i/b c",
  "files without text, paths with spaces kept whole",
);
assert.equal(canSend(withAttachments("", [{ name: "a", path: "/i/a" }]), "idle", true, false), true, "a drop alone can be sent");
assert.equal(canSend(withAttachments("   ", []), "idle", true, false), false, "nothing typed, nothing dropped");
assert.equal(MAX_ATTACHMENTS, 8);
const stash = new Map();
const one = [{ name: "a", path: "/i/a" }];
assert.deepEqual(swapAttached(stash, null, 1, one), [], "chat 1 had no files; the new chat's file waits for it");
assert.deepEqual(swapAttached(stash, 1, null, []), one, "back to the new chat: its file, and chat 1 keeps none");
assert.deepEqual(swapAttached(stash, null, 2, []), [], "chat 2 had no files");
assert.equal(stash.size, 0, "nothing stale is kept");

// No project is the default: an empty cwd, in every dictionary; the home folder is the "~" option.
assert.equal(HOME_FOLDER, "~");
for (const [lang, dict] of Object.entries(dicts)) {
  for (const key of ["agentchat.none", "agentchat.home", "agentchat.drop", "agentchat.drop.many", "agentchat.drop.remove"]) {
    assert.ok(typeof dict[key] === "string" && dict[key] !== "", `${lang} has ${key}`);
  }
  assert.ok(dict["agentchat.drop.many"].includes("{max}"), `${lang}: the limit is named`);
}
const view = readFileSync(fileURLToPath(new URL("../../src/views/agentchat.ts", import.meta.url)), "utf8");
assert.ok(!/defaultRoot/.test(view), "a new chat does not take the latest session's project");
assert.match(view, /cwd: draft\.root \?\? ""/, "a new chat without a chosen project opens with no project");

console.log("agent-chat-check: all passed");
