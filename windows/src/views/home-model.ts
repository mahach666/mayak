// The home page's pure parts (views/usage.ts): the 2 × 2 table of plan-limit rings, the short
// balance line, the rows a provider's popover shows and where the popover goes, and the integration
// tiles (which, how they pack, how many rows the island grows to). No DOM and no runtime imports but
// pure modules, so scripts/checks/home-rings.mjs runs these functions as they are.

import type { Usage } from "../core/bridge";
import type { Translate } from "../core/i18n-core";
import type { Key } from "../core/i18n";
import type { AgentTask, IntegrationInfo, PillBadge } from "../core/state";
import { claudeWindowMs } from "./usage-pace.ts";
import {
  money, moneyIn, moneyWhole, PROVIDER_TITLE, usageBlocks, usagePercent,
  type BalanceProviderId, type PercentTone, type UsageBlock,
} from "./usage-format.ts";

type Tr = Translate<Key>;

// ── Rings ─────────────────────────────────────────────────────────────────────

/** A ring's colour: percentTone's, or "none" for a window without a number (dashed track, "—"). */
export type RingTone = PercentTone | "none";

export interface RingCell {
  /** 0-100, rounded down; null for a new window not read yet, or no window at all. */
  percent: number | null;
  /** What the ring says inside: the percent without its sign, or "—". */
  text: string;
  tone: RingTone;
}

export const EMPTY_RING: RingCell = { percent: null, text: "—", tone: "none" };

/** The table's ring: 26 px, a 2.5 px stroke. */
export const RING_SIZE = 26;
export const RING_STROKE = 2.5;

export interface RingGeometry {
  /** Centre, both axes. */
  c: number;
  r: number;
  circumference: number;
  /** The fill's stroke-dasharray, two decimals; null at 0 % (a round cap would still draw a dot). */
  dash: string | null;
}

const two = (n: number) => Math.round(n * 100) / 100;

/** The circles of a ring of `size` px with a `stroke` px line, filled to `percent` (held within 0-100). */
export function ringGeometry(size: number, stroke: number, percent: number | null): RingGeometry {
  const c = size / 2;
  const r = (size - stroke) / 2;
  const circumference = 2 * Math.PI * r;
  const p = percent == null || !Number.isFinite(percent) ? 0 : Math.max(0, Math.min(100, percent));
  return { c, r, circumference, dash: p > 0 ? `${two((circumference * p) / 100)} ${two(circumference)}` : null };
}

/** One window's ring: its percent (usagePercent), or EMPTY_RING once its reset time has passed. */
export function ringCell(percent: number, resetsAtMs: number | undefined, nowMs: number): RingCell {
  if (resetsAtMs !== undefined && resetsAtMs <= nowMs) return EMPTY_RING;
  const p = usagePercent(percent);
  return { percent: p.ring, text: String(p.ring), tone: p.tone };
}

/** The table's columns, by window length: 5 hours, then 7 days. */
export const RING_COLUMNS_MS = [5 * 3_600_000, 7 * 86_400_000] as const;

/** The column headers, "5h" and "7d" in the page's language. */
export function ringColumns(tr: Tr): [string, string] {
  return [tr("unit.h", { n: 5 }), tr("unit.d", { n: 7 })];
}

export type RingProviderId = "claude" | "codex";

export interface RingRow {
  id: RingProviderId;
  title: string;
  /** One per RING_COLUMNS_MS column. */
  cells: [RingCell, RingCell];
}

/**
 * The rows of the table, Claude then Codex, always both. A cell is the provider's window of that
 * length (Claude: "5h" and "7d", not a model's own weekly window such as "7d:opus", which only the
 * popover lists; Codex: 300 and 10080 minutes); EMPTY_RING without one or without data.
 */
