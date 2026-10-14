//! Installed IDEs and agent apps the Apps tab offers (spec docs/superpowers/specs/2026-10-07-mayak-batch-design.md §D),
//! and the apps the user added, in the user's order (own-apps, ideas §30).
//! `found` only looks at files, URL protocols and the Codex package, and takes its folders as
//! arguments; `tiles` merges them with the settings; `open` is Windows only: elsewhere no app is
//! found and `open` answers «not supported». A user app's path is user data: never logged.

#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::settings::{CustomApp, Settings};

/// One tile of the Apps tab. Wire names are camelCase.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppEntry {
    pub id: &'static str,
    pub name: &'static str,
}

/// One tile of the Apps tab as the page gets it (`apps_list`). Wire names are camelCase.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppTile {
    pub id: String,
    pub name: String,
    /// Added by the user: it can be renamed and removed; a built-in one can be hidden.
    pub custom: bool,
    /// A user app whose file is no longer there: shown dimmed, it does not open.
    pub missing: bool,
    /// A user app's exe file name, without its folder (for its styled icon); `None` for a built-in one.
    pub exe: Option<String>,
}

/// What every user app id starts with: it never meets a built-in id.
pub const CUSTOM_PREFIX: &str = "user-";

/// The folder an exe's relative path starts from.
#[derive(Clone, Copy)]
enum Root {
    LocalAppData,
    ProgramFiles,
}

