# Mayak Pomodoro Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the one-shot focus mode with a pomodoro timer on its own island tab, with a "mute alerts during work" switch and a phase sign on Mochi.

**Architecture:** A pure module (`island/pomodoro.ts`) owns the phase arithmetic and the run transitions (start, advance, stop), so they can be checked under Node without DOM or Tauri. The island holds the run in `State.pomodoro`, with one `setTimeout` per phase end. A minute tick runs only while the island is visible, and a second tick only while the Focus tab is open. Rust only gains five settings fields.

**Tech Stack:** Tauri 2 (Rust, `windows/src-tauri`), TypeScript with no framework (`windows/src`), Canvas drawing for Mochi (`mochi/engine.ts`).

**Spec:** `docs/superpowers/specs/2026-10-07-mayak-pomodoro-design.md`

## Global Constraints

- 0 % CPU while the island is hidden: no ticks. The only timer is the one `setTimeout` to the end of the current phase.
- No new dependencies (npm or crates). Mochi is drawn in code: no images, no Rive/Lottie.
- Existing views do not change, except the header, Settings → General and the `focus-done` card that the spec replaces.
- UI text is in English. Doc comments are in English, like the surrounding code.
- Never log prompt text, file contents or full paths.
- Commit messages: one line, no trailers, no `Co-Authored-By`, and never mention Claude or AI.
- Stage by explicit path, never `git add -A` or `git add .`.
- Build environment for Rust: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target`.
- Checks: `cd windows && cargo test --workspace` (0 warnings), `npx tsc --noEmit`, `npx vite build`.
- Pure TS checks run under Node outside the repo: `node --experimental-strip-types C:/tmp/mayak-spike/<name>.mjs`. A module that such a check imports may only use `import type` for local imports, because Node cannot resolve extension-less relative runtime imports.
- Allowed values (spec §3): Work 15/20/25/30/45/50/60 (default 25), Short break 3/5/10 (5), Long break 10/15/20/30 (15), Long break after 2/3/4/5/6 (4). Mute defaults on; Let waiting agents through defaults on.
- JSON keys are camelCase: `pomodoroWork`, `pomodoroShort`, `pomodoroLong`, `pomodoroCycles`, `pomodoroMute`. `focusLetWaitsThrough` stays. `focusMinutes` stays in Rust and in the TS type but is no longer used.

## Review Focus

1. A hand-edited `settings.json` with `pomodoroWork: 0` (or a missing or odd value) must never create a 0-minute phase. A 0 ms phase would advance in a tight loop. `normalizeConfig` falls back to the default (Task 1 test).
2. Waking the PC hours after a phase should have ended advances exactly once, and the next phase counts from now (Task 1 `advanceRun` test).
3. Changing a length in the middle of a phase keeps the current `endsAt`, and only the next phase uses the new value (Task 1 test: `advanceRun` takes the config at advance time).
4. Stopping during a break shows no card; stopping during work shows a card only when something was held (Task 1 `stopRun` test, Task 2 wiring).
5. The minutes label never reads "0" while a phase runs (Task 1 `minutesLeft` test). The compact label is empty, not "0", when no run is on (Task 4).

---

### Task 1: Settings fields and the pure pomodoro module

**Files:**
- Modify: `windows/src-tauri/src/settings.rs` (struct `Settings` near line 50, `Default` near line 115, tests near line 305)
- Modify: `windows/src/core/state.ts` (interface `Settings` near line 119, `DEFAULT_SETTINGS` near line 144)
- Create: `windows/src/island/pomodoro.ts`
- Modify: `windows/src/island/focus.ts` (`shouldOpenForAlert` options)
- Modify: `windows/src/island/island.ts` and `windows/src/island/hooks.ts` (the two `shouldOpenForAlert` call sites)
- Test: `C:/tmp/mayak-spike/pomodoro-check.mjs` (outside the repo)

**Interfaces:**
- Produces (TS, `island/pomodoro.ts`):
  - `type Phase = "work" | "short" | "long"`
  - `interface PomodoroConfig { work: number; short: number; long: number; cycles: number }`
  - `interface PomodoroRun { phase: Phase; cycle: number; endsAt: number; records: FocusRecord[] }`
  - `const POMODORO_CHOICES: { work: readonly number[]; short: readonly number[]; long: readonly number[]; cycles: readonly number[] }`
  - `normalizeConfig(raw: { pomodoroWork?: unknown; pomodoroShort?: unknown; pomodoroLong?: unknown; pomodoroCycles?: unknown }): PomodoroConfig`
  - `nextPhase(phase: Phase, cycle: number, cfg: PomodoroConfig): { phase: Phase; cycle: number }`
  - `phaseMs(phase: Phase, cfg: PomodoroConfig): number`
  - `minutesLeft(endsAt: number, now: number): number`
  - `clock(endsAt: number, now: number): string`
  - `msToNextMinute(endsAt: number, now: number): number`
  - `phaseLabel(phase: Phase, cycle: number, cfg: PomodoroConfig): string`
  - `phaseCardTitle(phase: Phase, cycle: number, cfg: PomodoroConfig): string`
  - `startRun(cfg: PomodoroConfig, now: number): PomodoroRun`
  - `advanceRun(run: PomodoroRun, cfg: PomodoroConfig, now: number): { run: PomodoroRun; held: FocusRecord[]; title: string }`
  - `stopRun(run: PomodoroRun): FocusRecord[]`
- Produces (TS, `island/focus.ts`): `shouldOpenForAlert(kind: AlertKind, o: { muting: boolean; paused: boolean; letWaits: boolean }): boolean`
- Produces (TS, `core/state.ts` `Settings`): `pomodoroWork: number; pomodoroShort: number; pomodoroLong: number; pomodoroCycles: number; pomodoroMute: boolean`
- Produces (Rust, `settings::Settings`): `pomodoro_work: u32, pomodoro_short: u32, pomodoro_long: u32, pomodoro_cycles: u32, pomodoro_mute: bool`

- [ ] **Step 1: Write the failing Rust tests**

Add to the `tests` module of `windows/src-tauri/src/settings.rs`:

```rust
    #[test]
    fn an_older_settings_file_loads_with_the_default_pomodoro() {
        // A settings.json from before the pomodoro: it has focusMinutes and no pomodoro keys.
        let old = r#"{"soundEnabled":true,"soundVolume":0.12,"autoCloseInterval":15,"absenceInterval":180,"activeIntegrations":[],"screen":"primary","autostart":false,"model":"claude-opus-5","focusMinutes":50,"focusLetWaitsThrough":false}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(
            (s.pomodoro_work, s.pomodoro_short, s.pomodoro_long, s.pomodoro_cycles, s.pomodoro_mute),
            (25, 5, 15, 4, true)
        );
        assert!(!s.focus_let_waits_through);
        let d = Settings::default();
        assert_eq!(
            (d.pomodoro_work, d.pomodoro_short, d.pomodoro_long, d.pomodoro_cycles, d.pomodoro_mute),
            (25, 5, 15, 4, true)
        );
    }

    #[test]
    fn the_pomodoro_settings_round_trip_in_camel_case() {
        let mut s = Settings::default();
        s.pomodoro_work = 50;
        s.pomodoro_short = 10;
        s.pomodoro_long = 30;
        s.pomodoro_cycles = 2;
        s.pomodoro_mute = false;
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["pomodoroWork"].as_u64(), Some(50));
        assert_eq!(v["pomodoroShort"].as_u64(), Some(10));
        assert_eq!(v["pomodoroLong"].as_u64(), Some(30));
        assert_eq!(v["pomodoroCycles"].as_u64(), Some(2));
        assert_eq!(v["pomodoroMute"].as_bool(), Some(false));
        let back: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(
            (back.pomodoro_work, back.pomodoro_short, back.pomodoro_long, back.pomodoro_cycles, back.pomodoro_mute),
            (50, 10, 30, 2, false)
        );
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cd windows && cargo test -p mayak --lib settings::tests 2>&1 | tail -5`
Expected: compile error, `no field pomodoro_work`.

- [ ] **Step 3: Add the Rust fields**

In `struct Settings`, after `focus_let_waits_through`:

```rust
    /// Pomodoro lengths in minutes and the work intervals before a long break (pomodoro spec §3).
    /// The island only offers the listed values and falls back to the default for anything else.
    /// A settings.json from before the pomodoro loads with 25 / 5 / 15 / 4.
    #[serde(default = "default_pomodoro_work")]
    pub pomodoro_work: u32,
    #[serde(default = "default_pomodoro_short")]
    pub pomodoro_short: u32,
    #[serde(default = "default_pomodoro_long")]
    pub pomodoro_long: u32,
    #[serde(default = "default_pomodoro_cycles")]
    pub pomodoro_cycles: u32,
    /// Agent alerts are held back during work intervals (pomodoro spec §2.3).
    #[serde(default = "default_true")]
    pub pomodoro_mute: bool,
