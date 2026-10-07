// Mayak for Windows — app wiring and the commands the island calls.

mod claude;
mod codex_hooks;
mod files;
mod focus;
mod hookfile;
mod hooks;
mod hotkey;
#[allow(dead_code)]
#[path = "../../ident.rs"]
mod ident;
mod integrations;
mod island;
mod launcher;
mod log;
mod observe;
mod pipe;
mod platform;
mod secrets;
mod settings;
#[cfg(windows)]
mod strip;
#[cfg(test)]
mod testutil;
mod tray;

use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{ManagerExt, MacosLauncher};

use claude::{Chat, ChatContext, ChatReply};
use files::DroppedFile;
use hookfile::Target;
use hooks::{HookCtx, HookPreview, HookStatus};
use island::{PollGate, ScreenInfo};
use pipe::Pending;
use observe::model::{Alert, HostKind, SessionsSnapshot, Signal};
use settings::Settings;

pub struct Shared {
    pub settings: Mutex<Settings>,
    pub gate: Arc<PollGate>,
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
    /// True where a hidden island is the visible strip of `strip.rs` (Windows).
    native_strip: bool,
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
        native_strip: island::NATIVE_STRIP,
    }
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    shared: State<Shared>,
    observe: State<observe::Handle>,
    settings: Settings,
) {
    let (screen_changed, autostart_changed, hotkey_changed) = {
        let mut current = shared.settings.lock().unwrap();
        let screen_changed = current.screen != settings.screen;
        let autostart_changed = current.autostart != settings.autostart;
        let hotkey_changed = current.active_hotkey() != settings.active_hotkey();
        *current = settings.clone();
        (screen_changed, autostart_changed, hotkey_changed)
    };
    if let Err(err) = settings::save(&settings) {
        eprintln!("[mayak] could not save settings: {err}");
    }
    if autostart_changed {
        let manager = app.autolaunch();
        let result = if settings.autostart { manager.enable() } else { manager.disable() };
        if let Err(err) = result {
            eprintln!("[mayak] autostart: {err}");
        }
    }
    if screen_changed {
        let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
        place_island(&app, &settings.screen, collapsed);
    }
    // Only a real change reaches the hotkey thread: the island saves the sound and the
    // auto-close interval through here too (hotkey spec §3.1).
    if hotkey_changed {
        hotkey::apply(&app, settings.active_hotkey());
    }
    // Alert kinds the model may raise from now on (spec §8).
    observe.send(observe::model::Signal::AlertPrefs {
        waiting: settings.alert_waiting,
        finished: settings.alert_finished,
        error: settings.alert_error,
    });
    // Keep the other window in step (island ⇄ settings window).
    let _ = app.emit("settings-changed", settings);
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

// ── Sessions ──────────────────────────────────────────────────────────────────

#[tauri::command]
fn sessions_snapshot(observe: State<observe::Handle>) -> SessionsSnapshot {
    observe.snapshot()
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
#[tauri::command]
fn session_open(observe: State<observe::Handle>, key: String) -> bool {
    let snapshot = observe.snapshot();
    let Some((host, root)) = session_target(&snapshot, &key) else { return false };
    if !std::path::Path::new(&root).is_dir() {
        return false;
    }
    let agent_pid = session_agent_pid(&snapshot, &key);
    std::thread::spawn(move || {
        focus::switch_to(host, &root, agent_pid);
    });
    true
}

/// The home view became active: read Claude Code's cached plan limits again (usage-home spec §2.1).
/// Returns at once; the read runs on claude-io and its result comes with the next snapshot.
#[tauri::command]
fn usage_refresh(observe: State<observe::Handle>) {
    observe.claude_usage();
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

/// Pid of the agent process of a session in a snapshot.
fn session_agent_pid(snapshot: &SessionsSnapshot, key: &str) -> Option<u32> {
    snapshot
        .projects
        .iter()
        .find_map(|project| project.sessions.iter().find(|s| s.key == key).and_then(|s| s.agent_pid))
}

// ── Hook installers (spec §8) ─────────────────────────────────────────────────

/// What the installer works on. `app_paths()` is the only reader of
/// CLAUDE_CONFIG_DIR / CODEX_HOME / USERPROFILE for it.
fn hook_ctx() -> HookCtx {
    let paths = app_paths();
    HookCtx {
        claude_dir: paths.claude_dir,
        codex_home: paths.codex_home,
        hook_exe: settings::hook_exe_path(),
        plans: hookfile::Plans::default(),
    }
}

/// A fresh read of ~/.codex/hooks.json, for `Signal::CodexHooksInstalled`.
fn codex_hooks_installed(ctx: &HookCtx) -> bool {
    let our = codex_hooks::our_command(&ctx.hook_exe).ok();
    codex_hooks::installed_at(&ctx.codex_home, our.as_deref())
}

/// Read-only status of one target's config. `target` defaults to "claude".
#[tauri::command]
fn hooks_status(ctx: State<HookCtx>, target: Option<String>) -> Result<HookStatus, String> {
    let target = Target::parse(target.as_deref())?;
    // Only the target name: shows when a tool config was read (spec §8, D58).
    log::line(format!("hooks_status target={}", target.as_str()));
    Ok(hooks::status(&ctx, target))
}

/// The diff the user has to look at before anything is written. The plan
/// behind it stays in Rust; apply writes exactly those bytes or nothing.
#[tauri::command]
fn hooks_preview(
    ctx: State<HookCtx>,
    target: Option<String>,
    install: bool,
) -> Result<HookPreview, String> {
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
    target: Option<String>,
    install: bool,
    fingerprint: String,
) -> Result<String, String> {
    let target = Target::parse(target.as_deref())?;
    let result = hooks::apply(&ctx, target, install, &fingerprint);
    if target == Target::Codex {
        // After every Codex apply (spec §8), on success and failure alike, also
        // when no plan matched: read back from disk, never from `install`.
        observe.send(observe::model::Signal::CodexHooksInstalled(codex_hooks_installed(&ctx)));
    }
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
    }

    fn silent_wake(&self) {
        if let Err(err) = self.app.emit_to(island::WINDOW_LABEL, "session-silent", ()) {
            log::line(format!("session-silent emit failed: {err}"));
        }
    }
}

/// The router's view of the app (spec §5.3): Codex events feed the observer, Claude approvals
/// get their session key and project name from it.
struct AppSink {
    app: AppHandle,
    pending: Arc<Pending>,
    observe: observe::Handle,
}

impl pipe::RouterSink for AppSink {
    fn pending(&self) -> &Pending {
        &self.pending
    }

    /// Never blocks: the channel is unbounded.
    fn observe(&self, payload: serde_json::Value) {
        self.observe.codex_hook(payload);
    }

    fn approval_context(&self, payload: &serde_json::Value) -> pipe::ApprovalContext {
        observe::approval_context(payload)
    }

    fn show_approval(&self, payload: serde_json::Value) {
        if let Err(err) = self.app.emit_to(island::WINDOW_LABEL, "hook", payload) {
            log::line(format!("hook emit failed: {err}"));
        }
    }
}

#[tauri::command]
fn approval_decision(pending: State<Arc<Pending>>, request_id: String, decision: String) {
    pipe::answer(&pending, &request_id, &decision);
}

/// The island has the card on screen, so the long wait for a human may begin.
/// Until this arrives the relay only waits a few hundred milliseconds, which is
/// what stops a paused or unresponsive island from freezing Claude Code.
#[tauri::command]
fn approval_ack(pending: State<Arc<Pending>>, request_id: String) {
    pipe::acknowledge(&pending, &request_id);
}

/// Nobody can act on this request — the island is paused, or another card is
/// already up. Claude Code falls back to asking in the terminal immediately.
#[tauri::command]
fn approval_decline(pending: State<Arc<Pending>>, request_id: String) {
    pipe::decline(&pending, &request_id);
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

/// Copies a dropped file into the inbox and reports its name back.
#[tauri::command]
fn ingest_file(path: String) -> Result<DroppedFile, String> {
    files::ingest(&path)
}

/// A file dropped on the island page (an HTML5 drop: WebView2 hands the page the contents, not
/// a path). The bytes are the raw request body, the name is URI-encoded in `x-file-name`. Async,
/// so writing a large file never holds up the main thread.
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
    secrets::present(&key)
}

#[tauri::command]
fn secret_set(key: String, value: String) -> Result<(), String> {
    secrets::set(&key, &value)
}

#[tauri::command]
fn secret_clear(key: String) -> Result<(), String> {
    secrets::clear(&key)
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

/// Lets the island write to the same log as the Rust side.
#[tauri::command]
fn log_line(message: String) {
    log::line(format!("ui  {message}"));
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
        .inner_size(560.0, 680.0)
        .min_inner_size(460.0, 480.0)
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

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            let _ = app.emit_to(island::WINDOW_LABEL, "tray", "open".to_string());
        }))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .manage(Shared {
            settings: Mutex::new(loaded.clone()),
            gate: gate.clone(),
        })
        .manage(pending.clone())
        .manage(Chat::default())
        .manage(hook_ctx())
        .invoke_handler(tauri::generate_handler![
            boot,
            save_settings,
            set_collapsed,
            sessions_snapshot,
            session_ack,
            session_open,
            usage_refresh,
            launcher_targets,
            launcher_open,
            set_island_rect,
            focus_window,
            reposition,
            open_url,
            open_in_vscode,
            quit_app,
            hooks_status,
            hooks_preview,
            hooks_apply,
            approval_decision,
            approval_ack,
            approval_decline,
            log_line,
            chat_send,
            chat_models,
            chat_reset,
            ingest_file,
            ingest_bytes,
            secret_present,
            secret_set,
            secret_clear,
            refresh_integration,
            open_n8n,
            open_settings_window,
            set_paused,
            hotkey::hotkey_status,
            hotkey::hotkey_pause,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            log::line(format!("--- Mayak {} started ---", env!("CARGO_PKG_VERSION")));
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
            handle.manage(observe.clone());

            tray::build(&handle)?;
            // Before the island: see create_settings_window.
            create_settings_window(&handle);
            // The global shortcut, registered from the start (hotkey spec §3.1). A press before
            // the island's page listens is simply not seen.
            hotkey::apply(&handle, loaded.active_hotkey());

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

            let sink = Arc::new(AppSink {
                app: handle.clone(),
                pending: handle.state::<Arc<Pending>>().inner().clone(),
                observe,
            });
            tauri::async_runtime::spawn(async move {
                if let Err(err) = pipe::serve(pipe::pipe_name(), sink).await {
                    log::line(format!("cannot open the relay pipe: {err}"));
                }
            });
            integrations::start(handle.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Mayak");
}

#[cfg(test)]
mod observe_wiring_tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn env(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| OsString::from(v))
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
                claude_dir: PathBuf::from(r"C:\Users\dev\.claude"),
                claude_json: PathBuf::from(r"C:\Users\dev\.claude.json"),
                codex_home: PathBuf::from(r"C:\Users\dev\.codex"),
                appdata: PathBuf::from(r"C:\Users\dev\AppData\Roaming"),
                localappdata: PathBuf::from(r"C:\Users\dev\AppData\Local"),
            }
        );
    }

    #[test]
    fn app_paths_follow_claude_config_dir_and_codex_home() {
        let paths = app_paths_from(env(&[
            ("USERPROFILE", r"C:\Users\dev"),
            ("CLAUDE_CONFIG_DIR", r"D:\cfg\claude"),
            ("CODEX_HOME", r"D:\cfg\codex"),
        ]));
        assert_eq!(paths.claude_dir, PathBuf::from(r"D:\cfg\claude"));
        assert_eq!(paths.claude_json, PathBuf::from(r"D:\cfg\claude\.claude.json"));
        assert_eq!(paths.codex_home, PathBuf::from(r"D:\cfg\codex"));
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
            agent_pid: None,
        }
    }

    #[test]
    fn session_target_finds_host_and_project_root() {
        let mut snapshot = Store::new().snapshot(0);
        snapshot.projects.push(ProjectView {
            key: r"c:\work\app".into(),
            name: "app".into(),
            root: r"C:\work\app".into(),
            sessions: vec![session("claude:1:2", HostKind::Cursor), session("codex:t", HostKind::CodexDesktop)],
        });
        assert_eq!(session_target(&snapshot, "codex:t"), Some((HostKind::CodexDesktop, r"C:\work\app".to_string())));
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
}