/// How an app is found and opened.
enum Kind {
    /// The first of `paths` (components under its root) that is a file; `exe` is the image name whose window is raised.
    Exe { paths: &'static [(Root, &'static [&'static str])], exe: &'static str },
    /// A registered URL protocol, opened as `<scheme>://`.
    Protocol(&'static str),
    /// The AppX package `OpenAI.Codex_*`.
    Appx,
}

/// Every app, in the order of the tab.
const APPS: [(&str, &str, Kind); 7] = [
    ("cursor", "Cursor", Kind::Exe { paths: &[(Root::LocalAppData, &["Programs", "cursor", "Cursor.exe"])], exe: "Cursor.exe" }),
    (
        "vscode",
        "VS Code",
        Kind::Exe {
            paths: &[
                (Root::LocalAppData, &["Programs", "Microsoft VS Code", "Code.exe"]),
                (Root::ProgramFiles, &["Microsoft VS Code", "Code.exe"]),
            ],
            exe: "Code.exe",
        },
    ),
    (
        "windsurf",
        "Windsurf",
        Kind::Exe { paths: &[(Root::LocalAppData, &["Programs", "Windsurf", "Windsurf.exe"])], exe: "Windsurf.exe" },
    ),
    ("zed", "Zed", Kind::Exe { paths: &[(Root::LocalAppData, &["Programs", "Zed", "Zed.exe"])], exe: "Zed.exe" }),
    (
        "antigravity",
        "Antigravity",
        Kind::Exe {
            paths: &[(Root::LocalAppData, &["Programs", "antigravity", "Antigravity.exe"])],
            exe: "Antigravity.exe",
        },
    ),
    ("claude", "Claude", Kind::Protocol("claude")),
    ("codex", "Codex", Kind::Appx),
];

/// The exe of the app `id` that exists in the given folders; `None` for an app without an exe.
pub fn exe_for(id: &str, localappdata: &Path, program_files: &Path) -> Option<PathBuf> {
    let (_, _, Kind::Exe { paths, .. }) = APPS.iter().find(|(app, _, _)| *app == id)? else { return None };
    paths
        .iter()
        .map(|(root, parts)| {
            let base = match root {
                Root::LocalAppData => localappdata,
                Root::ProgramFiles => program_files,
            };
            parts.iter().fold(base.to_path_buf(), |path, part| path.join(part))
        })
        .find(|path| path.is_file())
}

/// The installed apps, in the order of `APPS`. `protocol` tells whether a URL scheme is
/// registered, `codex_family` is the Codex package folder if there is one.
pub fn found(
    localappdata: &Path,
    program_files: &Path,
    protocol: impl Fn(&str) -> bool,
    codex_family: Option<String>,
) -> Vec<AppEntry> {
    APPS.iter()
        .filter(|(id, _, kind)| match kind {
            Kind::Exe { .. } => exe_for(id, localappdata, program_files).is_some(),
            Kind::Protocol(scheme) => protocol(scheme),
            Kind::Appx => codex_family.is_some(),
        })
        .map(|(id, name, _)| AppEntry { id, name })
        .collect()
}

/// `id` is one of the built-in apps.
pub fn is_builtin(id: &str) -> bool {
    APPS.iter().any(|(app, _, _)| *app == id)
}

/// Every built-in app, installed or not, in the order of `APPS` (Settings lists the hidden ones).
pub fn builtins() -> Vec<AppEntry> {
    APPS.iter().map(|(id, name, _)| AppEntry { id, name }).collect()
}

/// `tiles` in the user's `order`: the ids it lists first, each once, then every other tile as it
/// comes (a new built-in app, a user app the order does not have yet). An id of `order` without a
/// tile (an app removed or not installed) is skipped.
pub fn ordered(tiles: Vec<AppTile>, order: &[String]) -> Vec<AppTile> {
    let mut rest: Vec<Option<AppTile>> = tiles.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(rest.len());
    for id in order {
        if let Some(slot) = rest.iter_mut().find(|t| t.as_ref().is_some_and(|t| &t.id == id)) {
            out.extend(slot.take());
        }
    }
    out.extend(rest.into_iter().flatten());
    out
}

/// The user apps that count: an id with `CUSTOM_PREFIX` and something after it, the first entry
/// of each id. Anything else in settings.json (an edited file) is skipped.
pub fn custom_apps(settings: &Settings) -> Vec<&CustomApp> {
    let mut seen: Vec<&str> = Vec::new();
    settings
        .custom_apps
        .iter()
        .filter(|app| {
            let id = app.id.as_str();
            let ok = id.len() > CUSTOM_PREFIX.len() && id.starts_with(CUSTOM_PREFIX) && !seen.contains(&id);
            if ok {
                seen.push(id);
            }
            ok
        })
        .collect()
}

/// A user app's path when it can be opened: an absolute path to an existing `.app` bundle.
#[cfg(target_os = "macos")]
pub fn custom_exe(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    let is_app = path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app"));
    (path.is_absolute() && is_app && path.is_dir()).then(|| path.to_path_buf())
}

/// A user app's path when it can be opened: an absolute path to an existing `.exe` file.
#[cfg(not(target_os = "macos"))]
pub fn custom_exe(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    let is_exe = path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
    (path.is_absolute() && is_exe && path.is_file()).then(|| path.to_path_buf())
}

/// The exe a picked file stands for: an `.exe` itself, a shortcut (`.lnk`) through `resolve_link`
/// to its target; `None` when that is not an exe that exists.
pub fn picked_exe(picked: &Path, resolve_link: impl Fn(&Path) -> Option<PathBuf>) -> Option<PathBuf> {
    let is_link = picked.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"));
    let target = if is_link { resolve_link(picked)? } else { picked.to_path_buf() };
    custom_exe(target.to_str()?)
}

/// Longest file description taken as a name: a longer one is a sentence, not a name.
const NAME_MAX: usize = 32;

/// The name a picked exe gets: its file description (version resource), else its file name
/// without `.exe`. The user can rename it.
pub fn default_name(description: Option<&str>, exe: &Path) -> String {
    match description.map(str::trim) {
        Some(d) if !d.is_empty() && d.chars().count() <= NAME_MAX => d.to_string(),
        _ => exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
    }
}

/// The Apps tab: the installed built-in apps the user did not hide and the user's apps, in the
/// user's order. `exists` tells whether a user app's path can be opened (`custom_exe`).
pub fn tiles(installed: Vec<AppEntry>, settings: &Settings, exists: impl Fn(&str) -> bool) -> Vec<AppTile> {
    let builtin = installed.into_iter().filter(|app| !settings.hidden_apps.iter().any(|h| h == app.id)).map(|app| {
        AppTile { id: app.id.to_string(), name: app.name.to_string(), custom: false, missing: false, exe: None }
    });
    let custom = custom_apps(settings).into_iter().map(|app| AppTile {
        id: app.id.clone(),
        name: app.name.clone(),
        custom: true,
        missing: !exists(&app.path),
        exe: Path::new(&app.path).file_name().map(|n| n.to_string_lossy().into_owned()),
    });
    ordered(builtin.chain(custom).collect(), &settings.app_order)
}

/// `%ProgramFiles%`; Windows always sets it, the default location is the fallback.
#[cfg(windows)]
fn program_files() -> PathBuf {
    std::env::var_os("ProgramFiles")
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from)
}

/// What the "+" tile's file dialog gave (`app_pick`). Wire: `{ kind: "picked", name, path }` and
/// `{ kind: "cancelled" | "notAnApp" | "failed" }`.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PickOutcome {
    /// An exe (a picked shortcut's target) and the name it gets.
    Picked { name: String, path: String },
    Cancelled,
    /// Not an exe, or a shortcut to something else.
    NotAnApp,
    Failed,
}

