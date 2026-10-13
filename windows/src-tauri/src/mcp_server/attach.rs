// How an agent CLI is told about Mayak's MCP server.
//
// Two jobs, both pure text:
//   * `attach` — the arguments and environment of a CLI that Mayak itself starts (the chat), so it
//     talks to this server for that one run. The token goes in the child's environment and is named
//     in the arguments, never written in them: a command line is readable by every process.
//   * `install_command` — the one line the user pastes into their own terminal to add the server to
//     an agent they run themselves. That line holds the token (the CLI keeps it in its own config),
//     so Settings hands it over by the clipboard, from Rust; "New token" revokes every copy of it.
//
// Nothing here writes to a CLI's config. Each flag below was read from `--help` and run against
// this server on this machine: Claude Code 2.1.296 and Codex 0.162.1.

use super::tools;
use super::SERVER_NAME;

/// The environment variable the token travels in, to the CLI Mayak starts.
pub const TOKEN_ENV: &str = "MAYAK_MCP_TOKEN";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cli {
    Claude,
    Codex,
}

impl Cli {
    pub fn parse(name: &str) -> Option<Cli> {
        match name {
            "claude" => Some(Cli::Claude),
            "codex" => Some(Cli::Codex),
            _ => None,
        }
    }
}

/// What a CLI Mayak starts needs to reach the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attach {
    /// Arguments to put among the CLI's own. A variadic option (`--mcp-config`) ends where the next
    /// option starts, so the prompt must not come right after these: send it on stdin or after `--`.
    pub args: Vec<String>,
    /// Environment variables to set on the child.
    pub env: Vec<(String, String)>,
    /// Claude Code only: the tools to name in `--allowedTools` for a run nobody can click through
    /// (`mcp__mayak__list_sessions`…): the read-only ones and `ask_user`, whose gate is the user's
    /// click on the island. Codex runs a server's tools in `codex exec` without a flag (read on a
    /// real run).
    pub allowed_tools: Vec<String>,
}

fn claude_tool(name: &str) -> String {
    format!("mcp__{SERVER_NAME}__{name}")
}

/// How long Claude Code lets one tool call of this server run, in milliseconds: the longest
/// `ask_user` wait and a margin. Without it Claude Code cuts a call off after 60 s (read off a real
/// run: "tool ask_user timed out after 60s"); with it a 130 s wait was answered.
const CLAUDE_TOOL_TIMEOUT_MS: u64 = (tools::ASK_MAX_SECS + 30) * 1000;

/// `--mcp-config`'s JSON for Claude Code. `${VAR}` in a header is read from the environment of the
/// run (read off a real run: the server got the token from `MAYAK_MCP_TOKEN`).
fn claude_config(url: &str) -> String {
    serde_json::json!({
        "mcpServers": {
            SERVER_NAME: {
                "type": "http",
                "url": url,
                "headers": { "Authorization": format!("Bearer ${{{TOKEN_ENV}}}") },
                "timeout": CLAUDE_TOOL_TIMEOUT_MS,
            }
        }
    })
    .to_string()
}

/// A TOML basic string for a `-c key=value` of Codex (its value is parsed as TOML).
fn toml_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// For a CLI Mayak starts. `url` is the server's address, `token` the bearer token.
pub fn attach(cli: Cli, url: &str, token: &str) -> Attach {
    let env = vec![(TOKEN_ENV.to_string(), token.to_string())];
    match cli {
        Cli::Claude => Attach {
            args: vec!["--mcp-config".into(), claude_config(url)],
            env,
            allowed_tools: tools::names().iter().map(|n| claude_tool(n)).collect(),
        },
        Cli::Codex => {
            let key = |name: &str| format!("mcp_servers.{SERVER_NAME}.{name}");
            let mut args = Vec::new();
            for (name, value) in [
                ("url", toml_string(url)),
                ("bearer_token_env_var", toml_string(TOKEN_ENV)),
                // Codex documents 60 s as the default. A 130 s wait went through without this on
                // 0.162.1, but the longest `ask_user` wait should not rest on that.
                ("tool_timeout_sec", (tools::ASK_MAX_SECS + 10).to_string()),
            ] {
                args.push("-c".to_string());
                args.push(format!("{}={value}", key(name)));
            }
            Attach { args, env, allowed_tools: Vec::new() }
        }
    }
}

/// How the pasted line is written: PowerShell on Windows, a POSIX shell elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    PowerShell,
    Posix,
}

impl Shell {
    pub fn native() -> Shell {
        if cfg!(windows) { Shell::PowerShell } else { Shell::Posix }
    }
}

