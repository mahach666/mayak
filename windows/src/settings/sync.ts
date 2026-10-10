// Keeps the Settings page in step with a `settings-changed` (island quick settings, another save).
// Every control registers an update; the event runs them all, except the one the user is editing.

export type SyncEntry<S> = {
  /** The input being edited, or null for a control that is only clicked (switches). */
  el: unknown;
  update: (s: S) => void;
};

export function createSyncRegistry<S>() {
  const entries: SyncEntry<S>[] = [];
  /** Controls skipped by `run` because they were being edited: redrawn by `settle`. */
  const skipped = new Set<SyncEntry<S>>();
  return {
    add(el: unknown, update: (s: S) => void) {
      entries.push({ el, update });
    },
    /** Runs every update with `s`, skipping the control that is `active` (the focused element). */
    run(s: S, active: unknown) {
      for (const e of entries) {
        if (e.el != null && e.el === active) {
          skipped.add(e);
          continue;
        }
        skipped.delete(e);
        e.update(s);
      }
    },
    /**
     * The user left `el`: if a settings-changed skipped it, redraw it from `s`, the latest settings.
     * Call it after the field's own change handler has applied its value to `s` (change fires
     * before focusout), so the user's value wins over the skipped one.
     */
    settle(el: unknown, s: S) {
      for (const e of skipped) {
        if (e.el !== el) continue;
        skipped.delete(e);
        e.update(s);
      }
    },
  };
}
