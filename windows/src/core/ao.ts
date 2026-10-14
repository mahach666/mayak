// Agent Orchestrator (AO) in the island: what Rust's poll sends (src-tauri/src/ao), which sessions
// the list shows, and which of them a project of the sessions list belongs to. Pure: no DOM, no
// Tauri, no dictionary (it hands out dictionary keys), so Node can check it (scripts/checks/ao.mjs).

import type { Key } from "./i18n";

/** The pill's id, the same as src-tauri/src/ao `ID`: a stable contract value. */
export const AO_ID = "integration_ao";
/** The pill's colour, Mayak's own pick (AO publishes none). */
export const AO_COLOR = "#38BDF8";
/** The ↑ ↓ key of the AO fold on the sessions list; never a session key. */
export const AO_FOLD_KEY = "ao:fold";
/** Rows the open fold lists; the rest is one «+N more» line. */
export const AO_MAX_ROWS = 30;

/** How AO stands (src-tauri/src/ao/model.rs `Status`); only `ok` has sessions. */
export type AoStatus = "ok" | "notInstalled" | "notRunning" | "unreachable" | "unsupported";
const STATUSES: readonly string[] = ["ok", "notInstalled", "notRunning", "unreachable", "unsupported"];

export interface AoSession {
  id: string;
  project: string | null;
  /** `worker` or `orchestrator`; another value AO adds later comes as it is. */
  role: string;
  name: string | null;
  /** AO's status code (`working`, `ci_failed`…), as it is. */
  status: string;
  activity: string | null;
  harness: string | null;
  prs: number;
  prsOpen: number;
  /** The worst CI of the open pull requests, null without one. */
  ci: string | null;
  /** The session's folder as a project key of the sessions list (observe/paths.rs), null when unknown. */
  workspace: string | null;
}

export interface AoData {
  status: AoStatus;
  /** An `ao` is on PATH. */
  cli: boolean;
  sessions: AoSession[];
}

const text = (v: unknown): string | null => (typeof v === "string" && v !== "" ? v : null);
const count = (v: unknown): number => (typeof v === "number" && Number.isFinite(v) && v > 0 ? Math.floor(v) : 0);

/** The poll's `data`, checked: null for anything that is not one; a row without id, role or status is dropped. */
export function readAo(data: unknown): AoData | null {
  if (typeof data !== "object" || data === null) return null;
  const d = data as Record<string, unknown>;
  if (typeof d.status !== "string" || !STATUSES.includes(d.status)) return null;
  const sessions: AoSession[] = [];
  for (const row of Array.isArray(d.sessions) ? d.sessions : []) {
    if (typeof row !== "object" || row === null) continue;
    const r = row as Record<string, unknown>;
    const id = text(r.id);
    const role = text(r.role);
    const status = text(r.status);
    if (!id || !role || !status) continue;
    sessions.push({
      id, role, status,
      project: text(r.project),
      name: text(r.name),
      activity: text(r.activity),
      harness: text(r.harness),
      prs: count(r.prs),
      prsOpen: count(r.prsOpen),
      ci: text(r.ci),
      workspace: text(r.workspace),
    });
  }
  return { status: d.status as AoStatus, cli: d.cli === true, sessions };
}

// ── Words and colours ─────────────────────────────────────────────────────────

/** AO's session status codes (its openapi, ControllersSessionView.status) as dictionary keys. */
const STATUS_WORDS: Record<string, Key> = {
  working: "ao.st.working",
  pr_open: "ao.st.prOpen",
  draft: "ao.st.draft",
  ci_failed: "ao.st.ciFailed",
  review_pending: "ao.st.reviewPending",
  changes_requested: "ao.st.changesRequested",
  commented: "ao.st.commented",
  approved: "ao.st.approved",
  mergeable: "ao.st.mergeable",
  merged: "ao.st.merged",
  needs_input: "ao.st.needsInput",
  exited: "ao.st.exited",
  idle: "ao.st.idle",
  terminated: "ao.st.terminated",
  no_signal: "ao.st.noSignal",
};

