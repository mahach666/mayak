/** Ends a recording: the chosen shortcut, or null to keep the old one. */
export type Finish = (chosen: string | null) => Promise<void>;

/**
 * At most one shortcut recorder records at a time. The page has two (the island's and the
 * next waiting session's), and a mouse click does not end a recording: without this, clicking
 * the second while the first records would let one key press set both shortcuts.
 */
export function createRecordingLock() {
  let active: Finish | null = null;
  return {
    /** Ends the other recording unchanged, then `finish` is the one recording. */
    async start(finish: Finish): Promise<void> {
      const other = active;
      active = finish;
      if (other && other !== finish) await other(null);
    },
    /** `finish` has ended. A recorder that another one already replaced changes nothing. */
    end(finish: Finish): void {
      if (active === finish) active = null;
    },
    active(): Finish | null {
      return active;
    },
  };
}
