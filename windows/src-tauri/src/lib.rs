// Mayak for Windows — app wiring and the commands the island calls.

mod apps;
#[cfg(windows)]
mod apps_win;
#[cfg(target_os = "macos")]
mod apps_mac;
mod autostart;
mod chat;
mod claude;
mod antigravity_hooks;
mod codex_hooks;
mod codex_limits;
mod deepseek;
mod files;
mod focus;
mod git_state;
mod github;
mod gitlab;
mod hookfile;
mod hooks;
mod hotkey;
#[allow(dead_code)]
#[path = "../../ident.rs"]
mod ident;
mod integrations;
mod island;
mod key_reads;
mod launcher;
mod log;
mod message;
mod media;
mod mcp_server;
mod moonshot;
mod oauth;
mod oauth_clients;
mod observe;
mod openrouter;
mod pipe;
mod platform;
mod resume;
mod secrets;
mod session_menu;
mod settings;
mod sounds;
mod stats;
#[cfg(windows)]
mod strip;
#[cfg(test)]
mod testutil;
mod tray;
mod turn_open;

use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::MacosLauncher;

use chat::ChatManager;
use claude::{Chat, ChatContext, ChatReply};
use files::DroppedFile;
use hookfile::Target;
use hooks::{HookCtx, HookPreview, HookStatus};
use message::Message;
use island::{PollGate, ScreenInfo};
use pipe::{CodexAnswers, Pending};
use observe::model::{Alert, HostKind, SessionsSnapshot, Signal};
use settings::Settings;

pub struct Shared {
    pub settings: Mutex<Settings>,
    pub gate: Arc<PollGate>,
    /// The window the hotkey was pressed in (hotkey_take_focus), given the keyboard back on fold.
    pub hotkey_return: Mutex<Option<isize>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootInfo {
    settings: Settings,
    screen: ScreenInfo,
    version: String,
    hook_path: String,
    /// False where the OS has no global cursor (Wayland): the page then reports
    /// the cursor from its own mouse events.
    cursor_poll: bool,
    /// True where a hidden island shows its line: Windows (`strip.rs`), and a Mac screen without
    /// a notch (`island::strip_visible`).
    native_strip: bool,
    /// The tray's Pause as Rust holds it: a page reloaded for a new language keeps quiet mode.
    paused: bool,
}

#[tauri::command]
fn boot(app: AppHandle, shared: State<Shared>) -> BootInfo {
    // Start-up never reads ~/.claude/settings.json (spec §8): the legacy hook
    // status is fetched only when the settings window asks for it.
    let settings = shared.settings.lock().unwrap().clone();
    let screen = island::screen_info(&app, &settings.screen);
    BootInfo {
        settings,
        screen,
        version: env!("CARGO_PKG_VERSION").to_string(),
        hook_path: settings::hook_exe_path().to_string_lossy().to_string(),
        cursor_poll: platform::CURSOR_POLL,
        native_strip: island::strip_visible(),
        paused: integrations::PAUSED.load(Ordering::Relaxed),
    }
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    webview: tauri::Webview,
    shared: State<Shared>,
    observe: State<observe::Handle>,
    mut settings: Settings,
) {
    // Only the Settings window changes launch at login: another page's copy of the flag may
    // predate a change made in Windows, and must not undo it.
    let previous_autostart = shared.settings.lock().unwrap().autostart;
    if webview.label() != "settings" {
        settings.autostart = previous_autostart;
    }
    if settings.autostart != previous_autostart {
        if let Err(real) = autostart::apply(&app, settings.autostart, previous_autostart) {
            settings.autostart = real;
        }
    }
    let (screen_changed, hotkey_changed, media_changed, gitlab_changed) = {
        let mut current = shared.settings.lock().unwrap();
        // The MCP server is switched by `mcp_set_enabled` alone: a page's copy of these two may
        // predate that, and must neither start the server nor stop it. Kept under the same lock
        // as the store below, so a switch cannot land between the two and be written back.
        keep_mcp_switch(&current, &mut settings);
        let screen_changed = current.screen != settings.screen;
        let hotkey_changed = current.active_hotkey() != settings.active_hotkey()
            || current.active_next_hotkey() != settings.active_next_hotkey();
        let media_changed = current.show_media != settings.show_media;
        let gitlab_changed = current.gitlab_url != settings.gitlab_url;
        *current = settings.clone();
        (screen_changed, hotkey_changed, media_changed, gitlab_changed)
    };
    // Another GitLab: what was seen on the old one says nothing about it (no events on its first read).
    if gitlab_changed {
        gitlab::forget();
    }
    if let Err(err) = save_latest(&shared.settings, settings::save) {
        log::line(format!("settings: could not save settings.json ({:?})", err.kind()));
    }
    if screen_changed {
        let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
        place_island(&app, &settings.screen, collapsed);
    }
    // Only a real change reaches the hotkey thread: the island saves the sound and the
    // auto-close interval through here too (hotkey spec §3.1).
    if hotkey_changed {
        hotkey::apply(&app, settings.active_hotkey(), settings.active_next_hotkey());
    }
    // "Show what's playing": the media thread subscribes again, or drops its subscriptions.
    if media_changed {
        media::set_enabled(settings.show_media);
    }
    // Alert kinds the model may raise from now on (spec §8).
    observe.send(observe::model::Signal::AlertPrefs {
        waiting: settings.alert_waiting,
        finished: settings.alert_finished,
        error: settings.alert_error,
    });
    // Keep the other window in step (island ⇄ settings window): with what the app holds now, which
    // a switch of the MCP server may have changed since the store above.
    let latest = shared.settings.lock().unwrap().clone();
    let _ = app.emit("settings-changed", latest);
}

/// `page`, a page's copy of the settings about to be stored over `current`, keeps what only a
/// dedicated command changes: the MCP server's switch and port.
fn keep_mcp_switch(current: &Settings, page: &mut Settings) {
    page.mcp_enabled = current.mcp_enabled;
    page.mcp_port = current.mcp_port;
}

/// One writer of settings.json at a time.
static SETTINGS_WRITE: Mutex<()> = Mutex::new(());

/// Writes the settings the app holds when this call gets its turn, not a copy taken earlier: a
/// writer that was slower than another (`save_settings`, `mcp_set_enabled`) cannot put its older
/// copy over the newer file. Every change is made in `settings` before it asks to be written, so
/// the last write always holds all of them.
fn save_latest(settings: &Mutex<Settings>, write: impl FnOnce(&Settings) -> std::io::Result<()>) -> std::io::Result<()> {
    let _turn = SETTINGS_WRITE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let latest = settings.lock().unwrap().clone();
    write(&latest)
}

/// Places the window for `collapsed`. On Windows a collapsed island is the visible strip, whose
/// thread is (re)started for this monitor; a visible island has no strip thread.
fn place_island(app: &AppHandle, pref: &str, collapsed: bool) {
    #[cfg(windows)]
    if !collapsed {
        strip::leave();
    }
    island::apply_geometry(app, pref, collapsed);
    #[cfg(windows)]
    if collapsed {
        match island::strip_geometry(app, pref) {
            Some(geo) => strip::enter(app.clone(), geo),
            None => log::line("strip: no monitor to place it on"),
        }
    }
}

/// Hidden island → the strip zone (Windows: the visible strip, its thread running; elsewhere the
/// invisible wake strip) and the cursor poll parked; anything else → full panel, no strip thread
/// and 60 Hz polling. The observer publishes every change only while the island is visible (spec §9).
#[tauri::command]
fn set_collapsed(app: AppHandle, shared: State<Shared>, observe: State<observe::Handle>, collapsed: bool) {
    let pref = shared.settings.lock().unwrap().screen.clone();
    shared.gate.collapsed.store(collapsed, Ordering::Relaxed);
    place_island(&app, &pref, collapsed);
    // A resize invalidates the click-through flag: the strip zone passes every click, the panel
    // takes the mouse until the next poll tick decides.
    island::refresh_click_through(&app, &shared.gate);
    shared.gate.set_active(!collapsed);
    observe.send(Signal::Visible(!collapsed));
}

/// The front end pushes the island shape; Rust decides click-through from it.
#[tauri::command]
fn set_island_rect(app: AppHandle, shared: State<Shared>, x: f64, y: f64, width: f64, height: f64) {
    shared.gate.set_rect(island::IslandRect { x, y, w: width, h: height });
    // Without the cursor poll the input region is the click-through: it follows the island.
    if !platform::CURSOR_POLL {
        island::refresh_click_through(&app, &shared.gate);
    }
}

#[tauri::command]
fn focus_window(app: AppHandle, focused: bool) {
    let Some(win) = island::window(&app) else { return };
    platform::set_activating(&win, focused);
    if focused {
        let _ = win.set_focus();
    }
}

/// The hotkey opened the island: remember the window that has the keyboard, then take it, so the
/// island's keys (↑ ↓ Enter Esc) reach the page. Only the hotkey path calls this.
#[tauri::command]
fn hotkey_take_focus(app: AppHandle, shared: State<Shared>) {
    let Some(win) = island::window(&app) else { return };
    *shared.hotkey_return.lock().unwrap() = platform::foreground_window(&win);
    platform::set_activating(&win, true);
    let _ = win.set_focus();
}

/// The hotkey-opened island folds, or another window took the keyboard: the island stops taking
/// it, and the window the hotkey came from gets it back if platform::should_restore_focus says so.
/// `keep_activating`: the chat is open and keeps taking the keyboard as it always did.
#[tauri::command]
fn hotkey_give_focus(app: AppHandle, shared: State<Shared>, opened_something: bool, keep_activating: bool) {
    let Some(win) = island::window(&app) else { return };
    let target = shared.hotkey_return.lock().unwrap().take();
    if let Some(target) = target {
        platform::restore_foreground(&win, target, opened_something);
    }
    if !keep_activating {
        platform::set_activating(&win, false);
    }
}

#[tauri::command]
fn reposition(app: AppHandle, shared: State<Shared>) {
    let pref = shared.settings.lock().unwrap().screen.clone();
    let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
    place_island(&app, &pref, collapsed);
}

#[tauri::command]
fn open_url(url: String) {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return;
    }
    platform::open_url(&url);
}

