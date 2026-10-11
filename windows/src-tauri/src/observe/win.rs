// Win32 primitives for the observer (spec §5.8): a directory watcher and process proof.
// Mayak's own Windows-only code, kept outside upstream's `platform/` on purpose (D85): it
// has no Linux counterpart, and `platform/` stays the OS layer shared with upstream.
//
// Blocking calls on plain std threads (spec §4). Every kernel handle is an `OwnedHandle`
// right after it is opened, so it can move into a thread (`HANDLE` is not `Send`) and is
// closed exactly once, by Drop.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::sync::mpsc;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_NOTIFY_ENUM_DIR, HANDLE, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_ACTION_ADDED, FILE_ACTION_MODIFIED,
    FILE_ACTION_REMOVED, FILE_ACTION_RENAMED_NEW_NAME, FILE_ACTION_RENAMED_OLD_NAME,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::CreateEventW;

pub const WATCH_BUFFER: usize = 64 * 1024;
/// sessions\ and projects\ watchers (spec §5.5).
pub const MASK_NAME_WRITE_SIZE: u32 =
    FILE_NOTIFY_CHANGE_FILE_NAME.0 | FILE_NOTIFY_CHANGE_LAST_WRITE.0 | FILE_NOTIFY_CHANGE_SIZE.0;
/// Only the mask test and the `#[ignore]` open-handle diagnostic use this one (spec §12),
/// so it exists in test builds only.
#[cfg(test)]
pub const MASK_NAME_WRITE: u32 = FILE_NOTIFY_CHANGE_FILE_NAME.0 | FILE_NOTIFY_CHANGE_LAST_WRITE.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction {
    Added,
    Removed,
    Modified,
    RenamedOld,
    RenamedNew,
    Other(u32),
}

