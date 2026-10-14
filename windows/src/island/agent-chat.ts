// The CLI chat's request for a click (src-tauri/src/chat, views/agentchat.ts): the island opens on
// the chat, with the approval sound, under the rules of every other alert (paused, a muted
// pomodoro). Unlike a hook card the request is not released when the island folds: it stays in its
// conversation, and the CLI waits for the click, until the user answers or stops the turn.
//
// A hook's or an MCP agent's card on the island is not replaced by it (hooks.ts): the request
// waits in its chat, and comes up once the last of those cards has been answered.

import { Bridge, onAgentChatPermission } from "../core/bridge";
import { pendingPermissions } from "../core/agent-chat";
import { CLI_CHAT_ENABLED } from "../core/features";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import { showAgentChat } from "../views/agentchat";
import { firstStillAsking } from "./approval-queue";
import { shouldOpenForAlert } from "./focus";
import type { Island } from "./island";

/** The chat's requests met so far, request id → conversation, oldest first; Rust says which still ask. */
const asked = new Map<string, number>();

function mayOpen(): boolean {
  const letWaits = State.settings.focusLetWaitsThrough;
  return shouldOpenForAlert("permission", { muting: State.muting, paused: State.paused, letWaits });
}

export function registerAgentChatHandlers(island: Island) {
  if (!CLI_CHAT_ENABLED) return;
  void onAgentChatPermission((request) => {
    asked.set(request.requestId, request.conv);
    // The conversation to show whenever the chat is next shown, whatever the island does now.
    showAgentChat(request.conv);
    if (!mayOpen()) return;
    // A card of a hook is already asking: it is not replaced (hooks.ts), the request waits in the chat.
    if (State.pendingApproval != null) return;
    Sound.play("approval");
    island.alert("agentchat");
  });
}

/**
 * The last hook card was answered with the island open: the oldest request of the chat still
 * waiting for a click comes up, as it would have when it came. Nothing when the island was folded
 * meanwhile, another card came, or alerts are held back.
 */
export async function resumeChatRequest(island: Island): Promise<void> {
  if (!CLI_CHAT_ENABLED || asked.size === 0) return;
  const { found, gone } = await firstStillAsking(asked, async (requestId, conv) => {
    const convo = await Bridge.agentChatGet(conv, null);
    return pendingPermissions(convo).some((m) => m.requestId === requestId);
  });
  for (const id of gone) asked.delete(id);
  if (!found || State.pendingApproval != null || State.mode !== "expanded" || !mayOpen()) return;
  showAgentChat(found.conv);
  Sound.play("approval");
  island.alert("agentchat");
}
