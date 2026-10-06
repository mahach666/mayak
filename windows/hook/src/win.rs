//! The Win32 the relay needs: who we are, who is on the other end of the pipe,
//! and which `codex.exe` started us.
//!
//! Named pipes live in a machine-wide namespace, so `\\.\pipe\mayak-<name>` can
//! be created by *any* account that gets there first. Two defences, both cheap:
//! the pipe name carries our SID, and once connected we check the server process
//! really belongs to us before sending anything.

use std::time::{Duration, Instant};

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, LocalFree, FILETIME, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken,
    QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::CONNECT_TIMEOUT;

/// `ERROR_PIPE_BUSY` — every instance is serving someone else right now. This is
/// the one error worth retrying: the server exists and a slot will free up.
const ERROR_PIPE_BUSY: i32 = 231;

/// `\\.\pipe\mayak-<sid>`. The SID keeps two accounts on the same machine from
/// ever meeting on the same pipe; the name falls back to the user name only if
/// the SID cannot be read at all, which should not happen.
pub fn pipe_path() -> String {
    let key = current_user_sid()
        .unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_else(|_| "user".into()));
    format!(r"\\.\pipe\{}{key}", crate::ident::PIPE_PREFIX)
}

/// Opens the pipe `path` (`pipe_path()`, or a test pipe). Retries only while the
/// server is busy: any other error means there is nothing to talk to, and
/// waiting would only delay the agent.
pub fn connect(path: &str) -> Option<std::fs::File> {
    use std::os::windows::io::AsRawHandle;
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) => {
                let handle = HANDLE(file.as_raw_handle());
                // Somebody else's server on our pipe name gets nothing from us.
                return pipe_server_is_same_user(handle).then_some(file);
            }
            Err(err) => {
                if err.raw_os_error() != Some(ERROR_PIPE_BUSY) || Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        }
    }
}

/// The SID of the account this process runs as, as `S-1-5-21-…`.
pub fn current_user_sid() -> Option<String> {
    unsafe { token_sid(GetCurrentProcess()) }
}

/// True when the process serving `handle` runs as the same user we do.
///
/// A failure to answer is treated as "not ours": refusing to talk to a pipe we
/// cannot vouch for costs one hook event, while trusting it could hand another
/// account on this machine the contents of every tool call.
pub fn pipe_server_is_same_user(handle: HANDLE) -> bool {
    let Some(mine) = current_user_sid() else { return false };
    unsafe {
        let mut pid = 0u32;
        if GetNamedPipeServerProcessId(handle, &mut pid).is_err() || pid == 0 {
            return false;
        }
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let theirs = token_sid(process);
        let _ = CloseHandle(process);
        theirs.as_deref() == Some(mine.as_str())
    }
}

/// The user SID behind a process handle. `process` is borrowed, never closed.
unsafe fn token_sid(process: HANDLE) -> Option<String> {
    let mut token = HANDLE::default();
    OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;

    // First call sizes the buffer, second fills it.
    let mut needed = 0u32;
    let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
    if needed == 0 {
        let _ = CloseHandle(token);
        return None;
    }
    let mut buf = vec![0u8; needed as usize];
    let ok = GetTokenInformation(
        token,
        TokenUser,
        Some(buf.as_mut_ptr().cast()),
        needed,
        &mut needed,
    )
    .is_ok();
    let _ = CloseHandle(token);
    if !ok {
        return None;
    }

    let user = &*(buf.as_ptr() as *const TOKEN_USER);
    let mut text = PWSTR::null();
    ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
    let sid = text.to_string().ok();
    let _ = LocalFree(Some(HLOCAL(text.0 as *mut _)));
    sid
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcRow {
    pub pid: u32,
    pub parent_pid: u32,
    /// `szExeFile`: the image file name, no directory.
    pub exe_name: String,
}

/// Toolhelp32 snapshot of every process. Empty on failure.
pub fn process_table() -> Vec<ProcRow> {
    let mut rows = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return rows;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
                rows.push(ProcRow {
                    pid: entry.th32ProcessID,
                    parent_pid: entry.th32ParentProcessID,
                    exe_name: String::from_utf16_lossy(&entry.szExeFile[..len]),
                });
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    rows
}

/// Creation time of `pid` as a FILETIME, `(high << 32) | low`.
pub fn creation_ft(pid: u32) -> Option<u64> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let ok = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(process);
        ok.then(|| ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64)
    }
}

/// Full Win32 path of the image of `pid`.
pub fn image_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 32_768];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
            .is_ok();
        let _ = CloseHandle(process);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// Pure ancestor walk over a process table. Starts at `pid` (its row and