impl FileAction {
    fn from_raw(code: u32) -> Self {
        match code {
            c if c == FILE_ACTION_ADDED.0 => Self::Added,
            c if c == FILE_ACTION_REMOVED.0 => Self::Removed,
            c if c == FILE_ACTION_MODIFIED.0 => Self::Modified,
            c if c == FILE_ACTION_RENAMED_OLD_NAME.0 => Self::RenamedOld,
            c if c == FILE_ACTION_RENAMED_NEW_NAME.0 => Self::RenamedNew,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug)]
pub enum WatchEvent {
    /// Path relative to the watched directory, as reported (`sub\name.jsonl` when recursive).
    Changes(Vec<(FileAction, String)>),
    /// 0 bytes returned, or the call failed with ERROR_NOTIFY_ENUM_DIR (1022): the buffer
    /// overflowed and the caller must re-enumerate.
    Overflow,
    /// Any other failure; the watcher thread ends after sending this.
    Error(io::Error),
}

/// Walks the FILE_NOTIFY_INFORMATION chain: NextEntryOffset u32, Action u32,
/// FileNameLength u32 (bytes), FileName UTF-16. Stops at a truncated record.
fn parse_notifications(bytes: &[u8]) -> Vec<(FileAction, String)> {
    let u32_at = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 12 <= bytes.len() {
        let next = u32_at(at) as usize;
        let action = u32_at(at + 4);
        let name_len = u32_at(at + 8) as usize;
        let name_end = at + 12 + name_len;
        if name_end > bytes.len() {
            break;
        }
        let units: Vec<u16> = bytes[at + 12..name_end]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        out.push((FileAction::from_raw(action), String::from_utf16_lossy(&units)));
        if next == 0 {
            break;
        }
        at += next;
    }
    out
}

pub struct DirWatcher;

impl DirWatcher {
    /// Opens `dir` synchronously, so a missing directory is `Err(NotFound)` here (the caller
    /// reports `Missing`). Then thread "dirwatch-<dir file name>" keeps one overlapped
    /// ReadDirectoryChangesW request queued, blocks in GetOverlappedResult until it completes
    /// and hands every result to `sink`; `sink` returning false ends the thread. Returns after
    /// the first ReadDirectoryChangesW request is queued: the kernel records changes from that
    /// request on, so an enumeration done after `spawn` returns cannot miss a change.
    pub fn spawn(
        dir: &Path,
        recursive: bool,
        mask: u32,
        mut sink: impl FnMut(WatchEvent) -> bool + Send + 'static,
    ) -> io::Result<()> {
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let raw = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .map_err(io_error)?;
        let handle = unsafe { OwnedHandle::from_raw_handle(raw.0) };
        // Manual-reset event that each request signals when it completes.
        let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(io_error)?;
        let done_event = unsafe { OwnedHandle::from_raw_handle(event.0) };
        let name = format!(
            "dirwatch-{}",
            dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        );
        let (ready, started) = mpsc::sync_channel::<()>(1);
        std::thread::Builder::new()
            .name(name)
            .spawn(move || watch_loop(&handle, &done_event, recursive, mask, ready, &mut sink))?;
        started
            .recv()
            .map_err(|_| io::Error::other("dirwatch thread ended before its first request"))
    }
}

fn watch_loop(
    dir: &OwnedHandle,
    done_event: &OwnedHandle,
    recursive: bool,
    mask: u32,
    ready: mpsc::SyncSender<()>,
    sink: &mut impl FnMut(WatchEvent) -> bool,
) {
    // u32 storage keeps the FILE_NOTIFY_INFORMATION records DWORD-aligned.
    let mut buf = vec![0u32; WATCH_BUFFER / 4];
    let mut ready = Some(ready);
    loop {
        let mut ov = OVERLAPPED { hEvent: HANDLE(done_event.as_raw_handle()), ..Default::default() };
        let issued = unsafe {
            ReadDirectoryChangesW(
                HANDLE(dir.as_raw_handle()),
                buf.as_mut_ptr().cast(),
                WATCH_BUFFER as u32,
                recursive,
                FILE_NOTIFY_CHANGE(mask),
                None,
                Some(&mut ov as *mut OVERLAPPED),
                None,
            )
        };
        // The first request is queued (or refused): from here on the kernel records changes.
        if let Some(ready) = ready.take() {
            // spawn() holds the receiver, blocked in recv(), so this send cannot fail. Never
            // return here: the kernel may still be writing into `buf` and `ov`.
            ready.send(()).ok();
        }
        // bWait = TRUE: returns only after the request has completed, so `buf` and `ov` are
        // never released while the kernel still owns them.
        let mut returned = 0u32;
        let result = issued.and_then(|()| unsafe {
            GetOverlappedResult(HANDLE(dir.as_raw_handle()), &ov, &mut returned, true)
        });
        let event = match result {
            Ok(()) if returned == 0 => WatchEvent::Overflow,
            Ok(()) => {
                let len = (returned as usize).min(WATCH_BUFFER);
                let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), len) };
                WatchEvent::Changes(parse_notifications(bytes))
            }
            Err(err) if err.code() == ERROR_NOTIFY_ENUM_DIR.to_hresult() => WatchEvent::Overflow,
            Err(err) => {
                sink(WatchEvent::Error(io_error(err)));
                return;
            }
        };
        if !sink(event) {
            return;
        }
    }
}

/// windows-rs wraps a Win32 error in an HRESULT, and its `From` for io::Error keeps the
/// HRESULT, which hides `ErrorKind::NotFound` & co. Give std the plain Win32 code back.
fn io_error(err: windows::core::Error) -> io::Error {
    match WIN32_ERROR::from_error(&err) {
        Some(code) => io::Error::from_raw_os_error(code.0 as i32),
        None => io::Error::other(err),
    }
}

pub mod proc {
    use std::collections::HashSet;
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::sync::{Arc, Mutex};

