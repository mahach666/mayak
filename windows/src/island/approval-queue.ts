// Permission and question requests that arrive while another card is on the island: the pure
// part, with no DOM, no Tauri and no State, so it can be checked on its own (island/hooks.ts).
//
// One card is on screen at a time; the others wait here in arrival order, and the agent of each
// keeps waiting for its own click. Past QUEUE_MAX the newest goes straight back to its agent.

/** Cards on screen plus waiting: past this, a request is handed back to its agent. */
export const QUEUE_MAX = 5;

export interface Waiting<T> {
  card: T;
  /** Date.now() ms when the request came: the agent's own wait started then. */
  arrivedAt: number;
}

export class ApprovalQueue<T extends { requestId: string }> {
  private items: Waiting<T>[] = [];

  get size(): number {
    return this.items.length;
  }

  has(requestId: string): boolean {
    return this.items.some((w) => w.card.requestId === requestId);
  }

  /**
   * Adds a request behind `shown` cards on screen (0 or 1). False, and nothing added, when the
   * line is full or the request is already in it.
   */
  push(card: T, arrivedAt: number, shown: number): boolean {
    if (this.has(card.requestId) || shown + this.items.length >= QUEUE_MAX) return false;
    this.items.push({ card, arrivedAt });
    return true;
  }

  /** The oldest waiting request, taken out of the line. */
  shift(): Waiting<T> | null {
    return this.items.shift() ?? null;
  }

  /** A request that is over on the other side (withdrawn, timed out): out of the line. */
  remove(requestId: string): boolean {
    const i = this.items.findIndex((w) => w.card.requestId === requestId);
    if (i < 0) return false;
    this.items.splice(i, 1);
    return true;
  }

  /** Everything still waiting, taken out of the line. */
  drain(): T[] {
    const all = this.items.map((w) => w.card);
    this.items = [];
    return all;
  }
}

/**
 * Takes the oldest request still in time out of the line, with how long it has left. The ones met
 * before it that are out of time, or that `mayShow` refuses now (alerts held back since they came),
 * are taken out too and returned in `dropped`: their agents ask in their own windows.
 */
export function nextInTime<T extends { requestId: string }>(
  line: ApprovalQueue<T>,
  now: number,
  limitMs: number,
  mayShow: (card: T) => boolean,
): { next: { card: T; leftMs: number } | null; dropped: { card: T; why: "limit" | "held back" }[] } {
  const dropped: { card: T; why: "limit" | "held back" }[] = [];
  for (let w = line.shift(); w; w = line.shift()) {
    const leftMs = remainingMs(w.arrivedAt, now, limitMs);
    if (leftMs <= 0) dropped.push({ card: w.card, why: "limit" });
    else if (!mayShow(w.card)) dropped.push({ card: w.card, why: "held back" });
    else return { next: { card: w.card, leftMs }, dropped };
  }
  return { next: null, dropped };
}

/**
 * Requests of the CLI chat that a card on the island kept off it (island/agent-chat.ts), request
 * id → conversation, oldest first. Only those come up again once the cards are answered; one that
 * opened the island when it came is the chat's own business from then on.
 */
export class KeptOffChat {
  private items = new Map<string, number>();

  get size(): number {
    return this.items.size;
  }

  /** A request of the chat came. Kept (true) only while a card is up; else it opens at once. */
  arrive(requestId: string, conv: number, cardUp: boolean): boolean {
    if (!cardUp) return false;
    this.items.set(requestId, conv);
    return true;
  }

  entries(): [string, number][] {
    return [...this.items];
  }

  /** Brought up, answered or gone: not to come up again. */
  forget(requestId: string): void {
    this.items.delete(requestId);
  }

  /** The cards were let go (folded, another view, paused): what they kept off stays in its chat. */
  clear(): void {
    this.items.clear();
  }
}

/** How long a request that came at `arrivedAt` can still wait for a click, 0 once its time is up. */
export function remainingMs(arrivedAt: number, now: number, limitMs: number): number {
  return Math.max(0, limitMs - Math.max(0, now - arrivedAt));
}

/**
 * The oldest request of the CLI chat still asking, among those met so far (request id →
 * conversation, oldest first), and the ones found answered or gone meanwhile, to forget.
 * `isPending` asks Rust; it stops at the first one still pending.
 */
export async function firstStillAsking(
  asked: Iterable<readonly [string, number]>,
  isPending: (requestId: string, conv: number) => Promise<boolean>,
): Promise<{ found: { requestId: string; conv: number } | null; gone: string[] }> {
  const gone: string[] = [];
  for (const [requestId, conv] of asked) {
    if (await isPending(requestId, conv)) return { found: { requestId, conv }, gone };
    gone.push(requestId);
  }
  return { found: null, gone };
}
