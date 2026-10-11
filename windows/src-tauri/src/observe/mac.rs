// macOS primitives for the observer (spec §5.8), the counterpart of `win.rs` with the same
// names: a directory watcher on FSEvents and process proof on libproc + kqueue.
//
// Blocking calls on plain std threads (spec §4), as on Windows. The process handle is a kqueue
// descriptor with the process's exit already registered, so a pid reused after `identify` can
// never be mistaken for the process we proved.

use std::ffi::{c_void, CStr, CString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// sessions/ and projects/ watchers (spec §5.5). FSEvents has no mask: every file event comes,
/// and the callers filter by name as they do on Windows.
pub const MASK_NAME_WRITE_SIZE: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction {
    Added,
    Removed,
    Modified,
    RenamedOld,
    RenamedNew,
    #[allow(dead_code)] // the same variants as on Windows
    Other(u32),
}

#[derive(Debug)]
pub enum WatchEvent {
    /// Path relative to the watched directory (`sub/name.jsonl` when recursive).
    Changes(Vec<(FileAction, String)>),
    /// FSEvents dropped events or asks for a rescan: the caller must re-enumerate.
    Overflow,
    /// Any other failure; the watcher thread ends after sending this.
    #[allow(dead_code)] // the same variants as on Windows
    Error(io::Error),
}

/// CoreServices' FSEvents and the CoreFoundation it needs, straight from the C API.
mod fs {
    use std::ffi::c_void;

    pub type Callback = extern "C" fn(
        stream: *const c_void,
        info: *mut c_void,
        count: usize,
        paths: *mut c_void,
        flags: *const u32,
        ids: *const u64,
    );

    #[repr(C)]
    pub struct Context {
        pub version: isize,
        pub info: *mut c_void,
        pub retain: *const c_void,
        pub release: *const c_void,
        pub copy_description: *const c_void,
    }

    pub const SINCE_NOW: u64 = u64::MAX;
    pub const CREATE_NO_DEFER: u32 = 0x02;
    pub const CREATE_WATCH_ROOT: u32 = 0x04;
    pub const CREATE_FILE_EVENTS: u32 = 0x10;

    pub const MUST_SCAN_SUBDIRS: u32 = 0x01;
    pub const USER_DROPPED: u32 = 0x02;
    pub const KERNEL_DROPPED: u32 = 0x04;
    pub const ROOT_CHANGED: u32 = 0x20;
    pub const ITEM_CREATED: u32 = 0x100;
    pub const ITEM_REMOVED: u32 = 0x200;
    pub const ITEM_INODE_META_MOD: u32 = 0x400;
    pub const ITEM_RENAMED: u32 = 0x800;
    pub const ITEM_MODIFIED: u32 = 0x1000;
    pub const ITEM_IS_FILE: u32 = 0x10000;

    pub const UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        pub fn FSEventStreamCreate(
            allocator: *const c_void,
            callback: Callback,
            context: *const Context,
            paths: *const c_void,
            since: u64,
            latency: f64,
            flags: u32,
        ) -> *mut c_void;
        pub fn FSEventStreamScheduleWithRunLoop(stream: *mut c_void, run_loop: *mut c_void, mode: *const c_void);
        pub fn FSEventStreamStart(stream: *mut c_void) -> u8;
        pub fn FSEventStreamStop(stream: *mut c_void);
        pub fn FSEventStreamInvalidate(stream: *mut c_void);
        pub fn FSEventStreamRelease(stream: *mut c_void);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub static kCFRunLoopDefaultMode: *const c_void;
        pub static kCFTypeArrayCallBacks: c_void;
        pub fn CFStringCreateWithCString(alloc: *const c_void, s: *const i8, encoding: u32) -> *const c_void;
        pub fn CFArrayCreate(alloc: *const c_void, values: *const *const c_void, n: isize, cb: *const c_void) -> *const c_void;
        pub fn CFRelease(cf: *const c_void);
        pub fn CFRunLoopGetCurrent() -> *mut c_void;
        pub fn CFRunLoopRun();
        pub fn CFRunLoopStop(run_loop: *mut c_void);
    }
}

/// What the stream's callback works with; lives on the watcher thread's stack for the whole
/// run loop.
struct Watch<'a> {
    root: PathBuf,
    /// The root as FSEvents reports it: symlinks resolved (`/var` → `/private/var`).
    real_root: PathBuf,
    recursive: bool,
    sink: &'a mut dyn FnMut(WatchEvent) -> bool,
    run_loop: *mut c_void,
}

