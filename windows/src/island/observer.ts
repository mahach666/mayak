// Agent sessions → island. Both listeners are registered before the initial
// fetch, so a snapshot emitted in between is never lost; the rev guard in
// SessionsStore drops the duplicate.

import { Bridge, onSessionAlert, onSessionSilent, onSessions, type SessionsSnapshot } from "../core/bridge";
import { Sessions } from "../core/sessions";
import type { Island } from "./island";

export async function registerObserver(island: Island): Promise<void> {
  const take = (s: SessionsSnapshot) => {
    if (Sessions.accept(s)) island.onSnapshot(s);
  };
  await onSessions(take);
  await onSessionAlert((alerts) => island.handleAlerts(alerts));
  await onSessionSilent(() => island.handleSilentWake());
  const initial = await Bridge.sessionsSnapshot();
  if (initial) take(initial);
}
