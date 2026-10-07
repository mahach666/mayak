// Parts of the app that are switched off for now. The code behind them stays.

/**
 * The chat tab (the launcher and the API chat) and the file drop that feeds it. Off since
 * 2026-10-07: the launcher could not attach a dropped file to Claude Desktop, and the tab was not
 * worth keeping. With it off, the island refuses drops instead of taking them.
 */
export const CHAT_ENABLED = false;
