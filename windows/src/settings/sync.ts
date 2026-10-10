// Keeps the Settings page in step with a `settings-changed` (island quick settings, another save).
// Every control registers an update; the event runs them all, except the one the user is editing.

export type SyncEntry<S> = {
  /** The input being edited, or null for a control that is only clicked (switches). */
  el: unknown;
  update: (s: S) => void;
};

export function createSyncRegistry<S>() {
  const entries: SyncEntry<S>[] = [];
  return {
    add(el: unknown, update: (s: S) => void) {
      entries.push({ el, update });
    },
    /** Runs every update with `s`, skipping the control that is `active` (the focused element). */
    run(s: S, active: unknown) {
      for (const e of entries) {
        if (e.el != null && e.el === active) continue;
        e.update(s);
      }
    },
  };
}
