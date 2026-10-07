//! The chat tab as a launcher (spec docs/superpowers/specs/2026-10-07-mayak-launcher-design.md):
//! which agents are installed (§2, §3) and opening one with the question in its field (§4).
//! Everything but `open` and its flows is pure or reads only the folders given to it; the Win32
//! part is in `win.rs`, Windows only: elsewhere no agent is found and `open` answers «not
//! supported».

#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::focus::{self, workspace::Workspaces, Pick, WindowInfo};
use crate::log;
use crate::observe::keys::HostKind;
#[cfg(windows)]
use crate::platform;

#[cfg(windows)]
pub mod win;

/// A target of the prompt view (spec §3). Wire names are camelCase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentId {
    ClaudeDesktop,
    ClaudeCursor,
    ClaudeVsCode,
    CodexDesktop,
    CodexCursor,
    CodexVsCode,
    /// The island chat over the saved Anthropic key; never launched.
    ClaudeApi,
}

impl AgentId {
    /// The wire name, also the one in the log line.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentId::ClaudeDesktop => "claudeDesktop",
            AgentId::ClaudeCursor => "claudeCursor",
            AgentId::ClaudeVsCode => "claudeVsCode",
            AgentId::CodexDesktop => "codexDesktop",
            AgentId::CodexCursor => "codexCursor",
            AgentId::CodexVsCode => "codexVsCode",
            AgentId::ClaudeApi => "claudeApi",
        }
    }
}

/// The editors that host the Claude and Codex extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editor {
    Cursor,
    VsCode,
}

impl Editor {
    /// URL scheme of the editor (`cursor://…`, `vscode://…`), also its protocol key.
    pub fn scheme(self) -> &'static str {
        match self {
            Editor::Cursor => "cursor",
            Editor::VsCode => "vscode",
        }
    }

    /// `%USERPROFILE%\.cursor\extensions` / `%USERPROFILE%\.vscode\extensions` (spec §2).
    pub fn extensions_dir(self, home: &Path) -> PathBuf {
        let profile = match self {
            Editor::Cursor => ".cursor",
            Editor::VsCode => ".vscode",
        };
        home.join(profile).join("extensions")
    }

    /// The host kind `focus::pick_window` tells this editor's titles by.
    pub fn host(self) -> HostKind {
        match self {
            Editor::Cursor => HostKind::Cursor,
            Editor::VsCode => HostKind::VsCode,
        }
    }

    /// The command-line launcher found on PATH (`cursor <root>`, `code <root>`).
    pub fn cli(self) -> &'static str {
        match self {
            Editor::Cursor => "cursor",
            Editor::VsCode => "code",
        }
    }

    /// Image name of the editor's processes, whose windows are searched.
    pub fn exe(self) -> &'static str {
        match self {
            Editor::Cursor => "Cursor.exe",
            Editor::VsCode => "Code.exe",
        }
    }
}

/// Folder name prefix of the Claude Code extension (`anthropic.claude-code-2.1.291-win32-x64`).
pub const CLAUDE_EXTENSION: &str = "anthropic.claude-code-";
/// Folder name prefix of the Codex extension (`openai.chatgpt-26.930.51102-win32-x64`).
pub const CODEX_EXTENSION: &str = "openai.chatgpt-";
/// AppX package name of Codex Desktop; its family is `OpenAI.Codex_<publisher id>`.
pub const CODEX_PACKAGE: &str = "OpenAI.Codex";
/// Credential Manager key of the Anthropic API key (secrets.rs).
pub const API_KEY: &str = "anthropic-api-key";

/// True when `dir` holds a folder of the extension `prefix` (`<prefix><digit>…`, any version)
/// that the editor has not marked `true` in `.obsolete` (uninstalled, folder not removed yet).
pub fn has_extension(dir: &Path, prefix: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    let obsolete = obsolete_folders(dir);
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { return false };
        is_extension_folder(name, prefix)
            && !obsolete.contains(name)
            && entry.file_type().is_ok_and(|kind| kind.is_dir())
    })
}

fn is_extension_folder(name: &str, prefix: &str) -> bool {
    name.len() > prefix.len()
        && name.is_char_boundary(prefix.len())
        && name[..prefix.len()].eq_ignore_ascii_case(prefix)
        && name.as_bytes()[prefix.len()].is_ascii_digit()
}

/// The folder names `.obsolete` marks `true`; empty when the file is missing or not a JSON object.
fn obsolete_folders(dir: &Path) -> HashSet<String> {
    let Ok(text) = std::fs::read_to_string(dir.join(".obsolete")) else { return HashSet::new() };
    let Ok(map) = serde_json::from_str::<serde_json::Map<String, Value>>(&text) else { return HashSet::new() };
    map.into_iter().filter(|(_, v)| v.as_bool() == Some(true)).map(|(k, _)| k).collect()
}

/// The package family folder of Codex Desktop under `%LOCALAPPDATA%\Packages`
/// (`OpenAI.Codex_2p2nqsd0c76g0`), as it is named on disk.
pub fn codex_family(packages: &Path) -> Option<String> {
    let entries = std::fs::read_dir(packages).ok()?;
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .find(|name| is_family_of(name, CODEX_PACKAGE))
}