export function ringRows(usage: Usage | null | undefined, nowMs: number): RingRow[] {
  const claude = (usage?.claude?.windows ?? [])
    .filter((w) => !w.id.includes(":"))
    .map((w) => ({ ms: claudeWindowMs(w.id), percent: w.percent, resetsAtMs: w.resetsAtMs }));
  const codex = (usage?.codex?.windows ?? [])
    .map((w) => ({ ms: w.minutes * 60_000, percent: w.percent, resetsAtMs: w.resetsAtMs }));
  const row = (id: RingProviderId, windows: typeof claude): RingRow => {
    const [a, b] = RING_COLUMNS_MS.map((ms) => {
      const w = windows.find((x) => x.ms === ms);
      return w ? ringCell(w.percent, w.resetsAtMs, nowMs) : EMPTY_RING;
    });
    return { id, title: PROVIDER_TITLE[id], cells: [a, b] };
  };
  return [row("claude", claude), row("codex", codex)];
}

/** The row's name for a screen reader: "Claude: 5h 42%, 7d No data yet". */
export function ringRowLabel(row: RingRow, tr: Tr): string {
  const cols = ringColumns(tr);
  const cell = (c: RingCell, i: number) => `${cols[i]} ${c.percent == null ? tr("usage.noData") : `${c.percent}%`}`;
  return `${row.title}: ${row.cells.map(cell).join(", ")}`;
}

// ── The balance line ──────────────────────────────────────────────────────────

export interface BalanceChip {
  id: BalanceProviderId;
  title: string;
  /** The main sum (balancesBlock's): OpenRouter what is left of the key limit, else what was spent. */
  text: string;
  /** OpenRouter with a key limit: the share of it spent, for a 12 px ring; null otherwise. */
  ring: number | null;
  /** Of the ring; "full" too for a balance too low for API calls. */
  tone: PercentTone;
}

/** With this many balances or more the line is compact: whole units ("$12 · ¥86 · $50"). */
export const BALANCE_COMPACT_FROM = 3;

/** The balance line under the rings, in PROVIDER_ORDER; [] without any balance read. */
export function balanceChips(usage: Usage | null | undefined): BalanceChip[] {
  const chips: (Omit<BalanceChip, "text"> & { full: string; whole: string })[] = [];
  const or = usage?.openrouter;
  if (or) {
    const limited = or.limit !== null && or.limit > 0 && or.limitRemaining !== null;
    const p = limited ? usagePercent((1 - or.limitRemaining! / or.limit!) * 100) : null;
    const sum = limited ? or.limitRemaining! : or.usage;
    chips.push({ id: "openrouter", title: PROVIDER_TITLE.openrouter, ring: p?.ring ?? null, tone: p?.tone ?? "dim", full: money(sum), whole: moneyWhole(sum, "USD") });
  }
  const ds = usage?.deepseek;
  if (ds) {
    const first = ds.balances[0];
    chips.push({
      id: "deepseek", title: PROVIDER_TITLE.deepseek, ring: null, tone: ds.isAvailable ? "dim" : "full",
      full: first ? moneyIn(first.total, first.currency) : "—", whole: first ? moneyWhole(first.total, first.currency) : "—",
    });
  }
  const ms = usage?.moonshot;
  if (ms) {
    chips.push({
      id: "moonshot", title: PROVIDER_TITLE.moonshot, ring: null, tone: ms.available > 0 ? "dim" : "full",
      full: moneyIn(ms.available, ms.currency), whole: moneyWhole(ms.available, ms.currency),
    });
  }
  const compact = chips.length >= BALANCE_COMPACT_FROM;
  return chips.map(({ full, whole, ...c }) => ({ ...c, text: compact ? whole : full }));
}

// ── Popovers ──────────────────────────────────────────────────────────────────

/** What a popover is about: a ring row, or the balance line. */
export type PopoverGroup = RingProviderId | "balances";

