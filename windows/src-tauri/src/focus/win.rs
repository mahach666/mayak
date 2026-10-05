//! The Win32 of the window switch: process table, top-level windows, raising a window.
//! Everything that is not Win32 (the ancestor walk, the host choice) is a pure function.

use super::WindowInfo;

use std::time::Duration;

use ::windows::core::BOOL;
use ::windows::Win32::Foundation::{CloseHandle, FILETIME, HWND, LPARAM};
use ::windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use ::windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use ::windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FlashWindowEx, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, SetForegroundWindow, ShowWindow, FLASHWINFO, FLASHW_ALL, FLASHW_TIMERNOFG, SW_RESTORE,
};

/// How far above the agent the host process is looked for (spec §9).
pub const MAX_ANCESTORS: usize = 8;

/// How long after the raise the foreground window is checked.
const SETTLE: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcRow {
    pub pid: u32,
    pub parent_pid: u32,
    /// Image file name without directory.
    pub exe_name: String,
}

/// Toolhelp32 snapshot of every process. Empty on failure.
pub fn process_table() -> Vec<ProcRow> {
    let mut rows = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return rows };
        let mut entry =
            PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
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
        let (mut created, mut exited, mut kernel, mut user) =
            (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let ok = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(process);
        ok.then(|| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }
}

/// Ancestors of `pid`, nearest first. A parent must exist in the table with a known creation time
/// and be strictly older than its child (a younger «parent» is a reused pid). The walk never
/// includes `explorer.exe` and never goes past it: the shell is not the host of an agent.
pub fn ancestors(rows: &[ProcRow], created: impl Fn(u32) -> Option<u64>, pid: u32, max: usize) -> Vec<u32> {
    let mut chain = Vec::new();
    let Some(mut child) = rows.iter().find(|r| r.pid == pid) else { return chain };
    let Some(mut child_ft) = created(pid) else { return chain };
    while chain.len() < max {
        let Some(parent) = rows.iter().find(|r| r.pid == child.parent_pid) else { break };
        if parent.exe_name.eq_ignore_ascii_case("explorer.exe") {
            break;
        }
        let Some(parent_ft) = created(parent.pid) else { break };
        if parent_ft >= child_ft {
            break;
        }
        chain.push(parent.pid);
        child = parent;
        child_ft = parent_ft;
    }
    chain
}

/// The nearest process of `chain` (nearest first) that owns a window.
pub fn host_pid_from(chain: &[u32], windows: &[(u32, WindowInfo)]) -> Option<u32> {
    chain.iter().copied().find(|pid| windows.iter().any(|(owner, _)| owner == pid))
}

/// Visible, titled top-level windows with their owner pid, topmost first (EnumWindows order).
pub fn top_level_windows() -> Vec<(u32, WindowInfo)> {
    let mut out: Vec<(u32, WindowInfo)> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect_window), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = unsafe { &mut *(lparam.0 as *mut Vec<(u32, WindowInfo)>) };
    if unsafe { IsWindowVisible(hwnd) }.as_bool() {
        let mut buf = [0u16; 512];
        let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
        if len > 0 {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            let title = String::from_utf16_lossy(&buf[..len as usize]);
            let minimized = unsafe { IsIconic(hwnd) }.as_bool();
            out.push((pid, WindowInfo { hwnd: hwnd.0 as isize, title, minimized }));
        }
    }
    true.into()
}

