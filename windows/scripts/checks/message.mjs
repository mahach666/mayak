// Texts built in Rust (batch spec §F): core/message.ts turns a { code, params, fallback } into the
// text of the chosen language, and falls back to Rust's English for a code it does not know.
// Run: node --experimental-strip-types message-check.mjs
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const src = new URL("../../src/core/", import.meta.url).href;
const { messageText, isRustMessage } = await import(src + "message.ts");
const { format } = await import(src + "i18n-core.ts");
const dicts = {};
for (const lang of ["en", "ru", "de", "fr", "es", "zh", "ja"]) dicts[lang] = (await import(`${src}i18n/${lang}.ts`))[lang];

// The app's wrapper: the dictionary's text when it has the code (own keys only), else null.
const trOf = (dict) => (key, vars) => (Object.hasOwn(dict, key) ? format(dict[key], vars) : null);

// 1. A known code is translated, with its params filled in verbatim.
const notObject = {
  code: "msg.hook.notObject",
  params: { path: "C:\\Users\\dev\\.claude\\settings.json", app: "Mayak" },
  fallback: "C:\\Users\\dev\\.claude\\settings.json isn't a JSON object — Mayak won't touch it.",
};
assert.equal(messageText(notObject, trOf(dicts.en)), notObject.fallback, "English is Rust's own sentence");
const deText = messageText(notObject, trOf(dicts.de));
assert.notEqual(deText, notObject.fallback, "de is translated");
assert.ok(deText.includes("C:\\Users\\dev\\.claude\\settings.json") && deText.includes("Mayak"), deText);
assert.ok(!/\{\w+\}/.test(deText), `no placeholder left: ${deText}`);

// 2. The OS error tail stays as Rust sent it, in every language.
const readFailed = {
  code: "msg.hook.readFailed",
  params: { path: "C:\\x\\hooks.json", err: "Der Prozess kann nicht auf die Datei zugreifen. (os error 32)" },
  fallback: "Can't read C:\\x\\hooks.json: Der Prozess kann nicht auf die Datei zugreifen. (os error 32)",
};
for (const [lang, dict] of Object.entries(dicts)) {
  const text = messageText(readFailed, trOf(dict));
  assert.ok(text.includes(readFailed.params.err) && text.includes(readFailed.params.path), `${lang}: ${text}`);
}
assert.equal(messageText(readFailed, trOf(dicts.en)), readFailed.fallback);

// 3. An unknown code shows the fallback, never the raw key; so does a prototype name.
const future = { code: "msg.hook.fromANewerBuild", params: { path: "C:\\x" }, fallback: "Something new about C:\\x." };
for (const dict of Object.values(dicts)) assert.equal(messageText(future, trOf(dict)), future.fallback);
assert.equal(messageText({ code: "constructor", params: {}, fallback: "fb" }, trOf(dicts.ru)), "fb");
// No params at all (an older Rust, or a hand-made message): still a sentence.
assert.equal(messageText({ code: "msg.hook.coucouRemoved", fallback: "x" }, trOf(dicts.en)), "Hook entries from Coucou are removed too.");

// 4. Params are substituted, also several and repeated ones.
const unusable = { code: "msg.codex.relayUnusable", params: { app: "Mayak" }, fallback: "" };
const en = messageText(unusable, trOf(dicts.en));
assert.equal(
  en,
  "Mayak's relay isn't available, so entries written with its short (8.3) path can't be recognised. Restart Mayak to restore the relay, then Remove.",
);
assert.equal(messageText({ code: "msg.hotkey.osError", params: { err: "Zugriff verweigert" }, fallback: "Zugriff verweigert" }, trOf(dicts.ja)), "Zugriff verweigert");

// 4b. English texts say exactly what Rust's fallbacks say (sentences copied from the Rust tests).
const tEn = trOf(dicts.en);
for (const [code, params, fallback] of [
  ["msg.codex.pathBadChar", { path: "C:\\Users\\a&b\\mayak-hook.exe", char: "&" },
    "C:\\Users\\a&b\\mayak-hook.exe can't be used as a Codex hook command: it contains '&'. The relay path must be a drive letter followed by folder names made only of letters, digits, '.', '_', '~' and '-'."],
  ["msg.codex.unknownKey", { key: "state", app: "Mayak" },
    'hooks.json has the top-level key "state". Codex only accepts "description" and "hooks" there, so Mayak won\'t touch it.'],
  ["msg.codex.eventNotList", { event: "Stop", app: "Mayak" }, 'hooks.json: "Stop" isn\'t a list of hook groups — Mayak won\'t touch it.'],
  ["msg.hook.unknownTarget", { target: "cursor" }, 'unknown target "cursor" (expected "claude" or "codex")'],
  ["msg.hook.noPreview", { target: "codex" }, "There is no codex preview to apply. Open the preview again."],
  ["msg.hook.lineEndings", { style: "CRLF" }, "Line endings will be normalised to CRLF."],
  ["msg.hook.badShape", { path: "C:\\x\\settings.json", key: "hooks", app: "Mayak" },
    'C:\\x\\settings.json: "hooks" isn\'t the shape Claude Code expects — Mayak won\'t touch it.'],
  ["msg.hook.relayMissing", { path: "C:\\b\\mayak-hook.exe", app: "Mayak" }, "The relay C:\\b\\mayak-hook.exe is not in place yet. Restart Mayak."],
  ["msg.health.watcherFailed", { folder: "projects", err: "gone" }, "projects watcher: gone"],
  ["msg.health.threadFailed", { err: "access denied" }, "thread failed to start: access denied"],
  ["msg.hotkey.unknownKey", {}, "Unknown key: use a letter, a digit, F1–F24 or Space."],
  ["msg.hotkey.unsupported", {}, "Global shortcut is not available on this platform yet"],
]) {
  assert.equal(messageText({ code, params, fallback: "" }, tEn), fallback, code);
}

// 5. Errors of rejected commands are recognised as messages; other errors are not.
assert.ok(isRustMessage(notObject));
assert.ok(!isRustMessage(new Error("not running inside Mayak")));
assert.ok(!isRustMessage("Can't read x"));
assert.ok(!isRustMessage(null));
assert.ok(!isRustMessage({ code: 1, fallback: "x" }));

// 6. Every code Rust sends has an English text, and every msg.* key is one Rust sends.
const rustDir = fileURLToPath(new URL("../../src-tauri/src", import.meta.url));
const rustFiles = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? rustFiles(join(dir, e.name)) : e.name.endsWith(".rs") ? [join(dir, e.name)] : [],
  );
const codes = new Set();
for (const file of rustFiles(rustDir)) {
  for (const m of readFileSync(file, "utf8").matchAll(/"(msg\.[a-z]+\.[A-Za-z0-9]+)"/g)) codes.add(m[1]);
}
const keys = Object.keys(dicts.en).filter((k) => k.startsWith("msg."));
for (const code of codes) assert.ok(keys.includes(code), `en.ts has no ${code}`);
for (const key of keys) assert.ok(codes.has(key), `${key} is never sent by Rust`);

console.log(`message-check: all passed (${codes.size} codes)`);