/// `<package>_<publisher id>`, the publisher id ASCII letters and digits only.
fn is_family_of(name: &str, package: &str) -> bool {
    let Some((head, publisher)) = name.split_once('_') else { return false };
    head.eq_ignore_ascii_case(package) && !publisher.is_empty() && publisher.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The installed targets in the spec's order (§3). `protocol` tells whether a URL scheme is
/// registered (`win::protocol_registered`); `api_key` whether the Anthropic key is saved.
pub fn installed_targets(home: &Path, localappdata: &Path, protocol: impl Fn(&str) -> bool, api_key: bool) -> Vec<AgentId> {
    let in_editor = |editor: Editor, prefix: &str| {
        protocol(editor.scheme()) && has_extension(&editor.extensions_dir(home), prefix)
    };
    let mut found = Vec::new();
    if protocol("claude") {
        found.push(AgentId::ClaudeDesktop);
    }
    if in_editor(Editor::Cursor, CLAUDE_EXTENSION) {
        found.push(AgentId::ClaudeCursor);
    }
    if in_editor(Editor::VsCode, CLAUDE_EXTENSION) {
        found.push(AgentId::ClaudeVsCode);
    }
    if protocol("codex") && codex_family(&localappdata.join("Packages")).is_some() {
        found.push(AgentId::CodexDesktop);
    }
    if in_editor(Editor::Cursor, CODEX_EXTENSION) {
        found.push(AgentId::CodexCursor);
    }
    if in_editor(Editor::VsCode, CODEX_EXTENSION) {
        found.push(AgentId::CodexVsCode);
    }
    if api_key {
        found.push(AgentId::ClaudeApi);
    }
    found
}

// ── Opening an agent (spec §4) ──────────────────────────────────────────────────

/// Longest question a link carries (spec §4); a longer one goes through the clipboard whole.
pub const MAX_PROMPT_CHARS: usize = 8000;
/// Longest question accepted at all: the webview never sends more, and nothing longer is counted,
/// encoded or put on the clipboard.
pub const MAX_INPUT_CHARS: usize = 100_000;
/// Most files one question takes (spec §4).
pub const MAX_FILES: usize = 10;
/// Longest link handed to Windows. The protocol handler gets the link on its command line, which
/// holds 32 767 characters; 8000 Cyrillic characters alone encode to 48 000.
pub const MAX_URI_LEN: usize = 30_000;
/// How long the project's editor window is waited for after `cursor <root>` (spec §4).
pub const EDITOR_WAIT: Duration = Duration::from_secs(15);
/// How long the Codex Desktop window is waited for in the foreground (spec §4).
pub const CODEX_WAIT: Duration = Duration::from_secs(5);
/// Step of both waits.
pub const WAIT_STEP: Duration = Duration::from_millis(250);
/// After the Codex window came forward, before Ctrl+V: its field takes the focus meanwhile.
#[cfg(windows)]
const PASTE_SETTLE: Duration = Duration::from_millis(300);

/// What the island gets back (spec §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LaunchOutcome {
    /// The agent is open with the question in its field.
    Opened,
    /// The agent is open; the question is on the clipboard for the user to paste.
    OpenedPasteYourself,
    /// Nothing usable opened. `reason` is Mayak's own text, never a path or the question.
    Failed { reason: String },
}

impl LaunchOutcome {
    /// The name in the log line.
    pub fn name(&self) -> &'static str {
        match self {
            LaunchOutcome::Opened => "Opened",
            LaunchOutcome::OpenedPasteYourself => "OpenedPasteYourself",
            LaunchOutcome::Failed { .. } => "Failed",
        }
    }
}

/// The only thing a launch writes to `mayak.log` (spec §4): no question, no path, no reason.
pub fn log_line(agent: AgentId, outcome: &LaunchOutcome) -> String {
    format!("launcher: agent={} outcome={}", agent.as_str(), outcome.name())
}

/// `encodeURIComponent`: every byte of the UTF-8 except ASCII letters, digits and `-_.!~*'()`
/// becomes `%XX`.
pub fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn with_query(base: &str, params: &[(&str, &str)]) -> String {
    if params.is_empty() {
        return base.to_string();
    }
    let query: Vec<String> = params.iter().map(|(k, v)| format!("{k}={}", encode_component(v))).collect();
    format!("{base}?{}", query.join("&"))
}

/// `claude://code/new?q=…&folder=…&file=…` (spec §2): a new Code session in the folder with the
/// question in its field; each part only when given.
pub fn claude_desktop_uri(prompt: Option<&str>, root: Option<&str>, files: &[String]) -> String {
    let mut params: Vec<(&str, &str)> = Vec::new();
    if let Some(q) = prompt {
        params.push(("q", q));
    }
    if let Some(folder) = root {
        params.push(("folder", folder));
    }
    for file in files {
        params.push(("file", file.as_str()));
    }
    with_query("claude://code/new", &params)
}

/// `cursor://anthropic.claude-code/open?prompt=…` (or `vscode://`): Claude in the last active
/// editor window with the question in its field; without a prompt, the panel alone.
pub fn editor_claude_uri(editor: Editor, prompt: Option<&str>) -> String {
    let base = format!("{}://anthropic.claude-code/open", editor.scheme());
    match prompt {
        Some(text) => with_query(&base, &[("prompt", text)]),
        None => base,
    }
}

/// `cursor://openai.chatgpt/` (or `vscode://`): the Codex panel of the editor.
pub fn editor_codex_uri(editor: Editor) -> String {
    format!("{}://openai.chatgpt/", editor.scheme())
}

/// The question for Claude in the editor: `@<path>` per file before it (spec §2), a path with
/// whitespace in quotes.
pub fn mention_prompt(prompt: &str, files: &[String]) -> String {
    let mut out = String::new();
    for file in files {
        if file.chars().any(char::is_whitespace) {
            let _ = write!(out, "@\"{file}\" ");
        } else {
            let _ = write!(out, "@{file} ");
        }
    }
    out.push_str(prompt);
    out
}

/// The clipboard text for Codex: the question, a blank line, one path per line (spec §2).
pub fn codex_text(prompt: &str, files: &[String]) -> String {
    if files.is_empty() {
        return prompt.to_string();
    }
    format!("{prompt}\n\n{}", files.join("\n"))
}

/// At most MAX_PROMPT_CHARS characters (not bytes).
pub fn prompt_fits(prompt: &str) -> bool {
    prompt.chars().count() <= MAX_PROMPT_CHARS
}

/// A link to open and, when the question did not fit in it, the whole question for the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UriPlan {
    pub uri: String,
    pub clipboard: Option<String>,
}

/// The link `with` the question when the question fits, else the link `without` it and the
/// question for the clipboard. `with` is built only for a question within MAX_PROMPT_CHARS.
fn plan(text: &str, with: impl FnOnce() -> String, without: impl FnOnce() -> String) -> UriPlan {
    if prompt_fits(text) {
        let uri = with();
        if uri.len() <= MAX_URI_LEN {
            return UriPlan { uri, clipboard: None };
        }
    }
    UriPlan { uri: without(), clipboard: Some(text.to_string()) }
}

/// Claude Desktop: the question in `q=` when it fits, else the folder and files only.
pub fn claude_desktop_plan(prompt: &str, root: Option<&str>, files: &[String]) -> UriPlan {
    plan(prompt, || claude_desktop_uri(Some(prompt), root, files), || claude_desktop_uri(None, root, files))
}

/// Claude in the editor: the question with its mentions in `prompt=` when it fits, else the panel.
pub fn editor_claude_plan(editor: Editor, prompt: &str, files: &[String]) -> UriPlan {
    let text = mention_prompt(prompt, files);
    plan(&text, || editor_claude_uri(editor, Some(&text)), || editor_claude_uri(editor, None))
}

