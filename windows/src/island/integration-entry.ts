// What an integration update makes of the pill's stored entry, and whether that changed anything.
// Pure (no Tauri, no DOM) so the Node checks can import it. Most pollers (Cal.com, Stripe, Vercel…)
// re-send their full data on every poll; an identical entry must not redraw the island.

import type { IntegrationUpdate } from "../core/bridge";
import type { IntegrationInfo } from "../core/state";

/** The entry an update leaves behind: an error keeps the last good data and its loaded flag. */
export function nextIntegration(previous: IntegrationInfo | undefined, update: IntegrationUpdate): IntegrationInfo {
  return {
    data: update.error ? (previous?.data ?? {}) : update.data,
    error: update.error,
    loaded: update.error ? (previous?.loaded ?? false) : true,
    configured: previous?.configured ?? true,
  };
}

/** Structural equality for JSON values; object key order does not matter. */
export function sameJson(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a)) {
    const bb = b as unknown[];
    return a.length === bb.length && a.every((v, i) => sameJson(v, bb[i]));
  }
  const ao = a as Record<string, unknown>;
  const bo = b as Record<string, unknown>;
  const keys = Object.keys(ao);
  if (keys.length !== Object.keys(bo).length) return false;
  return keys.every((k) => Object.prototype.hasOwnProperty.call(bo, k) && sameJson(ao[k], bo[k]));
}

export function sameIntegration(a: IntegrationInfo | undefined, b: IntegrationInfo): boolean {
  return (
    a !== undefined &&
    a.error === b.error &&
    a.loaded === b.loaded &&
    a.configured === b.configured &&
    sameJson(a.data, b.data)
  );
}

/** handle()'s gate: store and redraw unless the update carries no event and changes nothing. */
export function shouldStore(previous: IntegrationInfo | undefined, next: IntegrationInfo, event: unknown): boolean {
  return !!event || !sameIntegration(previous, next);
}
