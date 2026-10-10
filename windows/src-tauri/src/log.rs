// Small append-only log at %LOCALAPPDATA%\Mayak\mayak.log — the Windows
// equivalent of nbLog() in HookServer.swift. Nothing leaves the machine.
// Unit tests never write it: under cfg(test) a line goes to the running test's
// captured stderr instead (tests must not write to the user's log).

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

#[cfg(not(test))]
use crate::settings;

/// Start fresh once the file is past this many bytes (~1 MB).
const MAX_BYTES: u64 = 1_000_000;

/// Appends lines to one file. The private directory is set up and the file's
/// size read only the first time, and again after an open fails (folder or file
/// removed behind our back); every other line costs one open and one write.
struct Writer {
    dir: PathBuf,
    path: PathBuf,
    /// Size of the file as far as this writer knows; `None` until the folder is ready.
    size: Mutex<Option<u64>>,
    #[cfg(test)]
    setups: std::sync::atomic::AtomicUsize,
}

impl Writer {
    fn new(dir: PathBuf, file_name: &str) -> Self {
        let path = dir.join(file_name);
        Writer {
            dir,
            path,
            size: Mutex::new(None),
            #[cfg(test)]
            setups: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Creates the folder and reads the current size of the file.
    fn setup(&self) -> Option<u64> {
        #[cfg(test)]
        self.setups.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if crate::platform::ensure_private_dir(&self.dir).is_err() {
            return None;
        }
        Some(std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0))
    }

    /// Writes `text` (one whole line, newline included) as a single append.
    fn append(&self, text: &str) {
        let mut size = self.size.lock().unwrap_or_else(|e| e.into_inner());
        for attempt in 0..2 {
            let current = match *size {
                Some(n) => n,
                None => match self.setup() {
                    Some(n) => n,
                    None => return,
                },
            };
            // Keep it from growing forever: start fresh past ~1 MB.
            // If the removal fails (another process holds the file), keep the
            // count so the next line tries again.
            let current = if current > MAX_BYTES {
                match std::fs::remove_file(&self.path) {
                    Ok(()) => 0,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
                    Err(_) => current,
                }
            } else {
                current
            };
            match std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
                Ok(mut file) => {
                    let written = file.write_all(text.as_bytes()).is_ok();
                    *size = Some(if written { current + text.len() as u64 } else { current });
                    return;
                }
                Err(_) => {
                    // The folder may be gone: set it up and read the size again, once.
                    *size = None;
                    if attempt == 1 {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(not(test))]
pub fn line(message: impl AsRef<str>) {
    static WRITER: std::sync::OnceLock<Writer> = std::sync::OnceLock::new();
    let writer = WRITER.get_or_init(|| {
        Writer::new(settings::local_dir(), &format!("{}.log", crate::ident::APP_SLUG))
    });
    let t = crate::platform::local_time();
    let text = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {}\n",
        t.year,
        t.month,
        t.day,
        t.hour,
        t.minute,
        t.second,
        message.as_ref()
    );
    writer.append(&text);
}

/// Unit tests: the line goes to the captured stderr of the running test (shown only when
/// that test fails, or with `-- --nocapture`), never to the user's mayak.log.
#[cfg(test)]
pub fn line(message: impl AsRef<str>) {
    eprintln!("{}", message.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::Ordering;

    /// A fresh folder under the system temp dir, removed when dropped.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("mayak-log-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            TempDir(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn lines_are_appended_in_order() {
        let tmp = TempDir::new("order");
        let w = Writer::new(tmp.0.join("sub"), "a.log");
        w.append("one\n");
        w.append("two\n");
        w.append("three\n");
        assert_eq!(read(&w.path), "one\ntwo\nthree\n");
    }

    #[test]
    fn appends_to_an_existing_file_and_counts_its_size() {
        let tmp = TempDir::new("existing");
        std::fs::create_dir_all(&tmp.0).unwrap();
        std::fs::write(tmp.0.join("a.log"), "old\n").unwrap();
        let w = Writer::new(tmp.0.clone(), "a.log");
        w.append("new\n");
        assert_eq!(read(&w.path), "old\nnew\n");
        assert_eq!(*w.size.lock().unwrap(), Some(8));
    }

    #[test]
    fn folder_is_set_up_once() {
        let tmp = TempDir::new("once");
        let w = Writer::new(tmp.0.clone(), "a.log");
        for i in 0..20 {
            w.append(&format!("{i}\n"));
        }
        assert_eq!(w.setups.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn rotates_past_the_same_threshold() {
        let tmp = TempDir::new("rotate");
        std::fs::create_dir_all(&tmp.0).unwrap();
        let path = tmp.0.join("a.log");
        // Exactly at the limit: not past it, so the next line is still appended.
        std::fs::write(&path, vec![b'x'; MAX_BYTES as usize]).unwrap();
        let w = Writer::new(tmp.0.clone(), "a.log");
        w.append("at\n");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), MAX_BYTES + 3);
        // Now past it: the file starts fresh with this line.
        w.append("fresh\n");
        assert_eq!(read(&path), "fresh\n");
        w.append("next\n");
        assert_eq!(read(&path), "fresh\nnext\n");
        assert_eq!(w.setups.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn works_after_the_file_is_deleted() {
        let tmp = TempDir::new("deleted-file");
        let w = Writer::new(tmp.0.clone(), "a.log");
        w.append("before\n");
        std::fs::remove_file(&w.path).unwrap();
        w.append("after\n");
        assert_eq!(read(&w.path), "after\n");
    }

    #[test]
    fn works_after_the_folder_is_deleted() {
        let tmp = TempDir::new("deleted-dir");
        let w = Writer::new(tmp.0.clone(), "a.log");
        w.append("before\n");
        std::fs::remove_dir_all(&tmp.0).unwrap();
        w.append("after\n");
        assert_eq!(read(&w.path), "after\n");
        assert_eq!(w.setups.load(Ordering::SeqCst), 2);
        assert_eq!(*w.size.lock().unwrap(), Some(6));
    }

    #[test]
    fn concurrent_lines_are_whole() {
        let tmp = TempDir::new("threads");
        let w = std::sync::Arc::new(Writer::new(tmp.0.clone(), "a.log"));
        let handles: Vec<_> = (0..4)
            .map(|t| {
                let w = w.clone();
                std::thread::spawn(move || {
                    for i in 0..50 {
                        w.append(&format!("t{t} line {i}\n"));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let text = read(&w.path);
        assert_eq!(text.lines().count(), 200);
        assert!(text.lines().all(|l| l.starts_with('t') && l.contains(" line ")));
        assert_eq!(w.setups.load(Ordering::SeqCst), 1);
    }
}
