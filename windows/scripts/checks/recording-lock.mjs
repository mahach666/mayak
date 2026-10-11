// settings/recording.ts: only one shortcut recorder records at a time (ideas §2: the settings
// page has two, and a click on the second does not end the first by itself).
import assert from "node:assert/strict";
import { createRecordingLock } from "../../src/settings/recording.ts";

const lock = createRecordingLock();
const settings = { hotkey: "Alt+M", nextHotkey: "Alt+Shift+M" };
const keyListeners = new Set();
const pauses = [];

/** The recorder of settings/main.ts hotkeyRows, without the DOM. */
function recorder(combo) {
  let recording = false;
  let attempt = 0;
  async function finish(chosen) {
    if (!recording) return;
    recording = false;
    lock.end(finish);
    keyListeners.delete(onKey);
    if (chosen != null) settings[combo] = chosen;
    pauses.push(false);
  }
  function onKey(key) {
    void finish(key === "Escape" ? null : key);
  }
  async function click() {
    if (recording) return;
    recording = true;
    const mine = ++attempt;
    await lock.start(finish);
    if (!recording || mine !== attempt) return;
    pauses.push(true);
    keyListeners.add(onKey);
  }
  return { click, finish, isRecording: () => recording };
}

const press = (key) => [...keyListeners].forEach((l) => l(key));

const island = recorder("hotkey");
const next = recorder("nextHotkey");

// Row A records, then row B is clicked: A ends unchanged, only B records.
await island.click();
await next.click();
assert.equal(island.isRecording(), false, "the first recording ended");
assert.equal(next.isRecording(), true);
assert.equal(keyListeners.size, 1, "one keydown listener");
assert.equal(lock.active(), next.finish);
// A's resume reached Rust before B's pause, so the shortcuts stay released while B records.
assert.deepEqual(pauses.splice(0), [true, false, true]);

// The key press sets only B's shortcut.
press("Ctrl+F9");
assert.deepEqual(settings, { hotkey: "Alt+M", nextHotkey: "Ctrl+F9" });
assert.equal(lock.active(), null);
assert.equal(keyListeners.size, 0);
assert.deepEqual(pauses.splice(0), [false]);

// Esc ends the one recording only.
await island.click();
press("Escape");
assert.equal(island.isRecording(), false);
assert.deepEqual(settings, { hotkey: "Alt+M", nextHotkey: "Ctrl+F9" });
assert.equal(lock.active(), null);

// Clicking the row already recording changes nothing.
await next.click();
await next.click();
assert.equal(keyListeners.size, 1);
await next.finish(null);

// A recorder ending after another took over does not clear the new one.
const a = async () => {};
const b = async () => {};
await lock.start(a);
await lock.start(b);
lock.end(a);
assert.equal(lock.active(), b);
lock.end(b);
assert.equal(lock.active(), null);

console.log("one recording at a time");
