import assert from "node:assert/strict";
import {
  normalizeConfig, nextPhase, phaseMs, minutesLeft, clock, msToNextMinute,
  phaseLabel, phaseCardTitle, startRun, advanceRun, stopRun, encodeReloadEntry, decodeReloadEntry,
} from "../../src/island/pomodoro.ts";
import { shouldOpenForAlert } from "../../src/island/focus.ts";

const cfg = { work: 25, short: 5, long: 15, cycles: 4 };
const MIN = 60_000;

// normalizeConfig: listed values pass, anything else falls back to the default.
assert.deepEqual(normalizeConfig({ pomodoroWork: 50, pomodoroShort: 10, pomodoroLong: 30, pomodoroCycles: 2 }),
  { work: 50, short: 10, long: 30, cycles: 2 });
assert.deepEqual(normalizeConfig({}), cfg);
assert.deepEqual(normalizeConfig({ pomodoroWork: 0, pomodoroShort: NaN, pomodoroLong: "15", pomodoroCycles: 99 }), cfg);
assert.deepEqual(normalizeConfig({ pomodoroWork: 24 }), cfg);

// nextPhase: the full round at cycles = 4.
const seq = [];
let p = { phase: "work", cycle: 1 };
for (let i = 0; i < 9; i++) { seq.push(`${p.phase}${p.cycle}`); p = nextPhase(p.phase, p.cycle, cfg); }
assert.deepEqual(seq, ["work1", "short1", "work2", "short2", "work3", "short3", "work4", "long4", "work1"]);
// cycles = 2
const c2 = { ...cfg, cycles: 2 };
assert.deepEqual(nextPhase("work", 2, c2), { phase: "long", cycle: 2 });
assert.deepEqual(nextPhase("long", 2, c2), { phase: "work", cycle: 1 });

// phaseMs
assert.equal(phaseMs("work", cfg), 25 * MIN);
assert.equal(phaseMs("short", cfg), 5 * MIN);
assert.equal(phaseMs("long", cfg), 15 * MIN);

// minutesLeft: rounds up, at least 1 while running, 0 after the end.
assert.equal(minutesLeft(25 * MIN, 1_000), 25);
assert.equal(minutesLeft(25 * MIN, 0), 25);
assert.equal(minutesLeft(MIN, MIN - 1_000), 1);
assert.equal(minutesLeft(MIN, MIN - 1), 1);
assert.equal(minutesLeft(MIN, MIN), 0);
assert.equal(minutesLeft(MIN, 5 * MIN), 0);

// clock: MM:SS rounded up to the second, 00:00 after the end.
assert.equal(clock(25 * MIN, 0), "25:00");
assert.equal(clock(25 * MIN, 1), "25:00");
assert.equal(clock(25 * MIN, 1_000), "24:59");
assert.equal(clock(90 * MIN, 0), "90:00");
assert.equal(clock(1_000, 2_000), "00:00");

// msToNextMinute: the time until minutesLeft changes, plus 25 ms of slack.
assert.equal(msToNextMinute(25 * MIN, 0), MIN + 25);
assert.equal(msToNextMinute(25 * MIN, 1_000), 59_000 + 25);
assert.equal(msToNextMinute(MIN, 2 * MIN), 0);

// labels
assert.equal(phaseLabel("work", 2, cfg), "Work · cycle 2 of 4");
assert.equal(phaseLabel("short", 2, cfg), "Short break");
assert.equal(phaseLabel("long", 4, cfg), "Long break");
assert.equal(phaseCardTitle("short", 1, cfg), "Break · 5 min");
assert.equal(phaseCardTitle("long", 4, cfg), "Long break · 15 min");
assert.equal(phaseCardTitle("work", 2, cfg), "Work · 25 min · cycle 2 of 4");

// startRun
assert.deepEqual(startRun(cfg, 1_000), { phase: "work", cycle: 1, endsAt: 1_000 + 25 * MIN, records: [] });

// advanceRun: work → short hands the held records over and clears them.
const rec = { key: "k", project: "mayak", kind: "finished", at: 5 };
const run1 = { phase: "work", cycle: 1, endsAt: 25 * MIN, records: [rec] };
const a1 = advanceRun(run1, cfg, 25 * MIN);
assert.deepEqual(a1.run, { phase: "short", cycle: 1, endsAt: 30 * MIN, records: [] });
assert.deepEqual(a1.held, [rec]);
assert.equal(a1.title, "Break · 5 min");
// A break holds nothing.
const a2 = advanceRun(a1.run, cfg, 30 * MIN);
assert.deepEqual(a2.held, []);
assert.equal(a2.title, "Work · 25 min · cycle 2 of 4");
// Sleep: advanced hours late, exactly once, and the next phase counts from now.
const late = advanceRun(run1, cfg, 25 * MIN + 3 * 3_600_000);
assert.equal(late.run.phase, "short");
assert.equal(late.run.endsAt, 25 * MIN + 3 * 3_600_000 + 5 * MIN);
// A length changed mid-phase only applies to the next phase.
const changed = advanceRun(run1, { ...cfg, short: 10 }, 25 * MIN);
assert.equal(changed.run.endsAt, 25 * MIN + 10 * MIN);

