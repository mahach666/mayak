// Parts of the app that are switched on or off when it is built. The code behind a switched-off one stays.

/**
 * The chat tab (the launcher and the API chat) and the file drop that feeds it. Off since
 * 2026-10-07: the launcher could not attach a dropped file to Claude Desktop, and the tab was not
 * worth keeping. With it off, the island refuses drops instead of taking them.
 */
export const CHAT_ENABLED = false;

/**
 * The chat run by the user's own agent CLIs (Claude Code, Codex, Gemini CLI, Cursor Agent), with no
 * API key: the "Agents" tab, its view and the island opening on a permission request of the CLI
 * (src/views/agentchat.ts, src-tauri/src/chat). On since 2026-10-14. Off: no tab, no view, no call
 * to Rust, no thread.
 */
export const CLI_CHAT_ENABLED = true;
