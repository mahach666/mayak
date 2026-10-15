// The CLI chat's request for a click (src-tauri/src/chat, views/agentchat.ts): the island opens on
// the chat, with the approval sound, under the rules of every other alert (paused, a muted
// pomodoro). Unlike a hook card the request is not released when the island folds: it stays in its
// conversation, and the CLI waits for the click, until the user answers or stops the turn.
//
// A hook's or an MCP agent's card on the island is not replaced by it (hooks.ts): the request
// waits in its chat, and comes up once the last of those cards has been answered.

import { Bridge, acceptFileDrops, onAgentChatPermission } from "../core/bridge";
import { pendingPermissions } from "../core/agent-chat";
import { CLI_CHAT_ENABLED } from "../core/features";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import { dragOverAgentChat, dropOnAgentChat, showAgentChat } from "../views/agentchat";
import type { IslandViewName } from "../core/layout";
import { KeptOffChat, firstStillAsking } from "./approval-queue";
import { shouldOpenForAlert } from "./focus";
import type { Island } from "./island";

/** The chat's requests a card kept off the island, oldest first; Rust says which still ask. */
const keptOff = new KeptOffChat();

function mayOpen(): boolean {
  const letWaits = State.settings.focusLetWaitsThrough;
  return shouldOpenForAlert("permission", { muting: State.muting, paused: State.paused, letWaits });
}

export function registerAgentChatHandlers(island: Island) {
  if (!CLI_CHAT_ENABLED) return;
  void onAgentChatPermission((request) => {
    // The conversation to show whenever the chat is next shown, whatever the island does now.
    showAgentChat(request.conv);
    if (!mayOpen()) return;
    // A card of a hook is already asking: it is not replaced (hooks.ts), the request waits in the
    // chat and comes up once the cards are answered (resumeChatRequest).
    if (keptOff.arrive(request.requestId, request.conv, State.pendingApproval != null)) return;
    Sound.play("approval");
    island.alert("agentchat");
  });
}

/** The cards were let go without an answer (hooks.ts releaseCard): what they kept off stays in its chat. */
export function dropKeptOffChat() {
  keptOff.clear();
}

/** The pages of the island's tabs: a file dragged over one of them takes the island to the Agents chat. */
const PAGES = new Set<IslandViewName>(["empty", "overview", "sessions", "stats", "focus", "apps", "agentchat"]);

/**
 * Files dragged over the island: it opens on the Agents chat (unless a card is asking for something,
 * which a drag never takes away), the field says what a drop does, and the files hang on the open
 * chat's next message. Wherever on the island they land: the whole page takes the drag. The island's
 * other views turn files away, as before (CHAT_ENABLED).
 */
export function registerAgentChatDrops(island: Island) {
  if (!CLI_CHAT_ENABLED) return;
  acceptFileDrops(document.documentElement, (e) => {
    void Bridge.log(`chat drag ${e.type}${e.type === "drop" ? ` ${e.files.length} file(s), view ${State.view}` : ` (${State.mode}, ${State.view})`}`);
    if (e.type === "drop") {
      dragOverAgentChat(false);
      if (!State.paused && State.view === "agentchat") dropOnAgentChat(e.files);
    } else if (e.type === "enter") {
      if (State.paused) return;
      if (State.mode !== "expanded" || (State.view !== "agentchat" && PAGES.has(State.view))) island.alert("agentchat", { silent: true });
      dragOverAgentChat(true);
    } else {
      dragOverAgentChat(false);
    }
  });
}

/**
 * The last hook card was answered with the island open: the oldest request of the chat that a card
 * kept off and that still waits for a click comes up, as it would have when it came, once; the
 * other kept-off ones are then the chat's own business. Nothing
 * when the island was folded meanwhile, another card came, a focus phase card is up or held, or
 * alerts are held back.
 */
export async function resumeChatRequest(island: Island): Promise<void> {
  if (!CLI_CHAT_ENABLED || keptOff.size === 0) return;
  const { found, gone } = await firstStillAsking(keptOff.entries(), async (requestId, conv) => {
    const convo = await Bridge.agentChatGet(conv, null);
    return pendingPermissions(convo).some((m) => m.requestId === requestId);
  });
  for (const id of gone) keptOff.forget(id);
  // Another card came meanwhile: the request waits behind it in turn.
  if (!found || State.pendingApproval != null) return;
  // Now or never, for all of them: the chat shows its own waiting requests from here on. Nothing
  // comes up when the island folded meanwhile, alerts are held back, or a focus phase card that
  // closeCard opened (or still holds) comes first; the requests stay in their chats either way.
  keptOff.clear();
  if (State.mode !== "expanded" || !mayOpen() || State.phaseCardPending || State.view === "focus-phase") return;
  showAgentChat(found.conv);
  Sound.play("approval");
  island.alert("agentchat");
}