/// "Open terminal" opens the working folder in VS Code when `code` is on PATH,
/// and falls back to the file manager otherwise.
#[tauri::command]
fn open_in_vscode(path: Option<String>) -> bool {
    // No shell anywhere near this. The path is a project folder chosen by
    // whoever is using Claude Code, and a shell would happily read `&`, `^`, `%`
    // or `$` in a folder name as syntax. Finding the launcher ourselves and
    // handing the path over as a separate argument keeps it a path.
    let path = path.filter(|p| !p.is_empty());
    // It arrives in a hook payload: only an existing folder, given by its full
    // path, goes any further. `code` would read `--something` as an option, and
    // xdg-open would launch a file with whatever handles its type.
    if let Some(p) = path.as_deref() {
        let p = std::path::Path::new(p);
        if !(p.is_absolute() && p.is_dir()) {
            return false;
        }
    }
    if let Some(code) = platform::find_on_path("code") {
        let mut cmd = Command::new(code);
        if let Some(p) = path.as_deref() {
            cmd.arg(p);
        }
        if platform::no_console(&mut cmd).spawn().is_ok() {
            return true;
        }
    }
    if let Some(p) = path.as_deref() {
        platform::reveal_folder(p);
    }
    false
}

/// Base paths of the observer and the installer. `app_paths()` is the only reader of these
/// environment variables for them (spec §5.5 "Пути", §8 "CLAUDE_CONFIG_DIR и CODEX_HOME").
#[derive(Debug, Clone, PartialEq, Eq)]
struct AppPaths {
    claude_dir: std::path::PathBuf,
    /// `.claude.json`, where Claude Code caches the plan limits (usage-home spec §2.1).
    claude_json: std::path::PathBuf,
    codex_home: std::path::PathBuf,
    /// `~/.gemini`: Antigravity's global hooks file is `config\hooks.json` in it.
    gemini_dir: std::path::PathBuf,
    appdata: std::path::PathBuf,
    localappdata: std::path::PathBuf,
}

fn app_paths() -> AppPaths {
    app_paths_from(|name| std::env::var_os(name))
}

/// `var` reads one environment variable; an empty value counts as unset. Without
/// `USERPROFILE` (`platform::HOME_VAR`) the home is upstream's `platform::home_dir()`.
fn app_paths_from(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> AppPaths {
    use std::path::PathBuf;
    let get = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = get(platform::HOME_VAR).unwrap_or_else(platform::home_dir);
    let claude_config_dir = get("CLAUDE_CONFIG_DIR");
    AppPaths {
        // `.claude.json` sits inside $CLAUDE_CONFIG_DIR, else next to the `.claude` folder.
        claude_json: claude_config_dir.as_ref().map_or_else(|| home.join(".claude.json"), |dir| dir.join(".claude.json")),
        claude_dir: claude_config_dir.unwrap_or_else(|| home.join(".claude")),
        codex_home: get("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
        gemini_dir: home.join(".gemini"),
        appdata: get("APPDATA").unwrap_or_else(|| PathBuf::from(".")),
        localappdata: get("LOCALAPPDATA").unwrap_or_else(|| PathBuf::from(".")),
    }
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Tray → Pause and the island's quiet button. Paused means paused: the pollers stop talking to
/// the network, not just the island stopping showing things. The tray item shows the same state.
#[tauri::command]
fn set_paused(app: AppHandle, paused: bool) {
    integrations::set_paused(paused);
    tray::show_paused(&app, paused);
}

/// Focus mode lives in the island (night spec §A); this only keeps the tray's Focus check in step.
#[tauri::command]
fn set_focus_check(app: AppHandle, on: bool) {
    tray::show_focus(&app, on);
}

/// The tray's five texts in the island's language (batch spec §C); Rust keeps no dictionary.
/// Each text is cut to 64 characters.
#[tauri::command]
fn tray_texts(app: AppHandle, texts: tray::TrayTexts) {
    tray::set_texts(&app, texts.capped());
}

// ── Sessions ──────────────────────────────────────────────────────────────────

#[tauri::command]
fn sessions_snapshot(observe: State<observe::Handle>) -> SessionsSnapshot {
    observe.snapshot()
}

/// Git state of the projects named by `keys` (ideas #5), keyed by project key. Asked by the session
/// list only while it is on screen. Only roots of the current snapshot are read; a project that is no
/// repository, a git error or no git on PATH gives no entry. The reads run off the UI thread.
#[tauri::command]
async fn git_states(
    observe: State<'_, observe::Handle>,
    keys: Vec<String>,
) -> Result<std::collections::HashMap<String, git_state::GitState>, String> {
    let projects: Vec<(String, String)> =
        observe.snapshot().projects.into_iter().map(|p| (p.key, p.root)).collect();
    let roots = git_state::pick_roots(&projects, &keys);
    tauri::async_runtime::spawn_blocking(move || git_state::read_all(roots))
        .await
        .map_err(|_| "git state task failed".to_string())
}

/// Row click / "OK": marks the session's current episode as seen; never changes its phase.
#[tauri::command]
fn session_ack(observe: State<observe::Handle>, key: String) {
    observe.send(Signal::Ack(key));
}

/// The «↗ Open» action: bring the window of the session forward (sub-project 2), otherwise start
/// the launcher of its host, otherwise Explorer. True when the switch was started; an unknown key,
/// a relative root or a missing folder is false and does nothing. The switch itself runs on a
/// short-lived thread: raising a window settles for ~150 ms and must not hold the UI thread.
/// A Claude Desktop session with a known chat id also opens that chat, and when no window was
/// found the chat link replaces the Explorer fallback (quick wins §3.1, Windows). Such a session
/// whose folder is not on this disk (a scratch chat lives in the MSIX package, §3.2) is looked for
/// in the package's cache, and opens without a folder when it has its chat id.
#[tauri::command]
fn session_open(observe: State<observe::Handle>, key: String) -> bool {
    let snapshot = observe.snapshot();
    // Antigravity's own agent opens Antigravity, whose window needs no folder (a conversation
    // without a workspace has none).
    #[cfg(any(windows, target_os = "macos"))]
    if let Some(root) = antigravity_ide_root(&snapshot, &key) {
        let folder = root.filter(|root| std::path::Path::new(root).is_dir());
        std::thread::spawn(move || {
            focus::switch_to(HostKind::AntigravityIde, folder.as_deref(), None, None);
        });
        return true;
    }
    let Some((host, root)) = session_target(&snapshot, &key) else { return false };
    let agent_pid = session_agent_pid(&snapshot, &key);
    #[cfg(any(windows, target_os = "macos"))]
    let desktop_chat = session_desktop_chat(&snapshot, &key);
    #[cfg(not(any(windows, target_os = "macos")))]
    let desktop_chat: Option<String> = None;
    let folder = session_folder(
        host,
        &root,
        desktop_chat.is_some(),
        |path| path.is_dir(),
        |path| {
            let paths = app_paths();
            focus::msix::real_path(path, &paths.appdata, &paths.localappdata, |p| p.is_dir())
        },
    );
    let folder = match folder {
        Folder::There(folder) => Some(folder),
        Folder::Absent => None,
        Folder::Refused => return false,
    };
    std::thread::spawn(move || {
        focus::switch_to(host, folder.as_deref(), agent_pid, desktop_chat.as_deref());
    });
    true
}

/// The row menu of the session list (idea #8): the items a session offers, asked on the right-click.
/// Reads only the snapshot and whether the project folder exists. An unknown key offers nothing.
#[tauri::command]
fn session_menu(observe: State<observe::Handle>, key: String) -> Vec<session_menu::MenuItem> {
    let snapshot = observe.snapshot();
    match menu_row(&snapshot, &key) {
        Some(row) => session_menu::items(&row.facts()),
        None => Vec::new(),
    }
}

/// What a row menu item did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum MenuDone {
    /// Copied, or the folder opened.
    Done,
    /// Linux and macOS: the page puts `text` on the clipboard itself.
    #[cfg_attr(windows, allow(dead_code))]
    Copy { text: String },
    /// The item has nothing to act on (the session is gone, the folder is missing) or it failed.
    Failed,
}

/// A row menu item, on an explicit click or Enter only (idea #8). The copies go to the clipboard
/// from here on Windows, kept out of the clipboard history like the launcher's; the texts are never
/// logged and only the copy for the page's own clipboard call (not Windows) reaches the page.
#[tauri::command]
fn session_menu_run(observe: State<observe::Handle>, key: String, item: session_menu::MenuItem) -> MenuDone {
    let snapshot = observe.snapshot();
    let Some(row) = menu_row(&snapshot, &key) else { return MenuDone::Failed };
    let facts = row.facts();
    if !session_menu::items(&facts).contains(&item) {
        return MenuDone::Failed;
    }
    if item == session_menu::MenuItem::OpenFolder {
        let Some(folder) = facts.folder else { return MenuDone::Failed };
        return match Command::new(focus::FOLDER_OPENER).arg(folder).spawn() {
            Ok(_) => MenuDone::Done,
            Err(err) => {
                log::line(format!("session menu: the file manager did not start: {}", err.kind()));
                MenuDone::Failed
            }
        };
    }
    let Some(text) = session_menu::copy_text(item, &facts) else { return MenuDone::Failed };
    #[cfg(windows)]
    {
        match launcher::win::set_clipboard_text(&text) {
            Ok(()) => MenuDone::Done,
            Err(_) => {
                log::line("session menu: the clipboard is busy");
                MenuDone::Failed
            }
        }
    }
    #[cfg(not(windows))]
    {
        MenuDone::Copy { text }
    }
}

/// A session row as the row menu sees it: the session, and its project folder found as «Open»
/// finds it (`session_folder`, without the chat fallback).
struct MenuRow<'a> {
    session: &'a observe::model::SessionView,
    /// The folder on disk, or else the absolute project root; None for a relative or empty root.
    folder: Option<String>,
    folder_exists: bool,
}

impl MenuRow<'_> {
    fn facts(&self) -> session_menu::RowFacts<'_> {
        session_menu::RowFacts {
            agent: self.session.agent,
            logical_id: self.session.logical_id.as_deref().unwrap_or(""),
            reply: self.session.last_reply.as_deref(),
            folder: self.folder.as_deref(),
            folder_exists: self.folder_exists,
        }
    }
}