/// The "+" tile: the Open dialog, owned by the island window, then the exe the pick stands for
/// and its name. The log line has the outcome only.
#[cfg(windows)]
pub fn pick(owner: Option<isize>, title: &str, filter_name: &str) -> PickOutcome {
    use crate::apps_win::{file_description, pick_file, resolve_shortcut};

    let owner = owner.map(|raw| ::windows::Win32::Foundation::HWND(raw as *mut _));
    let outcome = match pick_file(owner, title, filter_name) {
        Ok(None) => PickOutcome::Cancelled,
        Err(err) => {
            // The error is a COM failure: it carries no path.
            crate::log::line(format!("apps: pick failed ({err})"));
            PickOutcome::Failed
        }
        Ok(Some(picked)) => match picked_exe(&picked, resolve_shortcut) {
            None => PickOutcome::NotAnApp,
            Some(exe) => PickOutcome::Picked {
                name: default_name(file_description(&exe).as_deref(), &exe),
                path: exe.to_string_lossy().into_owned(),
            },
        },
    };
    let kind = match outcome {
        PickOutcome::Picked { .. } => "picked",
        PickOutcome::Cancelled => "cancelled",
        PickOutcome::NotAnApp => "notAnApp",
        PickOutcome::Failed => "failed",
    };
    crate::log::line(format!("apps: pick outcome={kind}"));
    outcome
}

