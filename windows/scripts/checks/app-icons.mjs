// views/app-icon-art.ts: every app of apps.rs and every common app has artwork, and an app name or exe name finds its icon.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { SVG_ICONS, styledIconFor, styledIconForApp, styledIconId } from "../../src/views/app-icon-art.ts";

// every id in the Rust app table has an icon, and every icon is 16×16 line art
const rust = readFileSync(new URL("../../src-tauri/src/apps.rs", import.meta.url), "utf8");
const ids = [...rust.matchAll(/^\s*\(\s*"([a-z]+)",\s*"[^"]+",\s*Kind::/gm)].map((m) => m[1]);
assert.deepEqual(ids.sort(), ["antigravity", "claude", "codex", "cursor", "vscode", "windsurf", "zed"]);
// the common apps a user adds (own-apps, ideas §30)
const common = ["notepad", "notepadpp", "terminal", "powershell", "explorer", "obsidian", "chrome", "edge", "firefox", "gitbash"];
assert.deepEqual(Object.keys(SVG_ICONS).sort(), [...ids, ...common].sort(), "no icon without a name");
for (const id of [...ids, ...common]) {
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
  ["Antigravity", "antigravity"], ["Antigravity.exe", "antigravity"],
  [String.raw`C:\Users\me\AppData\Local\Programs\antigravity\Antigravity.exe`, "antigravity"],
  ["Claude", "claude"], ["claude.exe", "claude"], ["Codex", "codex"], ["codex.exe", "codex"],
  ["notepad.exe", "notepad"], ["Notepad", "notepad"], [String.raw`C:\Windows\System32\notepad.exe`, "notepad"],
  ["notepad++.exe", "notepadpp"], ["Notepad++", "notepadpp"],
  ["wt.exe", "terminal"], ["WindowsTerminal.exe", "terminal"], ["Windows Terminal", "terminal"],
  ["powershell.exe", "powershell"], ["pwsh.exe", "powershell"], ["Windows PowerShell", "powershell"],
  ["explorer.exe", "explorer"], ["File Explorer", "explorer"], ["Windows Explorer", "explorer"],
  ["Obsidian.exe", "obsidian"], ["Obsidian", "obsidian"],
  ["chrome.exe", "chrome"], ["Google Chrome", "chrome"],
  ["msedge.exe", "edge"], ["Microsoft Edge", "edge"],
  ["firefox.exe", "firefox"], ["Firefox", "firefox"], ["Mozilla Firefox", "firefox"],
  ["git-bash.exe", "gitbash"], ["Git Bash", "gitbash"],
];
for (const [input, id] of cases) {
  assert.equal(styledIconId(input), id, input);
  assert.equal(styledIconFor(input), SVG_ICONS[id], input);
}

// no partial or loose matches: unknown apps keep their monogram
for (const input of ["", "   ", "wordpad", "notepad+", "chromium", "edge.exe.lnk", "codex-cli", "vs", "code insiders", "Code - Insiders.exe", "claude desktop", "zed.exe.bak", "agy", "antigravity-ide", "toString", "__proto__", "hasOwnProperty"]) {
  assert.equal(styledIconId(input), null, input);
  assert.equal(styledIconFor(input), null, input);
}
// a user app: its name first, then its exe; a renamed app keeps the icon of its exe
assert.equal(styledIconForApp("Browser", "chrome.exe"), SVG_ICONS.chrome);
assert.equal(styledIconForApp("Notepad", "chrome.exe"), SVG_ICONS.notepad);
assert.equal(styledIconForApp("My tool", "tool.exe"), null);
assert.equal(styledIconForApp("My tool", null), null);
console.log(`app-icons: ok (${ids.length} apps, ${common.length} common apps, ${cases.length} names)`);