/// The row of `key` in a snapshot with its folder; None for an unknown key and for a silent row
/// that is no session (`silent:<id>`).
fn menu_row<'a>(snapshot: &'a SessionsSnapshot, key: &str) -> Option<MenuRow<'a>> {
    if key.starts_with("silent:") {
        return None;
    }
    let (project, session) = snapshot
        .projects
        .iter()
        .find_map(|p| p.sessions.iter().find(|s| s.key == key).map(|s| (p, s)))?;
    let root = Some(project.root.as_str()).filter(|r| std::path::Path::new(r).is_absolute());
    let found = root.map(|root| {
        session_folder(
            session.host,
            root,
            false,
            |path| path.is_dir(),
            |path| {
                let paths = app_paths();
                focus::msix::real_path(path, &paths.appdata, &paths.localappdata, |p| p.is_dir())
            },
        )
    });
    let (folder, folder_exists) = match (found, root) {
        (Some(Folder::There(folder)), _) => (Some(folder), true),
        (_, Some(root)) => (Some(root.to_string()), false),
        (_, None) => (None, false),
    };
    Some(MenuRow { session, folder, folder_exists })
}

/// «Итог хода» (ideas §3): opens the `index`th file the session's last turn changed in an editor,
/// at its first changed line. False for an unknown session or index, an index that no longer holds
/// `path` (the list changed since the island drew it), a path that is no file on
/// this disk, a path `turn_open::file_uri` refuses, or no Cursor / VS Code protocol registered.
#[tauri::command]
fn session_file_open(observe: State<observe::Handle>, key: String, index: usize, path: String) -> bool {
    let snapshot = observe.snapshot();
    let Some((host, file)) = turn_open::turn_file(&snapshot, &key, index, &path) else { return false };
    if !std::path::Path::new(&file.path).is_file() {
        log::line("session_file_open: not a file on this disk");
        return false;
    }
    turn_open::open(host, &file.path, file.line)
}

/// What `session_open` has to work with.
#[derive(Debug, PartialEq, Eq)]
enum Folder {
    /// A folder that exists: the root, or for Claude Desktop its place in the MSIX package.
    There(String),
    /// A Claude Desktop session with a chat id and no folder: only the chat link is left.
    Absent,
    /// Nothing to open.
    Refused,
}

/// The folder of a session for «Open». Every host but Claude Desktop needs its root to be a
/// folder. For Claude Desktop a root that is not one is looked up in the package's cache
/// (`msix`), and when still missing the session goes on if it has a chat id (`has_chat`).
fn session_folder(
    host: HostKind,
    root: &str,
    has_chat: bool,
    is_dir: impl Fn(&std::path::Path) -> bool,
    msix: impl FnOnce(&std::path::Path) -> Option<std::path::PathBuf>,
) -> Folder {
    let path = std::path::Path::new(root);
    if is_dir(path) {
        return Folder::There(root.to_string());
    }
    if host != HostKind::ClaudeDesktop {
        return Folder::Refused;
    }
    match msix(path).and_then(|real| real.into_os_string().into_string().ok()) {
        Some(real) => Folder::There(real),
        None if has_chat => Folder::Absent,
        None => Folder::Refused,
    }
}

/// The session card's and the error card's «Copy resume» (ideas §7): the resume command of a
/// Claude or Codex session on the list, in its folder, onto the clipboard. False for an unknown
/// key, another agent, a refused id or folder, or a busy clipboard.
#[tauri::command]
fn session_resume_copy(observe: State<observe::Handle>, key: String) -> bool {
    match resume::live_target(&observe.snapshot(), &key) {
        Some((agent, id, cwd)) => resume::copy(agent, &id, &cwd),
        None => false,
    }
}

/// The sessions list's «Recent» fold was opened (ideas §7): the Claude and Codex sessions of the
/// last 24 hours that are not on the list. Reads files, so it runs on its own thread; only then,
/// never on a timer.
#[tauri::command]
async fn recent_sessions(observe: State<'_, observe::Handle>) -> Result<Vec<resume::RecentSession>, ()> {
    let live = resume::live_ids(&observe.snapshot());
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let paths = app_paths();
        let roots = resume::RecentRoots { claude_projects: paths.claude_dir.join("projects"), codex_home: paths.codex_home };
        let _ = tx.send(resume::scan(&roots, observe::now_ms(), &live));
    });
    Ok(rx.await.unwrap_or_else(|_| {
        log::line("recent: scan stopped");
        Vec::new()
    }))
}

/// «Resume» on a recent row: a new console in the session's folder runs the agent's resume
/// command; without the CLI on PATH the command is copied instead.
#[tauri::command]
fn recent_resume(agent: resume::ResumeAgent, id: String, cwd: String) -> resume::ResumeOutcome {
    resume::start(agent, &id, &cwd)
}

/// «Copy» on a recent row: the resume command onto the clipboard.
#[tauri::command]
fn recent_copy(agent: resume::ResumeAgent, id: String, cwd: String) -> bool {
    resume::copy(agent, &id, &cwd)
}

/// The home view became active: read Claude Code's cached plan limits again (usage-home spec §2.1),
/// unless switched off, Codex's limits from its app server when the held ones are stale, and the
/// OpenRouter, DeepSeek and Moonshot balances whose key is saved when they are stale. Returns at
/// once; the reads run on their own threads and their results come with a later snapshot.
#[tauri::command]
fn usage_refresh(observe: State<observe::Handle>, shared: State<Shared>) {
    observe.claude_usage();
    if shared.settings.lock().unwrap().codex_fresh_limits {
        codex_limits::refresh(&observe, app_paths().localappdata);
    }
    refresh_balances(&observe);
}

/// The balances of the providers with a saved key (`openrouter`, `deepseek`, `moonshot`), each read
/// only when its gate lets it.
fn refresh_balances(observe: &observe::Handle) {
    openrouter::refresh(observe);
    deepseek::refresh(observe);
    moonshot::refresh(observe);
}

/// The home page's minute tick while it is on screen: the provider balances again once they are
/// older than 5 min (`refresh_balances`). Nothing else is read.
#[tauri::command]
fn balances_refresh(observe: State<observe::Handle>) {
    refresh_balances(&observe);
}

/// A key of a balance provider was saved or removed: its old balance goes.
fn balance_key_changed(observe: &observe::Handle, key: &str) {
    match key {
        openrouter::KEY => openrouter::key_changed(observe),
        deepseek::KEY => deepseek::key_changed(observe),
        moonshot::KEY => moonshot::key_changed(observe),
        _ => {}
    }
}

/// The Stats tab (night spec §C): tokens of the last `days` (7 or 30) local days, counted from the
/// agents' files with `stats-cache.json` sparing the unchanged ones. Only when the tab asks, on its
/// own thread: the first count streams through hundreds of megabytes.
#[tauri::command]
async fn stats_query(days: u32) -> Result<stats::StatsReport, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(stats_report(days));
    });
    rx.await.unwrap_or_else(|_| {
        log::line("stats: the count stopped");
        Err("The count stopped.".to_string())
    })
}

/// The count itself, on the calling thread (it streams through the agents' files): the Stats tab's
/// and the MCP server's `get_stats`.
pub(crate) fn stats_report(days: u32) -> Result<stats::StatsReport, String> {
    let paths = app_paths();
    let roots = stats::Roots {
        claude_projects: paths.claude_dir.join("projects"),
        codex_sessions: paths.codex_home.join("sessions"),
    };
    let dir = platform::local_dir();
    match platform::ensure_private_dir(&dir) {
        Ok(()) => {
            let now = observe::now_ms();
            let offset = stats::local_offset_minutes(&platform::local_time(), now);
            stats::query(days, &roots, &dir.join("stats-cache.json"), now, offset, observe::paths::git_probe)
        }
        Err(err) => Err(format!("{:?}", err.kind())),
    }
}

/// The agents the prompt view can open (launcher spec §2-§3), in the spec's order, "claudeApi"
/// last when the Anthropic key is saved. Reads folders, the registry and the Credential Manager,
/// so it runs on its own thread.
#[cfg(windows)]
#[tauri::command]
async fn launcher_targets() -> Vec<launcher::AgentId> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let targets = launcher::offered(launcher::installed_targets(
            &platform::home_dir(),
            &app_paths().localappdata,
            launcher::win::protocol_registered,
            secrets::present(launcher::API_KEY),
        ));
        let _ = tx.send(targets);
    });
    match rx.await {
        Ok(targets) => targets,
        Err(_) => {
            log::line("launcher: targets lookup stopped");
            Vec::new()
        }
    }
}

/// macOS and Linux: the launcher finds no agent yet.
#[cfg(not(windows))]
#[tauri::command]
async fn launcher_targets() -> Vec<launcher::AgentId> {
    Vec::new()
}

/// The prompt view's «Send» (launcher spec §4): opens `agent` in `root` with the question in its
/// field and never sends it. The work runs on its own thread, like `session_open`'s; the island
/// gets the outcome when it is done (an editor that has to start takes up to 15 s). Elsewhere than
/// on Windows the outcome is `Failed` («not supported on this platform»).
#[tauri::command]
async fn launcher_open(
    agent: launcher::AgentId,
    prompt: String,
    root: Option<String>,
    files: Vec<String>,
) -> launcher::LaunchOutcome {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let localappdata = app_paths().localappdata;
        let inbox = crate::files::inbox_dir();
        let _ = tx.send(launcher::open(agent, &prompt, root, &files, &localappdata, &inbox));
    });
    rx.await.unwrap_or_else(|_| launcher::LaunchOutcome::Failed { reason: "The launch stopped.".to_string() })
}

