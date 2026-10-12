//! The macOS the relay needs: the socket Mayak listens on, who is on the other
//! end of it, and which `codex` or Antigravity started us. The same names as
//! `win.rs`, so `main.rs` and `lib.rs` do not care which one they got.
//!
//! The socket lives in the per-user temporary folder (`confstr
//! _CS_DARWIN_USER_TEMP_DIR`, `/var/folders/…/T/`), which is ours and closed to
//! everyone else, so nobody else can even reach it. We still check, once
//! connected, that the process on the other end runs as us — the same promise
//! the Windows relay makes with the pipe server's SID.

use std::ffi::CStr;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::CONNECT_TIMEOUT;
pub use crate::procs::{ancestors, ProcRow};

/// File name of the socket inside the per-user temporary folder.
const SOCKET_NAME: &str = "mayak.sock";

/// Seconds between 1601-01-01 (FILETIME) and 1970-01-01 (Unix).
const FILETIME_UNIX_OFFSET_S: u64 = 11_644_473_600;

/// `<per-user temp folder>/mayak.sock`, or an empty string when that folder
/// cannot be read or is not private: `connect` then finds nothing to talk to.
/// Must match `platform::relay_socket_path()` in the app exactly.
pub fn pipe_path() -> String {
    user_temp_dir()
        .filter(|dir| is_private_dir(dir))
        .map(|dir| dir.join(SOCKET_NAME).to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `/var/folders/…/T/`, from the system rather than `$TMPDIR`, so a hook
/// started from a stripped-down environment finds the same socket as the app.
fn user_temp_dir() -> Option<PathBuf> {
    let mut buf = vec![0u8; 1024];
    let len = unsafe {
        libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, buf.as_mut_ptr().cast(), buf.len())
    };
    if len == 0 || len > buf.len() {
        return None;
    }
    let text = CStr::from_bytes_until_nul(&buf).ok()?.to_str().ok()?;
    Some(PathBuf::from(text))
}

/// A real directory (not a symlink), owned by us, no access for group or others.
fn is_private_dir(dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(dir)
        .map(|m| {
            m.file_type().is_dir() && m.uid() == unsafe { libc::getuid() } && m.mode() & 0o077 == 0
        })
        .unwrap_or(false)
}

/// Opens the socket at `path` (`pipe_path()`, or a test socket). Retries only
/// while the app's backlog is full: any other error — no socket, nobody
/// listening — means there is nothing to talk to, and waiting would only delay
/// the agent.
pub fn connect(path: &str) -> Option<UnixStream> {
    if path.is_empty() {
        return None;
    }
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match UnixStream::connect(path) {
            // Somebody else's server on our socket gets nothing from us.
            Ok(stream) => return server_is_same_user(&stream).then_some(stream),
            Err(err) => {
                if err.raw_os_error() != Some(libc::EAGAIN) || Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        }
    }
}

/// True when the process serving the socket runs as the same user we do.
/// A failure to answer is treated as "not ours", as on Windows.
fn server_is_same_user(stream: &UnixStream) -> bool {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    rc == 0 && uid == unsafe { libc::getuid() }
}

/// The BSD info of `pid`: parent, short name, start time.
fn bsd_info(pid: u32) -> Option<libc::proc_bsdinfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let got = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (got == size).then_some(info)
}

/// Every process we can see. Empty on failure. The name is the image file name
/// from the full path when it can be read, else the kernel's short name.
pub fn process_table() -> Vec<ProcRow> {
    // First call sizes the list; leave room for processes started meanwhile.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    let mut pids = vec![0 as libc::pid_t; count as usize + 64];
    let bytes = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    let got = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    if got <= 0 {
        return Vec::new();
    }
    pids.truncate(got as usize);
    pids.into_iter()
        .filter(|pid| *pid > 0)
        .filter_map(|pid| {
            let pid = pid as u32;
            let info = bsd_info(pid)?;
            let exe_name = image_path(pid)
                .and_then(|p| Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| c_chars(&info.pbi_comm));
            Some(ProcRow { pid, parent_pid: info.pbi_ppid, exe_name })
        })
        .collect()
}

