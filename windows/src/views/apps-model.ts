// The Apps tab's order and visibility (own-apps, ideas §30), as pure functions over the settings
// fields: Rust lists the tiles in `appOrder` (apps.rs `ordered`), the page edits the fields here.
// No imports: a Node check runs this file as is.

/** An app the user added; wire names of Rust `settings::CustomApp`. `path` is user data. */
export interface CustomApp {
  id: string;
  name: string;
  path: string;
}

/** The fields of the settings this module edits. */
export interface AppsFields {
  appOrder: string[];
  hiddenApps: string[];
  customApps: CustomApp[];
}

/** What every user app id starts with (apps.rs CUSTOM_PREFIX). */
export const CUSTOM_PREFIX = "user-";

/** Longest name a user app keeps. */
export const NAME_MAX = 40;

/** `order` with every id of `shown` it lacks added at the end, each id once. */
export function fullOrder(order: readonly string[], shown: readonly string[]): string[] {
  const out: string[] = [];
  for (const id of [...order, ...shown]) if (!out.includes(id)) out.push(id);
  return out;
}

/**
 * The order after the tiles were dragged into `shown` (the ids on screen, in their new order):
 * the slots the shown ids held in the saved order take them in the new order, and the ids that
 * are not on screen (a hidden or uninstalled app) keep their place.
 */
export function reorder(order: readonly string[], shown: readonly string[]): string[] {
  const full = fullOrder(order, shown);
  const next = [...shown];
  return full.map((id) => (shown.includes(id) ? next.shift()! : id));
}

/** `list` with the item at `from` moved to `to` (both clamped). */
export function moveItem<T>(list: readonly T[], from: number, to: number): T[] {
  const out = [...list];
  if (from < 0 || from >= out.length) return out;
  const [item] = out.splice(from, 1);
  out.splice(Math.max(0, Math.min(to, out.length)), 0, item);
  return out;
}

/** A user app id not in `taken`, from the clock. */
export function newCustomId(taken: readonly string[], now: number): string {
  const base = `${CUSTOM_PREFIX}${Math.max(0, Math.floor(now)).toString(36)}`;
  let id = base;
  for (let n = 2; taken.includes(id); n++) id = `${base}-${n}`;
  return id;
}

/** The name a rename keeps: trimmed and cut to NAME_MAX, or null for an empty one (no rename). */
export function cleanName(name: string): string | null {
  const trimmed = name.replace(/\s+/g, " ").trim();
  return trimmed === "" ? null : [...trimmed].slice(0, NAME_MAX).join("");
}

/** Adds a picked app as the last tile; `shown` is the tiles on screen. */
export function addApp(f: AppsFields, shown: readonly string[], picked: { name: string; path: string }, now: number): AppsFields & { id: string } {
  const id = newCustomId([...f.customApps.map((a) => a.id), ...f.appOrder], now);
  const name = cleanName(picked.name) ?? picked.path.split(/[\\/]/).pop() ?? id;
  return {
    id,
    appOrder: [...fullOrder(f.appOrder, shown), id],
    hiddenApps: [...f.hiddenApps],
    customApps: [...f.customApps, { id, name, path: picked.path }],
  };
}

/** Renames a user app; an empty name or another id changes nothing. */
export function renameApp(f: AppsFields, id: string, name: string): AppsFields {
  const clean = cleanName(name);
  return {
    ...f,
    customApps: f.customApps.map((a) => (a.id === id && clean !== null ? { ...a, name: clean } : a)),
  };
}

/** Removes a user app and its place in the order. */
export function removeApp(f: AppsFields, id: string): AppsFields {
  return {
    appOrder: f.appOrder.filter((x) => x !== id),
    hiddenApps: [...f.hiddenApps],
    customApps: f.customApps.filter((a) => a.id !== id),
  };
}

/** Hides a built-in app; it keeps its place in the order for when it is shown again. */
export function hideApp(f: AppsFields, id: string): AppsFields {
  if (id.startsWith(CUSTOM_PREFIX) || f.hiddenApps.includes(id)) return f;
  return { ...f, hiddenApps: [...f.hiddenApps, id] };
}

/** Shows a hidden built-in app again (Settings). */
export function showApp(f: AppsFields, id: string): AppsFields {
  return { ...f, hiddenApps: f.hiddenApps.filter((x) => x !== id) };
}

/** A tile's box, in any one coordinate space. */
export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The slot a dragged tile goes to: the box under the point, else the one whose centre is nearest; -1 for none. */
export function dropIndex(boxes: readonly Box[], px: number, py: number): number {
  let best = -1;
  let bestD = Infinity;
  for (let i = 0; i < boxes.length; i++) {
    const b = boxes[i];
    if (px >= b.x && px < b.x + b.w && py >= b.y && py < b.y + b.h) return i;
    const d = (b.x + b.w / 2 - px) ** 2 + (b.y + b.h / 2 - py) ** 2;
    if (d < bestD) {
      bestD = d;
      best = i;
    }
  }
  return best;
}
