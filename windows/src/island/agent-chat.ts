// The CLI chat's request for a click (src-tauri/src/chat, views/agentchat.ts): the island opens on
// the chat, with the approval sound, under the rules of every other alert (paused, a muted
// pomodoro). Unlike a hook card the request is not released when the island folds: it stays in its
// conversation, and the CLI waits for the click, until the user answers or stops the turn.

import { onAgentChatPermission } from "../core/bridge";
import { CLI_CHAT_ENABLED } from "../core/features";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import { showAgentChat } from "../views/agentchat";
import { shouldOpenForAlert } from "./focus";
import type { Island } from "./island";

export function registerAgentChatHandlers(island: Island) {
  if (!CLI_CHAT_ENABLED) return;
  void onAgentChatPermission((request) => {
    // The conversation to show whenever the chat is next shown, whatever the island does now.
    showAgentChat(request.conv);
    const letWaits = State.settings.focusLetWaitsThrough;
    if (!shouldOpenForAlert("permission", { muting: State.muting, paused: State.paused, letWaits })) return;
    // A card of a hook is already asking: it is not replaced (hooks.ts), the request waits in the chat.
    if (State.pendingApproval != null) return;
    Sound.play("approval");
    island.alert("agentchat");
  });
}