```

Next to `default_focus_minutes`:

```rust
fn default_pomodoro_work() -> u32 {
    25
}

fn default_pomodoro_short() -> u32 {
    5
}

fn default_pomodoro_long() -> u32 {
    15
}

fn default_pomodoro_cycles() -> u32 {
    4
}
```

In `impl Default for Settings`, after `focus_let_waits_through: true,`:

```rust
            pomodoro_work: default_pomodoro_work(),
            pomodoro_short: default_pomodoro_short(),
            pomodoro_long: default_pomodoro_long(),
            pomodoro_cycles: default_pomodoro_cycles(),
            pomodoro_mute: true,
```

If another test builds a `Settings { … }` literal or compares a full JSON string, add the five fields there too.

- [ ] **Step 4: Run the Rust tests**

Run: `cd windows && cargo test --workspace 2>&1 | grep -E "^test result|^warning|^error"`
Expected: all pass, no warnings.

- [ ] **Step 5: Add the TS settings fields**

In `windows/src/core/state.ts`, interface `Settings`, after `focusLetWaitsThrough`:

```ts
  /** Pomodoro lengths in minutes and the work intervals before a long break (pomodoro spec §3). */
  pomodoroWork: number;
  pomodoroShort: number;
  pomodoroLong: number;
  pomodoroCycles: number;
  /** Agent alerts are held back during work intervals (pomodoro spec §2.3). */
  pomodoroMute: boolean;
```

Change the doc comment of `focusMinutes` to `/** Unused since the pomodoro replaced the one-shot focus; kept so older files round-trip. */`. In `DEFAULT_SETTINGS`, after `focusLetWaitsThrough: true,`:

```ts
  pomodoroWork: 25,
  pomodoroShort: 5,
  pomodoroLong: 15,
  pomodoroCycles: 4,
  pomodoroMute: true,
```

- [ ] **Step 6: Change the `shouldOpenForAlert` options**

In `windows/src/island/focus.ts`, replace `shouldOpenForAlert` with:

```ts
/**
 * Whether an agent alert may open the island and sound. Pause wins over everything. While
 * `muting` (a pomodoro work interval with Mute on), a wait gets through only with `letWaits`,
 * and a finished or error turn never does.
 */
export function shouldOpenForAlert(
  kind: AlertKind,
  o: { muting: boolean; paused: boolean; letWaits: boolean },
): boolean {
  if (o.paused) return false;
  if (!o.muting) return true;
  return focusKind(kind) === "waited" && o.letWaits;
}
```

Update the callers so they still compile, with the meaning unchanged for now (Task 2 switches them to `State.muting`):
- `island/hooks.ts`: `{ focus: State.focusOn, … }` becomes `{ muting: State.focusOn, … }`.
- `island/island.ts` `handleAlerts`: `{ focus: true, paused: false, letWaits }` becomes `{ muting: true, paused: false, letWaits }`, and `{ focus: State.focusOn, … }` becomes `{ muting: State.focusOn, … }`.

- [ ] **Step 7: Write the failing Node check**

Create `C:/tmp/mayak-spike/pomodoro-check.mjs`:

```js
import assert from "node:assert/strict";
import {
  normalizeConfig, nextPhase, phaseMs, minutesLeft, clock, msToNextMinute,
  phaseLabel, phaseCardTitle, startRun, advanceRun, stopRun,
} from "file:///C:/Users/HP/source/repos/mayak/windows/src/island/pomodoro.ts";
import { shouldOpenForAlert } from "file:///C:/Users/HP/source/repos/mayak/windows/src/island/focus.ts";

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