/// macOS: the "+" tile's Open dialog (`apps_mac`), then the app bundle it picked and its name
/// (the bundle's, which the user can rename). The log line has the outcome only.
#[cfg(target_os = "macos")]
pub fn pick(_owner: Option<isize>, title: &str, _filter_name: &str) -> PickOutcome {
    let outcome = match crate::apps_mac::pick_app(title) {
        Ok(None) => PickOutcome::Cancelled,
        Err(_) => PickOutcome::Failed,
        Ok(Some(picked)) => match custom_exe(&picked.to_string_lossy()) {
            None => PickOutcome::NotAnApp,
            Some(app) => PickOutcome::Picked { name: default_name(None, &app), path: app.to_string_lossy().into_owned() },
        },
    };
    crate::log::line(format!(
        "apps: pick outcome={}",
        match outcome {
            PickOutcome::Picked { .. } => "picked",
            PickOutcome::Cancelled => "cancelled",
            PickOutcome::NotAnApp => "notAnApp",
            PickOutcome::Failed => "failed",
        }
    ));
    outcome
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn pick(_owner: Option<isize>, _title: &str, _filter_name: &str) -> PickOutcome {
    PickOutcome::Failed
}

/// The icon of the user app `id`, from its exe: RGBA, `ICON_SIZE` square. `None` when it is not
/// there or has no icon of its own (the page draws a monogram then).
#[cfg(windows)]
pub fn custom_icon(id: &str, settings: &Settings) -> Option<Vec<u8>> {
    crate::apps_win::icon_rgba(&custom_path(id, settings)?, ICON_SIZE)
}

#[cfg(target_os = "macos")]
pub fn custom_icon(id: &str, settings: &Settings) -> Option<Vec<u8>> {
    crate::apps_mac::icon_rgba(&custom_path(id, settings)?, ICON_SIZE)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn custom_icon(_id: &str, _settings: &Settings) -> Option<Vec<u8>> {
    None
}

/// The side of a user app's icon in pixels: the 28 px tile at 150 % and more stays sharp.
pub const ICON_SIZE: u32 = 48;

/// The built-in apps installed now. Reads folders and the registry, so it runs off the UI thread.
#[cfg(windows)]
pub fn installed() -> Vec<AppEntry> {
    let local = crate::app_paths().localappdata;
    let family = crate::launcher::codex_family(&local.join("Packages"));
    found(&local, &program_files(), crate::launcher::win::protocol_registered, family)
}

/// macOS: the app bundles each built-in app may be, in the order they are looked for. Codex
/// ships as its own app or inside ChatGPT.
#[cfg(any(target_os = "macos", test))]
const MAC_BUNDLES: &[(&str, &[&str])] = &[
    ("cursor", &["Cursor.app"]),
    ("vscode", &["Visual Studio Code.app"]),
    ("windsurf", &["Windsurf.app"]),
    ("zed", &["Zed.app"]),
    ("antigravity", &["Antigravity.app"]),
    ("claude", &["Claude.app"]),
    ("codex", &["Codex.app", "ChatGPT.app"]),
];

/// The bundle of the built-in app `id` in the first of `dirs` that has one.
#[cfg(any(target_os = "macos", test))]
pub fn mac_bundle(id: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let (_, names) = MAC_BUNDLES.iter().find(|(app, _)| *app == id)?;
    names.iter().flat_map(|name| dirs.iter().map(move |dir| dir.join(name))).find(|path| path.is_dir())
}

/// Where apps are installed: /Applications, then ~/Applications.
#[cfg(target_os = "macos")]
fn app_dirs() -> Vec<PathBuf> {
    vec![PathBuf::from("/Applications"), crate::platform::home_dir().join("Applications")]
}

#[cfg(target_os = "macos")]
pub fn installed() -> Vec<AppEntry> {
    let dirs = app_dirs();
    APPS.iter().filter(|(id, _, _)| mac_bundle(id, &dirs).is_some()).map(|(id, name, _)| AppEntry { id, name }).collect()
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn installed() -> Vec<AppEntry> {
    Vec::new()
}

/// The tiles of the Apps tab now. Off the UI thread, as `installed`.
#[cfg(windows)]
pub fn list(settings: &Settings) -> Vec<AppTile> {
    tiles(installed(), settings, |path| custom_exe(path).is_some())
}

#[cfg(target_os = "macos")]
pub fn list(settings: &Settings) -> Vec<AppTile> {
    tiles(installed(), settings, |path| custom_exe(path).is_some())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn list(_settings: &Settings) -> Vec<AppTile> {
    Vec::new()
}

/// The image name whose window means the app `id` is open: a built-in app's from `APPS`, a user
/// app's exe file name. None when it is not known: an app opened by a URL protocol or as an AppX
/// package (Claude, Codex) names no exe here, and an unknown id has none.
pub fn window_exe(id: &str, settings: &Settings) -> Option<String> {
    if let Some((_, _, kind)) = APPS.iter().find(|(app, _, _)| *app == id) {
        return match kind {
            Kind::Exe { exe, .. } => Some((*exe).to_string()),
            Kind::Protocol(_) | Kind::Appx => None,
        };
    }
    let app = custom_apps(settings).into_iter().find(|app| app.id == id)?;
    Path::new(&app.path).file_name().map(|name| name.to_string_lossy().into_owned())
}

/// `id` opens now: an installed built-in app, or a user app whose exe is there.
pub fn can_open(id: &str, settings: &Settings) -> bool {
    if is_builtin(id) {
        installed().iter().any(|app| app.id == id)
    } else {
        cfg!(any(windows, target_os = "macos"))
            && custom_apps(settings).iter().any(|app| app.id == id && custom_exe(&app.path).is_some())
    }
}

/// The path of the user app `id` when it can be opened (its icon is read from it).
pub fn custom_path(id: &str, settings: &Settings) -> Option<PathBuf> {
    custom_apps(settings).into_iter().find(|app| app.id == id).and_then(|app| custom_exe(&app.path))
}

/// Opens the app `id`: raises its window if it has one, else starts it. The log line carries the
/// id from `APPS` (a user app is logged as `custom`, an unknown id as `unknown`) and the outcome,
/// never a name or a path.
#[cfg(windows)]
pub fn open(id: &str, settings: &Settings) -> Result<(), String> {
    // COM for the whole call: `ShellExecuteW` (the claude link) may hand it to a shell extension.
    let _com = crate::launcher::win::Com::init();
    let known = APPS.iter().find(|(app, _, _)| *app == id);
    let custom = custom_apps(settings).into_iter().find(|app| app.id == id);
    let result = match (known, custom) {
        (Some(entry), _) => open_app(entry),
        (None, Some(app)) => open_custom(app),
        (None, None) => Err("Unknown app.".to_string()),
    };
    let logged = match (known, custom) {
        (Some((app, _, _)), _) => app,
        (None, Some(_)) => "custom",
        (None, None) => "unknown",
    };
    crate::log::line(format!("apps: open id={logged} outcome={}", if result.is_ok() { "ok" } else { "err" }));
    result
}

/// Opens the built-in app `id` as its tile does: raises its window, else starts it.
#[cfg(windows)]
pub fn open_builtin(id: &str) -> Result<(), String> {
    let entry = APPS.iter().find(|(app, _, _)| *app == id).ok_or_else(|| "Unknown app.".to_string())?;
    let _com = crate::launcher::win::Com::init();
    open_app(entry)
}

/// Opens the built-in app `id` as its tile does: `open` brings it forward, or starts it.
#[cfg(target_os = "macos")]
pub fn open_builtin(id: &str) -> Result<(), String> {
    let (_, name, _) = APPS.iter().find(|(app, _, _)| *app == id).ok_or_else(|| "Unknown app.".to_string())?;
    let bundle = mac_bundle(id, &app_dirs()).ok_or_else(|| format!("{name} is not installed."))?;
    open_bundle(&bundle, name)
}

#[cfg(target_os = "macos")]
fn open_bundle(bundle: &Path, name: &str) -> Result<(), String> {
    if crate::platform::bring_app_forward(bundle) { Ok(()) } else { Err(format!("{name} did not start.")) }
}

/// Opens the app `id` (built in or the user's). The log line carries the id from `APPS` (a user
/// app is logged as `custom`) and the outcome, never a name or a path.
#[cfg(target_os = "macos")]
pub fn open(id: &str, settings: &Settings) -> Result<(), String> {
    let custom = custom_apps(settings).into_iter().find(|app| app.id == id);
    let result = match (is_builtin(id), custom) {
        (true, _) => open_builtin(id),
        (false, Some(app)) => match custom_exe(&app.path) {
            Some(bundle) => open_bundle(&bundle, &app.name),
            None => Err(format!("{} is not there any more.", app.name)),
        },
        (false, None) => Err("Unknown app.".to_string()),
    };
    let logged = if is_builtin(id) { id } else if custom.is_some() { "custom" } else { "unknown" };
    crate::log::line(format!("apps: open id={logged} outcome={}", if result.is_ok() { "ok" } else { "err" }));
    result
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn open_builtin(_id: &str) -> Result<(), String> {
    Err("not supported".to_string())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn open(_id: &str, _settings: &Settings) -> Result<(), String> {
    Err("not supported".to_string())
}

/// A user app: raises a window of its exe (by image name) if it has one, else starts it in its folder.
#[cfg(windows)]
fn open_custom(app: &CustomApp) -> Result<(), String> {
    use crate::focus::win::{app_windows, raise, Raise};

    let name = &app.name;
    let path = custom_exe(&app.path).ok_or_else(|| format!("{name} is not there any more."))?;
    let exe = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(window) = app_windows(&exe).first() {
        if raise(window.hwnd) != Raise::Failed {
            return Ok(());
        }
    }
    let mut cmd = std::process::Command::new(&path);
    if let Some(folder) = path.parent() {
        cmd.current_dir(folder);
    }
    crate::platform::no_console(&mut cmd).spawn().map(|_| ()).map_err(|_| format!("{name} did not start."))
}

#[cfg(windows)]
fn open_app((id, name, kind): &(&str, &str, Kind)) -> Result<(), String> {
    use crate::focus::win::{app_windows, raise, Raise};
    use crate::launcher::win::{launch_appx, open_uri};

    match kind {
        Kind::Exe { exe, .. } => {
            let local = crate::app_paths().localappdata;
            let path = exe_for(id, &local, &program_files()).ok_or_else(|| format!("{name} is not installed."))?;
            if let Some(window) = app_windows(exe).first() {
                if raise(window.hwnd) != Raise::Failed {
                    return Ok(());
                }
            }
            let mut cmd = std::process::Command::new(path);
            crate::platform::no_console(&mut cmd).spawn().map(|_| ()).map_err(|_| format!("{name} did not start."))
        }
        Kind::Protocol(scheme) => open_uri(&format!("{scheme}://")),
        Kind::Appx => {
            let local = crate::app_paths().localappdata;
            let family = crate::launcher::codex_family(&local.join("Packages"))
                .ok_or_else(|| format!("{name} is not installed."))?;
            launch_appx(&family)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }

    fn ids(apps: &[AppEntry]) -> Vec<&'static str> {
        apps.iter().map(|a| a.id).collect()
    }

    #[test]
    fn nothing_installed_gives_no_apps() {
        let local = TempDir::new("apps-local");
        let pf = TempDir::new("apps-pf");
        assert!(found(&local.path, &pf.path, |_| false, None).is_empty());
    }

    #[test]
    fn cursor_is_found_by_its_exe_under_local_programs() {
        let local = TempDir::new("apps-local");
        let pf = TempDir::new("apps-pf");
        touch(&local.path.join("Programs").join("cursor").join("Cursor.exe"));
        let apps = found(&local.path, &pf.path, |_| false, None);
        assert_eq!(apps, vec![AppEntry { id: "cursor", name: "Cursor" }]);
    }

    #[test]
    fn vscode_is_found_under_program_files() {
        let local = TempDir::new("apps-local");
        let pf = TempDir::new("apps-pf");
        touch(&pf.path.join("Microsoft VS Code").join("Code.exe"));
        let apps = found(&local.path, &pf.path, |_| false, None);
        assert_eq!(apps, vec![AppEntry { id: "vscode", name: "VS Code" }]);
    }

    #[test]
    fn vscode_is_found_under_local_programs() {
        let local = TempDir::new("apps-local");
        let pf = TempDir::new("apps-pf");
        touch(&local.path.join("Programs").join("Microsoft VS Code").join("Code.exe"));
        assert_eq!(ids(&found(&local.path, &pf.path, |_| false, None)), vec!["vscode"]);
    }

    #[test]
    fn all_apps_come_in_the_spec_order() {
        let local = TempDir::new("apps-local");
        let pf = TempDir::new("apps-pf");
        let programs = local.path.join("Programs");
        touch(&programs.join("Zed").join("Zed.exe"));
        touch(&programs.join("Windsurf").join("Windsurf.exe"));
        touch(&programs.join("cursor").join("Cursor.exe"));
        touch(&programs.join("antigravity").join("Antigravity.exe"));
        touch(&pf.path.join("Microsoft VS Code").join("Code.exe"));
        let apps = found(&local.path, &pf.path, |scheme| scheme == "claude", Some("OpenAI.Codex_x".to_string()));
        assert_eq!(ids(&apps), vec!["cursor", "vscode", "windsurf", "zed", "antigravity", "claude", "codex"]);
        let names: Vec<_> = apps.iter().map(|a| a.name).collect();
        assert_eq!(names, vec!["Cursor", "VS Code", "Windsurf", "Zed", "Antigravity", "Claude", "Codex"]);
    }

    #[test]
    fn a_folder_without_the_exe_does_not_count() {
        let local = TempDir::new("apps-local");
        let pf = TempDir::new("apps-pf");
        std::fs::create_dir_all(local.path.join("Programs").join("cursor")).unwrap();
        std::fs::create_dir_all(local.path.join("Programs").join("Zed").join("Zed.exe")).unwrap();
        assert!(found(&local.path, &pf.path, |_| false, None).is_empty());
    }

    #[test]
    fn claude_needs_its_protocol_and_codex_its_package() {
        let local = TempDir::new("apps-local");
        let pf = TempDir::new("apps-pf");
        assert_eq!(ids(&found(&local.path, &pf.path, |s| s == "claude", None)), vec!["claude"]);
        assert_eq!(ids(&found(&local.path, &pf.path, |s| s == "codex", None)), Vec::<&str>::new());
        assert_eq!(ids(&found(&local.path, &pf.path, |_| false, Some("OpenAI.Codex_x".into()))), vec!["codex"]);
    }

    fn tile(id: &str, custom: bool) -> AppTile {
        AppTile { id: id.into(), name: id.into(), custom, missing: false, exe: None }
    }

    fn tile_ids(tiles: &[AppTile]) -> Vec<&str> {
        tiles.iter().map(|t| t.id.as_str()).collect()
    }

    fn order(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_order_keeps_the_tiles_as_they_come() {
        let tiles = vec![tile("cursor", false), tile("zed", false), tile("user-1", true)];
        assert_eq!(tile_ids(&ordered(tiles, &[])), vec!["cursor", "zed", "user-1"]);
    }

    #[test]
    fn the_order_puts_its_ids_first_and_new_tiles_after_them() {
        let tiles = vec![tile("cursor", false), tile("vscode", false), tile("zed", false), tile("user-2", true)];
        // vscode is new since the order was saved, user-2 was just added: both go after the listed ones.
        let got = ordered(tiles, &order(&["zed", "cursor"]));
        assert_eq!(tile_ids(&got), vec!["zed", "cursor", "vscode", "user-2"]);
    }

    #[test]
    fn ids_without_a_tile_and_repeats_in_the_order_are_skipped() {
        let tiles = vec![tile("cursor", false), tile("user-1", true)];
        let got = ordered(tiles, &order(&["windsurf", "user-1", "nope", "user-1", "", "cursor", "cursor"]));
        assert_eq!(tile_ids(&got), vec!["user-1", "cursor"]);
    }

    #[test]
    fn a_new_built_in_app_joins_a_saved_order_at_its_end() {
        let tiles = vec![tile("cursor", false), tile("zed", false), tile("antigravity", false), tile("claude", false)];
        let got = ordered(tiles, &order(&["zed", "cursor", "claude"]));
        assert_eq!(tile_ids(&got), vec!["zed", "cursor", "claude", "antigravity"]);
    }

    fn settings_with(customs: &[(&str, &str, &str)]) -> Settings {
        let mut s = Settings::default();
        s.custom_apps = customs
            .iter()
            .map(|(id, name, path)| CustomApp { id: id.to_string(), name: name.to_string(), path: path.to_string() })
            .collect();
        s
    }

    #[test]
    fn an_app_is_known_open_by_the_exe_of_its_window_or_not_at_all() {
        let notepad = crate::testutil::wp(r"C:\Tools\notepad++.exe");
        let s = settings_with(&[("user-1", "Notepad++", &notepad)]);
        assert_eq!(window_exe("cursor", &s).as_deref(), Some("Cursor.exe"));
        assert_eq!(window_exe("vscode", &s).as_deref(), Some("Code.exe"));
        assert_eq!(window_exe("user-1", &s).as_deref(), Some("notepad++.exe"));
        // Opened by a URL protocol or as a package: no exe to look for.
        assert_eq!(window_exe("claude", &s), None);
        assert_eq!(window_exe("codex", &s), None);
        assert_eq!(window_exe("user-9", &s), None);
    }

    #[test]
    fn tiles_merge_installed_apps_and_user_apps_in_the_users_order() {
        let (notepad, gone) = (crate::testutil::wp(r"C:\Tools\notepad++.exe"), crate::testutil::wp(r"D:\gone.exe"));
        let mut s = settings_with(&[("user-1", "Notepad++", &notepad), ("user-2", "Gone", &gone)]);
        s.app_order = order(&["user-2", "zed", "removed-app", "cursor"]);
        let installed = vec![AppEntry { id: "cursor", name: "Cursor" }, AppEntry { id: "zed", name: "Zed" }];
        let got = tiles(installed, &s, |path| path.contains("notepad"));
        assert_eq!(tile_ids(&got), vec!["user-2", "zed", "cursor", "user-1"]);
        assert_eq!(
            got[0],
            AppTile { id: "user-2".into(), name: "Gone".into(), custom: true, missing: true, exe: Some("gone.exe".into()) }
        );
        assert_eq!(got[1], AppTile { id: "zed".into(), name: "Zed".into(), custom: false, missing: false, exe: None });
        assert_eq!((got[3].missing, got[3].exe.as_deref()), (false, Some("notepad++.exe")));
    }

    #[test]
    fn hidden_built_in_apps_have_no_tile() {
        let mut s = Settings::default();
        s.hidden_apps = order(&["zed", "user-1"]);
        let installed = vec![AppEntry { id: "cursor", name: "Cursor" }, AppEntry { id: "zed", name: "Zed" }];
        assert_eq!(tile_ids(&tiles(installed, &s, |_| true)), vec!["cursor"]);
    }

    #[test]
    fn user_apps_need_a_user_id_of_their_own() {
        let s = settings_with(&[
            ("user-1", "A", r"C:\a.exe"),
            ("cursor", "Fake Cursor", r"C:\b.exe"),
            ("user-", "Empty", r"C:\c.exe"),
            ("user-1", "Again", r"C:\d.exe"),
            ("user-2", "B", r"C:\e.exe"),
        ]);
        let names: Vec<_> = custom_apps(&s).iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["A", "B"]);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn a_user_app_opens_only_from_an_absolute_existing_app_bundle() {
        let tmp = TempDir::new("apps-custom-mac");
        let app = tmp.path.join("Notes++.app");
        assert_eq!(custom_exe(&app.to_string_lossy()), None, "not there");
        std::fs::create_dir_all(&app).unwrap();
        assert_eq!(custom_exe(&app.to_string_lossy()), Some(app.clone()));
        assert_eq!(custom_exe("Notes++.app"), None, "relative");
        let file = tmp.path.join("tool");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(custom_exe(&file.to_string_lossy()), None, "not a bundle");
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn a_user_app_opens_only_from_an_absolute_existing_exe() {
        let dir = TempDir::new("apps-custom");
        let exe = dir.path.join("tool.EXE");
        touch(&exe);
        let text = dir.path.join("notes.txt");
        touch(&text);
        assert_eq!(custom_exe(exe.to_str().unwrap()), Some(exe.clone()));
        assert_eq!(custom_exe(text.to_str().unwrap()), None, "not an exe");
        assert_eq!(custom_exe(dir.path.join("gone.exe").to_str().unwrap()), None, "not there");
        assert_eq!(custom_exe("tool.exe"), None, "relative");
        assert_eq!(custom_exe(""), None);
        std::fs::create_dir_all(dir.path.join("folder.exe")).unwrap();
        assert_eq!(custom_exe(dir.path.join("folder.exe").to_str().unwrap()), None, "a folder");
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn a_missing_user_app_has_no_path_to_open() {
        let dir = TempDir::new("apps-custom-path");
        let exe = dir.path.join("tool.exe");
        touch(&exe);
        let s = settings_with(&[
            ("user-1", "Tool", exe.to_str().unwrap()),
            ("user-2", "Gone", dir.path.join("gone.exe").to_str().unwrap()),
        ]);
        assert_eq!(custom_path("user-1", &s), Some(exe));
        assert_eq!(custom_path("user-2", &s), None);
        assert_eq!(custom_path("cursor", &s), None);
        assert!(!can_open("user-2", &s) && !can_open("user-9", &s));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn a_picked_shortcut_stands_for_its_target_exe() {
        let dir = TempDir::new("apps-pick");
        let exe = dir.path.join("tool.exe");
        touch(&exe);
        let link = dir.path.join("Tool.LNK");
        let doc = dir.path.join("readme.txt");
        touch(&doc);
        let to_exe = |_: &Path| Some(exe.clone());
        assert_eq!(picked_exe(&exe, |_| panic!("an exe is not resolved")), Some(exe.clone()));
        assert_eq!(picked_exe(&link, to_exe), Some(exe.clone()));
        assert_eq!(picked_exe(&link, |_| None), None, "a broken shortcut");
        assert_eq!(picked_exe(&link, |_| Some(doc.clone())), None, "a shortcut to a document");
        assert_eq!(picked_exe(&link, |_| Some(dir.path.join("gone.exe"))), None, "a shortcut to nothing");
        assert_eq!(picked_exe(&doc, to_exe), None, "neither an exe nor a shortcut");
    }

    #[test]
    fn a_picked_exe_is_named_by_its_description_else_its_file_name() {
        let exe = &PathBuf::from(crate::testutil::wp(r"C:\Tools\notepad++.exe"));
        assert_eq!(default_name(Some("  Google Chrome "), exe), "Google Chrome");
        assert_eq!(default_name(None, exe), "notepad++");
        assert_eq!(default_name(Some("   "), exe), "notepad++");
        assert_eq!(default_name(Some("Notepad++ : a free (GNU) source code editor"), exe), "notepad++");
    }

    #[test]
    fn every_built_in_app_has_its_mac_bundles_and_the_first_installed_one_wins() {
        assert!(APPS.iter().all(|(id, _, _)| MAC_BUNDLES.iter().any(|(app, _)| app == id)), "a bundle per app");
        let apps = TempDir::new("apps-mac");
        let user = TempDir::new("apps-mac-user");
        let dirs = [apps.path.clone(), user.path.clone()];
        assert_eq!(mac_bundle("codex", &dirs), None);
        std::fs::create_dir_all(user.path.join("ChatGPT.app")).unwrap();
        assert_eq!(mac_bundle("codex", &dirs), Some(user.path.join("ChatGPT.app")));
        std::fs::create_dir_all(apps.path.join("Codex.app")).unwrap();
        assert_eq!(mac_bundle("codex", &dirs), Some(apps.path.join("Codex.app")), "Codex's own app first");
        assert_eq!(mac_bundle("nope", &dirs), None);
    }

    #[test]
    fn built_in_ids_are_known_and_never_user_ids() {
        assert!(builtins().iter().all(|app| is_builtin(app.id) && !app.id.starts_with(CUSTOM_PREFIX)));
        assert_eq!(builtins().len(), APPS.len());
        assert!(!is_builtin("user-1") && !is_builtin(""));
    }

    #[test]
    fn exe_for_returns_the_path_found_checked() {
        let local = TempDir::new("apps-local");
        let pf = TempDir::new("apps-pf");
        let exe = local.path.join("Programs").join("cursor").join("Cursor.exe");
        assert_eq!(exe_for("cursor", &local.path, &pf.path), None);
        touch(&exe);
        assert_eq!(exe_for("cursor", &local.path, &pf.path), Some(exe));
        assert_eq!(exe_for("claude", &local.path, &pf.path), None, "claude has no exe");
        assert_eq!(exe_for("nope", &local.path, &pf.path), None);
    }
}