extern "C" fn on_events(
    _stream: *const c_void,
    info: *mut c_void,
    count: usize,
    paths: *mut c_void,
    flags: *const u32,
    _ids: *const u64,
) {
    // SAFETY: `info` is the `Watch` the thread passed in the context, alive while its run loop runs.
    let watch = unsafe { &mut *(info as *mut Watch) };
    let paths = paths as *const *const i8;
    let mut changes = Vec::new();
    let mut overflow = false;
    for i in 0..count {
        let (path, flag) = unsafe { (CStr::from_ptr(*paths.add(i)), *flags.add(i)) };
        if flag & (fs::MUST_SCAN_SUBDIRS | fs::USER_DROPPED | fs::KERNEL_DROPPED | fs::ROOT_CHANGED) != 0 {
            overflow = true;
            continue;
        }
        if flag & fs::ITEM_IS_FILE == 0 {
            continue;
        }
        let path = Path::new(path.to_str().unwrap_or_default());
        let Some(rel) = relative(path, &watch.real_root).or_else(|| relative(path, &watch.root)) else { continue };
        if !watch.recursive && rel.contains('/') {
            continue;
        }
        if let Some(action) = action_of(flag, path.exists()) {
            changes.push((action, rel));
        }
    }
    let go_on = (!overflow || (watch.sink)(WatchEvent::Overflow))
        && (changes.is_empty() || (watch.sink)(WatchEvent::Changes(changes)));
    if !go_on {
        unsafe { fs::CFRunLoopStop(watch.run_loop) };
    }
}

fn relative(path: &Path, root: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?.to_str()?;
    (!rel.is_empty()).then(|| rel.to_string())
}

/// FSEvents coalesces: one event may carry "created", "modified" and "removed" at once, and a
/// rename is flagged the same on both names. Whether the file is there now settles it.
fn action_of(flag: u32, exists: bool) -> Option<FileAction> {
    if flag & fs::ITEM_RENAMED != 0 {
        return Some(if exists { FileAction::RenamedNew } else { FileAction::RenamedOld });
    }
    if !exists {
        return (flag & (fs::ITEM_REMOVED | fs::ITEM_MODIFIED | fs::ITEM_CREATED) != 0).then_some(FileAction::Removed);
    }
    if flag & fs::ITEM_CREATED != 0 && flag & fs::ITEM_MODIFIED == 0 {
        return Some(FileAction::Added);
    }
    if flag & (fs::ITEM_MODIFIED | fs::ITEM_CREATED | fs::ITEM_INODE_META_MOD) != 0 {
        return Some(FileAction::Modified);
    }
    None
}

pub struct DirWatcher;

impl DirWatcher {
    /// A missing directory is `Err(NotFound)` here (the caller reports `Missing`). Then thread
    /// "dirwatch-<dir file name>" runs an FSEvents stream on its own run loop and hands every
    /// batch to `sink`; `sink` returning false ends the thread. Returns once the stream has
    /// started: changes from then on are reported, so an enumeration done after `spawn`
    /// returns cannot miss one.
    pub fn spawn(
        dir: &Path,
        recursive: bool,
        _mask: u32,
        mut sink: impl FnMut(WatchEvent) -> bool + Send + 'static,
    ) -> io::Result<()> {
        if !std::fs::metadata(dir)?.is_dir() {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let root = dir.to_path_buf();
        let real_root = std::fs::canonicalize(dir).unwrap_or_else(|_| root.clone());
        let c_root = CString::new(real_root.to_string_lossy().as_bytes()).map_err(io::Error::other)?;
        let name = format!(
            "dirwatch-{}",
            dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        );
        let (ready, started) = mpsc::sync_channel::<io::Result<()>>(1);
        std::thread::Builder::new().name(name).spawn(move || unsafe {
            let run_loop = fs::CFRunLoopGetCurrent();
            let mut watch = Watch { root, real_root, recursive, sink: &mut sink, run_loop };
            let context = fs::Context {
                version: 0,
                info: (&mut watch as *mut Watch).cast(),
                retain: std::ptr::null(),
                release: std::ptr::null(),
                copy_description: std::ptr::null(),
            };
            let path = fs::CFStringCreateWithCString(std::ptr::null(), c_root.as_ptr(), fs::UTF8);
            let paths = fs::CFArrayCreate(std::ptr::null(), &path, 1, (&fs::kCFTypeArrayCallBacks as *const c_void).cast());
            let stream = fs::FSEventStreamCreate(
                std::ptr::null(),
                on_events,
                &context,
                paths,
                fs::SINCE_NOW,
                0.05,
                fs::CREATE_FILE_EVENTS | fs::CREATE_NO_DEFER | fs::CREATE_WATCH_ROOT,
            );
            fs::CFRelease(paths);
            fs::CFRelease(path);
            if stream.is_null() {
                let _ = ready.send(Err(io::Error::other("FSEventStreamCreate failed")));
                return;
            }
            fs::FSEventStreamScheduleWithRunLoop(stream, run_loop, fs::kCFRunLoopDefaultMode);
            if fs::FSEventStreamStart(stream) == 0 {
                fs::FSEventStreamInvalidate(stream);
                fs::FSEventStreamRelease(stream);
                let _ = ready.send(Err(io::Error::other("FSEventStreamStart failed")));
                return;
            }
            let _ = ready.send(Ok(()));
            fs::CFRunLoopRun();
            fs::FSEventStreamStop(stream);
            fs::FSEventStreamInvalidate(stream);
            fs::FSEventStreamRelease(stream);
        })?;
        started
            .recv()
            .map_err(|_| io::Error::other("dirwatch thread ended before its stream started"))?
    }
}

pub mod proc {
    use std::collections::HashSet;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::sync::{Arc, Mutex};