/// The apps tab's tiles (spec 2026-10-07 §D, own-apps ideas §30): the installed IDEs and agent apps
/// the user did not hide and the user's own apps, in the user's order. Reads folders and the
/// registry, so it runs on its own thread.
#[tauri::command]
async fn apps_list(app: AppHandle) -> Vec<apps::AppTile> {
    let settings = app.state::<Shared>().settings.lock().unwrap().clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(apps::list(&settings));
    });
    rx.await.unwrap_or_else(|_| {
        log::line("apps: lookup stopped");
        Vec::new()
    })
}

/// Every built-in app, installed or not: Settings names the hidden ones.
#[tauri::command]
fn apps_builtin() -> Vec<apps::AppEntry> {
    apps::builtins()
}

/// A tile's click: opens the app on its own thread (raising a window waits for the foreground) and
/// answers at once whether `id` is an installed app or a user app whose exe is there.
#[tauri::command]
async fn app_open(app: AppHandle, id: String) -> bool {
    let settings = app.state::<Shared>().settings.lock().unwrap().clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        if apps::can_open(&id, &settings) {
            let _ = tx.send(true);
            let _ = apps::open(&id, &settings);
        } else {
            let _ = tx.send(false);
        }
    });
    rx.await.unwrap_or(false)
}

/// The "+" tile: the Open dialog for an exe or a shortcut, on its own thread, owned by the island.
/// The page adds what it gives to the settings. `title` and `filter` are the page's texts.
#[tauri::command]
async fn app_pick(app: AppHandle, title: String, filter: String) -> apps::PickOutcome {
    #[cfg(windows)]
    let owner = app
        .get_webview_window(island::WINDOW_LABEL)
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize)
        .filter(|raw| *raw != 0);
    #[cfg(not(windows))]
    let owner = {
        let _ = &app;
        None
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(apps::pick(owner, &title, &filter));
    });
    rx.await.unwrap_or(apps::PickOutcome::Failed)
}

/// The own icon of the user app `id`: `apps::ICON_SIZE`² RGBA bytes, sent as they are. An error
/// when it has none: the page draws its monogram.
#[tauri::command]
async fn app_icon(app: AppHandle, id: String) -> Result<tauri::ipc::Response, String> {
    let settings = app.state::<Shared>().settings.lock().unwrap().clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(apps::custom_icon(&id, &settings));
    });
    rx.await.ok().flatten().map(tauri::ipc::Response::new).ok_or_else(|| "no icon".to_string())
}

/// Host and project root of a session in a snapshot. `None` for an unknown key and for a root
/// that is not an absolute path: a launcher must never see a root like `--x` (bef6558).
fn session_target(snapshot: &SessionsSnapshot, key: &str) -> Option<(HostKind, String)> {
    snapshot
        .projects
        .iter()
        .find_map(|project| {
            project.sessions.iter().find(|s| s.key == key).map(|s| (s.host, project.root.clone()))
        })
        .filter(|(_, root)| std::path::Path::new(root).is_absolute())
}

/// For a session of Antigravity's own agent: Some(its project root when that is an absolute path).
fn antigravity_ide_root(snapshot: &SessionsSnapshot, key: &str) -> Option<Option<String>> {
    snapshot.projects.iter().find_map(|project| {
        project
            .sessions
            .iter()
            .find(|s| s.key == key && s.host == HostKind::AntigravityIde)
            .map(|_| Some(project.root.clone()).filter(|root| std::path::Path::new(root).is_absolute()))
    })
}

/// Pid of the agent process of a session in a snapshot.
fn session_agent_pid(snapshot: &SessionsSnapshot, key: &str) -> Option<u32> {
    snapshot
        .projects
        .iter()
        .find_map(|project| project.sessions.iter().find(|s| s.key == key).and_then(|s| s.agent_pid))
}

/// The Claude Desktop chat id of a session in a snapshot: only for a Claude Desktop session.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
fn session_desktop_chat(snapshot: &SessionsSnapshot, key: &str) -> Option<String> {
    snapshot.projects.iter().find_map(|project| {
        project
            .sessions
            .iter()
            .find(|s| s.key == key && s.host == HostKind::ClaudeDesktop)
            .and_then(|s| s.host_session_id.clone())
    })
}

// ── Hook installers (spec §8) ─────────────────────────────────────────────────

/// What the installer works on. `app_paths()` is the only reader of
/// CLAUDE_CONFIG_DIR / CODEX_HOME / USERPROFILE for it.
fn hook_ctx() -> HookCtx {
    let paths = app_paths();
    HookCtx {
        claude_dir: paths.claude_dir,
        codex_home: paths.codex_home,
        gemini_dir: paths.gemini_dir,
        hook_exe: settings::hook_exe_path(),
        plans: hookfile::Plans::default(),
        answer_plans: hookfile::Plans::default(),
    }
}

/// A fresh read of ~/.codex/hooks.json, for `Signal::CodexHooksInstalled`.
fn codex_hooks_installed(ctx: &HookCtx) -> bool {
    let our = codex_hooks::our_command(&ctx.hook_exe).ok();
    codex_hooks::installed_at(&ctx.codex_home, our.as_deref())
}

/// Read-only status of one target's config. `target` defaults to "claude".
/// A Codex read also refreshes whether Codex permissions go to the island, from the file just
/// read: an edit made outside Mayak cannot leave the server holding Codex for a card while its
/// hook would give up at 10 s.
#[tauri::command]
fn hooks_status(
    ctx: State<HookCtx>,
    codex_answers: State<Arc<CodexAnswers>>,
    target: Option<String>,
) -> Result<HookStatus, Message> {
    let target = Target::parse(target.as_deref())?;
    // Only the target name: shows when a tool config was read (spec §8, D58).
    log::line(format!("hooks_status target={}", target.as_str()));
    let status = hooks::status(&ctx, target);
    if target == Target::Codex {
        codex_answers.set(status.answers == Some(hooks::Answers::On));
    }
    Ok(status)
}

/// The diff the user has to look at before anything is written. The plan
/// behind it stays in Rust; apply writes exactly those bytes or nothing.
#[tauri::command]
fn hooks_preview(
    ctx: State<HookCtx>,
    target: Option<String>,
    install: bool,
) -> Result<HookPreview, Message> {
    let target = Target::parse(target.as_deref())?;
    hooks::preview(&ctx, target, install, &hookfile::local_stamp())
}

/// Only ever called from an explicit click in the settings window. No path
/// comes from the webview: the plan is found by target, install and the
/// fingerprint of the preview the user looked at.
#[tauri::command]
fn hooks_apply(
    ctx: State<HookCtx>,
    observe: State<observe::Handle>,
    codex_answers: State<Arc<CodexAnswers>>,
    target: Option<String>,
    install: bool,
    fingerprint: String,
) -> Result<String, Message> {
    let target = Target::parse(target.as_deref())?;
    let result = hooks::apply(&ctx, target, install, &fingerprint);
    if target == Target::Codex {
        // After every Codex apply (spec §8), on success and failure alike, also
        // when no plan matched: read back from disk, never from `install`.
        observe.send(observe::model::Signal::CodexHooksInstalled(codex_hooks_installed(&ctx)));
        // An install or removal decides whether Codex permissions go to the island (spec §3).
        codex_answers.set(codex_answers_enabled(&ctx));
    }
    result
}

/// The diff of switching Codex answers from the island on or off (spec §3). The plan stays in
/// Rust, apart from the install plans; refused while Mayak's Codex hooks are not installed.
#[tauri::command]
fn codex_answers_preview(ctx: State<HookCtx>, on: bool) -> Result<HookPreview, Message> {
    log::line(format!("codex_answers_preview on={on}"));
    hooks::answers_preview(&ctx, on, &hookfile::local_stamp())
}

/// Only ever called from an explicit click on the diff `codex_answers_preview` showed: the plan
/// is found by `on` and that preview's fingerprint. The server's flag is then read back from
/// hooks.json, on success and failure alike.
#[tauri::command]
fn codex_answers_apply(
    ctx: State<HookCtx>,
    codex_answers: State<Arc<CodexAnswers>>,
    on: bool,
    fingerprint: String,
) -> Result<String, Message> {
    let result = hooks::answers_apply(&ctx, on, &fingerprint);
    codex_answers.set(codex_answers_enabled(&ctx));
    result
}

/// Publishes the observer's output to the island window (spec §9).
struct IslandPublisher {
    app: AppHandle,
}

impl observe::Publisher for IslandPublisher {
    fn alerts(&self, alerts: &[Alert]) {
        if let Err(err) = self.app.emit_to(island::WINDOW_LABEL, "session-alert", alerts) {
            log::line(format!("session-alert emit failed: {err}"));
        }
    }

    fn snapshot(&self, snapshot: &SessionsSnapshot) {
        if let Err(err) = self.app.emit_to(island::WINDOW_LABEL, "sessions", snapshot) {
            log::line(format!("sessions emit failed: {err}"));
        }
        // The number of waiting sessions on the tray icon, their names in its tooltip.
        tray::show_waiting(&self.app, snapshot);
    }

    fn silent_wake(&self) {
        if let Err(err) = self.app.emit_to(island::WINDOW_LABEL, "session-silent", ()) {
            log::line(format!("session-silent emit failed: {err}"));
        }
    }

    fn rollover(&self) {
        if let Err(err) = self.app.emit_to(island::WINDOW_LABEL, "usage-rollover", ()) {
            log::line(format!("usage-rollover emit failed: {err}"));
        }
    }

    /// Called on the store thread; returns at once (the read runs on its own thread).
    fn codex_reset(&self) {
        let enabled = self.app.state::<Shared>().settings.lock().unwrap().codex_fresh_limits;
        let Some(observe) = self.app.try_state::<observe::Handle>() else {
            log::line("codex limits: the observer is not ready, no read after the reset");
            return;
        };
        codex_limits::refresh_after_reset(&observe, app_paths().localappdata, enabled);
    }
}

/// The router's view of the app (spec §5.3): Codex events feed the observer, the island's cards
/// get their session key and project name from it.
struct AppSink {
    app: AppHandle,
    pending: Arc<Pending>,
    codex_answers: Arc<CodexAnswers>,
    observe: observe::Handle,
}

