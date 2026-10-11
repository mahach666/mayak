// The buttons of the session cards, left to right: a pure choice with no DOM, no Tauri and no
// State, so it can be checked on its own. Every card about one session offers «↗ Open» (the
// session's terminal or editor, as on its row); the alert cards (waiting, finished, error) add
// «OK», which acknowledges the alert. The waiting card is where a permission or question the
// island let go ends up (the agent asks in its own window): Open is how the user gets there.

/** A card that shows one session (State.alertKey). */
export type SessionCardView = "waiting" | "finished" | "error" | "session";

export type CardButton = "open" | "ok";

/** The card's buttons in order; the first one is the primary button. */
export function cardButtons(view: SessionCardView): readonly CardButton[] {
  return view === "session" ? ["open"] : ["open", "ok"];
}
