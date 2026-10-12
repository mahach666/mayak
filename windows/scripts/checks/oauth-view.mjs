// settings/oauth-view.ts: every connection state, Connect problem code (src-tauri oauth.rs
// StartProblem::code) and flow end maps to a text of the dictionaries, and the Connect controls
// sit in the integrations section of the «Integrations» page.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { connColor, connText, connectLabel, flowNotice, startProblem } from "../../src/settings/oauth-view.ts";
import { PAGES } from "../../src/settings/pages.ts";
import { en } from "../../src/core/i18n/en.ts";

const has = (key) => assert.ok(key in en, `en.ts has no ${key}`);

for (const state of ["none", "connected", "reconnect"]) {
  has(connText(state));
  has(connectLabel(state));
  assert.match(connColor(state), /^#[0-9a-f]{6}$/);
}
assert.equal(connText("connected"), "set.oauthStateConnected");
assert.equal(connText("reconnect"), "set.oauthStateReconnect");
assert.equal(connectLabel("none"), "set.oauthConnect");
assert.equal(connectLabel("reconnect"), "set.oauthReconnect");
assert.notEqual(connColor("connected"), connColor("none"));

// The codes Rust sends, each with its own notice; an unknown one is a failure, never a crash.
const rust = readFileSync(new URL("../../src-tauri/src/oauth.rs", import.meta.url), "utf8");
const codes = [...rust.matchAll(/StartProblem::\w+ => "(\w+)"/g)].map((m) => m[1]);
assert.deepEqual(codes.sort(), ["failed", "invalidHost", "invalidId", "notRegistered", "paused", "selfManaged"]);
for (const code of codes) has(startProblem(code).key);
assert.equal(startProblem("notRegistered").key, "set.oauthNotRegistered");
assert.equal(startProblem("notRegistered").tone, "warn");
assert.equal(startProblem("selfManaged").key, "set.oauthSelfManaged");
assert.equal(startProblem("unexpected").key, "set.oauthError");
assert.equal(startProblem("toString").key, "set.oauthError", "inherited names are not codes");

for (const state of ["connected", "disconnected", "expired", "denied", "error"]) has(flowNotice(state).key);
assert.equal(flowNotice("connected").tone, "ok");
assert.equal(flowNotice("expired").key, "set.oauthExpired");
assert.equal(flowNotice("denied").key, "set.oauthDenied");
assert.equal(flowNotice("whatever").key, "set.oauthError");

// The Connect controls are built inside integrationsSection, which sits on the «Integrations» page only.
const main = readFileSync(new URL("../../src/settings/main.ts", import.meta.url), "utf8");
const section = main.slice(main.indexOf("function integrationsSection("));
assert.ok(section.includes("oauthRows(") && section.includes("clientIdRow(") && section.includes("gitlabAddressRow("));
assert.deepEqual(PAGES.filter((p) => p.sections.includes("integrations")).map((p) => p.id), ["integrations"]);

console.log("oauth-view-check: all passed");
