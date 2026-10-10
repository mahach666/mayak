// Identity of sessions: one place builds the keys the router, the sources and
// the model all use.

/// `claude:<pid>:<created_ft>` or `codex:<session_id>`.
pub type SessionKey = String;

/// A Claude session is its `claude.exe` process: pid plus creation FILETIME
/// (decimal), so a reused pid never inherits an old row.
pub fn claude_key(pid: u32, created_ft: u64) -> SessionKey {
    format!("claude:{pid}:{created_ft}")
}

/// A Codex session is its `session_id`; subagents share the parent's.
pub fn codex_key(session_id: &str) -> SessionKey {
    format!("codex:{session_id}")
}

/// An Antigravity session is its `conversationId`.
pub fn antigravity_key(conversation_id: &str) -> SessionKey {
    format!("antigravity:{conversation_id}")
}

/// A process proven alive by `win::proc::identify` (spec §5.4): `created_ft` is the creation
/// FILETIME from GetProcessTimes (Claude's `procStart`), `image` the full Win32 image path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProcId {
    pub pid: u32,
    pub created_ft: u64,
    pub image: String,
}

/// The app that hosts an agent process (spec §5.4, §5.5 item 5, §5.6). Wire names (spec §10):
/// "claudeDesktop" | "cursor" | "vsCode" | "codexDesktop" | "antigravityIde" | "antigravityCli" |
/// "unknown".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HostKind {
    ClaudeDesktop,
    Cursor,
    VsCode,
    CodexDesktop,
    /// Antigravity's own agent (the IDE, or Antigravity 2.0).
    AntigravityIde,
    /// Antigravity's `agy` CLI.
    AntigravityCli,
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_keys_are_pid_and_decimal_creation_time() {
        assert_eq!(claude_key(30584, 134_043_100_123_456_789), "claude:30584:134043100123456789");
        assert_ne!(claude_key(1, 2), claude_key(1, 3));
    }

    #[test]
    fn codex_keys_are_the_session_id() {
        assert_eq!(codex_key("0199a-thread"), "codex:0199a-thread");
    }

    #[test]
    fn antigravity_keys_are_the_conversation_id() {
        assert_eq!(antigravity_key("conv-1"), "antigravity:conv-1");
    }

    #[test]
    fn host_kind_wire_names_are_camel_case() {
        let names: Vec<String> = [
            HostKind::ClaudeDesktop,
            HostKind::Cursor,
            HostKind::VsCode,
            HostKind::CodexDesktop,
            HostKind::AntigravityIde,
            HostKind::AntigravityCli,
            HostKind::Unknown,
        ]
        .iter()
        .map(|h| serde_json::to_value(h).unwrap().as_str().unwrap().to_string())
        .collect();
        assert_eq!(names, ["claudeDesktop", "cursor", "vsCode", "codexDesktop", "antigravityIde", "antigravityCli", "unknown"]);
    }
}
