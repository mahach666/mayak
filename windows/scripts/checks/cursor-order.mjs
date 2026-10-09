// Node check: onCursor records wasInIsland before the state machine hears the pointer cross.
// The petit and home transitions read wasInIsland; written after mouseEntered(), a transition
// made right away would see the pointer as still outside and start the close countdown.
// island.ts needs the DOM, so this reads its source rather than running it.
import { readFileSync } from "node:fs";

const src = readFileSync(new URL("../../src/island/island.ts", import.meta.url), "utf8");
const assert = (c, m) => { if (!c) { console.error("FAIL", m); process.exit(1); } };

const start = src.indexOf("  onCursor(");
assert(start >= 0, "onCursor found");
const end = src.indexOf("\n  }\n", start) >= 0 ? src.indexOf("\n  }\n", start) : src.indexOf("\n  }\r\n", start);
const body = src.slice(start, end);

const write = body.indexOf("this.wasInIsland = inIsland;");
const entered = body.indexOf("this.fsm.mouseEntered()");
const left = body.indexOf("this.fsm.mouseLeft()");
assert(write >= 0 && entered >= 0 && left >= 0, "onCursor writes the flag and calls both FSM hooks");
assert(body.split("this.wasInIsland = inIsland;").length === 2, "the flag is written once");
assert(write < entered && write < left, "the flag is written before mouseEntered() and mouseLeft()");
assert(!/if \(inIsland && !this\.wasInIsland\)|if \(!inIsland && this\.wasInIsland\)/.test(body),
  "the crossing tests read the value from before the write");
console.log("cursor-order: ok");
