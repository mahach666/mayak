// Small append-only log at %LOCALAPPDATA%\Mayak\mayak.log — the Windows
// equivalent of nbLog() in HookServer.swift. Nothing leaves the machine.
// Unit tests never write it: under cfg(test) a line goes to the running test's
// captured stderr instead (tests must not write to the user's log).

#[cfg(not(test))]
use std::io::Write;

#[cfg(not(test))]
use crate::{platform, settings};

#[cfg(not(test))]
pub fn line(message: impl AsRef<str>) {
    let t = platform::local_time();
    let stamp = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    );
    let dir = settings::local_dir();
    if platform::ensure_private_dir(&dir).is_err() {
        return;
    }
    let path = dir.join(format!("{}.log", crate::ident::APP_SLUG));
    // Keep it from growing forever: start fresh past ~1 MB.
    if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{stamp} {}", message.as_ref());
    }
}

/// Unit tests: the line goes to the captured stderr of the running test (shown only when
/// that test fails, or with `-- --nocapture`), never to the user's mayak.log.
#[cfg(test)]
pub fn line(message: impl AsRef<str>) {
    eprintln!("{}", message.as_ref());
}
