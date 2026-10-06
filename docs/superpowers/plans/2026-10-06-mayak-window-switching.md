# Mayak sub-project 2: window switching Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A click on «↗ Open» for a session brings the window where that session runs to the foreground (Cursor/VS Code window of the project folder, Claude Desktop, Codex Desktop), with today's launcher as the fallback.

**Architecture:** A new module `focus` with a pure part (`pick_window`, ancestor walk, host-window choice, fallback choice — all table-tested) and a thin Win32 part (`focus::win`: process table, top-level windows, raise). `session_open(key)` keeps its signature; it resolves the target, then does the switch on a short-lived thread. Everything happens on click: no timers, no polling.

**Tech Stack:** Rust, `windows` crate 0.61 (already a dependency; one new feature flag `Win32_System_Diagnostics_ToolHelp`), Tauri 2 commands. No new crates.

**Spec:** `docs/superpowers/specs/2026-10-06-mayak-window-switching-design.md` (Russian; new spec text in Russian). Sub-project 1 spec for context: `docs/superpowers/specs/2026-10-01-mayak-multi-session-core-design.md` (§13 extension points).

## Global Constraints

- Windows only; the Linux sources are not built (D84).
- 0 % CPU while the island is hidden: no timers, no polling, no new threads that outlive a click.
- Nothing is written into other tools' configs; `~/.claude/ide/*.lock` is never read.
- Window titles, folder names and paths are never logged: `mayak.log` gets only `host` and the outcome kind.
- `session_open(key)` keeps its signature `(key: String) -> bool`.
- No new crates; the only `Cargo.toml` change is the `windows` feature `Win32_System_Diagnostics_ToolHelp`.
- A relative or non-absolute project root never reaches a launcher (bef6558).
- Commit messages: conventional, one line, **no trailers** (no Co-Authored-By, no «Generated with»), never mention an AI assistant anywhere. Do not push.
- Build environment: `windows/target/debug` may hold build-script outputs with an old repo path. Always run cargo with `CARGO_TARGET_DIR=C:/tmp/mayak-target` and **never** touch `windows/target/release` or stop a running `mayak.exe`:
  `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace`
- Baseline before this plan: `cargo test --workspace` = 291 passed + 32 passed, 0 warnings.
- Never open `*.key`, `ide/*.lock`, `auth.json`, `claude_desktop_config.json`; never edit `~/.codex` or `~/.claude`.
- Branch: `feature/window-switching` (already created from `feature/multi-session-core`, spec at `7c59063`).

## Review Focus