/** The blocks a popover shows: the provider's own, or every balance block (homeBlocks decides how many). */
export function popoverBlocks(group: PopoverGroup, usage: Usage | undefined, nowMs: number, tr: Tr): UsageBlock[] {
  return usageBlocks(usage, nowMs, tr).filter((b) =>
    group === "balances" ? b.id !== "claude" && b.id !== "codex" : b.id === group);
}

export type PopoverRow =
  /** A block's title and the age of its data ("" for an empty block). */
  | { kind: "head"; title: string; updated: string }
  /** A window or a balance: the label (a percent with its ring, or a sum), its name, the reset or detail. */
  | { kind: "line"; label: string; ring: number | null; tone: PercentTone; name: string; resets: string }
  /** A window's whole pace under it ("+14% ahead · runs out in ~2d 8h"). */
  | { kind: "pace"; text: string }
  /** "No data yet" for a provider without data. */
  | { kind: "empty"; text: string }
  /** Where the data come from (an empty block), or a balance too low. */
  | { kind: "note"; text: string }
  /** Between two blocks. */
  | { kind: "sep" };

/** The rows of a popover over `blocks`: everything the home page's blocks showed, the pace whole. */
export function popoverRows(blocks: UsageBlock[], tr: Tr): PopoverRow[] {
  const rows: PopoverRow[] = [];
  blocks.forEach((b, i) => {
    if (i > 0) rows.push({ kind: "sep" });
    rows.push({ kind: "head", title: b.title, updated: b.updated });
    if (b.empty) rows.push({ kind: "empty", text: tr("usage.noData") });
    for (const l of b.lines) {
      rows.push({
        kind: "line", label: l.percent.text, ring: l.amount ? null : l.percent.ring, tone: l.percent.tone,
        name: l.name, resets: l.resets,
      });
      if (l.paceTitle) rows.push({ kind: "pace", text: l.paceTitle });
    }
    if (b.note) rows.push({ kind: "note", text: b.note });
  });
  return rows;
}

/** Between the popover and its anchor, and from the view's sides. */
export const POP_GAP = 6;
export const POP_EDGE = 6;
/**
 * How far below the view the popover may reach: 4 of the 10 px under the card (#content's padding,
 * or the player footer's gap above its row), so it never covers the player.
 */
export const POP_BOTTOM_SLACK = 4;

export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * Where a popover of `pop` size goes in a view of `area` size (coordinates of the view, whose top is
 * the header's bottom): left of its anchor (the ring column is on the card's right), else right of
 * it, centred on it vertically. It never goes above the view (the header and its top-centre stay
 * clear) nor past POP_BOTTOM_SLACK under it; taller than that, it gets `maxH` and scrolls.
 */
export function placePopover(
  anchor: Box, pop: { width: number; height: number }, area: { width: number; height: number },
): { left: number; top: number; maxH: number } {
  const maxH = Math.max(0, area.height + POP_BOTTOM_SLACK);
  const h = Math.min(pop.height, maxH);
  let left = anchor.left - POP_GAP - pop.width;
  if (left < POP_EDGE) left = anchor.left + anchor.width + POP_GAP;
  left = Math.max(POP_EDGE, Math.min(area.width - POP_EDGE - pop.width, left));
  const top = Math.max(0, Math.min(maxH - h, anchor.top + anchor.height / 2 - h / 2));
  return { left, top, maxH };
}

// ── Integration tiles ─────────────────────────────────────────────────────────

/** Settings → Integrations allows four (settings/main.ts MAX_ACTIVE); the page shows four at most. */
export const HOME_MAX_TILES = 4;

/** The tiles: the active integrations (State.tasks, in their declared order) with a key saved. */
export function homeTileIds(
  tasks: readonly Pick<AgentTask, "id">[],
  integrations: Readonly<Record<string, Pick<IntegrationInfo, "configured"> | undefined>>,
): string[] {
  return tasks.filter((t) => integrations[t.id]?.configured === true).slice(0, HOME_MAX_TILES).map((t) => t.id);
}