impl pipe::RouterSink for AppSink {
    fn pending(&self) -> &Pending {
        &self.pending
    }

    fn codex_answers(&self) -> bool {
        self.codex_answers.get()
    }

    /// Never blocks: the channel is unbounded.
    fn observe(&self, payload: serde_json::Value) {
        if payload.get("agent").and_then(serde_json::Value::as_str) == Some("antigravity") {
            self.observe.antigravity_hook(payload);
        } else {
            self.observe.codex_hook(payload);
        }
    }

    fn approval_context(&self, payload: &serde_json::Value) -> pipe::ApprovalContext {
        let mut ctx = observe::approval_context(payload);
        if payload.get("agent").and_then(serde_json::Value::as_str) == Some("codex") {
            ctx.session_key = codex_session_key(payload);
        }
        ctx
    }

    /// The `hook` event of the island: the payload with `request_id`, `kind`, `agent`, `canAlways`.
    fn show_approval(&self, payload: serde_json::Value) {
        if let Err(err) = self.app.emit_to(island::WINDOW_LABEL, "hook", payload) {
            log::line(format!("hook emit failed: {err}"));
        }
    }

    fn is_chat_session(&self, session_id: &str) -> bool {
        self.app.try_state::<ChatManager>().is_some_and(|chat| chat.owns_session(session_id))
    }
}

/// What the island's CLI chat (src/chat) needs of the app: to tell the page that a conversation
/// changed, or that a request waits for a click.
struct AppChatHost {
    app: AppHandle,
}

impl chat::Host for AppChatHost {
    /// Skipped while the island is hidden: the chat view reads the conversation when it shows.
    fn changed(&self, ids: &[u64]) {
        let hidden = self.app.state::<Shared>().gate.collapsed.load(Ordering::Relaxed);
        if hidden {
            return;
        }
        if let Err(err) = self.app.emit_to(island::WINDOW_LABEL, "agent-chat-changed", ids) {
            log::line(format!("agent-chat-changed emit failed: {err}"));
        }
    }

    /// Always sent, hidden or not: the island opens on the chat for it.
    fn permission_asked(&self, conv: u64, request_id: &str, tool: &str) {
        let payload = serde_json::json!({ "conv": conv, "requestId": request_id, "tool": tool });
        if let Err(err) = self.app.emit_to(island::WINDOW_LABEL, "agent-chat-permission", payload) {
            log::line(format!("agent-chat-permission emit failed: {err}"));
        }
    }
}

/// A Codex session is keyed by its `session_id`, not by its process (observe::keys), so its card
/// and the observer's waiting row name the same session.
fn codex_session_key(payload: &serde_json::Value) -> Option<observe::keys::SessionKey> {
    payload
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .map(observe::keys::codex_key)
}

/// Whether Mayak's Codex `PermissionRequest` hook in hooks.json waits for the island (600 s).
/// A missing, unreadable or invalid file is false: Codex stays observed only.
fn codex_answers_enabled(ctx: &HookCtx) -> bool {
    let our = codex_hooks::our_command(&ctx.hook_exe).ok();
    codex_hooks::answers_enabled_at(&ctx.codex_home, our.as_deref())
}

/// Allow / Always / Deny on a permission card. "always" for Codex, or without a rule to keep,
/// is a plain allow. An unknown or expired id changes nothing.
#[tauri::command]
fn approval_decision(pending: State<Arc<Pending>>, request_id: String, decision: String) {
    pipe::answer(&pending, &request_id, &decision);
}

/// A question card's answers: question text → label, a list of labels for a multi-select, or
/// `{"other": text}` for an "Other" answer (not blank once trimmed, at most 4000 characters;
/// mayak-hook prints it as a plain string). Anything else is refused and the card keeps waiting;
/// an unknown or expired id changes nothing.
#[tauri::command]
fn question_answer(pending: State<Arc<Pending>>, request_id: String, answers: serde_json::Value) -> Result<(), String> {
    pipe::answer_question(&pending, &request_id, &answers).map_err(|err| err.to_string())
}

/// The island has the card on screen, so the long wait for a human may begin.
/// Until this arrives the relay only waits a few hundred milliseconds, which is
/// what stops a paused or unresponsive island from freezing Claude Code.
#[tauri::command]
fn approval_ack(pending: State<Arc<Pending>>, request_id: String) {
    pipe::acknowledge(&pending, &request_id);
}

/// Release without a decision — the island is paused, another card is already
/// up, the island folded or "In the terminal". The agent asks in its own window
/// immediately.
#[tauri::command]
fn approval_decline(pending: State<Arc<Pending>>, request_id: String) {
    pipe::decline(&pending, &request_id);
}

/// The island has just booted: a webview reload left no card of the page before on screen, so
/// every request still waiting is released, and its agent asks in its own window at once.
#[tauri::command]
fn approvals_release_all(pending: State<Arc<Pending>>) {
    let released = pipe::release_all(&pending);
    log::line(format!("approvals_release_all released={released}"));
}

// ── CLI chat (src/chat): the user's own agent CLIs, no API key ───────────────────

/// Runs `f` off the UI thread: starting a process or writing to a pipe blocks.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, chat::ChatError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| chat::ChatError::new("internal", "The chat could not run that."))
}

/// The CLIs: found or not, signed in or not, their models and modes. `refresh` skips the 30 s memory.
#[tauri::command]
async fn agent_chat_agents(chat: State<'_, ChatManager>, refresh: bool) -> Result<Vec<chat::AgentInfo>, chat::ChatError> {
    let chat = chat.inner().clone();
    blocking(move || chat.agents(refresh)).await
}

#[tauri::command]
fn agent_chat_open(chat: State<ChatManager>, request: chat::OpenRequest) -> Result<u64, chat::ChatError> {
    chat.open(request)
}

#[tauri::command]
fn agent_chat_list(chat: State<ChatManager>) -> Vec<chat::Summary> {
    chat.list()
}

/// The conversation; null when it is gone or `since` is its current revision.
#[tauri::command]
fn agent_chat_get(chat: State<ChatManager>, conv: u64, since: Option<u64>) -> Option<chat::Convo> {
    chat.get(conv, since)
}

#[tauri::command]
fn agent_chat_set(chat: State<ChatManager>, conv: u64, model: String, mode: String) -> Result<(), chat::ChatError> {
    chat.set_choice(conv, model, mode)
}

#[tauri::command]
async fn agent_chat_send(chat: State<'_, ChatManager>, conv: u64, text: String) -> Result<(), chat::ChatError> {
    let chat = chat.inner().clone();
    blocking(move || chat.send(conv, &text)).await?
}

/// Allow (true) or Deny (false) on a request of the chat. Only the buttons call this.
#[tauri::command]
async fn agent_chat_permission(chat: State<'_, ChatManager>, request_id: String, allow: bool) -> Result<bool, chat::ChatError> {
    let chat = chat.inner().clone();
    blocking(move || chat.answer_permission(&request_id, allow)).await
}

#[tauri::command]
async fn agent_chat_cancel(chat: State<'_, ChatManager>, conv: u64) -> Result<(), chat::ChatError> {
    let chat = chat.inner().clone();
    blocking(move || chat.cancel(conv)).await?
}

#[tauri::command]
async fn agent_chat_close(chat: State<'_, ChatManager>, conv: u64) -> Result<(), chat::ChatError> {
    let chat = chat.inner().clone();
    blocking(move || chat.close(conv)).await
}

// ── Chat, files and secrets ───────────────────────────────────────────────────

/// One chat turn. The API key and any file bytes stay on the Rust side.
/// The models the saved Anthropic key can use (ideas §15). Err without a key or when the API refuses.
#[tauri::command]
async fn chat_models() -> Result<Vec<claude::ModelInfo>, String> {
    claude::list_models().await
}

#[tauri::command]
async fn chat_send(
    shared: State<'_, Shared>,
    chat: State<'_, Chat>,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    let model = shared.settings.lock().unwrap().model.clone();
    claude::send(&chat, &model, query, context).await
}

#[tauri::command]
fn chat_reset(chat: State<Chat>) {
    chat.reset();
}

/// A file dropped on the island page (an HTML5 drop: WebView2 hands the page the contents, not
/// a path). The bytes are the raw request body, the name is URI-encoded in `x-file-name`. Async,
/// so writing a large file never holds up the main thread.
///
/// The 200 MB check in `files::ingest_bytes` runs only after Tauri has already buffered the whole
/// raw body, so it does not bound memory: that gate is the page's own size check before it reads
/// the file (`droppedItem` in src/core/bridge.ts). The check here only refuses to write it.
#[tauri::command]
async fn ingest_bytes(request: tauri::ipc::Request<'_>) -> Result<DroppedFile, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Expected the file's contents.".into());
    };
    let name = files::header_name(request.headers().get("x-file-name").and_then(|v| v.to_str().ok()));
    files::ingest_bytes(&name, bytes)
}

/// The island may only ask whether a key exists — never read it.
#[tauri::command]
fn secret_present(key: String) -> bool {
    secrets::page_key(&key) && secrets::present(&key)
}

/// The service whose access token `key` is (the token field writes the same entry Connect does).
fn token_service(key: &str) -> Option<oauth::Service> {
    [oauth::Service::GitHub, oauth::Service::GitLab].into_iter().find(|s| s.token_key() == key)
}

/// A token typed into the field belongs to the address configured now (GitLab), has no refresh
/// token, and ends a needs-reconnect. Refused while the GitLab address is not valid.
fn manual_token(app: &AppHandle, key: &str, present: bool, write: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let Some(service) = token_service(key) else { return write() };
    if !present {
        // Removing a token never depends on the address.
        write()?;
        return oauth::manual_token(service, "", false);
    }
    let host = oauth::host_of(app, service).ok_or_else(|| "invalid GitLab address".to_string())?;
    write()?;
    oauth::manual_token(service, &host, true)
}

