// Pure parts of the prompt view's launcher (launcher spec §3, §5). No DOM and no runtime imports,
// so Node can run these functions as they are (C:\tmp\mayak-spike\launcher-model-check.mjs). The
// words come from `tr`, which the island passes as t() (core/i18n.ts).

import type { LaunchOutcome, LauncherAgent, SessionsSnapshot } from "../core/bridge";
import type { Translate } from "../core/i18n-core";
import type { Key } from "../core/i18n";

type Tr = Translate<Key>;

/** The one agent that answers in the island instead of being opened. */
export const API_AGENT: LauncherAgent = "claudeApi";

/** The agent's name in the picker: "Claude Desktop", "Claude in Cursor", "Claude API (answers here)". */
export function agentLabel(agent: LauncherAgent, tr: Tr): string {
  switch (agent) {
    case "claudeDesktop":
      return "Claude Desktop";
    case "claudeCursor":
      return tr("launcher.in", { agent: "Claude", host: "Cursor" });
    case "claudeVsCode":
      return tr("launcher.in", { agent: "Claude", host: "VS Code" });
    case "codexDesktop":
      return "Codex Desktop";
    case "codexCursor":
      return tr("launcher.in", { agent: "Codex", host: "Cursor" });
    case "codexVsCode":
      return tr("launcher.in", { agent: "Codex", host: "VS Code" });
    case "claudeApi":
      return tr("launcher.api");
  }
}

export interface ProjectChoice {
  root: string;
  name: string;
}

/** The snapshot's projects that have a session, each key once, in snapshot order. */
export function projectChoices(snapshot: SessionsSnapshot | null): ProjectChoice[] {
  const seen = new Set<string>();
  const out: ProjectChoice[] = [];
  for (const p of snapshot?.projects ?? []) {
    if (p.sessions.length === 0 || !p.root || seen.has(p.key)) continue;
    seen.add(p.key);
    out.push({ root: p.root, name: p.name });
  }
  return out;
}

/** Root of the project of the session with the latest `since` (spec §3); the first of equal ones; null without a session. */
export function defaultRoot(snapshot: SessionsSnapshot | null): string | null {
  let best: { root: string; since: number } | null = null;
  for (const p of snapshot?.projects ?? []) {
    if (!p.root) continue;
    for (const s of p.sessions) {
      if (best === null || s.since > best.since) best = { root: p.root, since: s.since };
    }
  }
  return best?.root ?? null;
}

/** The chosen root while it is still a choice, else no project. */
export function keptRoot(chosen: string | null, choices: ProjectChoice[]): string | null {
  return chosen !== null && choices.some((c) => c.root === chosen) ? chosen : null;
}

/** The remembered agent while it is still installed, else the first one; null when there is none. */
export function chooseAgent(targets: LauncherAgent[], saved: string): LauncherAgent | null {
  return targets.find((t) => t === saved) ?? targets[0] ?? null;
}

export interface KeyInfo {
  type: "keydown" | "keyup";
  key: string;
  shiftKey: boolean;
  repeat: boolean;
  isComposing: boolean;
}

export interface EnterStep {
  armed: boolean;
  send: boolean;
  prevent: boolean;
}

/**
 * Enter sends, Shift+Enter is a new line (spec §5). The question goes on the release of Enter,
 * never on its press: the agent takes the focus within milliseconds, and a repeated keydown of a
 * held Enter landing there would send the question. A repeat or a press this field did not see
 * start never arms.
 */
export function enterStep(e: KeyInfo, armed: boolean): EnterStep {
  if (e.key !== "Enter") return { armed, send: false, prevent: false };
  if (e.type === "keyup") return { armed: false, send: armed, prevent: armed };
  if (e.isComposing || e.shiftKey) return { armed: false, send: false, prevent: false };
  return { armed: armed || !e.repeat, send: false, prevent: true };
}

export interface FollowUp {
  /** Note to show, null to collapse at once. */
  note: string | null;
  /** After the note: collapse, or back to the prompt with the question kept. */
  then: "collapse" | "prompt";
  /** The question and the file were handed over: clear them. */
  clear: boolean;
}

/** What the island does after a launch (spec §4); null is a call that never reached Rust. */
export function launchFollowUp(outcome: LaunchOutcome | null, agent: LauncherAgent, tr: Tr): FollowUp {
  if (outcome === null) return { note: tr("launcher.couldNotOpen", { agent: agentLabel(agent, tr) }), then: "prompt", clear: false };
  switch (outcome.kind) {
    case "opened":
      return { note: null, then: "collapse", clear: true };
    case "openedPasteYourself":
      return { note: tr("launcher.paste"), then: "collapse", clear: true };
    case "failed":
      return { note: `${agentLabel(agent, tr)}: ${outcome.reason}`, then: "prompt", clear: false };
  }
}