console.log("pomodoro-check: all passed");
```

Run: `node --experimental-strip-types C:/tmp/mayak-spike/pomodoro-check.mjs`
Expected: fails, `Cannot find module …/pomodoro.ts`.

- [ ] **Step 8: Write `island/pomodoro.ts`**

```ts
// Pomodoro (spec docs/superpowers/specs/2026-10-07-mayak-pomodoro-design.md): the phase arithmetic
// and the run transitions, with no DOM, no Tauri and no State, so they can be checked on their own.

import type { FocusRecord } from "./focus";

export type Phase = "work" | "short" | "long";

export interface PomodoroConfig {
  work: number;
  short: number;
  long: number;
  cycles: number;
}

/** A running pomodoro: the current phase, its work cycle (1…cycles), when it ends, what work held. */
export interface PomodoroRun {
  phase: Phase;
  cycle: number;
  endsAt: number;
  records: FocusRecord[];
}

/** What the Focus tab offers (spec §3); the first value of each list is not the default. */
export const POMODORO_CHOICES = {
  work: [15, 20, 25, 30, 45, 50, 60],
  short: [3, 5, 10],
  long: [10, 15, 20, 30],
  cycles: [2, 3, 4, 5, 6],
} as const;

const DEFAULTS: PomodoroConfig = { work: 25, short: 5, long: 15, cycles: 4 };
const MINUTE = 60_000;
/** A minute tick lands this long after the boundary, so the label has already changed. */
const TICK_SLACK_MS = 25;

function pick(value: unknown, choices: readonly number[], fallback: number): number {
  return typeof value === "number" && choices.includes(value) ? value : fallback;
}

/** Settings as stored; anything not in the lists (0, NaN, a string, 24) falls back to the default. */
export function normalizeConfig(raw: {
  pomodoroWork?: unknown;
  pomodoroShort?: unknown;
  pomodoroLong?: unknown;
  pomodoroCycles?: unknown;
}): PomodoroConfig {
  return {
    work: pick(raw.pomodoroWork, POMODORO_CHOICES.work, DEFAULTS.work),
    short: pick(raw.pomodoroShort, POMODORO_CHOICES.short, DEFAULTS.short),
    long: pick(raw.pomodoroLong, POMODORO_CHOICES.long, DEFAULTS.long),
    cycles: pick(raw.pomodoroCycles, POMODORO_CHOICES.cycles, DEFAULTS.cycles),
  };
}

/** Spec §2.1: work → short, the last work of a round → long, a break → the next work. */
export function nextPhase(phase: Phase, cycle: number, cfg: PomodoroConfig): { phase: Phase; cycle: number } {
  if (phase === "work") return { phase: cycle >= cfg.cycles ? "long" : "short", cycle };
  if (phase === "short") return { phase: "work", cycle: cycle + 1 };
  return { phase: "work", cycle: 1 };
}

export function phaseMs(phase: Phase, cfg: PomodoroConfig): number {
  return cfg[phase] * MINUTE;
}

/** Whole minutes left, rounded up: 1 until the very end, 0 once it is over. */
export function minutesLeft(endsAt: number, now: number): number {
  const left = endsAt - now;
  return left > 0 ? Math.ceil(left / MINUTE) : 0;
}

