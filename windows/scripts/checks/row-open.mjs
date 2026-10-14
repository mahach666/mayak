// core/row-open.ts: the «↗» of a sessions-list row (ideas §43), which rows get it and its tooltip.
import assert from "node:assert/strict";
import { rowOpens, rowOpenTitle } from "../../src/core/row-open.ts";
import { format } from "../../src/core/i18n-core.ts";
import { en } from "../../src/core/i18n/en.ts";
import { ru } from "../../src/core/i18n/ru.ts";

const tr = (dict) => (key, vars) => {
  assert.ok(key in dict, `no ${key}`);
  return format(dict[key], vars);
};

// Every session row Open can open: Claude, Codex, Antigravity, and a known Codex session whose
// turn ran without hooks (it keeps its codex:<id> key).
for (const key of ["claude:1", "codex:019a-77", "antigravity:abc", "codex:silent-turn"]) {
  assert.equal(rowOpens(key), true, key);
}
// A pure silent Codex row is no real session; a «Recent» row has its own Resume and Copy.
assert.equal(rowOpens("silent:019a-77"), false);
assert.equal(rowOpens("recent:claude:1"), false);
assert.equal(rowOpens("recent:fold"), false);
// Agent Orchestrator's fold opens and closes; it opens no session.
assert.equal(rowOpens("ao:fold"), false);

// The tooltip names the host; an unknown host gets the plain "Open".
assert.equal(rowOpenTitle("cursor", "Cursor", tr(en)), "Open in Cursor");
assert.equal(rowOpenTitle("vsCode", "VS Code", tr(ru)), "Открыть в VS Code");
assert.equal(rowOpenTitle("unknown", "Other", tr(en)), "Open");
assert.equal(rowOpenTitle("unknown", "Другое", tr(ru)), "Открыть");
// A session the Agents chat runs opens in the chat, whatever its host.
assert.equal(rowOpenTitle("unknown", "Other", tr(en), true), "Open in the Agents chat");
assert.equal(rowOpenTitle("cursor", "Cursor", tr(ru), true), "Открыть в чате «Агенты»");
assert.equal(rowOpenTitle("cursor", "Cursor", tr(en), false), "Open in Cursor");

console.log("row-open-check: all passed");
