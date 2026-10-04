//! Product identifiers shared by the app (`src-tauri`) and the relay (`hook`).
//!
//! Included with `#[path = "../../ident.rs"] mod ident;` from both crates, so a
//! rename happens here and nowhere else.

/// Lowercase product slug: tray id, log file name, temp-file suffixes.
pub const APP_SLUG: &str = "mayak";
/// Product name.
pub const APP_NAME: &str = "Mayak";
/// Folder under %APPDATA% and %LOCALAPPDATA%. No spaces: the relay path is
/// written into the Codex hook command without quotes.
pub const APP_DIR: &str = "Mayak";
/// File name of the hook relay.
pub const HOOK_EXE: &str = "mayak-hook.exe";
/// Pipe name prefix: `\\.\pipe\mayak-<SID>`.
pub const PIPE_PREFIX: &str = "mayak-";
/// Marker that identifies our entries inside tool configs.
pub const MARKER: &str = "mayak-hook";
/// Markers of earlier builds; only for status and removal.
pub const LEGACY_MARKERS: &[&str] = &["coucou-hook"];
/// Folder of the previous product; only for the one-time settings.json copy.
pub const LEGACY_DIR: &str = "Coucou";
/// Bundle identifier: WebView2 profile, single-instance key, keyring service.
pub const BUNDLE_ID: &str = "io.github.mahach666.mayak";
