// The numbers of a session's context label and its tooltip (context-fill spec §4): pure, so the
// Node checks import it as it is (scripts/checks/context-tip.mjs).

import type { ContextFill } from "../core/bridge";
import type { Translate } from "../core/i18n-core";
import type { Key } from "../core/i18n";

/** Token count of a context label: "950", "612k", "1.2M", always rounded down (context-fill spec §4). */
export function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${Math.floor(n / 1000)}k`;
  return `${(Math.floor(n / 100_000) / 10).toFixed(1)}M`;
}

/**
 * floor(used / window × 100), at most 100, computed on integers (0.29 * 100 is 28.999… in
 * floating point); null without a window.
 */
export function contextPercent(c: ContextFill): number | null {
  if (c.window === undefined || c.window <= 0) return null;
  return Math.min(100, Math.floor((c.used * 100) / c.window));
}

/**
 * The tooltip of a context label: "Context: 612k of 1.0M (61%)", or "Context: 612k tokens" without
 * a window, then "Model: <id>" on its own line when the session named its model.
 */
export function contextTip(c: ContextFill, model: string | undefined, tr: Translate<Key>): string {
  const percent = contextPercent(c);
  const lines = [
    percent !== null && c.window !== undefined
      ? tr("sess.ctxOf", { used: formatTokens(c.used), window: formatTokens(c.window), percent })
      : tr("sess.ctxUsed", { used: formatTokens(c.used) }),
  ];
  if (model) lines.push(tr("sess.model", { model }));
  return lines.join("\n");
}