    use windows::core::PWSTR;
    use windows::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER, FILETIME,
        HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT, WIN32_ERROR,
    };
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, INFINITE,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };

    use super::io_error;
    use crate::observe::keys::ProcId;

    /// The process handle `identify` returns and `Registry::watch` waits on.
    pub type ProcHandle = OwnedHandle;

    #[derive(Debug)]
    pub enum ProcError {
        NotFound,
        AccessDenied,
        Exited,
        Os(io::Error),
    }

    /// Proof that `pid` is a live process (spec §5.5 step 3): OpenProcess(SYNCHRONIZE |
    /// QUERY_LIMITED_INFORMATION) → still running (a handle held elsewhere keeps an exited
    /// process object and its pid alive) → creation FILETIME → image path. The handle is
    /// returned for `Registry::watch`.
    pub fn identify(pid: u32) -> Result<(ProcId, OwnedHandle), ProcError> {
        let raw = unsafe {
            OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
        }
        .map_err(|err| match WIN32_ERROR::from_error(&err) {
            Some(ERROR_INVALID_PARAMETER) => ProcError::NotFound,
            Some(ERROR_ACCESS_DENIED) => ProcError::AccessDenied,
            _ => ProcError::Os(io_error(err)),
        })?;
        let handle = unsafe { OwnedHandle::from_raw_handle(raw.0) };
        let h = HANDLE(handle.as_raw_handle());

        let state = unsafe { WaitForSingleObject(h, 0) };
        if state == WAIT_OBJECT_0 {
            return Err(ProcError::Exited);
        }
        if state != WAIT_TIMEOUT {
            return Err(ProcError::Os(io::Error::last_os_error()));
        }

        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe { GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user) }
            .map_err(|err| ProcError::Os(io_error(err)))?;
        let created_ft = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);

        let image = image_path(h).map_err(ProcError::Os)?;
        Ok((ProcId { pid, created_ft, image }, handle))
    }

    fn image_path(h: HANDLE) -> io::Result<String> {
        for capacity in [1024usize, 32_768] {
            let mut buf = vec![0u16; capacity];
            let mut len = capacity as u32;
            match unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) } {
                Ok(()) => return Ok(String::from_utf16_lossy(&buf[..len as usize])),
                Err(err) if WIN32_ERROR::from_error(&err) == Some(ERROR_INSUFFICIENT_BUFFER) => continue,
                Err(err) => return Err(io_error(err)),
            }
        }
        Err(io::Error::from_raw_os_error(ERROR_INSUFFICIENT_BUFFER.0 as i32))
    }

    /// Exit waiters shared by claude-io and codex-io: at most one thread per (pid, created_ft).
    #[derive(Clone, Default)]
    pub struct Registry {
        watched: Arc<Mutex<HashSet<(u32, u64)>>>,
    }

    impl Registry {
        pub fn new() -> Self {
            Self::default()
        }

        /// Returns false (and drops `handle`) if this process is already watched, or if the
        /// waiter thread could not be started: then the entry is removed and the failure is
        /// logged, so a later proof registers again. Otherwise thread "proc-exit-<pid>" waits
        /// INFINITE on the given handle, removes the entry, then calls `on_exit(pid,
        /// created_ft)`. A failed wait is logged and does not call `on_exit` (the entry is
        /// removed, so the next proof registers again).
        pub fn watch(
            &self,
            id: &ProcId,
            handle: OwnedHandle,
            on_exit: impl FnOnce(u32, u64) + Send + 'static,
        ) -> bool {
            let key = (id.pid, id.created_ft);
            if !self.watched.lock().unwrap().insert(key) {
                return false;
            }
            let watched = Arc::clone(&self.watched);
            let spawned = std::thread::Builder::new()
                .name(format!("proc-exit-{}", id.pid))
                .spawn(move || {
                    let state = unsafe { WaitForSingleObject(HANDLE(handle.as_raw_handle()), INFINITE) };
                    let failure = (state != WAIT_OBJECT_0).then(io::Error::last_os_error);
                    drop(handle);
                    watched.lock().unwrap().remove(&key);
                    match failure {
                        None => on_exit(key.0, key.1),
                        Some(err) => crate::log::line(format!("proc-exit wait failed pid={}: {err}", key.0)),
                    }
                });
            if let Err(err) = spawned {
                self.watched.lock().unwrap().remove(&key);
                crate::log::line(format!("proc-exit thread not started pid={}: {err}", key.0));
                return false;
            }
            true
        }

        pub fn is_watched(&self, pid: u32, created_ft: u64) -> bool {
            self.watched.lock().unwrap().contains(&(pid, created_ft))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(action: u32, name: &str) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut out = Vec::new();
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&action.to_le_bytes());
        out.extend_from_slice(&((units.len() * 2) as u32).to_le_bytes());
        for u in units {
            out.extend_from_slice(&u.to_le_bytes());
        }
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }

    /// Chains records the way the kernel does: each NextEntryOffset = its own padded length.
    fn chain(records: Vec<Vec<u8>>) -> Vec<u8> {
        let last = records.len() - 1;
        let mut out = Vec::new();
        for (i, mut r) in records.into_iter().enumerate() {
            if i < last {
                let len = r.len() as u32;
                r[..4].copy_from_slice(&len.to_le_bytes());
            }
            out.extend_from_slice(&r);
        }
        out
    }

    #[test]
    fn notification_records_are_parsed_in_order() {
        let bytes = chain(vec![
            record(1, "123.json"),
            record(9, "Тест.json"),
            record(5, r"C--work-repo\abc.jsonl"),
        ]);
        assert_eq!(
            parse_notifications(&bytes),
            vec![
                (FileAction::Added, "123.json".to_string()),
                (FileAction::Other(9), "Тест.json".to_string()),
                (FileAction::RenamedNew, r"C--work-repo\abc.jsonl".to_string()),
            ]
        );
        let first_len = record(1, "123.json").len();
        assert_eq!(
            parse_notifications(&bytes[..first_len + 5]),
            vec![(FileAction::Added, "123.json".to_string())],
            "a truncated record is dropped, earlier ones are kept"
        );
        assert_eq!(FileAction::from_raw(2), FileAction::Removed);
        assert_eq!(FileAction::from_raw(3), FileAction::Modified);
        assert_eq!(FileAction::from_raw(4), FileAction::RenamedOld);
    }

    #[test]
    fn masks_are_the_documented_bits() {
        // FILE_NOTIFY_CHANGE_FILE_NAME 0x1, _SIZE 0x8, _LAST_WRITE 0x10.
        assert_eq!(MASK_NAME_WRITE_SIZE, 0x19);
        assert_eq!(MASK_NAME_WRITE, 0x11);
    }
}