/**
 * Whether the tile area shows the "connect services" hint: only once the first key check has
 * answered (island/integrations.ts refreshConfigured sets State.integrationsChecked, also when it
 * fails) and no integration became a tile. Until then the area stays empty, so the hint never
 * flashes at startup for someone whose integrations are about to show.
 */
export function homeHintShown(checked: boolean, tiles: number): boolean {
  return checked && tiles === 0;
}

/** Rows of tiles, which sets the island's height (core/layout.ts HOME_ROW_H): 1 up to two tiles (or none), else 2. */
export function homeTileRows(tiles: number): number {
  return Math.max(1, Math.min(2, Math.ceil(Math.min(tiles, HOME_MAX_TILES) / 2)));
}

/** Which tiles take the whole width: two per row, so an odd last one (1 or 3 tiles) is wide. */
export function tileWide(tiles: number): boolean[] {
  const n = Math.max(0, Math.min(tiles, HOME_MAX_TILES));
  return Array.from({ length: n }, (_, i) => n % 2 === 1 && i === n - 1);
}

/**
 * The badge in a tile's corner, as the pills had it: the pill badge an event left (approval,
 * finished, error), else the finished / error state an event put the task in (it goes back to idle
 * by itself, island/integrations.ts); null otherwise.
 */
export function tileBadge(task: Pick<AgentTask, "pillBadge" | "state">): PillBadge | null {
  if (task.pillBadge) return task.pillBadge;
  return task.state === "finished" ? "finished" : task.state === "error" ? "error" : null;
}

/**
 * A tile on a red tint: its task is in error, a GitLab pipeline or the CI of one of my GitHub pull
 * requests failed (ci.failed), or Vercel's latest deployment failed (state ERROR).
 */
export function tileAlert(id: string, task: Pick<AgentTask, "state">, data: Record<string, unknown> | undefined): boolean {
  if (task.state === "error") return true;
  if (id === "integration_gitlab" || id === "integration_github") {
    const ci = data?.ci as Record<string, unknown> | undefined;
    return Number(ci?.failed ?? 0) > 0;
  }
  if (id === "integration_vercel") {
    const deployments = data?.deployments;
    return Array.isArray(deployments) && (deployments[0] as Record<string, unknown> | undefined)?.state === "ERROR";
  }
  return false;
}

// ── GitHub tile ───────────────────────────────────────────────────────────────

/** One row of the GitHub tile: a pull request waiting for my review, or one of mine whose CI failed. */
export interface GithubRow {
  kind: "review" | "failed";
  title: string;
  /** `#12`. */
  number: string;
  /** github.com only (Rust dropped any other link, and opens only those). */
  url: string | null;
  draft: boolean;
}

function prs(data: Record<string, unknown> | undefined, key: string): Record<string, unknown>[] {
  const v = data?.[key];
  return Array.isArray(v) ? (v as Record<string, unknown>[]) : [];
}

/** True when the GitHub card carries pull requests (src-tauri github.rs card_data read them). */
export function githubHasPullRequests(data: Record<string, unknown> | undefined): boolean {
  return Array.isArray(data?.mine);
}

/** The GitHub tile's rows: review requests first, then my pull requests whose CI failed; `max` of them. */
export function githubRows(data: Record<string, unknown> | undefined, max: number): GithubRow[] {
  const row = (kind: GithubRow["kind"], p: Record<string, unknown>): GithubRow => ({
    kind,
    title: String(p.title ?? ""),
    number: `#${Number(p.number ?? 0)}`,
    url: typeof p.url === "string" ? p.url : null,
    draft: p.draft === true,
  });
  const rows = [
    ...prs(data, "reviews").map((p) => row("review", p)),
    ...prs(data, "mine").filter((p) => p.ci === "failed").map((p) => row("failed", p)),
  ];
  return rows.slice(0, Math.max(0, max));
}
