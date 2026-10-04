// Agent sessions → island. The snapshot listener is registered before the
// initial fetch, so a snapshot emitted in between is never lost; the rev guard
// in SessionsStore drops the duplicate.

import { Bridge, onSessions, type SessionsSnapshot } from "../core/bridge";
import { Sessions } from "../core/sessions";
import type { Island } from "./island";

export async function registerObserver(island: Island): Promise<void> {
  const take = (s: SessionsSnapshot) => {
    if (Sessions.accept(s)) island.onSnapshot(s);
  };
  await onSessions(take);
  const initial = await Bridge.sessionsSnapshot();
  if (initial) take(initial);
}