/// creation time must exist, else empty). Each step needs the parent's row and
/// creation time, and the parent must be strictly older than the child: a
/// younger "parent" is a reused PID, and the walk stops there. Nearest first.
pub fn ancestors(
    rows: &[ProcRow],
    created: impl Fn(u32) -> Option<u64>,
    pid: u32,
    max: usize,
) -> Vec<(ProcRow, u64)> {
    let mut chain = Vec::new();
    let Some(mut child) = rows.iter().find(|r| r.pid == pid) else { return chain };
    let Some(mut child_ft) = created(pid) else { return chain };
    while chain.len() < max {
        let Some(parent) = rows.iter().find(|r| r.pid == child.parent_pid) else { break };
        let Some(parent_ft) = created(parent.pid) else { break };
        if parent_ft >= child_ft {
            break;
        }
        chain.push((parent.clone(), parent_ft));
        child = parent;
        child_ft = parent_ft;
    }
    chain
}

fn is_codex(exe_name: &str) -> bool {
    exe_name.eq_ignore_ascii_case("codex.exe")
}

/// The nearest `codex.exe` among the first MAX_ANCESTOR_STEPS ancestors of `pid`.
pub fn find_codex_ancestor(pid: u32) -> Option<crate::AgentProc> {
    ancestors(&process_table(), creation_ft, pid, crate::MAX_ANCESTOR_STEPS)
        .into_iter()
        .find(|(row, _)| is_codex(&row.exe_name))
        .map(|(row, created_ft)| crate::AgentProc {
            pid: row.pid,
            created_ft,
            image: image_path(row.pid),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, parent_pid: u32, exe: &str) -> ProcRow {
        ProcRow { pid, parent_pid, exe_name: exe.into() }
    }

    fn table() -> Vec<ProcRow> {
        vec![
            row(100, 4, "explorer.exe"),
            row(200, 100, "Cursor.exe"),
            row(300, 200, "codex.exe"),
            row(400, 300, "powershell.exe"),
            row(500, 400, "mayak-hook.exe"),
        ]
    }

    fn created(pid: u32) -> Option<u64> {
        match pid {
            100 => Some(1_000),
            200 => Some(2_000),
            300 => Some(3_000),
            400 => Some(4_000),
            500 => Some(5_000),
            _ => None,
        }
    }

    #[test]
    fn walks_up_nearest_first_until_the_table_ends() {
        let chain = ancestors(&table(), created, 500, 6);
        let pids: Vec<u32> = chain.iter().map(|(r, _)| r.pid).collect();
        assert_eq!(pids, vec![400, 300, 200, 100]);
        assert_eq!(chain[1], (row(300, 200, "codex.exe"), 3_000));
    }

    #[test]
    fn stops_after_max_steps() {
        let pids: Vec<u32> = ancestors(&table(), created, 500, 2).iter().map(|(r, _)| r.pid).collect();
        assert_eq!(pids, vec![400, 300]);
    }

    #[test]
    fn a_reused_parent_pid_ends_the_walk() {
        // PID 300 now belongs to a process younger than its "child" 400.
        let reused = |pid: u32| if pid == 300 { Some(9_000) } else { created(pid) };
        let pids: Vec<u32> = ancestors(&table(), reused, 500, 6).iter().map(|(r, _)| r.pid).collect();
        assert_eq!(pids, vec![400]);
        // Same creation time is not older either.
        let same = |pid: u32| if pid == 400 { Some(5_000) } else { created(pid) };
        assert!(ancestors(&table(), same, 500, 6).is_empty());
    }

    #[test]
    fn unknown_start_or_unreadable_times_give_nothing_more() {
        assert!(ancestors(&table(), created, 999, 6).is_empty());
        assert!(ancestors(&table(), |_| None, 500, 6).is_empty());
        let gap = |pid: u32| if pid == 200 { None } else { created(pid) };
        let pids: Vec<u32> = ancestors(&table(), gap, 500, 6).iter().map(|(r, _)| r.pid).collect();
        assert_eq!(pids, vec![400, 300]);
    }

    #[test]
    fn codex_is_matched_by_exact_name_ignoring_ascii_case() {
        assert!(is_codex("codex.exe"));
        assert!(is_codex("Codex.EXE"));
        assert!(!is_codex("codex.exe.bak"));
        assert!(!is_codex("mycodex.exe"));
    }

    #[test]
    fn the_real_process_table_knows_this_process_and_its_parent() {
        let me = std::process::id();
        let rows = process_table();
        assert!(rows.iter().any(|r| r.pid == me));
        assert!(creation_ft(me).is_some());
        let image = image_path(me).expect("own image");
        let exe = std::env::current_exe().unwrap();
        assert!(image.eq_ignore_ascii_case(&exe.to_string_lossy()), "{image} vs {exe:?}");
        assert!(!ancestors(&rows, creation_ft, me, 6).is_empty());
    }
}
