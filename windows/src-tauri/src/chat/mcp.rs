//! The hook point for the Mayak MCP server (built elsewhere, `mcp_server/`): what a chat run hands
//! to the CLI it starts so that CLI can reach the server. Nothing here knows the server; whoever
//! has one calls `ChatManager::set_mcp_provider`, and every turn that starts a process asks the
//! provider anew, so a server that starts, stops or changes its token between turns is followed.
//!
//! The provider answers with the pieces the server's own `attach_for` makes: command-line
//! arguments and environment variables for the child. How they look is the server's business (it
//! checked them against the real CLIs):
//!   * Claude Code: `--mcp-config '<json>'`, with `${VAR}` in a header read from the child's
//!     environment and a per-server `timeout` (Claude ends an HTTP tool call after 60 s without it).
//!   * Codex: `-c mcp_servers.<name>.url=…`, `.bearer_token_env_var=…`, `.tool_timeout_sec=…`.
//! The token itself travels in `env`, never in `args`: a command line is readable by every process.
//!
//! Where the chat puts them: `--mcp-config` is variadic in Claude Code (it would swallow what
//! comes after it), so the adapter puts the arguments last and the prompt goes on stdin; Codex's
//! `-c` pairs go before its prompt marker. Gemini CLI and Cursor Agent take servers from their own
//! settings files only and are never given any.
//!
//! What the server calls `allowed_tools` (`--allowedTools` names for a run nobody can click
//! through) is not part of this: the chat's rule is that nothing is approved without a click, so a
//! tool of the server asks in the chat like any other.

use std::sync::Arc;

use super::agent::AgentKind;

/// What one CLI process is given to reach the server.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct McpAttach {
    /// Added to the CLI's own arguments.
    pub args: Vec<String>,
    /// Set in the CLI's environment (the token, named in `args` and never written in them).
    pub env: Vec<(String, String)>,
}

/// What a turn asks for when it starts a process, for a CLI of this kind in this conversation; None:
/// no server for this CLI right now. The conversation's id lets the server tell which chat an agent
/// speaks from (the agent's own status line in that chat's column, `set_status`).
pub type McpProvider = Arc<dyn Fn(AgentKind, u64) -> Option<McpAttach> + Send + Sync>;

/// What an adapter did with what it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpApply {
    /// Nothing was offered.
    None,
    /// The arguments and environment went into the invocation.
    Applied,
    /// This CLI cannot take them on its command line: why.
    Unsupported(&'static str),
}