#[tauri::command]
fn secret_set(app: AppHandle, observe: State<observe::Handle>, key: String, value: String) -> Result<(), String> {
    if !secrets::page_key(&key) {
        return Err(format!("unknown key {key}"));
    }
    manual_token(&app, &key, !value.is_empty(), || secrets::set(&key, &value))?;
    balance_key_changed(&observe, &key);
    Ok(())
}

#[tauri::command]
fn secret_clear(app: AppHandle, observe: State<observe::Handle>, key: String) -> Result<(), String> {
    if !secrets::page_key(&key) {
        return Err(format!("unknown key {key}"));
    }
    manual_token(&app, &key, false, || secrets::clear(&key))?;
    balance_key_changed(&observe, &key);
    Ok(())
}

// ── Connect (OAuth device flow, oauth.rs) ─────────────────────────────────────

fn service_of(name: &str) -> Result<oauth::Service, String> {
    oauth::Service::parse(name).ok_or_else(|| format!("unknown service {name}"))
}

/// Connect: the user code and the verification page for Settings, or the problem
/// (`oauth::StartError`: its code, and whether GitLab's token page is offered instead). The token
/// comes later, as an `oauth` event.
#[tauri::command]
async fn oauth_start(app: AppHandle, service: String) -> Result<oauth::DeviceStart, oauth::StartError> {
    let Some(service) = oauth::Service::parse(&service) else {
        return Err(oauth::StartError { code: "failed", fallback: false });
    };
    oauth::start(app, service).await.map_err(|p| oauth::StartError::of(service, p))
}

/// GitLab's «new personal access token» page, prefilled, on the configured https host only.
#[tauri::command]
fn open_gitlab_token_page(app: AppHandle) -> bool {
    gitlab::open_token_page(&app)
}

#[tauri::command]
fn oauth_cancel(service: String) -> Result<(), String> {
    oauth::cancel(service_of(&service)?);
    Ok(())
}

/// Disconnect: the token and what goes with it are deleted from the Credential Manager.
#[tauri::command]
fn oauth_disconnect(app: AppHandle, service: String) -> Result<(), String> {
    let service = service_of(&service)?;
    oauth::disconnect(service)?;
    let _ = app.emit("oauth", serde_json::json!({ "service": service.name(), "state": "disconnected" }));
    Ok(())
}

#[tauri::command]
fn oauth_status(service: String) -> Result<oauth::ConnState, String> {
    Ok(oauth::status(service_of(&service)?))
}

/// The GitLab address as Mayak will use it, or None when it is not a valid https host.
#[tauri::command]
fn gitlab_check_url(url: String) -> Option<String> {
    gitlab::normalize_base(&url).ok()
}

/// A link of the GitLab card: opened only when it is https on the configured GitLab host.
#[tauri::command]
fn open_gitlab(app: AppHandle, url: String) -> bool {
    gitlab::open_link(&app, &url)
}

/// What GitHub's Connect controls show: whether Connect signs in with a GitHub App, whether its
/// installation page can be opened (the slug is set), and how the saved token was had
/// (`oauth::github_mode`). Public values only.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GithubInfo {
    app: bool,
    installable: bool,
    mode: &'static str,
}

#[tauri::command]
fn github_info(app: AppHandle) -> GithubInfo {
    let service = oauth::Service::GitHub;
    GithubInfo {
        app: oauth::github_uses_app(&oauth::override_id(&app, service)),
        installable: github::install_url(oauth_clients::GITHUB_APP_SLUG).is_some(),
        mode: oauth::github_mode(secrets::present(service.token_key()), oauth::load_auth(service).as_ref()),
    }
}

/// A link of the GitHub card: opened only when it is https on github.com.
#[tauri::command]
fn open_github(url: String) -> bool {
    github::open_link(&url)
}

/// The installation page of Mayak's GitHub App (github.com/apps/<slug>/installations/new).
#[tauri::command]
fn open_github_install() -> bool {
    github::open_install_page()
}

/// Opens the configured n8n instance — the URL lives in the Credential Manager.
#[tauri::command]
fn open_n8n() {
    if let Some(url) = secrets::get("n8n-url") {
        open_url(url);
    }
}

/// Refresh buttons in the integration cards.
#[tauri::command]
async fn refresh_integration(app: AppHandle, id: String) {
    integrations::poll_once(app, &id).await;
}

/// Lets the island write to the same log as the Rust side. Sync on purpose: sync
/// commands run in the order the page sent them, so back-to-back lines keep their
/// order (an async command could land on any worker). The write is one append.
#[tauri::command]
fn log_line(message: String) {
    log::line(format!("ui  {message}"));
}

// ── MCP server (mcp_server) ───────────────────────────────────────────────────

/// The server's token: the one in the Credential Manager, or a new one saved there now. It never
/// leaves Rust but on the clipboard, by `mcp_copy_command`.
fn mcp_token() -> Result<String, String> {
    mcp_server::ensure_token(secrets::get(mcp_server::TOKEN_KEY), |t| secrets::set(mcp_server::TOKEN_KEY, t), mcp_server::new_token)
}

/// Starts the server on the saved port. A failure (the port is busy, no token) is kept for
/// Settings to show.
fn mcp_start(app: &AppHandle) -> Result<(), Message> {
    let port = app.state::<Shared>().settings.lock().unwrap().mcp_port;
    let server = app.state::<mcp_server::McpServer>();
    let token = mcp_token().map_err(|err| {
        log::line(format!("mcp: no token: {err}"));
        server.failed(Message::new("msg.mcp.noToken", "The token could not be read or saved in the Credential Manager."))
    })?;
    let backend = mcp_server::AppBackend::new(
        app.clone(),
        app.state::<Arc<Pending>>().inner().clone(),
        app.state::<observe::Handle>().inner().clone(),
    );
    server.start(port, token, backend)
}

/// What Settings shows of the server.
#[tauri::command]
fn mcp_status(shared: State<Shared>, server: State<mcp_server::McpServer>) -> mcp_server::Status {
    let settings = shared.settings.lock().unwrap();
    server.status(settings.mcp_enabled, settings.mcp_port)
}

/// Settings' switch: saves it, tells the other window, starts or stops the server.
#[tauri::command]
fn mcp_set_enabled(app: AppHandle, shared: State<Shared>, server: State<mcp_server::McpServer>, enabled: bool) -> mcp_server::Status {
    let settings = {
        let mut current = shared.settings.lock().unwrap();
        current.mcp_enabled = enabled;
        current.clone()
    };
    if let Err(err) = save_latest(&shared.settings, settings::save) {
        log::line(format!("settings: could not save settings.json ({:?})", err.kind()));
    }
    let latest = shared.settings.lock().unwrap().clone();
    let _ = app.emit("settings-changed", latest);
    if enabled {
        // A failure is in the status that comes back.
        let _ = mcp_start(&app);
    } else {
        server.stop();
    }
    server.status(enabled, settings.mcp_port)
}

/// What a copy to the clipboard did, as the row menu's does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum McpCopy {
    Done,
    /// Linux and macOS: the page puts `text` on the clipboard itself.
    #[cfg_attr(windows, allow(dead_code))]
    Copy { text: String },
    Failed,
}

/// «Copy command»: the line that adds the server to the user's own Claude Code or Codex, with
/// the token in it. It goes to the clipboard from here (kept out of the clipboard history on
/// Windows) and is never logged. Only while the server is switched on.
#[tauri::command]
fn mcp_copy_command(shared: State<Shared>, cli: String) -> McpCopy {
    let (enabled, port) = {
        let settings = shared.settings.lock().unwrap();
        (settings.mcp_enabled, settings.mcp_port)
    };
    let Some(cli) = mcp_server::Cli::parse(&cli).filter(|_| enabled) else { return McpCopy::Failed };
    let Ok(token) = mcp_token() else { return McpCopy::Failed };
    let text = mcp_server::install_command(cli, mcp_server::Shell::native(), &mcp_server::url_for(port), &token);
    #[cfg(windows)]
    {
        match launcher::win::set_clipboard_text(&text) {
            Ok(()) => McpCopy::Done,
            Err(_) => {
                log::line("mcp: the clipboard is busy");
                McpCopy::Failed
            }
        }
    }
    #[cfg(not(windows))]
    {
        McpCopy::Copy { text }
    }
}

/// «New token»: every agent that holds the old one is refused from now on.
#[tauri::command]
fn mcp_new_token(shared: State<Shared>, server: State<mcp_server::McpServer>) -> Result<mcp_server::Status, String> {
    let token = mcp_server::new_token()?;
    secrets::set(mcp_server::TOKEN_KEY, &token)?;
    server.set_token(token);
    log::line("mcp: a new token was made");
    let settings = shared.settings.lock().unwrap();
    Ok(server.status(settings.mcp_enabled, settings.mcp_port))
}

// ── Settings window ───────────────────────────────────────────────────────────

/// WebView2 allows exactly one browser environment per app, and its options are
/// fixed by whichever webview is created first. Every window must therefore ask
/// for the *same* arguments as the island (see `additionalBrowserArgs` in
/// tauri.conf.json) — a mismatch makes the second window come up blank, with no
/// error anywhere.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

/// In a dev build the pages are served by Vite, so the second window needs the
/// absolute dev URL; a bundled build resolves it inside the app bundle.
fn settings_page_url(app: &AppHandle) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(mut base) = app.config().build.dev_url.clone() {
        base.set_path("/settings.html");
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App("settings.html".into())
}

/// The settings window is created hidden at launch and only ever shown and
/// hidden afterwards. A WebView2 window created later — on the main thread or
/// not — silently comes up blank in this app, so the window that works is the
/// one that exists before the island's webview does.
fn create_settings_window(app: &AppHandle) {
    let url = settings_page_url(app);
    match WebviewWindowBuilder::new(app, "settings", url)
        .additional_browser_args(BROWSER_ARGS)
        .title("Settings — Mayak")
        .inner_size(680.0, 680.0)
        .min_inner_size(560.0, 480.0)
        .resizable(true)
        .visible(false)
        // Tauri focuses a new webview by default. This one is hidden, and its
        // first DOM focus builds the Claude Code section (D58), so that focus
        // must come from show_settings_window, never from start-up.
        .focused(false)
        .center()
        .build()
    {
        Ok(win) => {
            // Closing it must only hide it, or it could never be reopened.
            let hidden = win.clone();
            win.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = hidden.hide();
                }
            });
        }
        Err(err) => log::line(format!("settings window failed: {err}")),
    }
}