    use crate::observe::keys::ProcId;

    /// A kqueue with the process's exit registered: readable once it has exited.
    pub type ProcHandle = OwnedFd;

    #[allow(dead_code)] // the same variants as on Windows
    #[derive(Debug)]
    pub enum ProcError {
        NotFound,
        AccessDenied,
        Exited,
        Os(io::Error),
    }

    /// Seconds between 1601-01-01 (FILETIME) and 1970-01-01 (Unix).
    const FILETIME_UNIX_OFFSET_S: u64 = 11_644_473_600;
    /// `SZOMB`: exited, not yet reaped.
    const ZOMBIE: u32 = 5;

    /// Proof that `pid` is a live process (spec §5.5 step 3): its exit registered on a kqueue
    /// first (from then on a reuse of the pid cannot fool us) → BSD info, not a zombie →
    /// start time as a FILETIME, the unit the rest of the observer compares → image path.
    pub fn identify(pid: u32) -> Result<(ProcId, ProcHandle), ProcError> {
        let handle = exit_queue(pid)?;
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let got = unsafe {
            libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size)
        };
        if got != size {
            return Err(match io::Error::last_os_error().raw_os_error() {
                Some(libc::ESRCH) => ProcError::NotFound,
                Some(libc::EPERM) => ProcError::AccessDenied,
                _ => ProcError::Exited,
            });
        }
        if info.pbi_status == ZOMBIE {
            return Err(ProcError::Exited);
        }
        // Whole seconds: Claude Code's `procStart` has no more on macOS, and the relay rounds
        // the same way, so all three agree on one process.
        let created_ft = unix_secs_ft(info.pbi_start_tvsec);
        let image = image_path(pid).map_err(ProcError::Os)?;
        Ok((ProcId { pid, created_ft, image }, handle))
    }

    fn unix_secs_ft(secs: u64) -> u64 {
        (secs + FILETIME_UNIX_OFFSET_S) * 10_000_000
    }

    /// `Sun Oct 11 17:58:11 2026` (UTC, the `procStart` Claude Code writes on macOS) as the
    /// FILETIME `identify` gives the same process.
    pub fn start_date_ft(text: &str) -> Option<u64> {
        let c = std::ffi::CString::new(text.trim()).ok()?;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        let end = unsafe { libc::strptime(c.as_ptr(), c"%a %b %d %H:%M:%S %Y".as_ptr(), &mut tm) };
        if end.is_null() || unsafe { *end } != 0 {
            return None;
        }
        let secs = unsafe { libc::timegm(&mut tm) };
        (secs > 0).then(|| unix_secs_ft(secs as u64))
    }

