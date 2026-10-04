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
}