// stopRun: work returns its records, a break returns none.
assert.deepEqual(stopRun(run1), [rec]);
assert.deepEqual(stopRun({ phase: "short", cycle: 1, endsAt: 0, records: [rec] }), []);

// shouldOpenForAlert
const o = (muting, paused, letWaits) => ({ muting, paused, letWaits });
assert.equal(shouldOpenForAlert("finished", o(false, false, true)), true);
assert.equal(shouldOpenForAlert("finished", o(true, false, true)), false);
assert.equal(shouldOpenForAlert("error", o(true, false, true)), false);
assert.equal(shouldOpenForAlert("permission", o(true, false, true)), true);
assert.equal(shouldOpenForAlert("input", o(true, false, false)), false);
assert.equal(shouldOpenForAlert("permission", o(false, true, true)), false);

// encodeReloadEntry / decodeReloadEntry: what a language reload keeps (batch spec §F).
const kept = { phase: "short", cycle: 3, endsAt: 1_700_000_000_000, records: [rec] };
const card = { title: "Break · 5 min", records: [rec] };
const none = { reloaded: false, run: null, phaseCard: null, phaseCardPending: false };
const entry = (run, phaseCard = null, phaseCardPending = false) =>
  decodeReloadEntry(encodeReloadEntry({ run, phaseCard, phaseCardPending }));
// Round trips: a run, a run with a held card, a card with no run, nothing at all.
assert.deepEqual(entry(kept), { reloaded: true, run: kept, phaseCard: null, phaseCardPending: false });
assert.deepEqual(entry(run1), { reloaded: true, run: run1, phaseCard: null, phaseCardPending: false });
assert.deepEqual(entry(kept, card, true), { reloaded: true, run: kept, phaseCard: card, phaseCardPending: true });
assert.deepEqual(entry(kept, card, false), { reloaded: true, run: kept, phaseCard: card, phaseCardPending: false });
assert.deepEqual(entry(null, card, true), { reloaded: true, run: null, phaseCard: card, phaseCardPending: true });
assert.deepEqual(entry(null), { reloaded: true, run: null, phaseCard: null, phaseCardPending: false });
assert.deepEqual(JSON.parse(encodeReloadEntry({ run: null, phaseCard: null, phaseCardPending: false })), { reloaded: true });
// A card with no records is still a card (releaseHeldCard drops it); pending with no card is not.
assert.deepEqual(entry(null, { title: "", records: [] }, true).phaseCard, { title: "", records: [] });
assert.equal(entry(null, null, true).phaseCardPending, false);
// No entry, or one that is not an object: a real launch.
assert.equal(decodeReloadEntry(null), null);
assert.equal(decodeReloadEntry(""), null);
assert.equal(decodeReloadEntry("{not json"), null);
assert.equal(decodeReloadEntry("null"), null);
assert.equal(decodeReloadEntry("[]"), null);
assert.equal(decodeReloadEntry("7"), null);
// An entry written before the marker (the run alone) still decodes: the run comes back, no marker.
assert.deepEqual(decodeReloadEntry(JSON.stringify(kept)), { ...none, run: kept });
assert.deepEqual(decodeReloadEntry("{}"), none);
// A bad run is dropped, and only the run.
const withRun = (patch) => decodeReloadEntry(JSON.stringify({ ...kept, reloaded: true, phaseCard: card, phaseCardPending: true, ...patch }));
for (const patch of [
  { phase: "nap" }, { cycle: 0 }, { cycle: 1.5 }, { endsAt: "soon" }, { endsAt: null }, { records: "none" },
  { records: [{ ...rec, kind: "spam" }] }, { records: [{ ...rec, at: "now" }] }, { records: [{ ...rec, project: 7 }] },
]) {
  assert.deepEqual(withRun(patch), { reloaded: true, run: null, phaseCard: card, phaseCardPending: true }, JSON.stringify(patch));
}
// Only the known fields come back, whatever else the stored object carried.
assert.deepEqual(withRun({ extra: true, records: [{ ...rec, extra: 1 }] }), { reloaded: true, run: kept, phaseCard: card, phaseCardPending: true });
assert.deepEqual(withRun({ phaseCard: { ...card, extra: 1, records: [{ ...rec, extra: 1 }] } }).phaseCard, card);
// A malformed card is dropped (and is no longer pending); the run is kept.
for (const phaseCard of [
  null, "card", [], 7, { records: [rec] }, { title: 7, records: [rec] }, { title: "x" }, { title: "x", records: "none" },
  { title: "x", records: [{ ...rec, kind: "spam" }] }, { title: "x", records: [{ ...rec, at: Infinity }] },
]) {
  assert.deepEqual(withRun({ phaseCard }), { reloaded: true, run: kept, phaseCard: null, phaseCardPending: false }, JSON.stringify(phaseCard));
}
// Only a literal true counts.
assert.equal(withRun({ reloaded: "yes" }).reloaded, false);
assert.equal(withRun({ reloaded: 1 }).reloaded, false);
assert.equal(withRun({ phaseCardPending: "yes" }).phaseCardPending, false);

console.log("pomodoro-check: all passed");