/// A checked launch request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub prompt: String,
    pub root: Option<String>,
    pub files: Vec<String>,
}

/// Checks what the webview sent: a question that is not blank and has at most MAX_INPUT_CHARS
/// characters; `root` absent or "" (no project) or an existing absolute folder; at most MAX_FILES
/// files, each an existing file directly in `inbox`, both compared by their real (canonical)
/// paths so that no junction or symlink leads out. Nothing else reaches a link, a command line or
/// the clipboard.
pub fn prepare(prompt: &str, root: Option<String>, files: &[String], inbox: &Path) -> Result<Request, String> {
    if prompt.trim().is_empty() {
        return Err("The question is empty.".to_string());
    }
    if prompt.chars().nth(MAX_INPUT_CHARS).is_some() {
        return Err("The question is too long: at most 100 000 characters.".to_string());
    }
    let root = match root.filter(|r| !r.is_empty()) {
        None => None,
        Some(r) => {
            let path = Path::new(&r);
            if !(path.is_absolute() && path.is_dir()) {
                return Err("The project folder is not there any more.".to_string());
            }
            Some(r)
        }
    };
    if files.len() > MAX_FILES {
        return Err(format!("At most {MAX_FILES} files can go with a question."));
    }
    if !files.is_empty() {
        let not_in_inbox = || "The file is not in Mayak's inbox.".to_string();
        let inbox = std::fs::canonicalize(inbox).map_err(|_| not_in_inbox())?;
        for file in files {
            let path = Path::new(file);
            if !path.is_absolute() || !path.is_file() {
                return Err(not_in_inbox());
            }
            let real = std::fs::canonicalize(path).map_err(|_| not_in_inbox())?;
            if real.parent() != Some(inbox.as_path()) {
                return Err(not_in_inbox());
            }
        }
    }
    Ok(Request { prompt: prompt.to_string(), root, files: files.to_vec() })
}

/// Paste only when the foreground window belongs to the app (`app_pids`) and no key that would
/// change Ctrl+V, or reach the agent, is held (spec §4, §6).
pub fn should_paste(window_pid: Option<u32>, app_pids: &[u32], keys_down: bool) -> bool {
    !keys_down && window_pid.is_some_and(|pid| pid != 0 && app_pids.contains(&pid))
}

/// True when `image` lies in the install folder of the package `family` (`<Name>_<Publisher>`):
/// `…\WindowsApps\<Name>_<Version>_<Arch>_<ResourceId>_<Publisher>\…`, any case.
pub fn is_package_image(image: &str, family: &str) -> bool {
    let Some((name, publisher)) = family.split_once('_') else { return false };
    if name.is_empty() || publisher.is_empty() {
        return false;
    }
    let parts: Vec<&str> = image.split(['\\', '/']).collect();
    parts.windows(2).any(|pair| {
        let fields: Vec<&str> = pair[1].split('_').collect();
        pair[0].eq_ignore_ascii_case("WindowsApps")
            && fields.len() == 5
            && fields[0].eq_ignore_ascii_case(name)
            && fields[4].eq_ignore_ascii_case(publisher)
    })
}

/// One look at an editor's windows while its project window is brought forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorStep {
    /// The project's window is in the foreground: the link can go.
    Ready,
    /// The project's window exists: raise it.
    Raise(isize),
    /// No single window of the project: start or wait for the launcher.
    Wait,
}

/// `windows` are the editor's windows (topmost first), `foreground` the window in front. The
/// project's window is chosen as «Open» chooses it (`focus::pick_window`, workspaces included).
pub fn editor_step(
    host: HostKind,
    root: &str,
    foreground: Option<isize>,
    windows: &[WindowInfo],
    mut workspaces: impl FnMut(&[String]) -> Workspaces,
) -> EditorStep {
    if let Some(front) = foreground.and_then(|hwnd| windows.iter().find(|w| w.hwnd == hwnd)) {
        if focus::pick_window(host, root, std::slice::from_ref(front), &mut workspaces) == Pick::One(front.hwnd) {
            return EditorStep::Ready;
        }
    }
    match focus::pick_window(host, root, windows, &mut workspaces) {
        Pick::One(hwnd) => EditorStep::Raise(hwnd),
        Pick::Many | Pick::None => EditorStep::Wait,
    }
}

/// Probes now and then every `step` until `limit` has passed; the first Some wins. Bounded:
/// at most `limit / step + 1` probes. The only waits of the launcher (spec §4).
pub fn wait_for<T>(
    limit: Duration,
    step: Duration,
    mut sleep: impl FnMut(Duration),
    mut probe: impl FnMut() -> Option<T>,
) -> Option<T> {
    let mut waited = Duration::ZERO;
    loop {
        if let Some(found) = probe() {
            return Some(found);
        }
        if waited >= limit {
            return None;
        }
        sleep(step);
        waited += step;
    }
}

/// Opens `agent` with the question in its field (spec §4) and writes the one log line. Blocking
/// (up to EDITOR_WAIT, or CODEX_WAIT plus a settle): call it off the UI thread.
pub fn open(
    agent: AgentId,
    prompt: &str,
    root: Option<String>,
    files: &[String],
    localappdata: &Path,
    inbox: &Path,
) -> LaunchOutcome {
    #[cfg(windows)]
    let _com = win::Com::init();
    let outcome = match prepare(prompt, root, files, inbox) {
        Ok(req) => run(agent, &req, localappdata).unwrap_or_else(|reason| LaunchOutcome::Failed { reason }),
        Err(reason) => LaunchOutcome::Failed { reason },
    };
    log::line(log_line(agent, &outcome));
    outcome
}

/// macOS and Linux: no agent is opened from the island yet.
#[cfg(not(windows))]
fn run(_agent: AgentId, _req: &Request, _localappdata: &Path) -> Result<LaunchOutcome, String> {
    Err("Opening an agent is not supported on this platform.".to_string())
}

#[cfg(windows)]
fn run(agent: AgentId, req: &Request, localappdata: &Path) -> Result<LaunchOutcome, String> {
    match agent {
        AgentId::ClaudeDesktop => open_plan(&claude_desktop_plan(&req.prompt, req.root.as_deref(), &req.files)),
        AgentId::ClaudeCursor => claude_in_editor(Editor::Cursor, req),
        AgentId::ClaudeVsCode => claude_in_editor(Editor::VsCode, req),
        AgentId::CodexDesktop => codex_desktop(req, localappdata),
        AgentId::CodexCursor => codex_in_editor(Editor::Cursor, req),
        AgentId::CodexVsCode => codex_in_editor(Editor::VsCode, req),
        AgentId::ClaudeApi => Err("Claude API answers in the island.".to_string()),
    }
}

