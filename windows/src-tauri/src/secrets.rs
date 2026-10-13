// API keys live in the Windows Credential Manager or, on Linux, the Secret
// Service (GNOME Keyring, KWallet) — never on disk and never in the front end — the island can only ask whether a key is present.

use keyring::Entry;

const SERVICE: &str = crate::ident::BUNDLE_ID;

/// Every key Mayak may store. Anything outside this list is refused.
pub const KNOWN_KEYS: &[&str] = &[
    "anthropic-api-key",
    "n8n-url",
    "n8n-api-key",
    "vercel-token",
    "github-token",
    "stripe-api-key",
    "resend-api-key",
    "notion-api-key",
    "calcom-api-key",
    "openrouter-api-key",
    "deepseek-api-key",
    "moonshot-api-key",
    "gitlab-token",
    // What goes with a token from Connect (oauth.rs StoredAuth): Rust only, never the page.
    "github-auth",
    "gitlab-auth",
    // The bearer token of Mayak's own MCP server (mcp_server): Rust only.
    "mcp-token",
];

/// Entries only Rust reads and writes: the page may not ask for, set or clear them.
const RUST_ONLY: &[&str] = &["github-auth", "gitlab-auth", "mcp-token"];

/// True for a key the page may name in secret_present / secret_set / secret_clear.
pub fn page_key(key: &str) -> bool {
    KNOWN_KEYS.contains(&key) && !RUST_ONLY.contains(&key)
}

fn entry(key: &str) -> Option<Entry> {
    if !KNOWN_KEYS.contains(&key) {
        return None;
    }
    Entry::new(SERVICE, key).ok()
}

pub fn get(key: &str) -> Option<String> {
    entry(key)?.get_password().ok().filter(|v| !v.is_empty())
}

pub fn set(key: &str, value: &str) -> Result<(), String> {
    let entry = entry(key).ok_or_else(|| format!("unknown key {key}"))?;
    if value.is_empty() {
        let _ = entry.delete_credential();
        return Ok(());
    }
    entry.set_password(value).map_err(|e| e.to_string())
}

pub fn clear(key: &str) -> Result<(), String> {
    let entry = entry(key).ok_or_else(|| format!("unknown key {key}"))?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn present(key: &str) -> bool {
    get(key).is_some()
}