pub fn show_settings_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("settings") else {
        log::line("settings window missing");
        return;
    };
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
}

#[tauri::command]
fn open_settings_window(app: AppHandle) {
    show_settings_window(&app);
}

pub fn run() {
    platform::prepare_environment();
    let loaded = settings::load();
    let gate = Arc::new(PollGate::new());
    let pending = Arc::new(Pending::default());
    let codex_answers = Arc::new(CodexAnswers::default());

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            let _ = app.emit_to(island::WINDOW_LABEL, "tray", "open".to_string());
        }))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .manage(Shared {
            settings: Mutex::new(loaded.clone()),
            gate: gate.clone(),
            hotkey_return: Mutex::new(None),
        })
        .manage(pending.clone())
        .manage(codex_answers.clone())
        .manage(Chat::default())
        .manage(mcp_server::McpServer::default())
        .manage(hook_ctx())
        .invoke_handler(tauri::generate_handler![
            boot,
            save_settings,
            set_collapsed,
            sessions_snapshot,
            session_ack,
            session_open,
            session_menu,
            session_menu_run,
            session_file_open,
            git_states,
            session_resume_copy,
            recent_sessions,
            recent_resume,
            recent_copy,
            usage_refresh,
            balances_refresh,
            stats_query,
            launcher_targets,
            launcher_open,
            apps_list,
            apps_builtin,
            app_open,
            app_pick,
            app_icon,
            set_island_rect,
            focus_window,
            hotkey_take_focus,
            hotkey_give_focus,
            reposition,
            open_url,
            open_in_vscode,
            quit_app,
            hooks_status,
            hooks_preview,
            hooks_apply,
            codex_answers_preview,
            codex_answers_apply,
            approval_decision,
            question_answer,
            approval_ack,
            approval_decline,
            approvals_release_all,
            log_line,
            chat_send,
            chat_models,
            chat_reset,
            agent_chat_agents,
            agent_chat_open,
            agent_chat_list,
            agent_chat_get,
            agent_chat_set,
            agent_chat_send,
            agent_chat_permission,
            agent_chat_cancel,
            agent_chat_close,
            ingest_bytes,
            secret_present,
            secret_set,
            secret_clear,
            oauth_start,
            oauth_cancel,
            oauth_disconnect,
            oauth_status,
            gitlab_check_url,
            open_gitlab,
            open_gitlab_token_page,
            github_info,
            open_github,
            open_github_install,
            refresh_integration,
            open_n8n,
            mcp_status,
            mcp_set_enabled,
            mcp_copy_command,
            mcp_new_token,
            open_settings_window,
            autostart::autostart_status,
            set_paused,
            set_focus_check,
            tray_texts,
            hotkey::hotkey_status,
            hotkey::hotkey_next_status,
            hotkey::hotkey_pause,
            media::media_state,
            media::media_toggle,
            media::media_next,
            media::media_prev,
            media::media_seek,
            media::media_shuffle,
            media::media_repeat,
            sounds::custom_sounds,
            sounds::custom_sound,
            sounds::reveal_sounds_folder,
            sounds::reload_sounds,
            island::strip_visible,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            log::line(format!("--- Mayak {} started ---", env!("CARGO_PKG_VERSION")));
            // An accessory app, as the Mac app is: no Dock icon, not in ⌘-Tab; the island and
            // the menu bar item are the whole app.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            // Launch at login follows Windows (Task Manager or Settings → Apps → Startup).
            autostart::startup(&handle);
            hooks::ensure_hook_exe(&handle);
            // The observer is managed before any call that creates a webview: the
            // island's page is already navigating (tauri builds it from the config
            // before this closure) and WebView2 creation pumps messages, so its first
            // `sessions_snapshot` can run while the next lines are still executing.
            let paths = app_paths();
            let observe = observe::start(
                observe::ObserveConfig {
                    claude_dir: paths.claude_dir.clone(),
                    claude_json: paths.claude_json.clone(),
                    codex_home: paths.codex_home.clone(),
                    appdata: paths.appdata.clone(),
                    localappdata: paths.localappdata.clone(),
                },
                Box::new(IslandPublisher { app: handle.clone() }),
            );
            // The island launches expanded (greeting), so the store is told it is visible (D6).
            observe.send(Signal::Visible(true));
            // Installer state the model needs from the start (spec §8): which
            // alert kinds are on, and whether Mayak's Codex hooks are in hooks.json.
            observe.send(observe::model::Signal::AlertPrefs {
                waiting: loaded.alert_waiting,
                finished: loaded.alert_finished,
                error: loaded.alert_error,
            });
            observe.send(observe::model::Signal::CodexHooksInstalled(codex_hooks_installed(
                &handle.state::<HookCtx>(),
            )));
            // Whether Codex permissions go to the island, before the pipe opens (spec §3).
            codex_answers.set(codex_answers_enabled(&handle.state::<HookCtx>()));
            handle.manage(observe.clone());
            handle.manage(ChatManager::new(Arc::new(AppChatHost { app: handle.clone() })));

            tray::build(&handle)?;
            // Before the island: see create_settings_window.
            create_settings_window(&handle);
            // The global shortcut, registered from the start (hotkey spec §3.1). A press before
            // the island's page listens is simply not seen.
            hotkey::apply(&handle, loaded.active_hotkey(), loaded.active_next_hotkey());
            // What is playing (night spec §B): its own thread, which requests the session manager
            // there. The island asks `media_state` when it builds the player.
            media::start(&handle, loaded.show_media);

            if let Some(win) = island::window(&handle) {
                platform::make_non_activating(&win);
                island::apply_geometry(&handle, &loaded.screen, false);
                let _ = win.show();
            }
            gate.collapsed.store(false, Ordering::Relaxed);
            // Nothing drawn yet, so nothing takes the mouse until the page
            // reports the island's shape.
            if !platform::CURSOR_POLL {
                island::refresh_click_through(&handle, &gate);
            }
            gate.set_active(true);
            island::spawn_cursor_poll(handle.clone(), gate.clone());
            #[cfg(target_os = "macos")]
            island::watch_notch(&handle, gate.clone());

            let sink = Arc::new(AppSink {
                app: handle.clone(),
                pending: handle.state::<Arc<Pending>>().inner().clone(),
                codex_answers: codex_answers.clone(),
                observe,
            });
            tauri::async_runtime::spawn(async move {
                if let Err(err) = pipe::serve(pipe::pipe_name(), sink).await {
                    log::line(format!("cannot open the relay pipe: {err}"));
                }
            });
            integrations::start(handle.clone());
            // Mayak's MCP server, only when the user switched it on; off, nothing listens.
            if loaded.mcp_enabled {
                let _ = mcp_start(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Mayak")
        .run(|app, event| {
            // The CLI chat's agents end with the app (Windows ends them with their job object anyway).
            if let tauri::RunEvent::Exit = event {
                if let Some(chat) = app.try_state::<ChatManager>() {
                    chat.shutdown();
                }
            }
        });
}

#[cfg(test)]
mod observe_wiring_tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;

    /// The environment `vars` describes, its values given as Windows fixture paths (`wp`) and the
    /// home under this platform's variable (`USERPROFILE`, `HOME`).
    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| {
                let k = if *k == "USERPROFILE" { platform::HOME_VAR } else { *k };
                (k.to_string(), if v.is_empty() { String::new() } else { crate::testutil::wp(v) })
            })
            .collect();
        move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| OsString::from(v))
    }

    fn wpath(windows_path: &str) -> PathBuf {
        PathBuf::from(crate::testutil::wp(windows_path))
    }

    #[test]
    fn app_paths_default_under_the_user_profile() {
        let paths = app_paths_from(env(&[
            ("USERPROFILE", r"C:\Users\dev"),
            ("APPDATA", r"C:\Users\dev\AppData\Roaming"),
            ("LOCALAPPDATA", r"C:\Users\dev\AppData\Local"),
            ("CLAUDE_CONFIG_DIR", ""),
        ]));
        assert_eq!(
            paths,
            AppPaths {
                claude_dir: wpath(r"C:\Users\dev\.claude"),
                claude_json: wpath(r"C:\Users\dev\.claude.json"),
                codex_home: wpath(r"C:\Users\dev\.codex"),
                gemini_dir: wpath(r"C:\Users\dev\.gemini"),
                appdata: wpath(r"C:\Users\dev\AppData\Roaming"),
                localappdata: wpath(r"C:\Users\dev\AppData\Local"),
            }
        );
    }

    #[test]
    fn a_codex_card_names_the_session_the_observer_tracks() {
        let payload = serde_json::json!({ "agent": "codex", "session_id": "019a-thread" });
        assert_eq!(codex_session_key(&payload), Some(observe::keys::codex_key("019a-thread")));
        assert_eq!(codex_session_key(&serde_json::json!({ "session_id": "" })), None);
        assert_eq!(codex_session_key(&serde_json::json!({ "session_id": 7 })), None);
        assert_eq!(codex_session_key(&serde_json::json!({})), None);
    }

    #[test]
    fn app_paths_follow_claude_config_dir_and_codex_home() {
        let paths = app_paths_from(env(&[
            ("USERPROFILE", r"C:\Users\dev"),
            ("CLAUDE_CONFIG_DIR", r"D:\cfg\claude"),
            ("CODEX_HOME", r"D:\cfg\codex"),
        ]));
        assert_eq!(paths.claude_dir, wpath(r"D:\cfg\claude"));
        assert_eq!(paths.claude_json, wpath(r"D:\cfg\claude\.claude.json"));
        assert_eq!(paths.codex_home, wpath(r"D:\cfg\codex"));
    }
}

#[cfg(test)]
mod session_command_tests {
    use super::*;
    use observe::model::{PhaseView, ProjectView, SessionView, Source, Store};