1. Two editor windows with the same folder name → `Pick::Many`, never a guess; the fallback (launcher with the full root) handles it (Task 1 test).
2. A folder name that is only part of another folder name (`app` vs `app2`, `myapp`) or appears elsewhere in the title must not match (Task 1 test).
3. The ancestor walk reaching `explorer.exe` must never select a File Explorer window as the «host» (Task 3 test of `host_pid_from`).
4. A project root without a folder name (`C:\`) or a window title that is only `Cursor` → `Pick::None` (Task 1 test).
5. A window titles/paths must never reach `mayak.log` (Task 4 test of the log-line builder).

---

## File Structure

- Create `windows/src-tauri/src/focus/mod.rs` — pure part (`WindowInfo`, `Pick`, `pick_window`, `Outcome`, `Fallback`, `fallback_for`, `log_line`) and the orchestration `switch_to`.
- Create `windows/src-tauri/src/focus/win.rs` — the only Win32/`unsafe` of this sub-project: `process_table`, `creation_ft`, pure `ancestors` and `host_pid_from`, `top_level_windows`, `host_windows`, `raise`.
- Modify `windows/src-tauri/src/observe/model.rs` — `SessionView.agent_pid` (not serialized).
- Modify `windows/src-tauri/src/lib.rs` — `mod focus;`, `session_agent_pid`, new `session_open`, test helper `session()` gets the new field.
- Modify `windows/src-tauri/Cargo.toml` — one `windows` feature.
- Modify `docs/superpowers/specs/2026-10-06-mayak-window-switching-design.md` — spike results and the two sentences named in Task 6.

---

### Task 1: Pure window choice (`pick_window`)

**Files:**
- Create: `windows/src-tauri/src/focus/mod.rs`
- Modify: `windows/src-tauri/src/lib.rs` (add `mod focus;` after `mod files;`)

**Interfaces:**
- Consumes: `crate::observe::keys::HostKind` (`ClaudeDesktop | Cursor | VsCode | CodexDesktop | Unknown`).
- Produces (later tasks rely on these exact names):
  `pub struct WindowInfo { pub hwnd: isize, pub title: String, pub minimized: bool }`
  `pub enum Pick { One(isize), Many, None }`
  `pub fn pick_window(host: HostKind, root: &str, windows: &[WindowInfo]) -> Pick` — `windows` is in Z-order, topmost first.

- [ ] **Step 1: Create the module with the tests only**

`windows/src-tauri/src/focus/mod.rs`:

```rust
//! Switching to the window of a session (sub-project 2, spec 2026-10-06-mayak-window-switching-design.md).
//! The pure part lives here: choosing a window from data, choosing a fallback, naming outcomes.
//! The Win32 part is in `win.rs`.

use crate::observe::keys::HostKind;

/// A top-level window of the host process, as `focus::win` reports it (Z-order, topmost first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub title: String,
    pub minimized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    One(isize),
    Many,
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(hwnd: isize, title: &str) -> WindowInfo {
        WindowInfo { hwnd, title: title.into(), minimized: false }
    }

    #[test]
    fn an_editor_window_is_chosen_by_the_folder_before_the_app_suffix() {
        let wins = [w(1, "Refactor filters - other - Cursor"), w(2, "Fix the bug - app - Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins), Pick::One(2));
        let vs = [w(7, "main.rs - app - Visual Studio Code"), w(8, "lib.rs - other - Visual Studio Code")];
        assert_eq!(pick_window(HostKind::VsCode, r"C:\src\app", &vs), Pick::One(7));
    }

    #[test]
    fn the_folder_name_is_matched_whole_and_without_case() {
        let wins = [w(1, "app2 - Cursor"), w(2, "myapp - Cursor"), w(3, "x - app (Workspace) - Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins), Pick::None);
        let wins = [w(1, "Задача - ПРОЕКТ - Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\проект", &wins), Pick::One(1));
        let wins = [w(1, "a - APP - cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins), Pick::One(1));
    }

    #[test]
    fn a_name_elsewhere_in_the_title_does_not_match() {
        let wins = [w(1, "app - notes.md - other - Cursor"), w(2, "Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins), Pick::None);
    }

    #[test]
    fn two_windows_of_one_folder_are_never_guessed() {
        let wins = [w(1, "a.rs - app - Cursor"), w(2, "b.rs - app - Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins), Pick::Many);
    }

    #[test]
    fn a_root_without_a_folder_name_matches_nothing() {
        let wins = [w(1, "- Cursor"), w(2, "Cursor")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\", &wins), Pick::None);
        assert_eq!(pick_window(HostKind::Cursor, "", &wins), Pick::None);
    }

    #[test]
    fn a_suffix_of_another_app_is_not_this_editor() {
        let wins = [w(1, "x - app - Visual Studio Code")];
        assert_eq!(pick_window(HostKind::Cursor, r"C:\src\app", &wins), Pick::None);
    }

    #[test]
    fn desktop_apps_take_their_topmost_window() {
        let wins = [w(5, "Claude"), w(6, "Claude")];
        assert_eq!(pick_window(HostKind::ClaudeDesktop, r"C:\src\app", &wins), Pick::One(5));
        assert_eq!(pick_window(HostKind::CodexDesktop, r"C:\src\app", &wins), Pick::One(5));
        assert_eq!(pick_window(HostKind::CodexDesktop, r"C:\src\app", &[]), Pick::None);
    }

    #[test]
    fn an_unknown_host_picks_nothing() {
        assert_eq!(pick_window(HostKind::Unknown, r"C:\src\app", &[w(1, "app - Cursor")]), Pick::None);
    }
}
```

Add to `windows/src-tauri/src/lib.rs` after the line `mod files;`:

```rust
mod focus;
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak focus:: 2>&1 | tail -15`
Expected: FAIL to compile with ``error[E0425]: cannot find function `pick_window` in this scope``.

- [ ] **Step 3: Implement `pick_window`**

Insert above `#[cfg(test)]` in `focus/mod.rs`:

```rust
const CURSOR_SUFFIX: &str = " - Cursor";
const VS_CODE_SUFFIX: &str = " - Visual Studio Code";
const SEPARATOR: &str = " - ";

/// The window of a session among the windows of its host (spec §4). Editors are told apart by the
/// folder name just before the app suffix of the title; Desktop apps have one window, take the
/// topmost. Two editor windows of one folder name are `Many`: the caller falls back to the
/// launcher, which tells folders apart by their full path.
pub fn pick_window(host: HostKind, root: &str, windows: &[WindowInfo]) -> Pick {
    match host {
        HostKind::Cursor => pick_editor(CURSOR_SUFFIX, root, windows),
        HostKind::VsCode => pick_editor(VS_CODE_SUFFIX, root, windows),
        HostKind::ClaudeDesktop | HostKind::CodexDesktop => {
            windows.first().map_or(Pick::None, |w| Pick::One(w.hwnd))
        }
        HostKind::Unknown => Pick::None,
    }
}

fn pick_editor(suffix: &str, root: &str, windows: &[WindowInfo]) -> Pick {
    let Some(folder) = folder_name(root) else { return Pick::None };
    let folder = folder.to_lowercase();
    let mut hits = windows
        .iter()
        .filter(|w| title_folder(&w.title, suffix).is_some_and(|f| f.to_lowercase() == folder))
        .map(|w| w.hwnd);
    match (hits.next(), hits.next()) {
        (None, _) => Pick::None,
        (Some(hwnd), None) => Pick::One(hwnd),
        (Some(_), Some(_)) => Pick::Many,
    }
}

/// Last path component of the project root; `None` for `C:\` and for an empty root.
fn folder_name(root: &str) -> Option<&str> {
    std::path::Path::new(root).file_name()?.to_str().filter(|n| !n.is_empty())
}

/// The segment of `title` right before the editor suffix (`<chat or file> - <folder> - Cursor`).
/// `None` when the title does not end with the suffix. The suffix is ASCII, so its comparison
/// ignores case on bytes and never splits a character.
fn title_folder<'a>(title: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = title.len().checked_sub(suffix.len())?;
    if !title.is_char_boundary(cut) || !title[cut..].eq_ignore_ascii_case(suffix) {
        return None;
    }
    title[..cut].rsplit(SEPARATOR).next()
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak focus:: 2>&1 | grep -E "^test |test result|warning"`
Expected: 8 tests `ok`, `test result: ok. 8 passed`, no warning (a `dead_code` warning is acceptable only for items used by later tasks; if rustc prints any, add `#[allow(dead_code)]` on the three items for now and remove it in Task 4).

- [ ] **Step 5: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/focus/mod.rs windows/src-tauri/src/lib.rs && git commit -m "feat(focus): choose the window of a session from the titles of its host"
```

---

### Task 2: The agent pid in the snapshot

**Files:**
- Modify: `windows/src-tauri/src/observe/model.rs` (struct `SessionView` at ~line 261, `session_view` at ~line 861)
- Modify: `windows/src-tauri/src/lib.rs` (helper `session()` in `session_command_tests`, new `session_agent_pid`)
- Test: `windows/src-tauri/src/observe/model_tests.rs`, `windows/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `Session.proc: Option<ProcId>` (`ProcId { pid: u32, .. }`).
- Produces: `SessionView.agent_pid: Option<u32>` (`#[serde(skip)]` — not on the wire, the TypeScript types stay unchanged) and `fn session_agent_pid(snapshot: &SessionsSnapshot, key: &str) -> Option<u32>` in `lib.rs`.

- [ ] **Step 1: Write the failing tests**

In `model_tests.rs`, next to `proc_exit_removes_every_session_of_the_process` (it uses the helpers `run`, `xhook`, `ups`, `codex_proc`, `NOW`):

```rust
#[test]
fn the_snapshot_carries_the_agent_pid_but_not_on_the_wire() {
    let mut st = Store::new();
    run(&mut st, vec![xhook(ups("T1", 1_000))]);
    let snapshot = st.snapshot(NOW);
    let view = &snapshot.projects[0].sessions[0];
    assert_eq!(view.agent_pid, Some(codex_proc().pid));
    let wire = serde_json::to_value(view).unwrap();
    assert!(wire.get("agentPid").is_none() && wire.get("agent_pid").is_none(), "{wire}");
}
```

In `lib.rs`, inside `session_command_tests` add (and add `agent_pid: None,` to the literal in the helper `session()` after `acked: true,`):

```rust
    #[test]
    fn session_agent_pid_finds_the_pid_of_a_session() {
        let mut with_pid = session("claude:1:2", HostKind::Cursor);
        with_pid.agent_pid = Some(4242);
        let mut snapshot = Store::new().snapshot(0);
        snapshot.projects.push(ProjectView {
            key: r"c:\work\app".into(),
            name: "app".into(),
            root: r"C:\work\app".into(),
            sessions: vec![with_pid, session("codex:t", HostKind::CodexDesktop)],
        });
        assert_eq!(session_agent_pid(&snapshot, "claude:1:2"), Some(4242));
        assert_eq!(session_agent_pid(&snapshot, "codex:t"), None);
        assert_eq!(session_agent_pid(&snapshot, "nope"), None);
    }
```

- [ ] **Step 2: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak agent_pid 2>&1 | tail -12`
Expected: compile errors ``no field `agent_pid` on type `&SessionView` `` and ``cannot find function `session_agent_pid` ``.

- [ ] **Step 3: Implement**

`model.rs`, in `pub struct SessionView` after `pub acked: bool,`:

```rust
    /// Pid of the agent process, for the window switch (sub-project 2). Not serialized: the UI never needs it.
    #[serde(skip)]
    pub agent_pid: Option<u32>,
```

and in `fn session_view`, after `acked: !unacked(s),`:

```rust
        agent_pid: s.proc.as_ref().map(|p| p.pid),
```

`lib.rs`, right after `fn session_target(...)`:

```rust
/// Pid of the agent process of a session in a snapshot.
fn session_agent_pid(snapshot: &SessionsSnapshot, key: &str) -> Option<u32> {
    snapshot
        .projects
        .iter()
        .find_map(|project| project.sessions.iter().find(|s| s.key == key).and_then(|s| s.agent_pid))
}
```

If the compiler lists another `SessionView { .. }` literal that now misses the field, add `agent_pid: None,` there.

- [ ] **Step 4: Run the whole suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|warning|error"`
Expected: lib `292+` passed (baseline 291 + Task 1's 8 + 2 new), hook `32 passed`, 0 failed, no warning (`session_agent_pid` unused is a `dead_code` warning until Task 4: add `#[allow(dead_code)] // used by session_open from Task 4` above it and remove that attribute in Task 4).

- [ ] **Step 5: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/observe/model.rs windows/src-tauri/src/observe/model_tests.rs windows/src-tauri/src/lib.rs && git commit -m "feat(observe): keep the agent pid in the snapshot for the window switch"
```

---

### Task 3: The Win32 layer (`focus::win`)

**Files:**
- Create: `windows/src-tauri/src/focus/win.rs`
- Modify: `windows/src-tauri/src/focus/mod.rs` (add `pub mod win;` under the `use` line)
- Modify: `windows/src-tauri/Cargo.toml` (feature `Win32_System_Diagnostics_ToolHelp`)

**Interfaces:**
- Consumes: `focus::WindowInfo`.
- Produces:
  `pub struct ProcRow { pub pid: u32, pub parent_pid: u32, pub exe_name: String }`
  `pub fn process_table() -> Vec<ProcRow>` (empty on failure)
  `pub fn creation_ft(pid: u32) -> Option<u64>`
  `pub fn ancestors(rows: &[ProcRow], created: impl Fn(u32) -> Option<u64>, pid: u32, max: usize) -> Vec<u32>` — nearest first, stops at a parent that is not strictly older (pid reuse) **and at `explorer.exe`** (never included, never walked past)
  `pub fn host_pid_from(chain: &[u32], windows: &[(u32, WindowInfo)]) -> Option<u32>` — the first pid of `chain` (nearest first) that owns a window
  `pub fn top_level_windows() -> Vec<(u32, WindowInfo)>` — visible, titled, top-level, Z-order
  `pub fn host_windows(agent_pid: u32) -> Vec<WindowInfo>` — windows of the host process of that agent
  `pub enum Raise { Focused, Flashed, Failed }` and `pub fn raise(hwnd: isize) -> Raise`
  `pub const MAX_ANCESTORS: usize = 8;`

- [ ] **Step 1: Add the feature and the failing pure tests**

`Cargo.toml`: add `"Win32_System_Diagnostics_ToolHelp",` to the `windows` feature list, keeping alphabetical order (after `Win32_Storage_FileSystem`, before `Win32_System_IO`).

`focus/mod.rs`: add `pub mod win;` after the `use` line.

Create `focus/win.rs` with the pure tests first (the functions come in Step 3):

```rust
//! The Win32 of the window switch: process table, top-level windows, raising a window.
//! Everything that is not Win32 (the ancestor walk, the host choice) is a pure function.

use super::WindowInfo;

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
```

- [ ] **Step 2: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak focus::win 2>&1 | tail -12`
Expected: compile errors for the missing items (`ProcRow`, `ancestors`, `host_pid_from`, `top_level_windows`, `process_table`, `creation_ft`, `MAX_ANCESTORS`).

- [ ] **Step 3: Implement the layer**

Insert between the `use` line and `#[cfg(test)]` in `focus/win.rs` (this follows the code of `windows/hook/src/win.rs`; if rustc reports a signature difference in windows 0.61, fix it the way `platform/windows.rs` does — `::windows::core::BOOL`, `HWND(raw as *mut _)`):

```rust
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
```

- [ ] **Step 4: Run the tests and the suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak focus::win 2>&1 | grep -E "^test |test result|warning|error"`
Expected: 7 tests `ok`. Then `cargo test --workspace 2>&1 | grep -E "^test result|warning"`: 0 failed, no warning other than `dead_code` for items used by Task 4 (mark them `#[allow(dead_code)] // used from Task 4` and remove the attributes in Task 4).

- [ ] **Step 5: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/focus windows/src-tauri/Cargo.toml windows/Cargo.lock && git commit -m "feat(focus): find the windows of the host of an agent and bring one forward"
```

(If `windows/Cargo.lock` did not change, `git add` of that path is harmless; do not add any other file.)

---

### Task 4: Orchestration, fallback and the new `session_open`

**Files:**
- Modify: `windows/src-tauri/src/focus/mod.rs`
- Modify: `windows/src-tauri/src/lib.rs` (`session_open`, `launcher_for` stays, remove the `#[allow(dead_code)]` markers of Tasks 1-3)
- Test: both files

**Interfaces:**
- Consumes: Tasks 1-3 (`WindowInfo`, `Pick`, `pick_window`, `win::host_windows`, `win::raise`, `win::Raise`), `session_target`, `session_agent_pid`, `launcher_for`, `platform::find_on_path`, `platform::no_console`, `log::line`.
- Produces:
  `pub enum Outcome { Focused, Flashed, Launched, OpenedExplorer, Failed }` with `pub fn as_str(self) -> &'static str` (`"Focused"`…) and `pub fn log_line(host: HostKind, outcome: Outcome) -> String`
  `pub enum Fallback { Launcher(&'static str), Explorer }` and `pub fn fallback_for(host: HostKind) -> Fallback`
  `pub fn switch_to(host: HostKind, root: &str, agent_pid: Option<u32>) -> Outcome`

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` of `focus/mod.rs`:

```rust
    #[test]
    fn outcomes_have_stable_names() {
        let all = [Outcome::Focused, Outcome::Flashed, Outcome::Launched, Outcome::OpenedExplorer, Outcome::Failed];
        let names: Vec<_> = all.iter().map(|o| o.as_str()).collect();
        assert_eq!(names, ["Focused", "Flashed", "Launched", "OpenedExplorer", "Failed"]);
    }

    #[test]
    fn the_log_line_has_only_the_host_and_the_outcome() {
        let line = log_line(HostKind::Cursor, Outcome::Flashed);
        assert_eq!(line, "session_open: host=Cursor outcome=Flashed");
    }

    #[test]
    fn the_fallback_is_the_launcher_of_the_editors_and_explorer_otherwise() {
        assert_eq!(fallback_for(HostKind::Cursor), Fallback::Launcher("cursor"));
        assert_eq!(fallback_for(HostKind::VsCode), Fallback::Launcher("code"));
        for host in [HostKind::ClaudeDesktop, HostKind::CodexDesktop, HostKind::Unknown] {
            assert_eq!(fallback_for(host), Fallback::Explorer);
        }
    }
```

- [ ] **Step 2: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak focus::tests 2>&1 | tail -8`
Expected: compile errors for `Outcome`, `log_line`, `Fallback`, `fallback_for`.

- [ ] **Step 3: Implement**

Add to `focus/mod.rs` above `#[cfg(test)]`:

```rust
use std::process::Command;

use crate::{log, platform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The window is in the foreground.
    Focused,
    /// Windows refused the foreground; the taskbar button flashes.
    Flashed,
    /// No usable window; the launcher of the host started.
    Launched,
    /// No window and no launcher: the project folder opened in Explorer.
    OpenedExplorer,
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Focused => "Focused",
            Outcome::Flashed => "Flashed",
            Outcome::Launched => "Launched",
            Outcome::OpenedExplorer => "OpenedExplorer",
            Outcome::Failed => "Failed",
        }
    }
}

/// The only thing the switch writes to `mayak.log`: no title, no folder, no path (spec §6).
pub fn log_line(host: HostKind, outcome: Outcome) -> String {
    format!("session_open: host={host:?} outcome={}", outcome.as_str())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    Launcher(&'static str),
    Explorer,
}

/// What starts when no window of the host can be raised (D32; protocols may join after spike S3).
pub fn fallback_for(host: HostKind) -> Fallback {
    match host {
        HostKind::Cursor => Fallback::Launcher("cursor"),
        HostKind::VsCode => Fallback::Launcher("code"),
        HostKind::ClaudeDesktop | HostKind::CodexDesktop | HostKind::Unknown => Fallback::Explorer,
    }
}

/// Bring the window of the session forward; otherwise start the fallback. Blocking (a Win32 walk
/// and ~150 ms of settling): call it off the UI thread. `root` must be an absolute folder.
pub fn switch_to(host: HostKind, root: &str, agent_pid: Option<u32>) -> Outcome {
    let outcome = match agent_pid.map(win::host_windows).map(|w| (pick_window(host, root, &w), w)) {
        Some((Pick::One(hwnd), _)) => match win::raise(hwnd) {
            win::Raise::Focused => Outcome::Focused,
            win::Raise::Flashed => Outcome::Flashed,
            win::Raise::Failed => run_fallback(host, root),
        },
        _ => run_fallback(host, root),
    };
    log::line(log_line(host, outcome));
    outcome
}

fn run_fallback(host: HostKind, root: &str) -> Outcome {
    if let Fallback::Launcher(stem) = fallback_for(host) {
        if let Some(launcher) = platform::find_on_path(stem) {
            let mut cmd = Command::new(launcher);
            cmd.arg(root);
            match platform::no_console(&mut cmd).spawn() {
                Ok(_) => return Outcome::Launched,
                Err(err) => log::line(format!("session_open: launcher did not start: {err}")),
            }
        }
    }
    match Command::new("explorer").arg(root).spawn() {
        Ok(_) => Outcome::OpenedExplorer,
        Err(err) => {
            log::line(format!("session_open: explorer did not start: {err}"));
            Outcome::Failed
        }
    }
}
```

In `lib.rs` replace the whole `session_open` function (keep its doc comment's first sentence, rewrite the rest) with:

```rust
/// The «↗ Open» action: bring the window of the session forward (sub-project 2), otherwise start
/// the launcher of its host, otherwise Explorer. True when the switch was started; an unknown key,
/// a relative root or a missing folder is false and does nothing. The switch itself runs on a
/// short-lived thread: raising a window settles for ~150 ms and must not hold the UI thread.
#[tauri::command]
fn session_open(observe: State<observe::Handle>, key: String) -> bool {
    let snapshot = observe.snapshot();
    let Some((host, root)) = session_target(&snapshot, &key) else { return false };
    if !std::path::Path::new(&root).is_dir() {
        return false;
    }
    let agent_pid = session_agent_pid(&snapshot, &key);
    std::thread::spawn(move || {
        focus::switch_to(host, &root, agent_pid);
    });
    true
}
```

Remove every `#[allow(dead_code)] // used from Task 4` marker added in Tasks 1-3. `launcher_for` in `lib.rs` is now unused outside tests: delete it and its test `only_cursor_and_vs_code_have_launchers` (the same facts are covered by `fallback_for_…` in `focus`); `Command`/`platform` imports in `lib.rs` that become unused are removed as the compiler reports.

- [ ] **Step 4: Run the suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|warning|error"`
Expected: 0 failed, 0 warnings; the two older tests `session_target_finds_host_and_project_root` and `a_relative_project_root_is_never_a_target` still pass.

- [ ] **Step 5: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/focus/mod.rs windows/src-tauri/src/lib.rs && git commit -m "feat(focus): switch to the window of a session, fall back to the launcher"
```

---

### Task 5: Measurements with the user (spikes S1-S4) and the code they decide

**Files:**
- Create (scratch, not committed): `C:\tmp\mayak-spike\focus-probe.ps1`
- Modify (only if a spike says so): `windows/src-tauri/src/focus/mod.rs` (`fallback_for`, protocol fallback), `windows/src-tauri/src/focus/win.rs` (`raise` trick)
- Modify: `docs/superpowers/specs/2026-10-06-mayak-window-switching-design.md` (§8 results)

**Interfaces:** consumes the release/dev build of the branch; produces the results table of spec §8 and, per decision rule, code changes dispatched as separate commits.

This task is run **by the controller with the user** (clicks on windows, hosts that must be open). The implementer dispatch applies only to the code steps 8 and 9.

- [ ] **Step 1: Build and start the dev app outside the Claude package**

The session's processes see a virtualized `%LOCALAPPDATA%`; the app must run outside it (WMI). Stop any running `mayak.exe` that was started from this repo's `target\release` only if the user agrees, then start the debug build (it does not touch `target\release`):

In PowerShell (the same pattern that started the dev app in sub-project 1; `Create` returns at once, the tree keeps running):

```powershell
$cl = 'cmd.exe /d /c "set CARGO_TARGET_DIR=C:\tmp\mayak-target&& set WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222&& set PATH=C:\Users\HP\.cargo\bin;%PATH%&& cd /d C:\Users\HP\source\repos\mayak\windows&& npm run tauri dev > C:\tmp\mayak-spike\dev.log 2>&1"'
Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = $cl }
```

Expected: after the build, `mayak.exe` from `C:\tmp\mayak-target\debug` is running and `http://127.0.0.1:9222/json` answers; `dev.log` ends with `Running ... mayak.exe`. If port 1420 is taken by an old `vite`, stop that old `node.exe` (the one from this repo) first.

- [ ] **Step 2: S1 — does Windows allow the raise after a click on the island?**

With Cursor open on two different folders, one session of each, ask the user to: put the window of folder A behind others, click «↗ Open» on A's session in the sessions view, and report; repeat with the window minimized, and with the window on another monitor. Read the outcome from `mayak.log` (`session_open: host=Cursor outcome=…`). Record `Focused`/`Flashed` per case.
Decision rule: all `Focused` → no extra trick. Any `Flashed` while the window is not in the foreground → apply Step 8.

- [ ] **Step 3: S4 — two windows, same folder**

Ask the user to open the same folder in a second Cursor window (File → New Window → open the folder) and click «Open». Expected log: `outcome=Launched` (fallback `cursor <root>`), no wrong window raised. Record it.

- [ ] **Step 4: S2 — Codex Desktop**

Case a: Codex Desktop window visible → click «Open» on a Codex session → expected `Focused`. Case b: close the window to the tray (X) → click again → expected `OpenedExplorer` today. Record both and what the user sees.

- [ ] **Step 5: S3 — what `codex://` and `claude://` do**

Ask the user, for each protocol, to run in Win+R: `codex://` and `claude://` (no path), report whether the app comes forward from the tray. Then, only if the app documents session links, try one link. Do not guess addresses: find them in the app's own documentation or `Settings`; if nothing documents a session link, record «app-level only».

- [ ] **Step 6: Record the results**

Fill the table of spec §8 (one line per spike: observed outcome and the decision) and commit:

```bash
cd /c/Users/HP/source/repos/mayak && git add docs/superpowers/specs/2026-10-06-mayak-window-switching-design.md && git commit -m "docs: record the window switching measurements"
```

- [ ] **Step 7: Stop the dev app and clean up**

Stop only the processes this task started (`tauri dev` tree), never a release `mayak.exe` the user runs.

- [ ] **Step 8: Only if S1 says so — the foreground trick (one trick, tests first)**

Add to `win.rs` the single trick S1 selected (`AllowSetForegroundWindow(ASFW_ANY)` before the raise, or `AttachThreadInput` between the foreground thread and ours around `SetForegroundWindow`), with the same `Raise` result logic. There is no unit test for Windows' focus rules; the acceptance check is the S1 case that failed, re-run by the user. Run the suite, then:

```bash
git add windows/src-tauri/src/focus/win.rs && git commit -m "fix(focus): get the foreground right for the windows S1 refused"
```

- [ ] **Step 9: Only if S3 found a protocol that raises the app — extend the fallback (test first)**

Change `Fallback` to `Fallback::Protocol(&'static str)` for the hosts S3 confirmed (`"codex://"` for `CodexDesktop`, `"claude://"` for `ClaudeDesktop`), update the test `the_fallback_is_the_launcher_of_the_editors_and_explorer_otherwise` to the new expectation (red first), start the protocol with `platform::open_url(url)` in `run_fallback`, run the suite, and commit:

```bash
git add windows/src-tauri/src/focus/mod.rs && git commit -m "feat(focus): start the Desktop app protocol when its window cannot be raised"
```

---

### Task 6: Acceptance and documentation

**Files:**
- Modify: `docs/superpowers/specs/2026-10-06-mayak-window-switching-design.md`
- Modify: `.git/mayak-pr-notes.md` (local, not tracked)

**Interfaces:** consumes the branch after Tasks 1-5 and a release build; produces the filled acceptance table and the PR text.

- [ ] **Step 1: Update the spec to what was built**

In the spec: §3/§4 — the ancestor walk never includes or passes `explorer.exe`; §6 — `session_open` returns `true` when the switch **started** (the switch runs on a thread); §8 — results (done in Task 5). Commit:

```bash
cd /c/Users/HP/source/repos/mayak && git add docs/superpowers/specs/2026-10-06-mayak-window-switching-design.md && git commit -m "docs: align the window switching spec with the implementation"
```

- [ ] **Step 2: Build the release and run it outside the package**

With the user's antivirus exclusion for `windows\target` in place: `cd windows && npm run tauri build`, start `target\release\mayak.exe` via WMI (`Win32_Process.Create`) with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`. If `mayak.exe` disappears, ask the user to check Kaspersky.

- [ ] **Step 3: Run the seven acceptance items of spec §10 with the user**

For each item record PASS/FAIL and the log line (`session_open: host=… outcome=…`) in `.git/mayak-pr-notes.md`. Check item 7: `Select-String` the log for window titles/paths — nothing may appear; measure CPU of the hidden island with the sampler of sub-project 1 (`C:\tmp\mayak-accept\sample-cpu.ps1`), expected `tree_avg_percent_of_machine` < 0.100.

- [ ] **Step 4: Full verification**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|warning"` and `npm run build`.
Expected: 0 failed, 0 warnings, build OK.

- [ ] **Step 5: Ask before publishing**

Ask the user, verbatim: «Запушить ветку `feature/window-switching` в `origin` и открыть PR (поверх `feature/multi-session-core`, база — ветка подпроекта 1)?» Only on an explicit yes:

```bash
git push -u origin feature/window-switching
gh pr create --repo mahach666/mayak --base feature/multi-session-core --head feature/window-switching --title "Mayak sub-project 2: switch to the window of a session" --body-file "$(git rev-parse --git-dir)/mayak-pr2-body.md"
```

(`mayak-pr2-body.md` is written from the spec §10 table and the Task 5 results; nothing in braces may remain.)

---

## Self-Review

- **Spec coverage:** §1 goal and fallback → Tasks 1, 3, 4. §3 architecture → Tasks 1-4. §4 host rules → Task 1 (`pick_window`), Task 3 (`host_pid_from`, explorer stop), Task 4 (`fallback_for`). §5 raise → Task 3 (`raise`), Task 5 Step 8 (trick). §6 outcome/log → Task 4 (`Outcome`, `log_line`). §7 tests → Tasks 1, 3, 4. §8 spikes → Task 5. §9 limits → Task 6 Step 1 (documented). §10 acceptance → Task 6 Step 3.
- **Placeholder scan:** none; the two conditional steps (Task 5 Steps 8-9) carry their exact code shape and decision rule.
- **Type consistency:** `WindowInfo`, `Pick`, `pick_window` (Task 1) are used unchanged in Tasks 3-4; `Raise` (Task 3) matches the `switch_to` match in Task 4; `agent_pid: Option<u32>` (Task 2) matches `session_agent_pid` and `switch_to(host, &str, Option<u32>)`.
- **Review Focus:** items 1, 2, 4 → Task 1 tests; item 3 → Task 3 test `explorer_windows_never_become_the_host`; item 5 → Task 4 test `the_log_line_has_only_the_host_and_the_outcome`.