fn c_chars(chars: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = chars.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Start time of `pid` in FILETIME units (100 ns since 1601), the unit the app
/// already compares on Windows. Microseconds, so the ancestor walk can tell a
/// parent from a child started in the same second.
pub fn creation_ft(pid: u32) -> Option<u64> {
    let info = bsd_info(pid)?;
    Some((info.pbi_start_tvsec + FILETIME_UNIX_OFFSET_S) * 10_000_000 + info.pbi_start_tvusec * 10)
}

/// Full path of the image of `pid`.
pub fn image_path(pid: u32) -> Option<String> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let len = unsafe { libc::proc_pidpath(pid as libc::c_int, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if len <= 0 {
        return None;
    }
    buf.truncate(len as usize);
    String::from_utf8(buf).ok()
}

fn is_codex(exe_name: &str) -> bool {
    exe_name == "codex"
}

/// The nearest `codex` among the first MAX_ANCESTOR_STEPS ancestors of `pid`.
pub fn find_codex_ancestor(pid: u32) -> Option<crate::AgentProc> {
    find_ancestor(pid, is_codex)
}

/// Image names of the Antigravity processes a hook runs under: the IDE, the `agy` CLI.
const ANTIGRAVITY_EXES: &[&str] = &["Antigravity", "agy"];

fn is_antigravity(exe_name: &str) -> bool {
    ANTIGRAVITY_EXES.contains(&exe_name)
}

/// The nearest Antigravity or `agy` among the first MAX_ANCESTOR_STEPS ancestors of `pid`.
pub fn find_antigravity_ancestor(pid: u32) -> Option<crate::AgentProc> {
    find_ancestor(pid, is_antigravity)
}

/// The agent as the app will prove it: its start time to the second, which is
/// all `proc::identify` and Claude Code's `procStart` know on macOS.
fn find_ancestor(pid: u32, matches: fn(&str) -> bool) -> Option<crate::AgentProc> {
    crate::procs::find_ancestor(&process_table(), creation_ft, image_path, pid, matches)
        .map(|p| crate::AgentProc { created_ft: p.created_ft - p.created_ft % 10_000_000, ..p })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_is_in_a_private_per_user_folder() {
        let path = pipe_path();
        assert!(path.ends_with("/mayak.sock"), "{path}");
        assert!(is_private_dir(Path::new(&path).parent().unwrap()));
    }

    #[test]
    fn codex_and_antigravity_are_matched_by_exact_name() {
        assert!(is_codex("codex"));
        assert!(!is_codex("codex-helper"));
        assert!(is_antigravity("Antigravity"));
        assert!(is_antigravity("agy"));
        assert!(!is_antigravity("Antigravity Helper"));
    }

    #[test]
    fn the_real_process_table_knows_this_process_and_its_parent() {
        let me = std::process::id();
        let rows = process_table();
        assert!(rows.iter().any(|r| r.pid == me));
        assert!(creation_ft(me).is_some());
        let image = image_path(me).expect("own image");
        let exe = std::env::current_exe().unwrap();
        assert_eq!(std::fs::canonicalize(&image).unwrap(), std::fs::canonicalize(exe).unwrap());
        assert!(!ancestors(&rows, creation_ft, me, 6).is_empty());
    }

    #[test]
    fn a_listening_socket_of_ours_is_accepted_and_a_missing_one_is_not() {
        let dir = std::env::temp_dir().join(format!("mayak-hook-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("t.sock");
        let _ = std::fs::remove_file(&path);
        assert!(connect(&path.to_string_lossy()).is_none());
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(connect(&path.to_string_lossy()).is_some());
        assert!(connect("").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