/// Windows of the host process of the agent `agent_pid`, topmost first (empty when no ancestor
/// within `MAX_ANCESTORS` owns a window).
#[allow(dead_code)] // used by session_open in Task 4
pub fn host_windows(agent_pid: u32) -> Vec<WindowInfo> {
    let chain = ancestors(&process_table(), creation_ft, agent_pid, MAX_ANCESTORS);
    let all = top_level_windows();
    let Some(host) = host_pid_from(&chain, &all) else { return Vec::new() };
    all.into_iter().filter(|(pid, _)| *pid == host).map(|(_, w)| w).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raise {
    Focused,
    Flashed,
    Failed,
}

/// Restore a minimized window, bring it to the foreground and check the result after `SETTLE`.
/// When Windows refuses the foreground, the taskbar button flashes instead (`Flashed`).
/// `Failed` means the handle is stale (the window is gone or hidden): the caller falls back.
#[allow(dead_code)] // used by session_open in Task 4
pub fn raise(hwnd: isize) -> Raise {
    let target = HWND(hwnd as *mut _);
    unsafe {
        let minimized = IsIconic(target).as_bool();
        if !minimized && !IsWindowVisible(target).as_bool() {
            return Raise::Failed;
        }
        if minimized {
            let _ = ShowWindow(target, SW_RESTORE);
        }
        let _ = SetForegroundWindow(target);
    }
    std::thread::sleep(SETTLE);
    unsafe {
        if GetForegroundWindow() == target {
            return Raise::Focused;
        }
        // FlashWindowEx returns the previous flash state, not success: nothing to check.
        let info = FLASHWINFO {
            cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
            hwnd: target,
            dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
            uCount: 3,
            dwTimeout: 0,
        };
        let _ = FlashWindowEx(&info);
    }
    Raise::Flashed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, parent_pid: u32, exe: &str) -> ProcRow {
        ProcRow { pid, parent_pid, exe_name: exe.into() }
    }

    fn win(pid: u32, hwnd: isize, title: &str) -> (u32, WindowInfo) {
        (pid, WindowInfo { hwnd, title: title.into(), minimized: false })
    }

    /// agent(40) <- extension host(30) <- editor main(20) <- explorer(10)
    fn rows() -> Vec<ProcRow> {
        vec![
            row(10, 1, "explorer.exe"),
            row(20, 10, "Cursor.exe"),
            row(30, 20, "Cursor.exe"),
            row(40, 30, "claude.exe"),
        ]
    }

    fn born(pid: u32) -> Option<u64> {
        Some(u64::from(pid)) // a larger pid is younger in these tables
    }

    #[test]
    fn the_walk_goes_up_nearest_first_and_stops_before_explorer() {
        assert_eq!(ancestors(&rows(), born, 40, 8), vec![30, 20]);
    }

    #[test]
    fn the_walk_stops_at_a_parent_that_is_not_older() {
        // pid 20 reports parent 30, which is younger: a reused pid, not a parent
        let rows = vec![row(20, 30, "a.exe"), row(30, 1, "b.exe")];
        assert_eq!(ancestors(&rows, born, 20, 8), Vec::<u32>::new());
    }

    #[test]
    fn the_walk_respects_the_step_limit_and_unknown_pids() {
        assert_eq!(ancestors(&rows(), born, 40, 1), vec![30]);
        assert_eq!(ancestors(&rows(), born, 999, 8), Vec::<u32>::new());
        assert_eq!(ancestors(&rows(), |_| None, 40, 8), Vec::<u32>::new());
    }

    #[test]
    fn the_host_is_the_nearest_ancestor_with_a_window() {
        let windows = vec![win(20, 100, "x - app - Cursor"), win(77, 101, "Unrelated")];
        assert_eq!(host_pid_from(&[30, 20], &windows), Some(20));
        assert_eq!(host_pid_from(&[30], &windows), None);
        assert_eq!(host_pid_from(&[], &windows), None);
    }

    #[test]
    fn explorer_windows_never_become_the_host() {
        // the chain of an agent never contains explorer.exe (`ancestors` stops before it),
        // so a File Explorer window of pid 10 can only match if the caller adds it
        let windows = vec![win(10, 200, "Documents")];
        assert_eq!(host_pid_from(&ancestors(&rows(), born, 40, 8), &windows), None);
    }

    #[test]
    fn the_enumeration_runs_and_sees_no_untitled_window() {
        let all = top_level_windows();
        assert!(all.iter().all(|(_, w)| !w.title.is_empty()));
    }

    #[test]
    fn the_current_process_is_in_the_table_with_a_creation_time() {
        let me = std::process::id();
        assert!(process_table().iter().any(|r| r.pid == me));
        assert!(creation_ft(me).is_some());
    }
}