    fn exit_queue(pid: u32) -> Result<OwnedFd, ProcError> {
        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            return Err(ProcError::Os(io::Error::last_os_error()));
        }
        let fd = unsafe { OwnedFd::from_raw_fd(kq) };
        let change = libc::kevent {
            ident: pid as usize,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let rc = unsafe { libc::kevent(fd.as_raw_fd(), &change, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
        if rc < 0 {
            return Err(match io::Error::last_os_error().raw_os_error() {
                Some(libc::ESRCH) => ProcError::NotFound,
                Some(libc::EPERM) => ProcError::AccessDenied,
                _ => ProcError::Os(io::Error::last_os_error()),
            });
        }
        Ok(fd)
    }

    fn image_path(pid: u32) -> io::Result<String> {
        let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let len = unsafe { libc::proc_pidpath(pid as libc::c_int, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if len <= 0 {
            return Err(io::Error::last_os_error());
        }
        buf.truncate(len as usize);
        String::from_utf8(buf).map_err(io::Error::other)
    }

    /// Blocks until the process behind `handle` exits.
    fn wait_exit(handle: &ProcHandle) -> io::Result<()> {
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        loop {
            let n = unsafe { libc::kevent(handle.as_raw_fd(), std::ptr::null(), 0, &mut event, 1, std::ptr::null()) };
            if n > 0 {
                return Ok(());
            }
            let err = io::Error::last_os_error();
            if n < 0 && err.kind() != io::ErrorKind::Interrupted {
                return Err(err);
            }
        }
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

        /// The same contract as on Windows: false (and `handle` dropped) when already watched
        /// or when the waiter thread could not start; otherwise thread "proc-exit-<pid>" waits
        /// for the exit, removes the entry, then calls `on_exit(pid, created_ft)`.
        pub fn watch(&self, id: &ProcId, handle: ProcHandle, on_exit: impl FnOnce(u32, u64) + Send + 'static) -> bool {
            let key = (id.pid, id.created_ft);
            if !self.watched.lock().unwrap().insert(key) {
                return false;
            }
            let watched = Arc::clone(&self.watched);
            let spawned = std::thread::Builder::new().name(format!("proc-exit-{}", id.pid)).spawn(move || {
                let result = wait_exit(&handle);
                drop(handle);
                watched.lock().unwrap().remove(&key);
                match result {
                    Ok(()) => on_exit(key.0, key.1),
                    Err(err) => crate::log::line(format!("proc-exit wait failed pid={}: {err}", key.0)),
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
    use std::time::Duration;

    #[test]
    fn coalesced_flags_are_settled_by_whether_the_file_is_there() {
        assert_eq!(action_of(fs::ITEM_CREATED, true), Some(FileAction::Added));
        assert_eq!(action_of(fs::ITEM_CREATED | fs::ITEM_MODIFIED, true), Some(FileAction::Modified));
        assert_eq!(action_of(fs::ITEM_CREATED | fs::ITEM_REMOVED, false), Some(FileAction::Removed));
        assert_eq!(action_of(fs::ITEM_RENAMED, true), Some(FileAction::RenamedNew));
        assert_eq!(action_of(fs::ITEM_RENAMED, false), Some(FileAction::RenamedOld));
        assert_eq!(action_of(0, true), None);
    }

    #[test]
    fn a_new_file_and_an_append_are_reported_relative_to_the_root() {
        let dir = std::env::temp_dir().join(format!("mayak-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let (tx, rx) = mpsc::channel();
        DirWatcher::spawn(&dir, true, MASK_NAME_WRITE_SIZE, move |ev| tx.send(ev).is_ok()).unwrap();
        std::fs::write(dir.join("sub/a.jsonl"), b"{}\n").unwrap();
        let mut seen = Vec::new();
        while let Ok(ev) = rx.recv_timeout(Duration::from_secs(5)) {
            if let WatchEvent::Changes(c) = ev {
                seen.extend(c);
            }
            if seen.iter().any(|(_, rel)| rel == "sub/a.jsonl") {
                break;
            }
        }
        assert!(seen.iter().any(|(_, rel)| rel == "sub/a.jsonl"), "{seen:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_directory_is_not_found() {
        let err = DirWatcher::spawn(Path::new("/nonexistent/mayak"), false, 0, |_| true).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_child_is_proved_alive_and_its_exit_is_seen() {
        let mut child = std::process::Command::new("/bin/sleep").arg("0.3").spawn().unwrap();
        let (id, handle) = proc::identify(child.id()).expect("child is running");
        assert!(id.image.ends_with("/sleep"), "{}", id.image);
        let (tx, rx) = mpsc::channel();
        let registry = proc::Registry::new();
        assert!(registry.watch(&id, handle, move |pid, _| tx.send(pid).unwrap()));
        assert!(registry.is_watched(id.pid, id.created_ft));
        let _ = child.wait();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), id.pid);
    }

    #[test]
    fn claude_codes_start_date_names_the_same_second_as_identify() {
        let me = proc::identify(std::process::id()).unwrap().0;
        let out = std::process::Command::new("/bin/ps")
            .args(["-o", "lstart=", "-p", &std::process::id().to_string()])
            .env("LC_ALL", "C")
            .env("TZ", "UTC")
            .output()
            .unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        assert_eq!(proc::start_date_ft(&text), Some(me.created_ft), "{text:?}");
        assert_eq!(proc::start_date_ft("not a date"), None);
    }

    #[test]
    fn a_dead_pid_is_not_found() {
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(proc::identify(pid).is_err());
    }
}