/** "MM:SS" left, rounded up to the second; "00:00" once it is over. */
export function clock(endsAt: number, now: number): string {
  const s = Math.max(0, Math.ceil((endsAt - now) / 1000));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

/** Delay until minutesLeft changes (plus a little slack); 0 when the phase is already over. */
export function msToNextMinute(endsAt: number, now: number): number {
  const left = endsAt - now;
  if (left <= 0) return 0;
  const r = left % MINUTE;
  return (r === 0 ? MINUTE : r) + TICK_SLACK_MS;
}

/** The Focus tab's phase line. */
export function phaseLabel(phase: Phase, cycle: number, cfg: PomodoroConfig): string {
  if (phase === "work") return `Work · cycle ${cycle} of ${cfg.cycles}`;
  return phase === "short" ? "Short break" : "Long break";
}

/** The title of the card that opens when `phase` starts (spec §2.2). */
export function phaseCardTitle(phase: Phase, cycle: number, cfg: PomodoroConfig): string {
  if (phase === "work") return `Work · ${cfg.work} min · cycle ${cycle} of ${cfg.cycles}`;
  return phase === "short" ? `Break · ${cfg.short} min` : `Long break · ${cfg.long} min`;
}

export function startRun(cfg: PomodoroConfig, now: number): PomodoroRun {
  return { phase: "work", cycle: 1, endsAt: now + phaseMs("work", cfg), records: [] };
}

/**
 * The phase that follows `run`, counted from `now` (a late timer after sleep advances once and
 * does not catch up). `held` is what the ending work interval held back; a break holds nothing.
 */
export function advanceRun(
  run: PomodoroRun,
  cfg: PomodoroConfig,
  now: number,
): { run: PomodoroRun; held: FocusRecord[]; title: string } {
  const next = nextPhase(run.phase, run.cycle, cfg);
  return {
    run: { phase: next.phase, cycle: next.cycle, endsAt: now + phaseMs(next.phase, cfg), records: [] },
    held: run.phase === "work" ? run.records : [],
    title: phaseCardTitle(next.phase, next.cycle, cfg),
  };
}

/** What a stop shows in its summary: the current work interval's records, none during a break. */
export function stopRun(run: PomodoroRun): FocusRecord[] {
  return run.phase === "work" ? run.records : [];
}
```

- [ ] **Step 9: Run every check**

Run:
```bash
node --experimental-strip-types C:/tmp/mayak-spike/pomodoro-check.mjs
cd windows && npx tsc --noEmit && npx vite build 2>&1 | tail -1
cd windows && cargo test --workspace 2>&1 | grep -E "^test result|^warning|^error"
```
Expected: `pomodoro-check: all passed`, tsc clean, build ok, all Rust tests pass with 0 warnings.

- [ ] **Step 10: Commit**

```bash
git add windows/src-tauri/src/settings.rs windows/src/core/state.ts windows/src/island/pomodoro.ts windows/src/island/focus.ts windows/src/island/island.ts windows/src/island/hooks.ts
git commit -m "feat(island): pomodoro settings and phase arithmetic"
```

---

### Task 2: The run in the island, the phase card and the strip

**Files:**
- Modify: `windows/src/core/state.ts` (focus fields near lines 167-174, getter `focusOn` near line 215)
- Modify: `windows/src/core/layout.ts` (`IslandViewName` near line 28, `VIEW_LAYOUTS["focus-done"]` near line 99)
- Modify: `windows/src/island/island.ts` (focus mode section near lines 448-512, `handleAlerts` near line 548, `handleSilentWake` near line 592, `reveal`, `decide` in `build()` near line 185, `syncStrip` near line 930, the `focusTimer` field near line 125)
- Modify: `windows/src/island/hooks.ts`, `windows/src/island/integrations.ts` (focus checks)
- Modify: `windows/src/views/views.ts` (`ViewActions.toggleFocusMode`, header `focusBtn`, `buildFocusDone`, registry `map.set("focus-done", …)`)
- Modify: `windows/src/main.ts` (tray case `"focus"`)
- Modify: `windows/src/style.css` (strip break colour next to `#strip-line.focus::after`)

**Interfaces:**
- Consumes (Task 1): `PomodoroRun`, `normalizeConfig`, `startRun`, `advanceRun`, `stopRun`, `shouldOpenForAlert` with `muting`, `addFocusRecord`, `focusSummary`, `focusCountsLine`, `focusProjectsLine`.
- Produces:
  - `State.pomodoro: PomodoroRun | null`
  - `State.pomodoroOn: boolean` (getter)
  - `State.muting: boolean` (getter: work phase and `settings.pomodoroMute`)
  - `State.phaseCard: { title: string; records: FocusRecord[] } | null`
  - `State.phaseCardPending: boolean`
  - `Island.togglePomodoro(): void` (public: the tray and the Focus tab call it)
  - `Island.checkPomodoro(): void` (public: advances when the phase is over, else reschedules; ticks call it)
  - `Island.flushPhaseCard(): boolean`
  - `ViewActions.togglePomodoro(): void` and `ViewActions.checkPomodoro(): void`, replacing `toggleFocusMode`
  - view name `"focus-phase"` replacing `"focus-done"`

- [ ] **Step 1: State**

In `core/state.ts`, replace the focus fields (`focusStartedAt`, `focusRecords`, `focusDone`, `focusDonePending`) and the `focusOn` getter with:

```ts
  /** The running pomodoro (pomodoro spec §2), null while off. Island.togglePomodoro owns it. */
  pomodoro: PomodoroRun | null = null;
  /** The card a phase change opens (spec §2.2): its title and what the work interval held. */
  phaseCard: { title: string; records: FocusRecord[] } | null = null;
  /** A phase card waits for an Allow/Deny card to close (spec §2.2). */
  phaseCardPending = false;
```

```ts
  /** A pomodoro runs (not the focused pill, which is focusTask). */
  get pomodoroOn(): boolean {
    return this.pomodoro != null;
  }

  /** Agent alerts are held back: a work interval with Mute on (spec §2.3). */
  get muting(): boolean {
    return this.pomodoro?.phase === "work" && this.settings.pomodoroMute;
  }
```

Import `type PomodoroRun` from `"../island/pomodoro"`. Keep the `FocusRecord` import.

- [ ] **Step 2: Layout**

In `core/layout.ts`, rename the view `"focus-done"` to `"focus-phase"` in `IslandViewName` and in `VIEW_LAYOUTS`, keeping the same entry: `"focus-phase": { height: 160, botX: 62, botY: null, botDiameter: 58, agentMode: "column" }`. Update its comment: `// The card a pomodoro phase change opens (pomodoro spec §2.2): the same place as the finished card.`

- [ ] **Step 3: The phase card view**

In `views/views.ts`, replace `buildFocusDone` with:

```ts
// ── Pomodoro phase ────────────────────────────────────────────────────────────

/** A phase change or a stop (State.phaseCard, pomodoro spec §2.2), with what work held back. */
function buildFocusPhase(actions: ViewActions): ViewHost {
  const title = h("div", { class: "title" });
  const counts = h("div", { class: "sub" });
  const projects = h("div", { class: "focus-projects sess-clamp" });
  const row = h("div", { class: "actions sess-actions" }, btn("OK", "primary", () => actions.collapse()));
  const el = h("div", { class: "view" }, card("indigo", stack(116, 16, title, counts, projects, row)));
  return {
    el,
    sync() {
      const shown = State.phaseCard;
      const records = shown?.records ?? [];
      const summary = focusSummary(records);
      title.textContent = shown?.title ?? "";
      counts.textContent = focusCountsLine(summary.counts);
      counts.style.display = records.length > 0 ? "" : "none";
      projects.textContent = focusProjectsLine(summary.projects);
      projects.style.display = records.length > 0 ? "" : "none";
    },
  };
}
```

In the registry, `map.set("focus-done", buildFocusDone(actions));` becomes `map.set("focus-phase", buildFocusPhase(actions));`. Remove `focusDoneTitle` from the import of `../island/focus`.

In `ViewActions`, replace `toggleFocusMode(): void;` and its comment with:

```ts
  /** The Focus tab's Start/Stop and the tray's Focus: start a pomodoro, or stop the running one. */
  togglePomodoro(): void;
  /** A tick of the Focus tab: move on if the phase is over (a timer late after sleep). */
  checkPomodoro(): void;
```

In `buildHeader`, the `focusBtn` stays until Task 3 but calls the new action, and its title no longer uses `focusLength`:

```ts
  const focusBtn = h("button", { title: "Focus", onclick: () => actions.togglePomodoro() }, svg(ICONS.target, 14));
```

```ts
      focusBtn.classList.toggle("on", State.pomodoroOn);
      focusBtn.title = State.pomodoroOn ? "Focus is on — click to stop it" : "Start a pomodoro";
```

Remove `focusLength` from the `../island/focus` import if nothing else uses it.

- [ ] **Step 4: The island: start, advance, stop, card**

In `island/island.ts`:

Imports: `import { addFocusRecord, shouldOpenForAlert, type FocusRecord } from "./focus";` and `import { advanceRun, normalizeConfig, startRun, stopRun } from "./pomodoro";` (drop `focusElapsedMinutes` and `focusLength`).

Replace the field `private focusTimer: number | null = null;` and its comment with:

```ts
  /** The one timer of a pomodoro: the end of the current phase (spec §1). No tick while hidden. */
  private phaseTimer: number | null = null;
```

Replace the whole `// ── Focus mode (night spec §A)` section (`toggleFocusMode`, `endFocus`, `flushFocusDone`) with:

```ts
  // ── Pomodoro (pomodoro spec) ────────────────────────────────────────────────

  /** The Focus tab's Start/Stop and the tray's Focus: start a pomodoro, or stop the running one. */
  togglePomodoro() {
    Sound.resume();
    if (State.pomodoro) {
      this.stopPomodoro();
      return;
    }
    State.pomodoro = startRun(normalizeConfig(State.settings), Date.now());
    this.schedulePhaseEnd();
    void Bridge.setFocusCheck(true);
    Sound.play("blip");
    State.notify();
  }

  /** Spec §2.4: a stop shows what the current work interval held, else nothing. */
  private stopPomodoro() {
    const run = State.pomodoro;
    if (!run) return;
    this.clearPhaseTimer();
    State.pomodoro = null;
    void Bridge.setFocusCheck(false);
    const held = stopRun(run);
    if (held.length > 0) this.queuePhaseCard("Focus stopped", held);
    State.notify();
  }

  /** Moves on when the current phase is over, else waits for its end again (spec §2.5). */
  checkPomodoro() {
    const run = State.pomodoro;
    if (!run) return;
    if (Date.now() >= run.endsAt) this.advancePhase();
    else this.schedulePhaseEnd();
  }

  private advancePhase() {
    const run = State.pomodoro;
    if (!run) return;
    const next = advanceRun(run, normalizeConfig(State.settings), Date.now());
    State.pomodoro = next.run;
    this.schedulePhaseEnd();
    this.queuePhaseCard(next.title, next.held);
    State.notify();
  }

  private schedulePhaseEnd() {
    this.clearPhaseTimer();
    const run = State.pomodoro;
    if (!run) return;
    this.phaseTimer = window.setTimeout(() => {
      this.phaseTimer = null;
      this.checkPomodoro();
    }, Math.max(0, run.endsAt - Date.now()));
  }

  private clearPhaseTimer() {
    if (this.phaseTimer != null) window.clearTimeout(this.phaseTimer);
    this.phaseTimer = null;
  }

  /**
   * Spec §2.2: the card for a phase change or a stop. Quiet mode drops it (no card, no sound).
   * While an Allow/Deny card is up it waits; a later card replaces its title and adds its records.
   */
  private queuePhaseCard(title: string, held: FocusRecord[]) {
    if (State.paused) {
      State.phaseCard = null;
      State.phaseCardPending = false;
      return;
    }
    const earlier = State.phaseCardPending ? State.phaseCard?.records ?? [] : [];
    State.phaseCard = { title, records: [...earlier, ...held] };
    State.phaseCardPending = true;
    if (State.pendingApproval == null) this.flushPhaseCard();
  }

  /** Shows a pending phase card with the finished sound. False when none is pending. */
  flushPhaseCard(): boolean {
    if (!State.phaseCardPending) return false;
    State.phaseCardPending = false;
    if (State.paused) {
      State.phaseCard = null;
      return false;
    }
    Sound.play("finish");
    this.alert("focus-phase", { silent: true });
    return true;
  }
```

In `build()`:
- `toggleFocusMode: () => this.toggleFocusMode(),` becomes:
  ```ts
  togglePomodoro: () => this.togglePomodoro(),
  checkPomodoro: () => this.checkPomodoro(),
  ```
- In `decide`, the line `if (!this.flushFocusDone()) this.setView(State.defaultView());` and its comment become:
  ```ts
  // A phase card that came while this card was up shows now.
  if (!this.flushPhaseCard()) this.setView(State.defaultView());
  ```

Search `island.ts` for `flushFocusDone` and replace every other call the same way.

- [ ] **Step 5: Muting in alerts, reveal and silent wake**

In `handleAlerts`, replace the focus block and the filter with:

```ts
    const letWaits = State.settings.focusLetWaitsThrough;
    const run = State.pomodoro;
    if (run && State.muting) {
      const at = Date.now();
      for (const a of fresh) {
        if (!shouldOpenForAlert(a.kind, { muting: true, paused: false, letWaits })) {
          addFocusRecord(run.records, { key: a.key, project: a.project, kind: a.kind, at });
        }
      }
    }
    const batch = fresh.filter(
      (a) => shouldOpenForAlert(a.kind, { muting: State.muting, paused: State.paused, letWaits }),
    );
```

Update the doc comment of `handleAlerts`: "During a muted work interval (pomodoro spec §2.3), what it holds back is recorded for the phase card, paused or not."

- `reveal()`: `if (State.focusOn) return;` becomes `if (State.muting) return;`, and the doc comment says "never during a muted work interval (pomodoro spec §2.3)".
- `handleSilentWake()`: `State.focusOn` becomes `State.muting`.
- `island/hooks.ts`: `{ muting: State.focusOn, … }` becomes `{ muting: State.muting, … }`.
- `island/integrations.ts`: `if (!State.focusOn) Sound.play(…)` becomes `if (!State.muting) Sound.play(…)`, and the comment says "A muted work interval (pomodoro spec §2.3) keeps it quiet".

Then `grep -rn "focusOn\|focusRecords\|focusDone\|focusStartedAt\|toggleFocusMode" windows/src` must return nothing.

- [ ] **Step 6: Strip colour per phase**

In `syncStrip()` replace the `focus` line with:

```ts
    const run = State.paused ? null : State.pomodoro;
    this.stripLine.classList.toggle("focus", run?.phase === "work");
    this.stripLine.classList.toggle("focus-break", run != null && run.phase !== "work");
```

Update its doc comment: "a static violet during pomodoro work and green during its breaks".

In `style.css`, after the `#strip-line.focus::after` rule:

```css
/* Pomodoro break: a static green, no animation (pomodoro spec §4). */
#strip-line.focus-break::after {
  background: rgba(52, 211, 153, 0.7);
}
```

Update the comment of the `.focus` rule: `/* Pomodoro work: a static violet, no animation (pomodoro spec §4). */`.

- [ ] **Step 7: Tray**

In `main.ts`, the tray case:

```ts
      // Same as the Focus tab's Start/Stop: starts a pomodoro, or stops it (pomodoro spec §4).
      case "focus":
        island.togglePomodoro();
        break;
```

- [ ] **Step 8: Checks**

Run:
```bash
cd windows && npx tsc --noEmit && npx vite build 2>&1 | tail -1
node --experimental-strip-types C:/tmp/mayak-spike/pomodoro-check.mjs
cd windows && cargo test --workspace 2>&1 | grep -E "^test result|^warning|^error"
grep -rn "focusOn\|focusRecords\|focusDone\|focusStartedAt\|toggleFocusMode\|focus-done" windows/src
```
Expected: tsc clean, build ok, check passes, Rust passes, and the grep prints nothing.

- [ ] **Step 9: Commit**

```bash
git add windows/src/core/state.ts windows/src/core/layout.ts windows/src/island/island.ts windows/src/island/hooks.ts windows/src/island/integrations.ts windows/src/views/views.ts windows/src/main.ts windows/src/style.css
git commit -m "feat(island): run the pomodoro, phase cards and break colour on the strip"
```

---

### Task 3: The Focus tab, the header, Settings → General

**Files:**
- Create: `windows/src/views/focus.ts`
- Modify: `windows/src/core/layout.ts` (view `focus`)
- Modify: `windows/src/views/views.ts` (header tabs, remove `focusBtn`, registry)
- Modify: `windows/src/settings/main.ts` (General section near lines 819-857, import near line 13)
- Modify: `windows/src/island/focus.ts` (remove the one-shot focus helpers)
- Modify: `windows/src/style.css` (new `.focus-tab*` block at the end)

**Interfaces:**
- Consumes (Task 1): `POMODORO_CHOICES`, `normalizeConfig`, `clock`, `phaseLabel`, `phaseMs`. Consumes (Task 2): `State.pomodoro`, `ViewActions.togglePomodoro`, `ViewActions.checkPomodoro`.
- Produces: `buildFocusTab(actions: ViewActions): ViewHost` and the view name `"focus"`.

- [ ] **Step 1: Layout**

In `core/layout.ts`, add `| "focus"` to `IslandViewName`, and to `VIEW_LAYOUTS`, next to `stats`:

```ts
  // The Focus tab (pomodoro spec §3): no Mochi, like Stats.
  focus: { height: 260, botX: 320, botY: 90, botDiameter: 0, agentMode: "none" },
```

- [ ] **Step 2: The tab view**

Create `windows/src/views/focus.ts`:

```ts
// The Focus tab (pomodoro spec §3): the countdown, Start/Stop and the pomodoro settings.
// Its 1 s clock runs only while the tab is open (setActive); the run itself lives in the island.

import { h } from "./dom";
import { Bridge } from "../core/bridge";
import { State } from "../core/state";
import { POMODORO_CHOICES, clock, normalizeConfig, phaseLabel, phaseMs } from "../island/pomodoro";
import type { ViewActions, ViewHost } from "./views";

type LengthKey = "pomodoroWork" | "pomodoroShort" | "pomodoroLong" | "pomodoroCycles";

function save() {
  void Bridge.saveSettings(State.settings);
  State.notify();
}

export function buildFocusTab(actions: ViewActions): ViewHost {
  const time = h("div", { class: "focus-tab-clock" });
  const label = h("div", { class: "focus-tab-phase" });
  const startBtn = h("button", { class: "btn primary", onclick: () => actions.togglePomodoro() });

  /** One row of segment buttons for a length (the island's `.seg`, like Auto-close). */
  function lengthRow(text: string, key: LengthKey, choices: readonly number[], unit: string) {
    const buttons = choices.map((v) =>
      h("button", {
        text: `${v}${unit}`,
        onclick: () => {
          State.settings[key] = v;
          save();
        },
      }),
    );
    const row = h(
      "div",
      { class: "settings-row focus-tab-row" },
      h("span", { class: "focus-tab-label", text }),
      h("div", { class: "seg" }, ...buttons),
    );
    return { row, sync: (current: number) => buttons.forEach((b, i) => b.classList.toggle("on", choices[i] === current)) };
  }

  const work = lengthRow("Work", "pomodoroWork", POMODORO_CHOICES.work, "");
  const short = lengthRow("Short break", "pomodoroShort", POMODORO_CHOICES.short, "");
  const long = lengthRow("Long break", "pomodoroLong", POMODORO_CHOICES.long, "");
  const cycles = lengthRow("Long break after", "pomodoroCycles", POMODORO_CHOICES.cycles, "");

  const muteSwitch = h("button", {
    class: "switch",
    onclick: () => {
      State.settings.pomodoroMute = !State.settings.pomodoroMute;
      save();
    },
  });
  const waitsSwitch = h("button", {
    class: "switch",
    onclick: () => {
      if (!State.settings.pomodoroMute) return;
      State.settings.focusLetWaitsThrough = !State.settings.focusLetWaitsThrough;
      save();
    },
  });
  const waitsRow = h(
    "div",
    { class: "settings-row focus-tab-row" },
    waitsSwitch,
    h("span", { text: "Let waiting agents through" }),
  );

  const el = h(
    "div",
    { class: "view" },
    h(
      "div",
      { class: "card focus-tab-card" },
      h(
        "div",
        { class: "focus-tab" },
        h("div", { class: "focus-tab-left" }, time, label, startBtn),
        h(
          "div",
          { class: "focus-tab-right" },
          work.row,
          short.row,
          long.row,
          cycles.row,
          h("div", { class: "settings-row focus-tab-row" }, muteSwitch, h("span", { text: "Mute alerts during work" })),
          waitsRow,
        ),
      ),
    ),
  );

  function render() {
    const cfg = normalizeConfig(State.settings);
    const run = State.pomodoro;
    const now = Date.now();
    time.textContent = run ? clock(run.endsAt, now) : clock(phaseMs("work", cfg), 0);
    label.textContent = run ? phaseLabel(run.phase, run.cycle, cfg) : "Not running";
    time.classList.toggle("break", run != null && run.phase !== "work");
    startBtn.textContent = run ? "Stop" : "Start";
    startBtn.className = run ? "btn secondary" : "btn primary";
    work.sync(cfg.work);
    short.sync(cfg.short);
    long.sync(cfg.long);
    cycles.sync(cfg.cycles);
    muteSwitch.classList.toggle("on", State.settings.pomodoroMute);
    waitsSwitch.classList.toggle("on", State.settings.focusLetWaitsThrough);
    waitsRow.classList.toggle("disabled", !State.settings.pomodoroMute);
  }

  let ticker: number | null = null;

  return {
    el,
    sync: render,
    setActive(on: boolean) {
      if (ticker != null) window.clearInterval(ticker);
      ticker = null;
      if (!on) return;
      render();
      ticker = window.setInterval(() => {
        actions.checkPomodoro();
        render();
      }, 1000);
    },
  };
}
```

- [ ] **Step 3: The header and the registry**

In `views/views.ts`, `buildHeader`:
- Add next to `tabStats`:
  ```ts
  const tabFocus = h("button", { class: "tab", title: "Focus", onclick: () => go("focus") }, svg(ICONS.target, 13));
  ```
- Tabs: `tabSessions, tabHome, tabStats, tabFocus, ...(CHAT_ENABLED ? [tabChat, tabDrop] : [])`.
- Remove `focusBtn`: its declaration, its place in `header-actions`, and its two lines in `sync()`.
- In `sync()` add `tabFocus.classList.toggle("on", v === "focus");`.

In `buildViews`, next to `map.set("stats", buildStats());`, add `map.set("focus", buildFocusTab(actions));`, with `import { buildFocusTab } from "./focus";` at the top.

- [ ] **Step 4: Settings → General and the old helpers**

In `settings/main.ts`:
- remove the `focusMinutes` select (the `const focusMinutes = …` block and its change listener);
- remove the two rows "Focus length" and "Let waiting agents through";
- remove the import line `import { FOCUS_LENGTHS, focusLength } from "../island/focus";`.

In `island/focus.ts`, remove `FOCUS_LENGTHS`, `focusLength`, `focusElapsedMinutes` and `focusDoneTitle`. Then `grep -rn "FOCUS_LENGTHS\|focusLength\|focusElapsedMinutes\|focusDoneTitle" windows/src` must print nothing.

- [ ] **Step 5: Styles**

Append to `style.css`:

```css
/* ── Focus tab (pomodoro spec §3) ───────────────────────────────────────── */

.focus-tab-card {
  height: 100%;
}

.focus-tab {
  display: flex;
  gap: 24px;
  height: 100%;
  padding: 14px 18px;
  box-sizing: border-box;
}

.focus-tab-left {
  flex: 0 0 150px;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  justify-content: center;
  gap: 6px;
}

.focus-tab-clock {
  font: 700 38px/1 var(--font);
  font-variant-numeric: tabular-nums;
  color: rgb(167, 139, 250);
}

.focus-tab-clock.break {
  color: rgb(52, 211, 153);
}

.focus-tab-phase {
  font: 500 12px var(--font);
  color: var(--dim);
  margin-bottom: 6px;
}

.focus-tab-right {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 6px;
  min-width: 0;
}

.focus-tab-label {
  flex: 0 0 104px;
  color: var(--dim);
}

.focus-tab-row.disabled {
  opacity: 0.4;
}
```

If `--dim` or `--font` is not defined in `style.css`, use the variables the `.stats*` rules use for the same purpose.

- [ ] **Step 6: Checks and a look**

Run:
```bash
cd windows && npx tsc --noEmit && npx vite build 2>&1 | tail -1
node --experimental-strip-types C:/tmp/mayak-spike/pomodoro-check.mjs
```
Expected: tsc clean, build ok, check passes.

Look at it: `cd windows && npm run dev` serves the island in a browser at `http://localhost:1420/`. Outside Tauri, open the Focus tab by running `document.querySelector('#header .tab[title="Focus"]').click()` in the browser console. The two columns must fit the 260 px view without scrolling or clipping. If they don't fit, raise `VIEW_LAYOUTS.focus.height` to the smallest value that fits and say so in the report.

- [ ] **Step 7: Commit**

```bash
git add windows/src/views/focus.ts windows/src/core/layout.ts windows/src/views/views.ts windows/src/settings/main.ts windows/src/island/focus.ts windows/src/style.css
git commit -m "feat(island): Focus tab with the countdown and the pomodoro settings"
```

---

### Task 4: Mochi's headphones and the minutes

**Files:**
- Modify: `windows/src/mochi/engine.ts` (a field on `BotEngine`, `draw` near line 643, two new private methods)
- Modify: `windows/src/island/island.ts` (`setMode` near line 363, the pomodoro section from Task 2, the compact DOM built near line 261, a new field)
- Modify: `windows/src/style.css` (`#focus-mins` next to `#counters`)

**Interfaces:**
- Consumes (Task 1): `minutesLeft`, `msToNextMinute`. Consumes (Task 2): `State.pomodoro`, `Island.checkPomodoro`, `advancePhase`, `togglePomodoro`, `stopPomodoro`.
- Produces: `BotEngine.focusGear: { color: RGB; minutes: number | null } | null`.

- [ ] **Step 1: The engine field and drawing**

In `mochi/engine.ts`, add to `BotEngine` after `badge`:

```ts
  /**
   * Pomodoro (spec §4): headphones in the phase colour, and the minutes left next to Mochi when
   * `minutes` is set. Never drawn on mini bots. Read at draw time: nothing animates it.
   */
  focusGear: { color: RGB; minutes: number | null } | null = null;
```

In `draw()`, inside the transformed block, right after `this.drawBody(x, body, R, rx, ry);`:

```ts
    if (this.focusGear && !this.isMini) this.drawHeadphones(x, R, rx, ry, this.focusGear.color);
```

and after `this.drawParticles(x, R, cx, cy);` at the end of `draw()`:

```ts
    if (this.focusGear?.minutes != null && !this.isMini) {
      this.drawMinutes(x, this.focusGear.minutes, this.focusGear.color, R, cx, cy);
    }
```

Add the two methods next to `drawBadge`:

```ts
  /** A band over the head and a cup on each side, in the body's own coordinates. */
  private drawHeadphones(x: CanvasRenderingContext2D, R: number, rx: number, ry: number, color: RGB) {
    x.save();
    x.strokeStyle = rgba(color);
    x.fillStyle = rgba(color);
    x.lineWidth = R * 0.11;
    x.lineCap = "round";
    x.beginPath();
    x.arc(0, -ry * 0.05, rx * 1.06, Math.PI * 1.1, Math.PI * 1.9);
    x.stroke();
    const cw = R * 0.24;
    const ch = R * 0.46;
    for (const sd of [-1, 1]) {
      roundRectPath(x, sd * rx * 1.02 - cw / 2, -ch * 0.35, cw, ch, cw / 2);
      x.fill();
    }
    x.restore();
  }

  /** The minutes left in a dark pill at Mochi's top right, opposite the badge. */
  private drawMinutes(x: CanvasRenderingContext2D, minutes: number, color: RGB, R: number, cx: number, cy: number) {
    const text = String(minutes);
    const fs = R * 0.34;
    x.save();
    x.font = `800 ${fs}px ${FONT}`;
    const pw = Math.max(fs * 1.5, x.measureText(text).width + fs * 0.8);
    const ph = fs * 1.35;
    const bx = cx + R * 0.78 * this.sx;
    const by = cy - R * 0.72 * this.sy;
    roundRectPath(x, bx - pw / 2, by - ph / 2, pw, ph, ph / 2);
    x.fillStyle = "#000";
    x.fill();
    x.fillStyle = rgba(color);
    x.textAlign = "center";
    x.textBaseline = "middle";
    x.fillText(text, bx, by + fs * 0.04);
    x.restore();
  }
```

- [ ] **Step 2: The compact label**

In `island/island.ts`, next to the field `counters`, add `private focusMins!: HTMLElement;`. Where `this.counters = h("div", { id: "counters" });` is built, add `this.focusMins = h("div", { id: "focus-mins" });` and append it to the same parent as `this.counters` (find where `this.counters` is appended and add `this.focusMins` right after it).

In `style.css`, after the `#counters` rule:

```css
/* Compact island: the pomodoro minutes, right of Mochi and left of the collapse button
   (pomodoro spec §4). Empty while no pomodoro runs or the island is not compact. */
#focus-mins {
  position: absolute;
  top: 0;
  left: 54px;
  height: 32px;
  display: flex;
  align-items: center;
  font: 700 11px var(--font);
  font-variant-numeric: tabular-nums;
  pointer-events: none;
}
```

- [ ] **Step 3: The gear and the minute tick in the island**

In `island/island.ts`, import `minutesLeft` and `msToNextMinute` from `./pomodoro`. Add constants near the top of the file:

```ts
/** Pomodoro phase colours (pomodoro spec §4): the strip's violet and green. */
const WORK_RGB: RGB = [167 / 255, 139 / 255, 250 / 255];
const BREAK_RGB: RGB = [52 / 255, 211 / 255, 153 / 255];
```

(import `type RGB` from `../mochi/engine` if it is not imported yet). Add a field next to `phaseTimer`:

```ts
  /** The minutes label's tick: only while the island is visible and a pomodoro runs (spec §4). */
  private minuteTimer: number | null = null;
```

Add to the pomodoro section:

```ts
  /**
   * Mochi's headphones and the minutes (spec §4): minutes in the Mochi pill while expanded, in the
   * compact label while compact, none while hidden. One frame is drawn; nothing loops.
   */
  private refreshGear() {
    const run = State.pomodoro;
    const now = Date.now();
    const color = run?.phase === "work" ? WORK_RGB : BREAK_RGB;
    this.engine.focusGear = run
      ? { color, minutes: State.mode === "expanded" ? Math.max(1, minutesLeft(run.endsAt, now)) : null }
      : null;
    const compact = run != null && State.mode === "compact";
    this.focusMins.textContent = compact ? String(Math.max(1, minutesLeft(run.endsAt, now))) : "";
    this.focusMins.style.color = run?.phase === "work" ? "rgb(167, 139, 250)" : "rgb(52, 211, 153)";
    if (State.mode !== "hidden") this.ensureRunning();
  }

  /** (Re)arms the minute tick for the visible island; stops it while hidden or off. */
  private syncMinuteTick() {
    if (this.minuteTimer != null) window.clearTimeout(this.minuteTimer);
    this.minuteTimer = null;
    const run = State.pomodoro;
    if (!run || State.mode === "hidden") return;
    this.minuteTimer = window.setTimeout(() => {
      this.minuteTimer = null;
      this.checkPomodoro();
      this.refreshGear();
      this.syncMinuteTick();
    }, Math.max(0, msToNextMinute(run.endsAt, Date.now())));
  }
```

`refreshGear` uses `this.focusMins`, built in `build()`: check that `setMode` is never called before `build()` (it is not in the current constructor order); if it is, guard with an early return. Call `this.refreshGear(); this.syncMinuteTick();` at the end of `togglePomodoro()` (after `State.notify()`), of `stopPomodoro()` and of `advancePhase()`.

In `setMode(mode)`, just before `this.updateWindowCollapsed();`:

```ts
    // The pomodoro: a phase that ended while hidden (or asleep) moves on as the island shows,
    // and the minute tick runs only while it is visible (spec §2.5, §4). The check is deferred:
    // a phase change opens its card, and that must not re-enter setMode halfway through.
    if (mode !== "hidden") window.setTimeout(() => this.checkPomodoro(), 0);
    this.refreshGear();
    this.syncMinuteTick();
```

- [ ] **Step 4: Checks**

Run:
```bash
cd windows && npx tsc --noEmit && npx vite build 2>&1 | tail -1
node --experimental-strip-types C:/tmp/mayak-spike/pomodoro-check.mjs
cd windows && cargo test --workspace 2>&1 | grep -E "^test result|^warning|^error"
```
Expected: all clean.

Look at it in `npm run dev` (browser). In the console, start a run through the tab (`document.querySelector('#header .tab[title="Focus"]').click()`, then click Start). Collapse to compact. The headphones and the label must be visible and not overlap the counters. Expanded on the home view, the minutes pill must sit at Mochi's top right. Report what you saw. Screenshots are not required.

- [ ] **Step 5: Commit**

```bash
git add windows/src/mochi/engine.ts windows/src/island/island.ts windows/src/style.css
git commit -m "feat(island): headphones on Mochi and the minutes left during a pomodoro"
```

---

### Task 5: Live check and docs (controller)

Not for an implementer subagent: the controller does this after the final review.

- [ ] Start the dev app with CDP (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`). Through CDP, set `State.pomodoro.endsAt = Date.now() + 3000` and call `island.checkPomodoro()` through the page's own objects. Check:
  - the phase changes;
  - the card appears with the finished sound;
  - the strip turns green on a break;
  - a held alert lands in the card;
  - the tray check follows Start and Stop;
  - with the island hidden, only one timer is pending (wrap `setTimeout` and `setInterval` with a counter before the run).
- [ ] Update `docs/superpowers/ideas/2026-10-06-mayak-next-ideas.md` §18: the pomodoro replaced the one-shot focus, with a link to the spec.
- [ ] Commit the docs, push `feature/pomodoro`, and report to the user.