#[cfg(windows)]
fn open_plan(plan: &UriPlan) -> Result<LaunchOutcome, String> {
    if let Some(text) = &plan.clipboard {
        win::set_clipboard_text(text)?;
    }
    win::open_uri(&plan.uri)?;
    Ok(if plan.clipboard.is_some() { LaunchOutcome::OpenedPasteYourself } else { LaunchOutcome::Opened })
}

#[cfg(windows)]
fn claude_in_editor(editor: Editor, req: &Request) -> Result<LaunchOutcome, String> {
    let plan = editor_claude_plan(editor, &req.prompt, &req.files);
    if let Some(root) = &req.root {
        place_editor(editor, root)?;
    }
    open_plan(&plan)
}

/// Codex in the editor (spec §4): the project's window first, then the clipboard, then the panel.
/// The clipboard is set only once the window is in place: a failed wait leaves it untouched.
fn codex_in_editor_steps(
    place: impl FnOnce() -> Result<(), String>,
    set_clipboard: impl FnOnce() -> Result<(), String>,
    open_panel: impl FnOnce() -> Result<(), String>,
) -> Result<LaunchOutcome, String> {
    place()?;
    set_clipboard()?;
    open_panel()?;
    // No automatic paste: the focus may be in the code editor, and a paste there would change a file.
    Ok(LaunchOutcome::OpenedPasteYourself)
}

#[cfg(windows)]
fn codex_in_editor(editor: Editor, req: &Request) -> Result<LaunchOutcome, String> {
    codex_in_editor_steps(
        || req.root.as_deref().map_or(Ok(()), |root| place_editor(editor, root)),
        || win::set_clipboard_text(&codex_text(&req.prompt, &req.files)),
        || win::open_uri(&editor_codex_uri(editor)),
    )
}

/// The last check before Ctrl+V: `now` (the foreground window and its pid, when its image is in
/// the Codex package) is still the window `first` that came forward, and no key is held.
pub fn paste_now(first: isize, now: Option<(isize, u32)>, keys_down: bool) -> bool {
    now.is_some_and(|(hwnd, pid)| hwnd == first && should_paste(Some(pid), &[pid], keys_down))
}

#[cfg(windows)]
fn codex_desktop(req: &Request, localappdata: &Path) -> Result<LaunchOutcome, String> {
    let family =
        codex_family(&localappdata.join("Packages")).ok_or_else(|| "Codex Desktop is not installed.".to_string())?;
    win::set_clipboard_text(&codex_text(&req.prompt, &req.files))?;
    win::launch_appx(&family)?;
    let Some((first, _)) = wait_for(CODEX_WAIT, WAIT_STEP, std::thread::sleep, || codex_window(&family)) else {
        return Ok(LaunchOutcome::OpenedPasteYourself);
    };
    std::thread::sleep(PASTE_SETTLE);
    // Checked again right before the keys: the same window, its image, and only then the keys.
    let now = codex_window(&family);
    let keys_down = win::keys_held();
    if !paste_now(first, now, keys_down) || win::send_ctrl_v().is_err() {
        return Ok(LaunchOutcome::OpenedPasteYourself);
    }
    Ok(LaunchOutcome::Opened)
}

/// The foreground window and its pid, when the image of its process lies in the Codex package.
#[cfg(windows)]
fn codex_window(family: &str) -> Option<(isize, u32)> {
    let (hwnd, pid) = win::foreground()?;
    win::image_path(pid).is_some_and(|image| is_package_image(&image, family)).then_some((hwnd, pid))
}

/// Brings the project's editor window to the foreground (spec §4 steps 1-3): raised when found,
/// else `cursor <root>` / `code <root>` and up to EDITOR_WAIT in WAIT_STEP steps. Err when it never
/// came forward: the link would then land in whichever window was active last.
#[cfg(windows)]
fn place_editor(editor: Editor, root: &str) -> Result<(), String> {
    let host = editor.host();
    let look = || {
        let windows = focus::win::app_windows(editor.exe());
        let front = win::foreground().map(|(hwnd, _)| hwnd);
        editor_step(host, root, front, &windows, |names| focus::workspace::load(host, names))
    };
    let mut flashed: Option<isize> = None;
    if settle(look(), &mut flashed) {
        return Ok(());
    }
    let launcher =
        platform::find_on_path(editor.cli()).ok_or_else(|| format!("`{}` is not on PATH.", editor.cli()))?;
    let mut cmd = Command::new(launcher);
    cmd.arg(root);
    platform::no_console(&mut cmd).spawn().map_err(|_| format!("`{}` did not start.", editor.cli()))?;
    wait_for(EDITOR_WAIT, WAIT_STEP, std::thread::sleep, || settle(look(), &mut flashed).then_some(()))
        .ok_or_else(|| "The project window did not open in time.".to_string())
}

/// Acts on one look: true when the project's window is in front. A window that was raised but
/// only flashed (or was stale) is not raised again, or it would flash at every step.
#[cfg(windows)]
fn settle(step: EditorStep, flashed: &mut Option<isize>) -> bool {
    match step {
        EditorStep::Ready => true,
        EditorStep::Raise(hwnd) if *flashed != Some(hwnd) => {
            if focus::win::raise(hwnd) == focus::win::Raise::Focused {
                return true;
            }
            *flashed = Some(hwnd);
            false
        }
        EditorStep::Raise(_) | EditorStep::Wait => false,
    }
}


#[cfg(test)]
mod tests {
    use super::AgentId::*;
    use super::*;
    use crate::testutil::TempDir;
    use serde_json::json;
    use crate::observe::keys::HostKind;
    use std::time::Duration;

    const ALL: [AgentId; 7] = [ClaudeDesktop, ClaudeCursor, ClaudeVsCode, CodexDesktop, CodexCursor, CodexVsCode, ClaudeApi];