    fn session(key: &str, host: HostKind) -> SessionView {
        SessionView {
            key: key.into(),
            agent: Source::Claude,
            host,
            title: None,
            phase: PhaseView::Idle,
            since: 0,
            activity: None,
            detail: None,
            outcome: None,
            attention: None,
            acked: true,
            silent: None,
            context: None,
            model: None,
            compacted: None,
            turn: None,
            agent_pid: None,
            host_session_id: None,
            logical_id: None,
            cwd: None,
            last_reply: None,
        }
    }

    #[test]
    fn session_target_finds_host_and_project_root() {
        let mut snapshot = Store::new().snapshot(0);
        snapshot.projects.push(ProjectView {
            key: crate::testutil::wp(r"c:\work\app"),
            name: "app".into(),
            root: crate::testutil::wp(r"C:\work\app"),
            sessions: vec![session("claude:1:2", HostKind::Cursor), session("codex:t", HostKind::CodexDesktop)],
        });
        assert_eq!(session_target(&snapshot, "codex:t"), Some((HostKind::CodexDesktop, crate::testutil::wp(r"C:\work\app"))));
        assert_eq!(session_target(&snapshot, "claude:9:9"), None);
    }

    #[test]
    fn session_agent_pid_finds_the_pid_of_a_session() {
        let mut with_pid = session("claude:1:2", HostKind::Cursor);
        with_pid.agent_pid = Some(4242);
        let mut snapshot = Store::new().snapshot(0);
        snapshot.projects.push(ProjectView {
            key: r"c:\work\app".into(),
            name: "app".into(),
            root: r"C:\work\app".into(),
            sessions: vec![with_pid, session("codex:t", HostKind::CodexDesktop)],
        });
        assert_eq!(session_agent_pid(&snapshot, "claude:1:2"), Some(4242));
        assert_eq!(session_agent_pid(&snapshot, "codex:t"), None);
        assert_eq!(session_agent_pid(&snapshot, "nope"), None);
    }

    #[test]
    fn only_a_claude_desktop_session_gives_its_chat_id() {
        let mut desktop = session("claude:1:2", HostKind::ClaudeDesktop);
        desktop.host_session_id = Some("local_abc".into());
        let mut vscode = session("claude:3:4", HostKind::VsCode);
        vscode.host_session_id = Some("local_def".into());
        let mut snapshot = Store::new().snapshot(0);
        snapshot.projects.push(ProjectView {
            key: r"c:\work\app".into(),
            name: "app".into(),
            root: r"C:\work\app".into(),
            sessions: vec![desktop, vscode, session("claude:5:6", HostKind::ClaudeDesktop)],
        });
        assert_eq!(session_desktop_chat(&snapshot, "claude:1:2").as_deref(), Some("local_abc"));
        assert_eq!(session_desktop_chat(&snapshot, "claude:3:4"), None);
        assert_eq!(session_desktop_chat(&snapshot, "claude:5:6"), None);
        assert_eq!(session_desktop_chat(&snapshot, "nope"), None);
    }

    #[test]
    fn an_existing_root_is_the_folder_of_every_host() {
        for host in [HostKind::ClaudeDesktop, HostKind::CodexDesktop, HostKind::Cursor, HostKind::VsCode, HostKind::Unknown] {
            let folder = session_folder(host, r"C:\work\app", false, |_| true, |_| panic!("the root is there"));
            assert_eq!(folder, Folder::There(r"C:\work\app".to_string()));
        }
    }

    #[test]
    fn a_missing_root_of_another_host_is_refused_and_never_mapped() {
        for host in [HostKind::CodexDesktop, HostKind::Cursor, HostKind::VsCode, HostKind::Unknown] {
            for has_chat in [false, true] {
                let folder = session_folder(host, r"C:\gone", has_chat, |_| false, |_| panic!("only Claude Desktop is mapped"));
                assert_eq!(folder, Folder::Refused, "{host:?}");
            }
        }
    }

    #[test]
    fn a_missing_claude_desktop_root_takes_its_place_in_the_package() {
        let folder = session_folder(
            HostKind::ClaudeDesktop,
            r"C:\Users\HP\AppData\Roaming\Claude\scratch",
            false,
            |_| false,
            |p| {
                assert_eq!(p, std::path::Path::new(r"C:\Users\HP\AppData\Roaming\Claude\scratch"));
                Some(std::path::PathBuf::from(r"C:\pkg\scratch"))
            },
        );
        assert_eq!(folder, Folder::There(r"C:\pkg\scratch".to_string()));
    }

    #[test]
    fn a_claude_desktop_root_nowhere_on_disk_goes_on_with_its_chat_only() {
        let gone = |has_chat, mapped| {
            session_folder(HostKind::ClaudeDesktop, r"C:\gone", has_chat, |_| false, |_| mapped)
        };
        assert_eq!(gone(true, None), Folder::Absent);
        assert_eq!(gone(false, None), Folder::Refused);
        assert_eq!(gone(false, Some(std::path::PathBuf::from(r"C:\pkg"))), Folder::There(r"C:\pkg".to_string()));
    }

    #[test]
    fn a_relative_project_root_is_never_a_target() {
        // bef6558: `cursor` / `code` would read a root like `--x` as an option.
        for root in ["--x", r"work\app"] {
            let mut snapshot = Store::new().snapshot(0);
            snapshot.projects.push(ProjectView {
                key: root.to_lowercase(),
                name: "app".into(),
                root: root.into(),
                sessions: vec![session("claude:1:2", HostKind::VsCode)],
            });
            assert_eq!(session_target(&snapshot, "claude:1:2"), None, "{root}");
        }
    }

    #[test]
    fn the_row_menu_offers_what_the_row_and_the_disk_have() {
        use session_menu::MenuItem::*;
        let tmp = testutil::TempDir::new("menu");
        let here = tmp.path.to_string_lossy().into_owned();
        let gone = tmp.path.join("gone").to_string_lossy().into_owned();
        let mut full = session("claude:1:2", HostKind::VsCode);
        full.logical_id = Some("0f8b1d2e-0000-4000-8000-000000000001".into());
        full.last_reply = Some("Done.".into());
        let bare = session("claude:3:4", HostKind::VsCode);
        let mut snapshot = Store::new().snapshot(0);
        for (root, s) in [(here.clone(), full), (gone.clone(), bare.clone()), (r"work\app".to_string(), session("claude:5:6", HostKind::VsCode))] {
            snapshot.projects.push(ProjectView { key: root.to_lowercase(), name: "app".into(), root, sessions: vec![s] });
        }
        let items = |key: &str| menu_row(&snapshot, key).map(|row| session_menu::items(&row.facts()));

        assert_eq!(items("claude:1:2"), Some(vec![CopyReply, CopyPath, CopyResume, OpenFolder]));
        let row = menu_row(&snapshot, "claude:1:2").unwrap();
        assert_eq!(session_menu::copy_text(CopyPath, &row.facts()), Some(here));
        // A folder gone from the disk: its path is still copied, it is not opened; no id, no command.
        assert_eq!(items("claude:3:4"), Some(vec![CopyPath]));
        // A relative root is no path.
        assert_eq!(items("claude:5:6"), Some(vec![]));
        // Unknown keys and pure silent rows have no menu.
        assert!(menu_row(&snapshot, "claude:9:9").is_none());
        snapshot.projects[1].sessions.push(SessionView { key: "silent:x".into(), ..bare });
        assert!(menu_row(&snapshot, "silent:x").is_none());
    }
}

#[cfg(test)]
mod settings_save_tests {
    use super::*;

    fn with_mcp(enabled: bool, port: u16) -> Settings {
        Settings { mcp_enabled: enabled, mcp_port: port, ..Settings::default() }
    }

    #[test]
    fn a_pages_copy_keeps_the_mcp_switch_the_app_holds() {
        // The page's copy predates the switch: on, port 1; the app holds off, port 2.
        let current = with_mcp(false, 2);
        let mut page = Settings { sound_volume: 0.5, ..with_mcp(true, 1) };
        keep_mcp_switch(&current, &mut page);
        assert_eq!((page.mcp_enabled, page.mcp_port), (false, 2));
        // Everything else of the page's is kept.
        assert_eq!(page.sound_volume, 0.5);
    }

    #[test]
    fn the_switch_is_kept_and_stored_in_one_step_so_a_switch_in_between_is_never_undone() {
        // `save_settings` does both under one lock; hammering it from a page that always holds a
        // stale "off" while the switch is turned on must leave "on".
        let held = std::sync::Arc::new(Mutex::new(with_mcp(false, 7)));
        let pages: Vec<_> = (0..4)
            .map(|_| {
                let held = held.clone();
                std::thread::spawn(move || {
                    for _ in 0..2000 {
                        let mut page = with_mcp(false, 7);
                        let mut current = held.lock().unwrap();
                        keep_mcp_switch(&current, &mut page);
                        *current = page;
                    }
                })
            })
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(5));
        held.lock().unwrap().mcp_enabled = true;
        for page in pages {
            page.join().unwrap();
        }
        assert!(held.lock().unwrap().mcp_enabled);
    }

    #[test]
    fn a_slow_writer_cannot_put_an_older_file_over_a_newer_one() {
        let held = Mutex::new(with_mcp(false, 7));
        let written = Mutex::new(Vec::<bool>::new());
        let write = |s: &Settings| {
            written.lock().unwrap().push(s.mcp_enabled);
            Ok(())
        };
        // One writer has stored its change and not yet written; the switch is turned on and
        // written; then the first one writes.
        held.lock().unwrap().sound_volume = 0.3;
        held.lock().unwrap().mcp_enabled = true;
        save_latest(&held, write).unwrap();
        save_latest(&held, write).unwrap();
        // Both files say on, because each write holds what the app holds.
        assert_eq!(*written.lock().unwrap(), [true, true]);
    }

    #[test]
    fn a_failed_write_is_the_callers_to_report() {
        let held = Mutex::new(Settings::default());
        let err = save_latest(&held, |_| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        // And the next writer still gets its turn.
        assert!(save_latest(&held, |_| Ok(())).is_ok());
    }
}