/** The word of a status code, null for a code AO has added since (the row then shows the code itself). */
export function statusWord(status: string): Key | null {
  return STATUS_WORDS[status] ?? null;
}

const ROLE_WORDS: Record<string, Key> = { worker: "ao.role.worker", orchestrator: "ao.role.orchestrator" };

export function roleWord(role: string): Key | null {
  return ROLE_WORDS[role] ?? null;
}

const ACTIVITY_WORDS: Record<string, Key> = { active: "ao.act.active", idle: "ao.act.idle" };

export function activityWord(activity: string): Key | null {
  return ACTIVITY_WORDS[activity] ?? null;
}

const CI_WORDS: Record<string, Key> = {
  failing: "ao.ci.failing",
  pending: "ao.ci.pending",
  passing: "ao.ci.passing",
  unknown: "ao.ci.unknown",
};

export function ciWord(ci: string): Key {
  return CI_WORDS[ci] ?? "ao.ci.unknown";
}

/** A CI state's dot colour: failing red, pending amber, passing green, else dim. */
export function ciColor(ci: string | null): string {
  return ci === "failing" ? "#F4505E" : ci === "pending" ? "#F5A524" : ci === "passing" ? "#22C55E" : "#6B7079";
}

/** Statuses that ask for someone: the agent is blocked on an answer, its CI failed, a review wants changes. */
const NEEDS_YOU = new Set(["needs_input", "ci_failed", "changes_requested"]);

export function needsYou(s: Pick<AoSession, "status">): boolean {
  return NEEDS_YOU.has(s.status);
}

export type AoTone = "amber" | "red" | "green" | "blue" | "dim";

/** The sessions list's tone classes: what asks for someone stands out, a finished or ready one is green. */
export function statusTone(status: string): AoTone {
  switch (status) {
    case "ci_failed":
      return "red";
    case "needs_input":
    case "changes_requested":
      return "amber";
    case "working":
      return "blue";
    case "mergeable":
    case "approved":
    case "merged":
      return "green";
    default:
      return "dim";
  }
}

// ── Which sessions, in which order ────────────────────────────────────────────

/** What the card and the list count: sessions asking for someone, sessions working, open pull requests. */
export function aoCounts(sessions: readonly AoSession[]): { attention: number; working: number; prsOpen: number; ciFailing: number } {
  let attention = 0, working = 0, prsOpen = 0, ciFailing = 0;
  for (const s of sessions) {
    if (needsYou(s)) attention++;
    if (s.status === "working") working++;
    prsOpen += s.prsOpen;
    if (s.ci === "failing") ciFailing++;
  }
  return { attention, working, prsOpen, ciFailing };
}

/** Sessions that ask for someone first, then the ones working, then the rest; AO's order inside each. */
export function orderSessions(sessions: readonly AoSession[]): AoSession[] {
  const rank = (s: AoSession) => (needsYou(s) ? 0 : s.status === "working" ? 1 : 2);
  return sessions.map((s, i) => ({ s, i })).sort((a, b) => rank(a.s) - rank(b.s) || a.i - b.i).map((x) => x.s);
}

/** The rows of the open fold: the first AO_MAX_ROWS in order, and how many are left out. */
export function aoRows(sessions: readonly AoSession[]): { rows: AoSession[]; more: number } {
  const ordered = orderSessions(sessions);
  return { rows: ordered.slice(0, AO_MAX_ROWS), more: Math.max(0, ordered.length - AO_MAX_ROWS) };
}

/** The 24 px lines the fold adds to the sessions list: none without sessions, its line, then its rows (and the «+N more» line). */
export function aoLines(total: number, open: boolean): number {
  if (total <= 0) return 0;
  if (!open) return 1;
  const rows = Math.min(total, AO_MAX_ROWS);
  return 1 + rows + (total > rows ? 1 : 0);
}

// ── What the state hands out ──────────────────────────────────────────────────