/// The line to paste to add the server to an agent the user runs themselves.
pub fn install_command(cli: Cli, shell: Shell, url: &str, token: &str) -> String {
    match cli {
        // `--scope user`: available in every project, not only in the folder it is run from.
        Cli::Claude => format!(
            "claude mcp add --transport http --scope user {SERVER_NAME} {url} --header \"Authorization: Bearer {token}\""
        ),
        // `codex mcp add` can only name the variable the token is read from, so the line sets it first.
        Cli::Codex => {
            let set = match shell {
                Shell::PowerShell => format!("setx {TOKEN_ENV} \"{token}\""),
                Shell::Posix => format!("export {TOKEN_ENV}=\"{token}\""),
            };
            format!("{set}; codex mcp add {SERVER_NAME} --url {url} --bearer-token-env-var {TOKEN_ENV}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://127.0.0.1:47831/mcp";
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn claude_gets_a_config_that_names_the_token_variable_not_the_token() {
        let a = attach(Cli::Claude, URL, TOKEN);
        assert_eq!(a.args[0], "--mcp-config");
        assert_eq!(a.args.len(), 2);
        let config: serde_json::Value = serde_json::from_str(&a.args[1]).unwrap();
        let server = &config["mcpServers"]["mayak"];
        assert_eq!(server["type"], "http");
        assert_eq!(server["url"], URL);
        assert_eq!(server["headers"]["Authorization"], "Bearer ${MAYAK_MCP_TOKEN}");
        // Longer than the longest wait of `ask_user` (240 s), or Claude Code ends the call at 60 s.
        assert_eq!(server["timeout"], 270_000);
        assert_eq!(a.env, [("MAYAK_MCP_TOKEN".to_string(), TOKEN.to_string())]);
        assert!(a.args.iter().all(|arg| !arg.contains(TOKEN)), "the token is not on the command line");
    }

    #[test]
    fn claude_may_call_the_read_only_tools_and_ask_user_by_their_full_names() {
        let a = attach(Cli::Claude, URL, TOKEN);
        assert_eq!(a.allowed_tools.len(), 6);
        assert!(a.allowed_tools.contains(&"mcp__mayak__list_sessions".to_string()));
        assert!(a.allowed_tools.contains(&"mcp__mayak__ask_user".to_string()));
        assert!(a.allowed_tools.iter().all(|t| t.starts_with("mcp__mayak__")));
    }

    #[test]
    fn codex_gets_toml_overrides_that_name_the_token_variable() {
        let a = attach(Cli::Codex, URL, TOKEN);
        assert_eq!(
            a.args,
            [
                "-c",
                "mcp_servers.mayak.url=\"http://127.0.0.1:47831/mcp\"",
                "-c",
                "mcp_servers.mayak.bearer_token_env_var=\"MAYAK_MCP_TOKEN\"",
                "-c",
                "mcp_servers.mayak.tool_timeout_sec=250",
            ]
        );
        assert_eq!(a.env, [("MAYAK_MCP_TOKEN".to_string(), TOKEN.to_string())]);
        assert!(a.args.iter().all(|arg| !arg.contains(TOKEN)));
        assert!(a.allowed_tools.is_empty());
    }

    #[test]
    fn toml_strings_are_escaped() {
        assert_eq!(toml_string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\u000Ad\"");
    }

    #[test]
    fn the_install_lines_carry_the_token_and_use_the_shell_of_the_platform() {
        assert_eq!(
            install_command(Cli::Claude, Shell::PowerShell, URL, TOKEN),
            format!("claude mcp add --transport http --scope user mayak {URL} --header \"Authorization: Bearer {TOKEN}\"")
        );
        assert_eq!(
            install_command(Cli::Codex, Shell::PowerShell, URL, TOKEN),
            format!("setx MAYAK_MCP_TOKEN \"{TOKEN}\"; codex mcp add mayak --url {URL} --bearer-token-env-var MAYAK_MCP_TOKEN")
        );
        assert_eq!(
            install_command(Cli::Codex, Shell::Posix, URL, TOKEN),
            format!("export MAYAK_MCP_TOKEN=\"{TOKEN}\"; codex mcp add mayak --url {URL} --bearer-token-env-var MAYAK_MCP_TOKEN")
        );
    }

    #[test]
    fn only_the_two_known_clis_parse() {
        assert_eq!(Cli::parse("claude"), Some(Cli::Claude));
        assert_eq!(Cli::parse("codex"), Some(Cli::Codex));
        for other in ["", "Claude", "gemini", "cursor-agent", "claude --foo"] {
            assert_eq!(Cli::parse(other), None, "{other:?}");
        }
    }
}
