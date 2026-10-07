//! Installed IDEs and agent apps the Apps tab offers (spec docs/superpowers/specs/2026-10-07-mayak-batch-design.md §D).
//! `found` only looks at files, URL protocols and the Codex package, and takes its folders as
//! arguments; `open` is Windows only: elsewhere no app is found and `open` answers «not supported».

#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};

use serde::Serialize;

/// One tile of the Apps tab. Wire names are camelCase.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppEntry {
    pub id: &'static str,
    pub name: &'static str,
}

/// The folder an exe's relative path starts from.
#[derive(Clone, Copy)]
enum Root {
    LocalAppData,
    ProgramFiles,
}

/// How an app is found and opened.
enum Kind {
    /// The first of `paths` (under its root) that is a file; `exe` is the image name whose window is raised.
    Exe { paths: &'static [(Root, &'static str)], exe: &'static str },
    /// A registered URL protocol, opened as `<scheme>://`.
    Protocol(&'static str),
    /// The AppX package `OpenAI.Codex_*`.
    Appx,
}

/// Every app, in the order of the tab.
const APPS: [(&str, &str, Kind); 6] = [
    ("cursor", "Cursor", Kind::Exe { paths: &[(Root::LocalAppData, r"Programs\cursor\Cursor.exe")], exe: "Cursor.exe" }),
    (
        "vscode",
        "VS Code",
        Kind::Exe {
            paths: &[
                (Root::LocalAppData, r"Programs\Microsoft VS Code\Code.exe"),
                (Root::ProgramFiles, r"Microsoft VS Code\Code.exe"),
            ],
            exe: "Code.exe",
        },
    ),
    (
        "windsurf",
        "Windsurf",
        Kind::Exe { paths: &[(Root::LocalAppData, r"Programs\Windsurf\Windsurf.exe")], exe: "Windsurf.exe" },
    ),
    ("zed", "Zed", Kind::Exe { paths: &[(Root::LocalAppData, r"Programs\Zed\Zed.exe")], exe: "Zed.exe" }),
    ("claude", "Claude", Kind::Protocol("claude")),
    ("codex", "Codex", Kind::Appx),
];

/// The exe of the app `id` that exists in the given folders; `None` for an app without an exe.
pub fn exe_for(id: &str, localappdata: &Path, program_files: &Path) -> Option<PathBuf> {
    let (_, _, Kind::Exe { paths, .. }) = APPS.iter().find(|(app, _, _)| *app == id)? else { return None };
    paths
        .iter()
        .map(|(root, rel)| match root {
            Root::LocalAppData => localappdata.join(rel),
            Root::ProgramFiles => program_files.join(rel),
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

/// `%ProgramFiles%`; Windows always sets it, the default location is the fallback.
#[cfg(windows)]
fn program_files() -> PathBuf {
    std::env::var_os("ProgramFiles")
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from)
}

/// The apps installed now. Reads folders and the registry, so it runs off the UI thread.
#[cfg(windows)]
pub fn list() -> Vec<AppEntry> {
    let local = crate::app_paths().localappdata;
    let family = crate::launcher::codex_family(&local.join("Packages"));
    found(&local, &program_files(), crate::launcher::win::protocol_registered, family)
}

#[cfg(not(windows))]
pub fn list() -> Vec<AppEntry> {
    Vec::new()
}

/// Opens the app `id`: raises its window if it has one, else starts it. The log line carries the
/// id and the outcome, never a path.
#[cfg(windows)]
pub fn open(id: &str) -> Result<(), String> {
    let result = open_app(id);
    crate::log::line(format!("apps: open id={id} outcome={}", if result.is_ok() { "ok" } else { "err" }));
    result
}

#[cfg(not(windows))]
pub fn open(_id: &str) -> Result<(), String> {
    Err("not supported".to_string())
}

#[cfg(windows)]
fn open_app(id: &str) -> Result<(), String> {
    use crate::focus::win::{app_windows, raise, Raise};
    use crate::launcher::win::{launch_appx, open_uri};

    let (_, name, kind) = APPS.iter().find(|(app, _, _)| *app == id).ok_or_else(|| "Unknown app.".to_string())?;
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
        touch(&pf.path.join("Microsoft VS Code").join("Code.exe"));
        let apps = found(&local.path, &pf.path, |scheme| scheme == "claude", Some("OpenAI.Codex_x".to_string()));
        assert_eq!(ids(&apps), vec!["cursor", "vscode", "windsurf", "zed", "claude", "codex"]);
        let names: Vec<_> = apps.iter().map(|a| a.name).collect();
        assert_eq!(names, vec!["Cursor", "VS Code", "Windsurf", "Zed", "Claude", "Codex"]);
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
