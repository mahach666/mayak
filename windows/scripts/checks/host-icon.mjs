// views/host-icon.ts: every session host Rust sends has its app icon in the session row's badge,
// and an unknown host (or a value the front end does not know) keeps the letter.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { hostIconId } from "../../src/views/host-icon.ts";
import { SVG_ICONS } from "../../src/views/app-icon-art.ts";

// the hosts of bridge.ts are the ones Rust's HostKind sends (camelCase), all mapped
const bridge = readFileSync(new URL("../../src/core/bridge.ts", import.meta.url), "utf8");
const hosts = [...bridge.match(/export type Host = ([^;]+);/)[1].matchAll(/"([A-Za-z]+)"/g)].map((m) => m[1]);
const rust = readFileSync(new URL("../../src-tauri/src/observe/keys.rs", import.meta.url), "utf8");
const kinds = [...rust.match(/pub enum HostKind \{([^}]+)\}/)[1].matchAll(/^\s*([A-Z][A-Za-z]+),/gm)]
  .map((m) => m[1][0].toLowerCase() + m[1].slice(1));
assert.deepEqual([...hosts].sort(), [...kinds].sort(), "bridge.ts Host matches Rust HostKind");

const expected = {
  claudeDesktop: "claude",
  cursor: "cursor",
  vsCode: "vscode",
  codexDesktop: "codex",
  antigravityIde: "antigravity",
  antigravityCli: "antigravity",
  unknown: null,
};
assert.deepEqual([...hosts].sort(), Object.keys(expected).sort(), "every host is listed here");
for (const host of hosts) {
  const id = hostIconId(host);
  assert.equal(id, expected[host], host);
  if (id !== null) assert.ok(Object.hasOwn(SVG_ICONS, id), `${host}: ${id} has artwork`);
}
// a host a newer Rust might send, or prototype names: the letter stays
for (const host of ["", "terminal", "windowsTerminal", "toString", "__proto__", "hasOwnProperty"]) {
  assert.equal(hostIconId(host), null, host);
}
console.log(`host-icon: ok (${hosts.length} hosts)`);
