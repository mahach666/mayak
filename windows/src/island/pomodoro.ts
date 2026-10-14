// Pomodoro (spec docs/superpowers/specs/2026-10-07-mayak-pomodoro-design.md): the phase arithmetic
// and the run transitions, with no DOM, no Tauri and no State, so they can be checked on their own.

import type { FocusRecord } from "./focus";
import type { Translate } from "../core/i18n-core";

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

/**
 * The English texts of the labels below, the default when no translator is passed (the Node
 * check). The island passes t(), whose core/i18n/en.ts holds the same texts under the same keys.
 */
const EN = {
  "pomodoro.work": "Work · cycle {cycle} of {cycles}",
  "pomodoro.short": "Short break",
  "pomodoro.long": "Long break",
  "pomodoro.cardWork": "Work · {min} min · cycle {cycle} of {cycles}",
  "pomodoro.cardShort": "Break · {min} min",
  "pomodoro.cardLong": "Long break · {min} min",
} as const;

export type PomodoroText = keyof typeof EN;

/** core/i18n-core.ts format() over EN: a runtime import would stop Node from loading this module. */
const english: Translate<PomodoroText> = (key, vars = {}) =>
  EN[key].replace(/\{(\w+)\}/g, (whole, name: string) =>
    Object.hasOwn(vars, name) ? String(vars[name]) : whole,
  );

/** The Focus tab's phase line. */
export function phaseLabel(
  phase: Phase, cycle: number, cfg: PomodoroConfig, tr: Translate<PomodoroText> = english,
): string {
  if (phase === "work") return tr("pomodoro.work", { cycle, cycles: cfg.cycles });
  return tr(phase === "short" ? "pomodoro.short" : "pomodoro.long");
}

/** The title of the card that opens when `phase` starts (spec §2.2). */
export function phaseCardTitle(
  phase: Phase, cycle: number, cfg: PomodoroConfig, tr: Translate<PomodoroText> = english,
): string {
  if (phase === "work") return tr("pomodoro.cardWork", { min: cfg.work, cycle, cycles: cfg.cycles });
  return phase === "short"
    ? tr("pomodoro.cardShort", { min: cfg.short })
    : tr("pomodoro.cardLong", { min: cfg.long });
}

/** What Rust keeps of the run (src-tauri/src/pomodoro.rs `Focus`), for the MCP server's `get_focus`. */
export interface FocusSnapshot {
  phase: Phase;
  cycle: number;
  cycles: number;
  endsAt: number;
  /** Agent alerts are held back now: State.muting's rule (a work interval with Mute on). */
  muted: boolean;
}

/** The run as Rust is told it; null for no run. */
export function focusSnapshot(run: PomodoroRun | null, cfg: PomodoroConfig, mute: boolean): FocusSnapshot | null {
  if (!run) return null;
  return { phase: run.phase, cycle: run.cycle, cycles: cfg.cycles, endsAt: run.endsAt, muted: run.phase === "work" && mute };
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
  tr: Translate<PomodoroText> = english,
): { run: PomodoroRun; held: FocusRecord[]; title: string } {
  const next = nextPhase(run.phase, run.cycle, cfg);
  return {
    run: { phase: next.phase, cycle: next.cycle, endsAt: now + phaseMs(next.phase, cfg), records: [] },
    held: run.phase === "work" ? run.records : [],
    title: phaseCardTitle(next.phase, next.cycle, cfg, tr),
  };
}

/** What a stop shows in its summary: the current work interval's records, none during a break. */
export function stopRun(run: PomodoroRun): FocusRecord[] {
  return run.phase === "work" ? run.records : [];
}

/** The sessionStorage key of what a language reload keeps (main.ts): the run, the marker, the card. */
export const SAVED_RUN_KEY = "mayak.pomodoroRun";

/** A phase card as State keeps it (State.phaseCard). */
export interface PhaseCard {
  title: string;
  records: FocusRecord[];
}

/**
 * What main.ts keeps across a language reload (batch spec §F): `reloaded` says the page was
 * reloaded, not launched; the pomodoro run and its phase card go on. An entry written before the
 * marker existed is the run alone.
 */
export interface ReloadEntry {
  reloaded: boolean;
  run: PomodoroRun | null;
  phaseCard: PhaseCard | null;
  phaseCardPending: boolean;
}

const PHASES: readonly string[] = ["work", "short", "long"];
const RECORD_KINDS: readonly string[] = ["permission", "input", "error", "finished"];

/**
 * The entry main.ts writes just before a language reload: the run's fields as before (the old
 * format), next to them the marker and the card. With no run and no card it is `{"reloaded":true}`.
 */
export function encodeReloadEntry(e: Omit<ReloadEntry, "reloaded">): string {
  const run = e.run
    ? { phase: e.run.phase, cycle: e.run.cycle, endsAt: e.run.endsAt, records: e.run.records }
    : {};
  const card = e.phaseCard
    ? { phaseCard: { title: e.phaseCard.title, records: e.phaseCard.records }, phaseCardPending: e.phaseCardPending }
    : {};
  return JSON.stringify({ ...run, reloaded: true, ...card });
}

function isRecord(v: unknown): v is FocusRecord {
  if (typeof v !== "object" || v === null) return false;
  const r = v as Record<string, unknown>;
  return typeof r.key === "string" && typeof r.project === "string" &&
    typeof r.kind === "string" && RECORD_KINDS.includes(r.kind) &&
    typeof r.at === "number" && Number.isFinite(r.at);
}

/** The records back with their four fields only, or null when one of them is not a record. */
function recordsFrom(v: unknown): FocusRecord[] | null {
  if (!Array.isArray(v) || !v.every(isRecord)) return null;
  return v.map((r) => ({ key: r.key, project: r.project, kind: r.kind, at: r.at }));
}

function runFrom(o: Record<string, unknown>): PomodoroRun | null {
  if (typeof o.phase !== "string" || !PHASES.includes(o.phase)) return null;
  if (typeof o.cycle !== "number" || !Number.isInteger(o.cycle) || o.cycle < 1) return null;
  if (typeof o.endsAt !== "number" || !Number.isFinite(o.endsAt)) return null;
  const records = recordsFrom(o.records);
  if (!records) return null;
  return { phase: o.phase as Phase, cycle: o.cycle, endsAt: o.endsAt, records };
}

function cardFrom(v: unknown): PhaseCard | null {
  if (typeof v !== "object" || v === null || Array.isArray(v)) return null;
  const o = v as Record<string, unknown>;
  if (typeof o.title !== "string") return null;
  const records = recordsFrom(o.records);
  return records ? { title: o.title, records } : null;
}

/**
 * The kept entry back, or null when there is none or it is not an object (a real launch). Each
 * part is checked on its own: a bad run is dropped and the card kept, a bad card is dropped and
 * the run kept; only a literal `true` counts as the marker or as pending.
 */
export function decodeReloadEntry(raw: string | null): ReloadEntry | null {
  if (!raw) return null;
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof v !== "object" || v === null || Array.isArray(v)) return null;
  const o = v as Record<string, unknown>;
  const phaseCard = cardFrom(o.phaseCard);
  return {
    reloaded: o.reloaded === true,
    run: runFrom(o),
    phaseCard,
    phaseCardPending: phaseCard != null && o.phaseCardPending === true,
  };
}