let memo: { info: unknown; active: readonly string[]; list: AoSession[]; rev: number } | null = null;

/**
 * AO's sessions while the AO pill is on and AO answers; none otherwise (a disabled pill shows
 * nothing of AO, whatever was read before). `rev` changes when the list was read anew, so a view can
 * tell. Read from `info` (State.integrations[AO_ID]) once per change, not per call.
 */
export function aoSessions(info: { data: Record<string, unknown> } | undefined, active: readonly string[]): { list: AoSession[]; rev: number } {
  if (memo && memo.info === info && memo.active === active) return memo;
  const read = active.includes(AO_ID) ? readAo(info?.data) : null;
  memo = { info, active, list: read?.status === "ok" ? read.sessions : [], rev: (memo?.rev ?? 0) + 1 };
  return memo;
}

// ── Which session a project of the list belongs to ────────────────────────────

/** Whether the project key `project` is the session's folder `workspace` or lies inside it (both as `observe/paths` keys). */
export function isInside(project: string, workspace: string): boolean {
  if (project === "" || workspace === "") return false;
  return project === workspace || project.startsWith(`${workspace}\\`) || project.startsWith(`${workspace}/`);
}

/** The AO session whose folder holds the project (the deepest folder when sessions nest), null for an unrelated project. */
export function tagFor(sessions: readonly AoSession[], projectKey: string): AoSession | null {
  let found: AoSession | null = null;
  for (const s of sessions) {
    if (s.workspace === null || !isInside(projectKey, s.workspace)) continue;
    if (found === null || s.workspace.length > (found.workspace ?? "").length) found = s;
  }
  return found;
}

// ── …or which session an agent's own environment names ────────────────────────

/**
 * The AO session an agent's environment names (`AO_SESSION_ID`, `AO_PROJECT_ID`, which the relay
 * forwards with its hooks): the observer's `ao` of a Codex or Antigravity row (model.rs AoRef), or
 * the ids of the hook behind a permission or question card.
 */
export interface AoRef {
  session: string;
  project: string | null;
}

/** An AO id as Rust accepts one (src-tauri/src/ao/model.rs valid_id): letters, digits, `-`, `_`, `.`; never `.` or `..`. */
export function validAoId(id: string): boolean {
  return /^[A-Za-z0-9._-]{1,64}$/.test(id) && id !== "." && id !== "..";
}

/** Ids from a payload or a row as an AoRef; null without a valid session id (a bad project id is dropped). */
export function aoRefOf(session: unknown, project: unknown): AoRef | null {
  const id = typeof session === "string" ? session.trim() : "";
  if (!validAoId(id)) return null;
  const p = typeof project === "string" ? project.trim() : "";
  return { session: id, project: validAoId(p) ? p : null };
}

/** The session AO lists under that id (and that project, when both say one); null when AO does not know it. */
export function byRef(sessions: readonly AoSession[], ref: AoRef | null | undefined): AoSession | null {
  if (!ref) return null;
  return sessions.find((s) => s.id === ref.session && (ref.project === null || s.project === null || s.project === ref.project)) ?? null;
}

/**
 * The AO session of one agent session: the folder of its project first (exact, from AO itself), else
 * the id the agent's environment named. Either way only a session AO lists right now, so a switched off
 * pill or a stopped AO shows nothing.
 */
export function aoFor(sessions: readonly AoSession[], projectKey: string | null, ref: AoRef | null | undefined): AoSession | null {
  return (projectKey === null ? null : tagFor(sessions, projectKey)) ?? byRef(sessions, ref);
}

/** The AO session of a project of the sessions list: its folder first, else the first of its rows whose ids AO knows. */
export function aoForProject(
  sessions: readonly AoSession[],
  project: { key: string; sessions: readonly { ao?: AoRef | null }[] },
): AoSession | null {
  const byFolder = tagFor(sessions, project.key);
  if (byFolder) return byFolder;
  for (const row of project.sessions) {
    const found = byRef(sessions, row.ao);
    if (found) return found;
  }
  return null;
}