/// Windows integration tests of DirWatcher (spec §12): real directories, real notifications.
#[cfg(all(test, windows))]
mod watch_tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::io::Write;
    use std::time::{Duration, Instant};

    /// Waits up to `limit` for an event matching `want`; other events are skipped.
    fn wait_for(
        rx: &mpsc::Receiver<WatchEvent>,
        limit: Duration,
        want: impl Fn(&WatchEvent) -> bool,
    ) -> Option<WatchEvent> {
        let deadline = Instant::now() + limit;
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            match rx.recv_timeout(left) {
                Ok(event) if want(&event) => return Some(event),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }

    fn names(event: &WatchEvent, name: &str) -> bool {
        matches!(event, WatchEvent::Changes(c) if c.iter().any(|(_, n)| n == name))
    }

    /// Writes `path` once (open → write → close) and waits up to 5 s for a Changes event naming
    /// `name`. One write is enough: `spawn` returns after the first request is queued.
    fn write_and_expect(rx: &mpsc::Receiver<WatchEvent>, path: &Path, name: &str) -> bool {
        std::fs::write(path, br#"{"status":"busy"}"#).unwrap();
        wait_for(rx, Duration::from_secs(5), |e| names(e, name)).is_some()
    }

    #[test]
    fn dir_watcher_on_a_missing_dir_is_not_found() {
        let tmp = TempDir::new("dirwatch-missing");
        let err = DirWatcher::spawn(&tmp.path.join("absent"), false, MASK_NAME_WRITE_SIZE, |_| true)
            .expect_err("a missing directory must fail synchronously");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn dir_watcher_reports_a_write_then_close_and_an_error_when_the_dir_goes() {
        let tmp = TempDir::new("dirwatch");
        let dir = tmp.path.join("sessions");
        std::fs::create_dir(&dir).unwrap();
        let (tx, rx) = mpsc::channel();
        DirWatcher::spawn(&dir, false, MASK_NAME_WRITE_SIZE, move |e| tx.send(e).is_ok()).unwrap();

        // Claude Code rewrites its session file with open → write → close.
        assert!(
            write_and_expect(&rx, &dir.join("4242.json"), "4242.json"),
            "no notification for a write-then-close"
        );

        std::fs::remove_file(dir.join("4242.json")).unwrap();
        std::fs::remove_dir(&dir).unwrap();
        // Overflow may come first (an empty batch before the failing call); Changes are skipped.
        let end = wait_for(&rx, Duration::from_secs(5), |e| matches!(e, WatchEvent::Error(_)));
        assert!(end.is_some(), "the watcher must report Error once its directory is gone");
        assert!(
            matches!(rx.recv_timeout(Duration::from_secs(5)), Err(mpsc::RecvTimeoutError::Disconnected)),
            "the watcher thread must end after Error"
        );
    }

    #[test]
    fn a_recursive_watcher_reports_paths_relative_to_its_dir() {
        let tmp = TempDir::new("dirwatch-rec");
        let root = tmp.path.join("projects");
        let slug = root.join("C--work-repo");
        std::fs::create_dir_all(&slug).unwrap();
        let (tx, rx) = mpsc::channel();
        DirWatcher::spawn(&root, true, MASK_NAME_WRITE_SIZE, move |e| tx.send(e).is_ok()).unwrap();
        assert!(
            write_and_expect(&rx, &slug.join("abc.jsonl"), r"C--work-repo\abc.jsonl"),
            "recursive changes must be reported as <subdir>\\<name>"
        );
    }

    #[test]
    fn a_sink_returning_false_ends_the_watcher() {
        let tmp = TempDir::new("dirwatch-stop");
        let (tx, rx) = mpsc::channel();
        DirWatcher::spawn(&tmp.path, false, MASK_NAME_WRITE_SIZE, move |e| {
            tx.send(e).unwrap();
            false
        })
        .unwrap();
        std::fs::write(tmp.path.join("1.json"), b"x").unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok(), "no event for a write-then-close");
        assert!(
            matches!(rx.recv_timeout(Duration::from_secs(5)), Err(mpsc::RecvTimeoutError::Disconnected)),
            "the thread must end, dropping the sink, after the sink returned false"
        );
    }

    /// Records whether appends through a handle that stays open are reported (spec §3: three
    /// such appends gave no `Changed`). An observation, not an OS guarantee; never run in CI.
    #[test]
    #[ignore = "diagnostic: prints what this machine reports"]
    fn diagnostic_appends_through_an_open_handle() {
        let tmp = TempDir::new("dirwatch-diag");
        let path = tmp.path.join("rollout.jsonl");
        std::fs::write(&path, b"{\"type\":\"session_meta\"}\n").unwrap();
        let (tx, rx) = mpsc::channel();
        DirWatcher::spawn(&tmp.path, false, MASK_NAME_WRITE, move |e| tx.send(e).is_ok()).unwrap();
        assert!(write_and_expect(&rx, &tmp.path.join("armed.txt"), "armed.txt"), "the watcher reports nothing");

        let mut writer = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        for n in 0..3 {
            writer.write_all(format!("{{\"type\":\"event_msg\",\"n\":{n}}}\n").as_bytes()).unwrap();
            std::thread::sleep(Duration::from_millis(200));
        }
        let open_phase = count_mentions(&rx, "rollout.jsonl", Duration::from_secs(2));
        drop(writer);
        let close_phase = count_mentions(&rx, "rollout.jsonl", Duration::from_secs(2));
        println!("appends through an open handle: {open_phase} notification(s) within 2 s");
        println!("after closing the handle: {close_phase} notification(s) within 2 s");
    }

    fn count_mentions(rx: &mpsc::Receiver<WatchEvent>, name: &str, window: Duration) -> usize {
        let deadline = Instant::now() + window;
        let mut count = 0;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(WatchEvent::Changes(c)) => count += c.iter().filter(|(_, n)| n == name).count(),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        count
    }
}

/// Windows integration tests of the process proof (spec §12).
#[cfg(all(test, windows))]
mod proc_tests {
    use super::proc::{self, ProcError};
    use std::process::{Command, Stdio};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn identify_matches_the_current_process() {
        let (id, _handle) = proc::identify(std::process::id()).expect("own process");
        assert_eq!(id.pid, std::process::id());
        let exe = std::env::current_exe().unwrap();
        assert_eq!(id.image.to_lowercase(), exe.to_string_lossy().to_lowercase());
        // FILETIME = 100 ns ticks since 1601-01-01; this test process started < 1 h ago.
        let unix_ticks = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64 / 100;
        let now_ft = unix_ticks + 116_444_736_000_000_000;
        assert!(id.created_ft <= now_ft, "created in the future: {}", id.created_ft);
        assert!(now_ft - id.created_ft < 36_000_000_000, "created over an hour ago: {}", id.created_ft);
        let (again, _h) = proc::identify(std::process::id()).unwrap();
        assert_eq!(again, id, "same process, same identity");
    }

    #[test]
    fn identify_rejects_the_idle_process_and_an_exited_one() {
        assert!(matches!(proc::identify(0), Err(ProcError::NotFound)));
        let mut child = Command::new("cmd")
            .args(["/d", "/c", "exit"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        child.wait().unwrap();
        // `child` still holds its handle: the pid is not reused, the object is a zombie.
        assert!(matches!(proc::identify(child.id()), Err(ProcError::Exited)));
    }
}

/// Windows integration tests of the exit-waiter registry (spec §12, D38).
#[cfg(all(test, windows))]
mod registry_tests {
    use super::proc::Registry;
    use crate::observe::keys::ProcId;
    use std::os::windows::io::AsHandle;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn registry_fires_once_per_process_and_refuses_a_second_waiter() {
        // `set /p` keeps cmd alive until its stdin closes, so both watch calls see it running.
        let mut child = Command::new("cmd")
            .args(["/d", "/c", "set /p _="])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // D38: duplicated child handles and a hand-built ProcId (identify could race the exit).
        let id = ProcId { pid: child.id(), created_ft: 1, image: String::new() };
        let registry = Registry::new();
        let (tx, rx) = mpsc::channel();
        let dup = child.as_handle().try_clone_to_owned().unwrap();
        assert!(registry.watch(&id, dup, move |pid, ft| tx.send((pid, ft)).unwrap()));
        assert!(registry.is_watched(id.pid, 1));
        let dup2 = child.as_handle().try_clone_to_owned().unwrap();
        assert!(
            !registry.watch(&id, dup2, |_, _| panic!("a second waiter must not exist")),
            "the same (pid, created) must not get a second thread"
        );

        drop(child.stdin.take()); // EOF → `set /p` returns → cmd exits
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), (id.pid, 1));
        assert!(!registry.is_watched(id.pid, 1), "the entry is removed before on_exit runs");
        child.wait().unwrap();
    }
}
