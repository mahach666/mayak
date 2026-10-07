// Focus mode (night spec §A): the pure decisions, with no DOM, no Tauri and no State,
// so they can be checked on their own.

import type { Alert } from "../core/bridge";
import type { Translate } from "../core/i18n-core";

export type AlertKind = Alert["kind"];
/** How the summary groups the kinds: a permission or a question is a wait. */
export type FocusKind = "finished" | "error" | "waited";

/** One alert focus held back. */
export interface FocusRecord {
  key: string;
  project: string;
  kind: AlertKind;
  at: number;
}

export interface FocusSummary {
  counts: Record<FocusKind, number>;
  /** In order of first appearance, at most FOCUS_MAX_PROJECTS; each with its kinds in that order too. */
  projects: { name: string; kinds: FocusKind[] }[];
}

export const FOCUS_MAX_PROJECTS = 6;

export function focusKind(kind: AlertKind): FocusKind {
  return kind === "permission" || kind === "input" ? "waited" : kind;
}

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

/**
 * Adds a held alert unless the same session already has one of the same kind in this focus,
 * so each session counts at most once per kind. True when it was added.
 */
export function addFocusRecord(records: FocusRecord[], r: FocusRecord): boolean {
  if (records.some((x) => x.key === r.key && x.kind === r.kind)) return false;
  records.push(r);
  return true;
}

export function focusSummary(records: readonly FocusRecord[]): FocusSummary {
  const counts: Record<FocusKind, number> = { finished: 0, error: 0, waited: 0 };
  const byName = new Map<string, FocusKind[]>();
  for (const r of records) {
    const kind = focusKind(r.kind);
    counts[kind] += 1;
    const kinds = byName.get(r.project);
    if (!kinds) byName.set(r.project, [kind]);
    else if (!kinds.includes(kind)) kinds.push(kind);
  }
  const projects = [...byName].slice(0, FOCUS_MAX_PROJECTS).map(([name, kinds]) => ({ name, kinds }));
  return { counts, projects };
}

/**
 * The English texts of the lines below, the default when no translator is passed (the Node
 * check). The island passes t(), whose core/i18n/en.ts holds the same texts under the same keys.
 */
const EN = {
  "focus.countFinished": "{n} finished",
  "focus.countError": "{n} error",
  "focus.countErrors": "{n} errors",
  "focus.countWaited": "{n} waited for you",
  "focus.kindFinished": "finished",
  "focus.kindError": "error",
  "focus.kindWaited": "waited",
  "focus.projectLine": "{name}: {kinds}",
} as const;

export type FocusText = keyof typeof EN;

/** core/i18n-core.ts format() over EN: a runtime import would stop Node from loading this module. */
const english: Translate<FocusText> = (key, vars = {}) =>
  EN[key].replace(/\{(\w+)\}/g, (whole, name: string) =>
    Object.hasOwn(vars, name) ? String(vars[name]) : whole,
  );

const KIND_TEXT: Record<FocusKind, FocusText> = {
  finished: "focus.kindFinished",
  error: "focus.kindError",
  waited: "focus.kindWaited",
};

/** "3 finished · 1 error · 1 waited for you"; zero counts are left out, "" when all are zero. */
export function focusCountsLine(c: Record<FocusKind, number>, tr: Translate<FocusText> = english): string {
  const parts: string[] = [];
  if (c.finished > 0) parts.push(tr("focus.countFinished", { n: c.finished }));
  if (c.error > 0) parts.push(tr(c.error === 1 ? "focus.countError" : "focus.countErrors", { n: c.error }));
  if (c.waited > 0) parts.push(tr("focus.countWaited", { n: c.waited }));
  return parts.join(" · ");
}

/** "mayak: finished, waited · api: error" — the projects of a summary on one line. */
export function focusProjectsLine(projects: FocusSummary["projects"], tr: Translate<FocusText> = english): string {
  return projects
    .map((p) => tr("focus.projectLine", { name: p.name, kinds: p.kinds.map((k) => tr(KIND_TEXT[k])).join(", ") }))
    .join(" · ");
}