    fn mkdirs(root: &Path, dirs: &[&str]) {
        for dir in dirs {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
    }

    /// A profile with both extensions in both editors and the Codex package: (home, localappdata).
    fn machine(tmp: &TempDir) -> (PathBuf, PathBuf) {
        let home = tmp.path.join("home");
        let local = tmp.path.join("local");
        mkdirs(
            &home,
            &[
                r".cursor\extensions\anthropic.claude-code-2.1.291-win32-x64",
                r".cursor\extensions\openai.chatgpt-26.721.30844-win32-x64",
                r".vscode\extensions\anthropic.claude-code-2.1.289-win32-x64",
                r".vscode\extensions\openai.chatgpt-26.930.51102-win32-x64",
            ],
        );
        mkdirs(&local, &[r"Packages\OpenAI.Codex_2p2nqsd0c76g0"]);
        (home, local)
    }

    #[test]
    fn agent_ids_have_stable_wire_names() {
        let names: Vec<_> = ALL.iter().map(|a| serde_json::to_value(a).unwrap()).collect();
        assert_eq!(
            names,
            [
                json!("claudeDesktop"),
                json!("claudeCursor"),
                json!("claudeVsCode"),
                json!("codexDesktop"),
                json!("codexCursor"),
                json!("codexVsCode"),
                json!("claudeApi"),
            ]
        );
        assert_eq!(serde_json::from_value::<AgentId>(json!("claudeVsCode")).unwrap(), ClaudeVsCode);
        assert!(serde_json::from_value::<AgentId>(json!("cursor")).is_err());
    }

    #[test]
    fn an_extension_folder_of_any_version_is_found() {
        let tmp = TempDir::new("launcher-ext");
        mkdirs(&tmp.path, &["anthropic.claude-code-2.1.288-win32-x64", "anthropic.claude-code-2.1.291-win32-x64"]);
        assert!(has_extension(&tmp.path, CLAUDE_EXTENSION));
        assert!(!has_extension(&tmp.path, CODEX_EXTENSION));
    }

    #[test]
    fn no_folder_another_extension_or_a_file_is_no_target() {
        let tmp = TempDir::new("launcher-ext-none");
        assert!(!has_extension(&tmp.path.join("missing"), CLAUDE_EXTENSION), "no extensions folder");
        mkdirs(&tmp.path, &["openai.codex-audio-26.930.41038", "anthropic.claude-code-insiders", "anthropic.claude-code-"]);
        std::fs::write(tmp.path.join("openai.chatgpt-26.930.51102-win32-x64"), b"").unwrap();
        assert!(!has_extension(&tmp.path, CLAUDE_EXTENSION), "no version after the prefix");
        assert!(!has_extension(&tmp.path, CODEX_EXTENSION), "codex-audio is another extension, a file is no folder");
    }

    #[test]
    fn a_folder_listed_as_obsolete_does_not_count() {
        // Both editors keep the folders of uninstalled versions and list them in `.obsolete`.
        let tmp = TempDir::new("launcher-ext-obsolete");
        mkdirs(&tmp.path, &["anthropic.claude-code-2.1.288-win32-x64", "openai.chatgpt-26.623.31443-win32-x64"]);
        std::fs::write(
            tmp.path.join(".obsolete"),
            r#"{"anthropic.claude-code-2.1.288-win32-x64":true,"openai.chatgpt-26.623.31443-win32-x64":false}"#,
        )
        .unwrap();
        assert!(!has_extension(&tmp.path, CLAUDE_EXTENSION), "uninstalled, its folder not yet removed");
        assert!(has_extension(&tmp.path, CODEX_EXTENSION), "false is not obsolete");
        mkdirs(&tmp.path, &["anthropic.claude-code-2.1.291-win32-x64"]);
        assert!(has_extension(&tmp.path, CLAUDE_EXTENSION), "a newer version is installed");
        std::fs::remove_dir_all(tmp.path.join("anthropic.claude-code-2.1.291-win32-x64")).unwrap();
        std::fs::write(tmp.path.join(".obsolete"), b"{ torn").unwrap();
        assert!(has_extension(&tmp.path, CLAUDE_EXTENSION), "an unreadable list hides nothing");
    }

    #[test]
    fn codex_family_is_the_package_folder_name() {
        let tmp = TempDir::new("launcher-packages");
        assert_eq!(codex_family(&tmp.path.join("missing")), None);
        mkdirs(&tmp.path, &["Claude_pzs8sxrjxfjjc", "OpenAI.CodexBeta_2p2nqsd0c76g0", "OpenAI.Codex_", "OpenAI.Codex_a_b"]);
        std::fs::write(tmp.path.join("OpenAI.Codex_filenotfolder"), b"").unwrap();
        assert_eq!(codex_family(&tmp.path), None);
        mkdirs(&tmp.path, &["OpenAI.Codex_2p2nqsd0c76g0"]);
        assert_eq!(codex_family(&tmp.path).as_deref(), Some("OpenAI.Codex_2p2nqsd0c76g0"));
    }

    #[test]
    fn installed_targets_follow_the_spec_order() {
        let tmp = TempDir::new("launcher-targets");
        let (home, local) = machine(&tmp);
        assert_eq!(installed_targets(&home, &local, |_| true, true), ALL);
    }

    #[test]
    fn a_target_needs_its_protocol_and_the_api_needs_the_key() {
        let tmp = TempDir::new("launcher-targets-partial");
        let (home, local) = machine(&tmp);
        assert_eq!(
            installed_targets(&home, &local, |s| s != "claude", false),
            [ClaudeCursor, ClaudeVsCode, CodexDesktop, CodexCursor, CodexVsCode],
            "no claude protocol, no key"
        );
        assert_eq!(
            installed_targets(&home, &local, |s| s != "codex", true),
            [ClaudeDesktop, ClaudeCursor, ClaudeVsCode, CodexCursor, CodexVsCode, ClaudeApi],
            "a Codex package folder without its protocol"
        );
        assert_eq!(
            installed_targets(&home, &local, |s| s != "vscode", false),
            [ClaudeDesktop, ClaudeCursor, CodexDesktop, CodexCursor],
            "a profile folder left by an uninstalled VS Code"
        );
        let nobody = tmp.path.join("nobody");
        assert_eq!(installed_targets(&nobody, &nobody, |_| true, false), [ClaudeDesktop], "only the protocol");
    }

    // ── Opening (spec §4, §6) ──

    #[test]
    fn encode_component_matches_encode_uri_component() {
        // Expected values: Node, encodeURIComponent.
        assert_eq!(encode_component("AZaz09-_.!~*'()"), "AZaz09-_.!~*'()");
        assert_eq!(encode_component(r"a b&c=d?e#f+g/h\i:j%"), "a%20b%26c%3Dd%3Fe%23f%2Bg%2Fh%5Ci%3Aj%25");
        assert_eq!(encode_component("Привет"), "%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82");
        assert_eq!(encode_component("line\nnext\r\t"), "line%0Anext%0D%09");
        assert_eq!(encode_component("😀"), "%F0%9F%98%80");
        assert_eq!(encode_component(""), "");
    }

    #[test]
    fn the_claude_desktop_uri_carries_the_question_the_folder_and_each_file() {
        let files = [r"C:\in\a.txt".to_string(), r"C:\in\b c.txt".to_string()];
        assert_eq!(
            claude_desktop_uri(Some("fix a & b #1"), Some(r"C:\src\app"), &files),
            "claude://code/new?q=fix%20a%20%26%20b%20%231&folder=C%3A%5Csrc%5Capp&file=C%3A%5Cin%5Ca.txt&file=C%3A%5Cin%5Cb%20c.txt"
        );
        assert_eq!(claude_desktop_uri(Some("hi"), None, &[]), "claude://code/new?q=hi", "no project: no folder");
        assert_eq!(claude_desktop_uri(Some("Привет"), None, &[]), "claude://code/new?q=%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82");
        assert_eq!(claude_desktop_uri(None, Some(r"C:\a"), &[]), "claude://code/new?folder=C%3A%5Ca");
        assert_eq!(claude_desktop_uri(None, None, &[]), "claude://code/new");
    }

    #[test]
    fn the_editor_uris_name_the_extension_routes() {
        assert_eq!(
            editor_claude_uri(Editor::Cursor, Some("Привет & #")),
            "cursor://anthropic.claude-code/open?prompt=%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82%20%26%20%23"
        );
        assert_eq!(editor_claude_uri(Editor::VsCode, Some("x")), "vscode://anthropic.claude-code/open?prompt=x");
        assert_eq!(editor_claude_uri(Editor::Cursor, None), "cursor://anthropic.claude-code/open");
        assert_eq!(editor_codex_uri(Editor::Cursor), "cursor://openai.chatgpt/");
        assert_eq!(editor_codex_uri(Editor::VsCode), "vscode://openai.chatgpt/");
        let text = mention_prompt("why?", &[r"C:\in\a.txt".to_string(), r"C:\in\my doc.pdf".to_string()]);
        assert_eq!(
            editor_claude_uri(Editor::VsCode, Some(&text)),
            "vscode://anthropic.claude-code/open?prompt=%40C%3A%5Cin%5Ca.txt%20%40%22C%3A%5Cin%5Cmy%20doc.pdf%22%20why%3F"
        );
    }

    #[test]
    fn files_become_mentions_for_claude_in_the_editor() {
        let files = [r"C:\in\a.txt".to_string(), r"C:\in\my doc.pdf".to_string()];
        assert_eq!(mention_prompt("why?", &files), r#"@C:\in\a.txt @"C:\in\my doc.pdf" why?"#);
        assert_eq!(mention_prompt("why?", &[]), "why?");
    }

    #[test]
    fn codex_text_puts_each_path_on_its_own_line() {
        let files = [r"C:\in\a.txt".to_string(), r"C:\in\b c.txt".to_string()];
        assert_eq!(codex_text("why?\nreally", &files), "why?\nreally\n\nC:\\in\\a.txt\nC:\\in\\b c.txt");
        assert_eq!(codex_text("why?", &[]), "why?");
    }

    #[test]
    fn the_limits_count_characters_not_bytes() {
        assert_eq!((MAX_PROMPT_CHARS, MAX_FILES, MAX_URI_LEN), (8000, 10, 30_000));
        assert!(prompt_fits(&"x".repeat(MAX_PROMPT_CHARS)));
        assert!(!prompt_fits(&"x".repeat(MAX_PROMPT_CHARS + 1)));
        assert!(prompt_fits(&"я".repeat(MAX_PROMPT_CHARS)), "16 000 bytes, 8000 characters");
    }

    #[test]
    fn a_question_too_long_for_the_link_goes_to_the_clipboard_whole() {
        // 8000 Cyrillic characters fit the character limit but encode to 48 000 link characters.
        let long = "я".repeat(MAX_PROMPT_CHARS);
        assert_eq!(
            claude_desktop_plan(&long, Some(r"C:\src\app"), &[]),
            UriPlan { uri: "claude://code/new?folder=C%3A%5Csrc%5Capp".into(), clipboard: Some(long.clone()) }
        );
        let over = "x".repeat(MAX_PROMPT_CHARS + 1);
        assert_eq!(claude_desktop_plan(&over, None, &[]), UriPlan { uri: "claude://code/new".into(), clipboard: Some(over) });
        assert_eq!(claude_desktop_plan(&"x".repeat(MAX_PROMPT_CHARS), None, &[]).clipboard, None);
        assert_eq!(
            editor_claude_plan(Editor::Cursor, &long, &[]),
            UriPlan { uri: "cursor://anthropic.claude-code/open".into(), clipboard: Some(long) }
        );
        let half = "я".repeat(4_000); // 24 000 encoded characters: fits
        let plan = editor_claude_plan(Editor::VsCode, &half, &[]);
        assert_eq!((plan.clipboard, plan.uri.len()), (None, "vscode://anthropic.claude-code/open?prompt=".len() + 24_000));
    }

    /// A temporary inbox (`<tmp>\inbox`) with one file in it.
    fn temp_inbox(tmp: &TempDir) -> (PathBuf, String) {
        let inbox = tmp.path.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let file = inbox.join("note (2).txt");
        std::fs::write(&file, b"x").unwrap();
        (inbox, file.to_string_lossy().into_owned())
    }

    #[test]
    fn prepare_accepts_a_question_a_folder_and_inbox_files() {
        let tmp = TempDir::new("launcher-prepare");
        let (inbox, file) = temp_inbox(&tmp);
        let root = tmp.path.to_string_lossy().into_owned();
        assert_eq!(
            prepare("why?", Some(root.clone()), &[file.clone()], &inbox),
            Ok(Request { prompt: "why?".into(), root: Some(root), files: vec![file] })
        );
        assert_eq!(prepare("why?", None, &[], &inbox).map(|r| r.root), Ok(None));
        assert_eq!(prepare("why?", Some(String::new()), &[], &inbox).map(|r| r.root), Ok(None), "\"\" is No project");
    }

    #[test]
    fn prepare_refuses_files_outside_the_inbox() {
        let tmp = TempDir::new("launcher-prepare-files");
        let (inbox, _) = temp_inbox(&tmp);
        let outside = tmp.path.join("secret.txt");
        std::fs::write(&outside, b"x").unwrap();
        std::fs::create_dir_all(inbox.join("folder")).unwrap();
        let refused = Err("The file is not in Mayak's inbox.".to_string());
        for file in [
            outside.to_string_lossy().into_owned(),
            format!(r"{}\..\secret.txt", inbox.display()),
            inbox.join("folder").to_string_lossy().into_owned(),
            inbox.join("gone.txt").to_string_lossy().into_owned(),
            "note (2).txt".to_string(),
        ] {
            assert_eq!(prepare("q", None, &[file.clone()], &inbox), refused, "{file}");
        }
        let eleven: Vec<String> = (0..11)
            .map(|i| {
                let path = inbox.join(format!("{i}.txt"));
                std::fs::write(&path, b"x").unwrap();
                path.to_string_lossy().into_owned()
            })
            .collect();
        assert_eq!(prepare("q", None, &eleven, &inbox), Err("At most 10 files can go with a question.".to_string()));
        assert!(prepare("q", None, &eleven[..10], &inbox).is_ok());
    }

    #[test]
    fn prepare_refuses_a_root_that_is_not_an_existing_absolute_folder() {
        let tmp = TempDir::new("launcher-prepare-root");
        let (inbox, file) = temp_inbox(&tmp);
        let gone = Err("The project folder is not there any more.".to_string());
        // bef6558: a launcher must never see a root like `--x`.
        for root in ["--x".to_string(), r"work\app".to_string(), tmp.path.join("missing").to_string_lossy().into_owned(), file] {
            assert_eq!(prepare("q", Some(root.clone()), &[], &inbox), gone, "{root}");
        }
    }

    #[test]
    fn prepare_refuses_an_empty_question() {
        let tmp = TempDir::new("launcher-prepare-empty");
        let (inbox, _) = temp_inbox(&tmp);
        for prompt in ["", "  \n\t"] {
            assert_eq!(prepare(prompt, None, &[], &inbox), Err("The question is empty.".to_string()));
        }
    }

    #[test]
    fn prepare_refuses_a_question_over_100_000_characters() {
        let tmp = TempDir::new("launcher-prepare-long");
        let (inbox, _) = temp_inbox(&tmp);
        assert_eq!(MAX_INPUT_CHARS, 100_000);
        assert!(prepare(&"я".repeat(MAX_INPUT_CHARS), None, &[], &inbox).is_ok(), "characters, not bytes");
        assert_eq!(
            prepare(&"x".repeat(MAX_INPUT_CHARS + 1), None, &[], &inbox),
            Err("The question is too long: at most 100 000 characters.".to_string())
        );
    }

    #[test]
    fn a_question_over_the_link_limit_never_builds_the_encoded_link() {
        let over = "x".repeat(MAX_PROMPT_CHARS + 1);
        assert_eq!(
            plan(&over, || panic!("the link with the question is not built"), || "base".into()),
            UriPlan { uri: "base".into(), clipboard: Some(over) }
        );
        assert_eq!(
            plan("why?", || "with".into(), || panic!("the bare link is not needed")),
            UriPlan { uri: "with".into(), clipboard: None }
        );
    }

    /// A directory junction `link` → `target` (no privilege needed, unlike a symlink).
    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) {
        let out = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(out.status.success(), "mklink /J failed");
    }

    #[cfg(windows)]
    #[test]
    fn the_inbox_is_compared_by_its_real_path() {
        let tmp = TempDir::new("launcher-prepare-junction");
        let (inbox, file) = temp_inbox(&tmp);
        let outside = tmp.path.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"x").unwrap();
        let alias = tmp.path.join("inbox-alias");
        junction(&alias, &inbox);
        assert!(prepare("q", None, &[file.clone()], &alias).is_ok(), "the inbox named through a junction");
        let through = alias.join("note (2).txt").to_string_lossy().into_owned();
        assert!(prepare("q", None, &[through], &inbox).is_ok(), "the file named through a junction");
        let door = inbox.join("door");
        junction(&door, &outside);
        let refused = Err("The file is not in Mayak's inbox.".to_string());
        for escape in [door.join("secret.txt"), door] {
            let escape = escape.to_string_lossy().into_owned();
            assert_eq!(prepare("q", None, &[escape.clone()], &inbox), refused, "{escape}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_symlink_in_the_inbox_does_not_lead_out_of_it() {
        let tmp = TempDir::new("launcher-prepare-symlink");
        let (inbox, _) = temp_inbox(&tmp);
        let secret = tmp.path.join("secret.txt");
        std::fs::write(&secret, b"x").unwrap();
        let link = inbox.join("link.txt");
        match std::os::windows::fs::symlink_file(&secret, &link) {
            // ERROR_PRIVILEGE_NOT_HELD: file symlinks need Developer Mode or elevation.
            Err(err) if err.raw_os_error() == Some(1314) => {
                eprintln!("skipped: this account cannot create symlinks");
                return;
            }
            other => other.unwrap(),
        }
        let link = link.to_string_lossy().into_owned();
        assert_eq!(prepare("q", None, &[link], &inbox), Err("The file is not in Mayak's inbox.".to_string()));
    }

    #[test]
    fn codex_in_the_editor_sets_the_clipboard_only_once_the_window_is_in_place() {
        let calls = std::cell::RefCell::new(Vec::new());
        let step = |name: &'static str, ok: bool| {
            let calls = &calls;
            move || {
                calls.borrow_mut().push(name);
                if ok { Ok(()) } else { Err(format!("{name} failed")) }
            }
        };
        assert_eq!(
            codex_in_editor_steps(step("place", true), step("clipboard", true), step("open", true)),
            Ok(LaunchOutcome::OpenedPasteYourself)
        );
        assert_eq!(*calls.borrow(), ["place", "clipboard", "open"]);
        calls.borrow_mut().clear();
        assert_eq!(
            codex_in_editor_steps(step("place", false), step("clipboard", true), step("open", true)),
            Err("place failed".to_string())
        );
        assert_eq!(*calls.borrow(), ["place"], "a failed wait leaves the clipboard untouched");
        calls.borrow_mut().clear();
        assert_eq!(
            codex_in_editor_steps(step("place", true), step("clipboard", false), step("open", true)),
            Err("clipboard failed".to_string())
        );
        assert_eq!(*calls.borrow(), ["place", "clipboard"]);
    }

    #[test]
    fn the_paste_needs_the_first_codex_window_still_in_front() {
        assert!(paste_now(10, Some((10, 4242)), false));
        assert!(!paste_now(10, Some((11, 4242)), false), "another Codex window came forward");
        assert!(!paste_now(10, None, false), "Codex is no longer in front");
        assert!(!paste_now(10, Some((10, 4242)), true), "Enter, Shift, Alt or Win held");
        assert!(!paste_now(10, Some((10, 0)), false), "no process");
    }

    #[test]
    fn paste_only_into_the_app_window_with_no_key_held() {
        assert!(should_paste(Some(4242), &[4242], false));
        assert!(!should_paste(Some(4242), &[4242], true), "Enter, Shift, Alt or Win held");
        assert!(!should_paste(Some(77), &[4242], false), "another app is in front");
        assert!(!should_paste(None, &[4242], false), "no foreground window");
        assert!(!should_paste(Some(0), &[0], false), "no process");
        assert!(!should_paste(Some(4242), &[], false), "not a Codex window");
    }

    #[test]
    fn only_an_image_inside_the_codex_package_folder_counts() {
        let family = "OpenAI.Codex_2p2nqsd0c76g0";
        let app = r"C:\Program Files\WindowsApps\OpenAI.Codex_26.930.7945.0_x64__2p2nqsd0c76g0\app\Codex.exe";
        assert!(is_package_image(app, family));
        assert!(is_package_image(&app.to_lowercase(), family), "any case");
        for other in [
            r"C:\Program Files\WindowsApps\Claude_2.19675.1.0_x64__pzs8sxrjxfjjc\app\Claude.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.930.7945.0_x64__otherpublisher\app\Codex.exe",
            r"C:\Program Files\WindowsApps\OpenAI.CodexBeta_1.0.0.0_x64__2p2nqsd0c76g0\app\Codex.exe",
            r"C:\Users\dev\Downloads\OpenAI.Codex_26.930.7945.0_x64__2p2nqsd0c76g0\Codex.exe",
            r"C:\Users\dev\AppData\Local\Mayak\mayak.exe",
            "",
        ] {
            assert!(!is_package_image(other, family), "{other}");
        }
        assert!(!is_package_image(app, "OpenAI.Codex"), "a family without its publisher");
    }

    fn w(hwnd: isize, title: &str) -> WindowInfo {
        WindowInfo { hwnd, title: title.into(), minimized: false }
    }

    fn no_workspaces(_: &[String]) -> Workspaces {
        Workspaces::new()
    }

    #[test]
    fn the_editor_step_is_ready_raise_or_wait() {
        let root = r"C:\src\app";
        let wins = [w(1, "a.rs - other - Cursor"), w(2, "b.rs - app - Cursor")];
        let step = |front: Option<isize>, wins: &[WindowInfo]| editor_step(HostKind::Cursor, root, front, wins, no_workspaces);
        assert_eq!(step(Some(2), &wins), EditorStep::Ready, "the project window is in front");
        assert_eq!(step(Some(1), &wins), EditorStep::Raise(2), "another project is in front");
        assert_eq!(step(Some(99), &wins), EditorStep::Raise(2), "the island is in front");
        assert_eq!(step(None, &wins), EditorStep::Raise(2));
        assert_eq!(step(Some(1), &wins[..1]), EditorStep::Wait, "the project is not open yet");
        let twins = [w(1, "a.rs - app - Cursor"), w(2, "b.rs - app - Cursor")];
        assert_eq!(step(Some(99), &twins), EditorStep::Wait, "two windows of one folder name: the launcher decides");
        assert_eq!(step(Some(2), &twins), EditorStep::Ready, "the launcher brought one forward");
        let ws = [w(3, "x - all (Workspace) - Cursor")];
        let known = |_: &[String]| Workspaces::from([("all".to_string(), vec![r"c:\src".to_string()])]);
        assert_eq!(editor_step(HostKind::Cursor, root, Some(3), &ws, known), EditorStep::Ready, "a workspace holding the root");
        assert_eq!(editor_step(HostKind::VsCode, root, Some(2), &wins, no_workspaces), EditorStep::Wait, "Cursor titles are no VS Code window");
    }

    #[test]
    fn the_waits_are_bounded() {
        let (mut probes, mut slept) = (0, Duration::ZERO);
        let found: Option<()> = wait_for(EDITOR_WAIT, WAIT_STEP, |d| slept += d, || {
            probes += 1;
            None
        });
        assert_eq!((found, probes, slept), (None, 61, Duration::from_secs(15)), "15 s in 250 ms steps, then stop");
        let (mut probes, mut slept) = (0, Duration::ZERO);
        let found: Option<()> = wait_for(CODEX_WAIT, WAIT_STEP, |d| slept += d, || {
            probes += 1;
            None
        });
        assert_eq!((found, probes, slept), (None, 21, Duration::from_secs(5)));
        let (mut probes, mut slept) = (0, Duration::ZERO);
        let found = wait_for(EDITOR_WAIT, WAIT_STEP, |d| slept += d, || {
            probes += 1;
            (probes == 3).then_some(7)
        });
        assert_eq!((found, probes, slept), (Some(7), 3, Duration::from_millis(500)), "stops at the first hit");
    }

    #[test]
    fn outcomes_have_stable_wire_shapes_and_names() {
        assert_eq!(serde_json::to_value(LaunchOutcome::Opened).unwrap(), json!({ "kind": "opened" }));
        assert_eq!(serde_json::to_value(LaunchOutcome::OpenedPasteYourself).unwrap(), json!({ "kind": "openedPasteYourself" }));
        assert_eq!(
            serde_json::to_value(LaunchOutcome::Failed { reason: "x".into() }).unwrap(),
            json!({ "kind": "failed", "reason": "x" })
        );
        let all = [LaunchOutcome::Opened, LaunchOutcome::OpenedPasteYourself, LaunchOutcome::Failed { reason: String::new() }];
        let names: Vec<&str> = all.iter().map(LaunchOutcome::name).collect();
        assert_eq!(names, ["Opened", "OpenedPasteYourself", "Failed"]);
        for agent in ALL {
            assert_eq!(serde_json::to_value(agent).unwrap(), json!(agent.as_str()));
        }
    }

    #[test]
    fn the_log_line_has_only_the_agent_and_the_outcome() {
        let failed = LaunchOutcome::Failed { reason: r"C:\Users\dev\secret question".into() };
        assert_eq!(log_line(AgentId::CodexDesktop, &failed), "launcher: agent=codexDesktop outcome=Failed");
        assert_eq!(log_line(AgentId::ClaudeVsCode, &LaunchOutcome::Opened), "launcher: agent=claudeVsCode outcome=Opened");
    }
}
