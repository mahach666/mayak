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
