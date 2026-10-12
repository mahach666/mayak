// Test-only helpers shared by the app's unit tests (`#[cfg(test)]` in lib.rs).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT: AtomicU32 = AtomicU32::new(0);

/// A fresh, empty directory `%TEMP%\mayak-test-<tag>-<pid>-<n>`, removed with
/// everything in it when the value is dropped. `n` makes every call unique
/// inside one test binary, `pid` keeps parallel test binaries apart.
pub struct TempDir {
    pub path: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mayak-test-{tag}-{}-{n}",
            std::process::id()
        ));
        // A crashed earlier run with a recycled pid may have left this behind.
        if path.exists() {
            std::fs::remove_dir_all(&path)
                .unwrap_or_else(|e| panic!("cannot clear {}: {e}", path.display()));
        }
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("cannot create {}: {e}", path.display()));
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Never panic in drop (a failing test is already unwinding); say it instead.
        if let Err(err) = std::fs::remove_dir_all(&self.path) {
            eprintln!("could not remove {}: {err}", self.path.display());
        }
    }
}

/// A path written the Windows way in a test, as this platform spells it: itself on Windows;
/// elsewhere the drive letter goes and every '\' becomes '/' (`C:\work\app` → `/work/app`,
/// `work\app` → `work/app`). Lets one fixture serve tests of platform-neutral logic everywhere.
pub fn wp(windows_path: &str) -> String {
    if cfg!(windows) {
        return windows_path.to_string();
    }
    let b = windows_path.as_bytes();
    let rest = if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' { &windows_path[2..] } else { windows_path };
    let unix = rest.replace('\\', "/");
    if unix.is_empty() { "/".to_string() } else { unix }
}

/// The same for a JSON string value: `wp` with each '\' of the fixture already unescaped.
pub fn wpj(windows_path: &str) -> String {
    serde_json::to_string(&wp(windows_path)).unwrap()
}

#[test]
fn windows_fixture_paths_become_this_platforms_paths() {
    if cfg!(windows) {
        assert_eq!(wp(r"C:\work\app"), r"C:\work\app");
    } else {
        assert_eq!(wp(r"C:\work\app"), "/work/app");
        assert_eq!(wp(r"c:\"), "/");
        assert_eq!(wp(r"work\app"), "work/app");
        assert_eq!(wpj(r"C:\a b"), "\"/a b\"");
    }
}

/// A child process that does nothing for about `secs` seconds and then exits: `ping` on
/// Windows (one echo a second, so `secs + 1` of them), `sleep` elsewhere.
pub fn sleeper(secs: u32) -> std::process::Child {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/c", &format!("ping -n {} 127.0.0.1 >nul", secs + 1)]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = std::process::Command::new("/bin/sleep");
        c.arg(secs.to_string());
        c
    };
    cmd.stdout(std::process::Stdio::null()).spawn().expect("the sleeper starts")
}
