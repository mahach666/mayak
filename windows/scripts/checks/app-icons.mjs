// views/app-icon-art.ts: every app of apps.rs has artwork, and an app name or exe name finds its icon.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { SVG_ICONS, styledIconFor, styledIconId } from "../../src/views/app-icon-art.ts";

// every id in the Rust app table has an icon, and every icon is 16×16 line art
const rust = readFileSync(new URL("../../src-tauri/src/apps.rs", import.meta.url), "utf8");
const ids = [...rust.matchAll(/^\s*\(\s*"([a-z]+)",\s*"[^"]+",\s*Kind::/gm)].map((m) => m[1]);
assert.deepEqual(ids.sort(), ["claude", "codex", "cursor", "vscode", "windsurf", "zed"]);
for (const id of ids) {
  const s = SVG_ICONS[id];
  assert.ok(s?.startsWith('<svg viewBox="0 0 16 16"'), `${id}: 16×16 svg`);
  assert.ok(s.includes('fill="none"') && s.includes('stroke-linecap="round"'), `${id}: line art`);
}

// names, exe names and paths, any case
const cases = [
  ["cursor", "cursor"], ["Cursor", "cursor"], ["Cursor.exe", "cursor"],
  [String.raw`C:\Users\me\AppData\Local\Programs\cursor\Cursor.exe`, "cursor"],
  ["code", "vscode"], ["Code.exe", "vscode"], ["Visual Studio Code", "vscode"], ["VS Code", "vscode"],
  ["  windsurf  ", "windsurf"], ["Windsurf.EXE", "windsurf"], ["ZED", "zed"], ["/usr/bin/zed", "zed"],
  ["Claude", "claude"], ["claude.exe", "claude"], ["Codex", "codex"], ["codex.exe", "codex"],
];
for (const [input, id] of cases) {
  assert.equal(styledIconId(input), id, input);
  assert.equal(styledIconFor(input), SVG_ICONS[id], input);
}

// no partial or loose matches: unknown apps keep their monogram
for (const input of ["", "   ", "notepad", "codex-cli", "vs", "code insiders", "Code - Insiders.exe", "claude desktop", "zed.exe.bak", "toString", "__proto__", "hasOwnProperty"]) {
  assert.equal(styledIconId(input), null, input);
  assert.equal(styledIconFor(input), null, input);
}
console.log(`app-icons: ok (${ids.length} apps, ${cases.length} names)`);
