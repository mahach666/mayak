# Mayak: silent Codex Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a Codex session writes its rollout but no hook reaches Mayak (the thread's folder is gone, hooks are not installed, or not trusted), Mayak shows a muted «no hooks: <reason>» row in the island and explains it in Settings → Codex.

**Architecture:** A pure module `observe/silent.rs` (rollout `session_meta` parsing, host of an originator, cause table, 2-minute liveness). `codex-io` gets a recursive directory watcher on `<codex_home>/sessions` (event-driven, the first line of an unknown rollout is read once) and sends a new `Signal::CodexSilent`. The store keeps a separate `silent` map and adds the rows to the snapshot (`SessionView.silent`); they never touch counts, `hold`, `aggregate`, alerts or sound. The front end draws the row and the Settings text.

**Tech Stack:** Rust (existing `DirWatcher`, `windows` crate, no new crates), TypeScript (no new packages), Tauri 2.

**Spec:** `docs/superpowers/specs/2026-10-06-mayak-silent-codex-design.md` (Russian; new spec text in Russian). Sub-project 1 spec for context: `docs/superpowers/specs/2026-10-01-mayak-multi-session-core-design.md` §5.6, §7, §15.

## Global Constraints

- Windows only. No new crates, no new `windows` features unless the compiler demands one for an API already used elsewhere.
- 0 % CPU while the island is hidden: **no timers, no polling**; the watcher wakes only on file-system events; the first line of each rollout file is read **once** (cached); liveness (120 s) is evaluated when the snapshot is built.
- Only the **first line** of a rollout file is read (cap 256 KiB); prompts and answers are never read, stored or logged. Logs get no paths, cwd or ids (counts and kinds only).
- Nothing is written into other tools' configs.
- Silent rows never change `counts`, `hold`, `aggregate`, `attention`, alerts or sounds.
- Commit messages: conventional, one line, **no trailers** (no Co-Authored-By, no «Generated with»), never mention an AI assistant anywhere. Do not push. **Tests run locally only** (GitHub Actions is out of quota: never push to trigger CI).
- Build environment: `windows/target/debug` may hold stale old-path build outputs. Always run cargo with `CARGO_TARGET_DIR=C:/tmp/mayak-target` and **never** touch `windows/target/release` or stop a running `mayak.exe`:
  `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace`
- Baseline before this plan: `cargo test --workspace` = 314 lib passed (1 ignored) + 32 passed, 0 warnings (the count may be 313 on the sub-project 2 branch; run it once first and use that as the baseline).
- Keep each file's existing line endings (check with `file`); do not rewrite whole files.
- Never open `*.key`, `ide/*.lock`, `auth.json`, `claude_desktop_config.json`; never edit `~/.codex` or `~/.claude`.
- Branch: `feature/silent-codex` (created from `feature/window-switching`; spec at `798ce24`).

## Review Focus

1. A rollout that already belongs to a session with hooks (its path is tracked) must never become a silent row (Task 3 test).
2. A subagent rollout (`parent_thread_id` at top level or under `source.subagent.thread_spawn`) and a `.zst` file never become silent rows (Task 1 + Task 3 tests).
3. The 120 s boundary: alive at exactly 120 000 ms, dead at 120 001 ms; an expired row is not in the snapshot (Task 1 + Task 2 tests).
4. A real hook for the same session id replaces the silent row; a later silent signal for that id is ignored (Task 2 test).
5. A first line that is not JSON, not `session_meta`, has no `id`, or is longer than the cap must not panic or loop and must not emit a signal (Task 1 + Task 3 tests).

---

## File Structure

- Create `windows/src-tauri/src/observe/silent.rs` — pure logic and its tests.
- Modify `windows/src-tauri/src/observe/mod.rs` — `mod silent;` (and `pub use` of what the model needs).
- Modify `windows/src-tauri/src/observe/model.rs` — `Signal::CodexSilent`, `Store.silent`, `SessionView.silent`, snapshot assembly.
- Modify `windows/src-tauri/src/observe/model_tests.rs` — model tests.
- Modify `windows/src-tauri/src/observe/codex.rs` (module `io_thread`) — the directory watcher and the first-line read.
- Modify `windows/src/core/bridge.ts`, `windows/src/views/sessions.ts`, `windows/src/settings/main.ts`, `windows/src/style.css` (only if a class is needed) — the row and the Settings text.
- Modify `docs/superpowers/specs/2026-10-06-mayak-silent-codex-design.md` — the clarifications named in Task 5.

---

### Task 1: Pure silent logic (`observe/silent.rs`)

**Files:**
- Create: `windows/src-tauri/src/observe/silent.rs`
- Modify: `windows/src-tauri/src/observe/mod.rs` (add `pub mod silent;` next to the other `mod` lines)

**Interfaces:**
- Consumes: `crate::observe::keys::HostKind`; `serde_json::Value`.
- Produces (later tasks rely on these exact names):
  `pub const SILENT_TTL_MS: i64 = 120_000;`
  `pub struct SilentMeta { pub id: String, pub cwd: Option<String>, pub originator: Option<String>, pub subagent: bool }`
  `pub fn parse_session_meta(first_line: &str) -> Option<SilentMeta>`
  `pub fn host_of(originator: Option<&str>) -> HostKind`
  `#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "camelCase")] pub enum SilentReason { HooksNotInstalled, FolderMissing, HooksNotReaching }`
  `pub fn reason_of(hooks_not_installed: bool, folder_exists: bool) -> SilentReason`
  `pub fn is_alive(last_write_ms: i64, now_ms: i64) -> bool`

- [ ] **Step 1: Write the file with the tests only**

`windows/src-tauri/src/observe/silent.rs`:

```rust
//! «Silent» Codex (spec 2026-10-06-mayak-silent-codex-design.md): a Codex session that writes its
//! rollout while no hook reaches Mayak. The pure part: reading the first line of a rollout,
//! the host of an originator, the cause table and the liveness rule.

use serde_json::Value;

use super::keys::HostKind;

#[cfg(test)]
mod tests {
    use super::*;

    const META: &str = r#"{"timestamp":"2026-10-06T10:00:00.000Z","ordinal":0,"type":"session_meta","payload":{"id":"11111111-1111-4111-8111-111111111111","cwd":"C:\\Users\\dev\\src\\demo","originator":"Codex Desktop","source":"vscode","cli_version":"0.160.0","base_instructions":{"text":"generic"}}}"#;

    #[test]
    fn a_session_meta_line_gives_id_cwd_and_originator() {
        let m = parse_session_meta(META).unwrap();
        assert_eq!(m.id, "11111111-1111-4111-8111-111111111111");
        assert_eq!(m.cwd.as_deref(), Some(r"C:\Users\dev\src\demo"));
        assert_eq!(m.originator.as_deref(), Some("Codex Desktop"));
        assert!(!m.subagent);
    }

    #[test]
    fn a_subagent_is_recognised_at_the_top_level_and_under_the_spawn() {
        let top = META.replace(r#""source":"vscode""#, r#""source":"vscode","parent_thread_id":"22222222-2222-4222-8222-222222222222""#);
        assert!(parse_session_meta(&top).unwrap().subagent);
        let nested = META.replace(
            r#""source":"vscode""#,
            r#""source":{"subagent":{"thread_spawn":{"parent_thread_id":"22222222-2222-4222-8222-222222222222","agent_role":null}}}"#,
        );
        assert!(parse_session_meta(&nested).unwrap().subagent);
    }

    #[test]
    fn lines_that_are_not_a_session_meta_with_an_id_give_nothing() {
        assert!(parse_session_meta("").is_none());
        assert!(parse_session_meta("not json").is_none());
        assert!(parse_session_meta(r#"{"type":"event_msg","payload":{"type":"task_started"}}"#).is_none());
        assert!(parse_session_meta(r#"{"type":"session_meta","payload":{"cwd":"C:\\x"}}"#).is_none());
        assert!(parse_session_meta(r#"{"type":"session_meta","payload":{"id":""}}"#).is_none());
        assert!(parse_session_meta(r#"{"type":"session_meta","payload":{"id":7}}"#).is_none());
    }

    #[test]
    fn only_the_codex_app_originator_names_a_host() {
        assert_eq!(host_of(Some("Codex Desktop")), HostKind::CodexDesktop);
        // VS Code and Cursor both write codex_vscode: the editor cannot be told from the rollout
        assert_eq!(host_of(Some("codex_vscode")), HostKind::Unknown);
        assert_eq!(host_of(Some("something else")), HostKind::Unknown);
        assert_eq!(host_of(None), HostKind::Unknown);
    }

    #[test]
    fn the_reason_follows_the_table_from_the_top() {
        assert_eq!(reason_of(true, true), SilentReason::HooksNotInstalled);
        assert_eq!(reason_of(true, false), SilentReason::HooksNotInstalled);
        assert_eq!(reason_of(false, false), SilentReason::FolderMissing);
        assert_eq!(reason_of(false, true), SilentReason::HooksNotReaching);
    }

    #[test]
    fn the_wire_names_of_the_reasons_are_camel_case() {
        let name = |r| serde_json::to_value(r).unwrap();
        assert_eq!(name(SilentReason::HooksNotInstalled), "hooksNotInstalled");
        assert_eq!(name(SilentReason::FolderMissing), "folderMissing");
        assert_eq!(name(SilentReason::HooksNotReaching), "hooksNotReaching");
    }

    #[test]
    fn a_row_is_alive_for_exactly_two_minutes() {
        assert!(is_alive(1_000, 1_000));
        assert!(is_alive(1_000, 1_000 + SILENT_TTL_MS));
        assert!(!is_alive(1_000, 1_000 + SILENT_TTL_MS + 1));
        // a clock that went back does not kill the row
        assert!(is_alive(5_000, 1_000));
    }
}
```

Add `pub mod silent;` to `windows/src-tauri/src/observe/mod.rs` next to the other `mod` declarations.

- [ ] **Step 2: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak silent:: 2>&1 | tail -12`
Expected: compile errors (`cannot find function parse_session_meta`, `host_of`, `reason_of`, `is_alive`, type `SilentReason`).

- [ ] **Step 3: Implement**

Insert above `#[cfg(test)]`:

```rust
/// A silent row lives while the rollout was written to within this time (spec §3).
pub const SILENT_TTL_MS: i64 = 120_000;

/// What the first line of a rollout says about its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SilentMeta {
    pub id: String,
    pub cwd: Option<String>,
    pub originator: Option<String>,
    /// A subagent session (it has a parent thread): never shown.
    pub subagent: bool,
}

/// `session_meta` of the first rollout line; `None` for anything else (not JSON, another record,
/// no or empty string `payload.id`). Only these four facts are taken: nothing else of the line
/// is kept.
pub fn parse_session_meta(first_line: &str) -> Option<SilentMeta> {
    let v: Value = serde_json::from_str(first_line).ok()?;
    if v.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let p = v.get("payload")?.as_object()?;
    let text = |key: &str| p.get(key).and_then(Value::as_str).map(str::to_owned);
    let id = text("id").filter(|id| !id.is_empty())?;
    let nested_parent = p
        .get("source")
        .and_then(|s| s.get("subagent"))
        .and_then(|s| s.get("thread_spawn"))
        .and_then(|s| s.get("parent_thread_id"))
        .and_then(Value::as_str);
    let subagent = text("parent_thread_id").is_some() || nested_parent.is_some();
    Some(SilentMeta { id, cwd: text("cwd"), originator: text("originator"), subagent })
}

/// The host an originator names. VS Code and Cursor both write `codex_vscode`, so only the
/// Codex app is told apart; everything else is `Unknown`.
pub fn host_of(originator: Option<&str>) -> HostKind {
    match originator {
        Some("Codex Desktop") => HostKind::CodexDesktop,
        _ => HostKind::Unknown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SilentReason {
    HooksNotInstalled,
    FolderMissing,
    HooksNotReaching,
}

/// Why no hook arrives (spec §3.1), checked from the top.
pub fn reason_of(hooks_not_installed: bool, folder_exists: bool) -> SilentReason {
    if hooks_not_installed {
        SilentReason::HooksNotInstalled
    } else if !folder_exists {
        SilentReason::FolderMissing
    } else {
        SilentReason::HooksNotReaching
    }
}

/// Whether a row last written at `last_write_ms` is still shown at `now_ms`.
pub fn is_alive(last_write_ms: i64, now_ms: i64) -> bool {
    now_ms.saturating_sub(last_write_ms) <= SILENT_TTL_MS
}
```

- [ ] **Step 4: Run to see the tests pass, then the suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak silent:: 2>&1 | grep -E "^test |test result"` then `cargo test --workspace 2>&1 | grep -E "^test result|warning"`
Expected: 7 tests ok; suite green. Unused items give `dead_code` warnings until Task 2: put `#[allow(dead_code)] // wired in Task 2` on each public item only if rustc warns.

- [ ] **Step 5: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/observe/silent.rs windows/src-tauri/src/observe/mod.rs && git commit -m "feat(observe): read the first rollout line, decide the reason and liveness of a silent Codex"
```

---

### Task 2: The model: signal, silent rows in the snapshot

**Files:**
- Modify: `windows/src-tauri/src/observe/model.rs` (`enum Signal`, `struct Store`, `SessionView` + `session_view`, `fn view`, the `apply` dispatch, the place a real Codex session is created)
- Test: `windows/src-tauri/src/observe/model_tests.rs`

**Interfaces:**
- Consumes: Task 1 (`SilentReason`, `reason_of`, `is_alive`, `host_of` is used by the sender), `ProjectRef`, `HostKind`, `CodexHooksView::NotInstalled`.
- Produces:
  `Signal::CodexSilent { id: String, project: ProjectRef, host: HostKind, folder_exists: bool, written_ms: i64 }` (the sender computes the project and the folder test; `written_ms` is the store clock `now_ms()` at the file event)
  `SessionView.silent: Option<SilentReason>` — `#[serde(skip_serializing_if = "Option::is_none")]`, key `silent`
  `Store` keeps `silent: BTreeMap<String /*id*/, SilentRow { project: ProjectRef, host: HostKind, folder_exists: bool, last_write_ms: i64 }>`
  snapshot rule: a row is in `projects[].sessions` only when `is_alive(last_write_ms, now_ms)`; the view is built with `now_ms` (give `view()` a `now_ms: i64` parameter if it has none).

- [ ] **Step 1: Write the failing tests**

In `model_tests.rs`, next to the Codex tests (use the existing helpers `run`, `xhook`, `ups`, `project`, `NOW`, `xkey`; adapt only names that differ):

```rust
fn silent(id: &str, written_ms: i64) -> Signal {
    Signal::CodexSilent {
        id: id.into(),
        project: project("Gamma"),
        host: HostKind::CodexDesktop,
        folder_exists: true,
        written_ms,
    }
}

fn silent_rows(snapshot: &SessionsSnapshot) -> Vec<&SessionView> {
    snapshot.projects.iter().flat_map(|p| p.sessions.iter()).filter(|s| s.silent.is_some()).collect()
}

#[test]
fn a_silent_signal_adds_a_muted_row_that_changes_nothing_else() {
    let mut st = Store::new();
    let before = st.snapshot(NOW);
    let out = run(&mut st, vec![silent("s-1", NOW)]);
    assert!(out.alerts.is_empty());
    let snap = st.snapshot(NOW);
    let rows = silent_rows(&snap);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].silent, Some(SilentReason::HooksNotReaching));
    assert_eq!(rows[0].host, HostKind::CodexDesktop);
    assert_eq!(rows[0].agent, Source::Codex);
    assert!(rows[0].attention.is_none() && rows[0].acked);
    assert_eq!((snap.counts.clone(), snap.hold, snap.aggregate, snap.attention.clone()),
               (before.counts, before.hold, before.aggregate, before.attention));
}

#[test]
fn the_row_dies_exactly_two_minutes_after_the_last_write() {
    let mut st = Store::new();
    run(&mut st, vec![silent("s-1", NOW)]);
    assert_eq!(silent_rows(&st.snapshot(NOW + 120_000)).len(), 1);
    assert!(silent_rows(&st.snapshot(NOW + 120_001)).is_empty());
    // a later write brings it back
    run(&mut st, vec![silent("s-1", NOW + 130_000)]);
    assert_eq!(silent_rows(&st.snapshot(NOW + 140_000)).len(), 1);
}

#[test]
fn the_reason_follows_the_installer_state_and_the_folder() {
    let mut st = Store::new();
    run(&mut st, vec![Signal::CodexHooksInstalled(false), silent("s-1", NOW)]);
    assert_eq!(silent_rows(&st.snapshot(NOW))[0].silent, Some(SilentReason::HooksNotInstalled));
    run(&mut st, vec![Signal::CodexHooksInstalled(true)]);
    assert_eq!(silent_rows(&st.snapshot(NOW))[0].silent, Some(SilentReason::HooksNotReaching));
    run(&mut st, vec![Signal::CodexSilent {
        id: "s-2".into(), project: project("Gamma"), host: HostKind::Unknown, folder_exists: false, written_ms: NOW,
    }]);
    let snap = st.snapshot(NOW);
    assert!(silent_rows(&snap).iter().any(|s| s.silent == Some(SilentReason::FolderMissing)));
}

#[test]
fn a_real_hook_replaces_the_silent_row_and_later_silent_signals_are_ignored() {
    let mut st = Store::new();
    run(&mut st, vec![silent("T1", NOW)]); // `T1` is the session id `ups("T1", ..)` uses
    run(&mut st, vec![xhook(ups("T1", 1_000))]);
    let snap = st.snapshot(NOW);
    assert!(silent_rows(&snap).is_empty());
    run(&mut st, vec![silent("T1", NOW + 1)]);
    assert!(silent_rows(&st.snapshot(NOW + 1)).is_empty());
}

#[test]
fn silent_rows_never_raise_alerts_or_count_as_working() {
    let mut st = Store::new();
    let out = run(&mut st, vec![silent("s-1", NOW), silent("s-2", NOW), silent("s-1", NOW + 5)]);
    assert!(out.alerts.is_empty());
    let snap = st.snapshot(NOW + 5);
    assert_eq!(snap.counts, Counts::default());
    assert_eq!(silent_rows(&snap).len(), 2);
}
```

(`ups("T1", ..)` builds a Codex hook event whose `session_id` must equal `"T1"`: if the helper derives a different id, take the real one from the helper and use it in the test.)

- [ ] **Step 2: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak silent 2>&1 | tail -12`
Expected: compile errors for `Signal::CodexSilent`, `SessionView.silent`.

- [ ] **Step 3: Implement**

1. `Signal` gets the variant above (doc comment: «codex-io: a rollout of a session with no hook (spec silent-codex §2)»).
2. `SessionView` gets `#[serde(skip_serializing_if = "Option::is_none")] pub silent: Option<SilentReason>,` and `session_view` sets `silent: None`; fix every other `SessionView { .. }` literal the compiler lists (`lib.rs` test helper) with `silent: None`.
3. `Store` gets the `silent` map (`BTreeMap<String, SilentRow>`, `SilentRow` defined next to it). In the apply dispatch:
   - `Signal::CodexSilent { id, project, host, folder_exists, written_ms }`: if `self.sessions` already holds the Codex session of this id (`codex_key(&id)`), do nothing; otherwise upsert the row with `last_write_ms = max(old, written_ms)`. Bound the map at 64 rows by dropping the oldest `last_write_ms`.
   - where a real Codex session is created from a hook (the `Signal::CodexHook` arm): `self.silent.remove(&event.session_id)`.
4. Snapshot: after the projects of the real sessions are built in `view()`, append each **alive** silent row (`is_alive(row.last_write_ms, now_ms)`) as a `SessionView { key: format!("silent:{id}"), agent: Source::Codex, host, title: None, phase: PhaseView::Idle, since: row.last_write_ms, activity: None, detail: None, outcome: None, attention: None, acked: true, silent: Some(reason_of(self.codex_hooks == CodexHooksView::NotInstalled, folder_exists)) }` to the `ProjectView` with the same project key (create it with `name` from the project display names when no real session has that project; such a project sorts after every project that has a real session). Expired rows are removed from the map in `snapshot` only if that method already takes `&mut self`; otherwise they are just left out (they are bounded by the 64-row cap).
   - Do **not** feed silent rows into `counts`, `attention`, `hold`, `aggregate`, `flagged`, alerts or `update_ticker`.
   - The display names (`display_names(&self.project_pairs())`) must include silent projects so names stay unique: extend `project_pairs` accordingly.

- [ ] **Step 4: Run the tests and the suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|warning|error"`
Expected: 5 new tests ok, 0 failed, 0 warnings (remove the Task 1 `#[allow(dead_code)]` markers now that the model uses the items).

- [ ] **Step 5: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/observe windows/src-tauri/src/lib.rs && git commit -m "feat(observe): show a silent Codex session as a muted row with its reason"
```

---

### Task 3: The watcher in `codex-io`

**Files:**
- Modify: `windows/src-tauri/src/observe/codex.rs` (module `io_thread`: `CodexInput`, `CodexConfig`, `CodexIo`, `spawn`, `handle`)
- Test: same file (`io_thread` tests; use the existing temp-dir helper `crate::testutil::TempDir` and the way the other io-thread tests feed inputs)

**Interfaces:**
- Consumes: Task 1 (`parse_session_meta`, `host_of`), `DirWatcher`, `WatchEvent`, `FileAction`, `MASK_NAME_WRITE_SIZE` (from `observe::win`), `paths::project_of`, `Signal::CodexSilent` (Task 2), `is_readable_rollout`, the existing `rollouts` map of tracked paths.
- Produces: `CodexInput::Sessions(WatchEvent)`; `CodexConfig.watch: bool` (false in tests: inputs are fed by hand, as claude-io does); the watcher thread `dirwatch-sessions` started by `spawn` when `watch` is true and `<codex_home>/sessions` exists.

- [ ] **Step 1: Write the failing tests**

In the `io_thread` tests, with the existing harness that builds a `CodexIo` and collects the `Signal`s it sends (adapt names to the harness; the intent of each test is fixed):

```rust
fn rollout_file(dir: &std::path::Path, name: &str, first_line: &str) -> std::path::PathBuf {
    let day = dir.join("sessions").join("2026").join("10").join("06");
    std::fs::create_dir_all(&day).unwrap();
    let path = day.join(name);
    std::fs::write(&path, format!("{first_line}\n{{\"type\":\"event_msg\"}}\n")).unwrap();
    path
}

const SESSION_META: &str = r#"{"type":"session_meta","payload":{"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","cwd":"C:\\no\\such\\folder","originator":"Codex Desktop"}}"#;

fn change(name: &str, action: FileAction) -> CodexInput {
    CodexInput::Sessions(WatchEvent::Changes(vec![(action, format!(r"2026\10\06\{name}"))]))
}

#[test]
fn a_new_untracked_rollout_gives_one_silent_signal_and_is_read_once() {
    // harness: io = CodexIo with codex_home = tmp, watch = false
    let path = rollout_file(&tmp, "rollout-2026-10-06T10-00-00-aaaa.jsonl", SESSION_META);
    io.handle(change("rollout-2026-10-06T10-00-00-aaaa.jsonl", FileAction::Added));
    // the file is replaced by garbage: a second event must come from the cache, not a re-read
    std::fs::write(&path, "garbage\n").unwrap();
    io.handle(change("rollout-2026-10-06T10-00-00-aaaa.jsonl", FileAction::Modified));
    let silent: Vec<_> = signals().into_iter().filter(|s| matches!(s, Signal::CodexSilent { .. })).collect();
    assert_eq!(silent.len(), 2); // two events, one id
    for s in &silent {
        let Signal::CodexSilent { id, folder_exists, host, .. } = s else { unreachable!() };
        assert_eq!(id, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        assert!(!folder_exists);
        assert_eq!(*host, HostKind::CodexDesktop);
    }
}

#[test]
fn a_rollout_that_a_hook_already_tracks_is_never_silent() {
    // harness: feed a Hook whose transcript_path is the rollout (as the other io tests do), then the change
    // expect: no Signal::CodexSilent for that file
}

#[test]
fn subagents_zst_and_unreadable_first_lines_give_no_signal() {
    let sub = SESSION_META.replace(r#""originator""#, r#""parent_thread_id":"bbbb","originator""#);
    rollout_file(&tmp, "rollout-sub.jsonl", &sub);
    rollout_file(&tmp, "rollout-bad.jsonl", "not json");
    rollout_file(&tmp, "rollout-noid.jsonl", r#"{"type":"session_meta","payload":{"cwd":"C:\\x"}}"#);
    rollout_file(&tmp, "rollout-z.jsonl.zst", SESSION_META);
    for name in ["rollout-sub.jsonl", "rollout-bad.jsonl", "rollout-noid.jsonl", "rollout-z.jsonl.zst"] {
        io.handle(change(name, FileAction::Added));
    }
    assert!(signals().iter().all(|s| !matches!(s, Signal::CodexSilent { .. })));
}

#[test]
fn removals_overflow_and_other_files_are_ignored() {
    rollout_file(&tmp, "rollout-r.jsonl", SESSION_META);
    io.handle(change("rollout-r.jsonl", FileAction::Removed));
    io.handle(CodexInput::Sessions(WatchEvent::Overflow));
    io.handle(change("session_index.jsonl", FileAction::Modified));
    assert!(signals().iter().all(|s| !matches!(s, Signal::CodexSilent { .. })));
}

#[test]
fn only_the_first_line_up_to_the_cap_is_read() {
    let huge = format!(r#"{{"type":"session_meta","payload":{{"id":"x","pad":"{}"}}}}"#, "a".repeat(300 * 1024));
    rollout_file(&tmp, "rollout-huge.jsonl", &huge);
    io.handle(change("rollout-huge.jsonl", FileAction::Added));
    assert!(signals().iter().all(|s| !matches!(s, Signal::CodexSilent { .. })));
}
```

- [ ] **Step 2: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak io_thread 2>&1 | tail -12`
Expected: compile errors for `CodexInput::Sessions`.

- [ ] **Step 3: Implement**

In `io_thread`:

```rust
/// The first line of a rollout is read up to this size; longer first lines are not silent rows.
const FIRST_LINE_CAP: usize = 256 * 1024;

/// One rollout file the watcher has looked at (kept so its first line is read once).
enum SilentFile {
    /// Subagent, unreadable or not a session: never a row.
    Skip,
    Session { id: String, project: ProjectRef, host: HostKind, folder_exists: bool },
}

/// First line of `path` as text, at most `FIRST_LINE_CAP` bytes, `None` on any error or when the
/// line is longer than the cap.
fn read_first_line(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(8 * 1024);
    let file = std::fs::File::open(path).ok()?;
    file.take(FIRST_LINE_CAP as u64 + 1).read_to_end(&mut buf).ok()?;
    let end = buf.iter().position(|b| *b == b'\n').unwrap_or(buf.len());
    (end <= FIRST_LINE_CAP).then(|| String::from_utf8_lossy(&buf[..end]).trim_end_matches('\r').to_owned())
}
```

(When the file has no newline within the cap `end == buf.len() > FIRST_LINE_CAP` → `None`.)

- `CodexInput` gets `Sessions(WatchEvent)`; `CodexConfig` gets `pub watch: bool` (set `true` where `observe::start` builds it, `false` in the test harness).
- `CodexIo` gets `silent_files: HashMap<String, SilentFile>` keyed by the normalized display path (use the same normalizer the `rollouts` map uses), and `sessions_dir: PathBuf`.
- `handle(CodexInput::Sessions(WatchEvent::Changes(list)))`: for each `(action, name)` with `action` `Added | Modified | RenamedNew` and a file name starting with `rollout-` that passes `is_readable_rollout`: build the full path under `sessions_dir`, normalize it; if it is a key of `self.rollouts` (tracked by a hook) → skip; look up `silent_files`, otherwise `read_first_line` → `parse_session_meta` → `SilentFile` (`Skip` for subagents and parse failures; for a session compute `folder_exists = Path::new(cwd).is_dir()` and `project = paths::project_of(cwd, probe)` exactly as the hook path does for its `cwd`, falling back to the last component of `cwd` as the name when the folder does not exist); then for `Session` send `Signal::CodexSilent { id, project, host, folder_exists, written_ms: crate::observe::now_ms() }`. `WatchEvent::Overflow` and `Error` are ignored (log one line kind only). Bound `silent_files` at 256 entries (drop the oldest inserted).
- `spawn`: when `cfg.watch` and `<codex_home>/sessions` exists, `DirWatcher::spawn(&dir, true, MASK_NAME_WRITE_SIZE, move |ev| tx_watch.send(CodexInput::Sessions(ev)).is_ok())` with a clone of the channel sender made before the thread starts; a missing directory or an error is logged as one line (`observe: codex sessions watcher not started: <kind>`) and is not an error of the source.

- [ ] **Step 4: Run the tests and the suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|warning|error"`
Expected: the new tests ok, 0 failed, 0 warnings. (`a_rollout_that_a_hook_already_tracks...` must be filled in with the harness's real way of tracking a rollout; it must fail if the `rollouts` check is removed.)

- [ ] **Step 5: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/observe/codex.rs windows/src-tauri/src/observe/mod.rs && git commit -m "feat(observe): watch the Codex sessions folder for rollouts that no hook reports"
```

---

### Task 4: The island row and the Settings text

**Files:**
- Modify: `windows/src/core/bridge.ts` (`SessionView`)
- Modify: `windows/src/views/sessions.ts` (a pure `silentText`, the row)
- Modify: `windows/src/settings/main.ts` (`codexSection`)
- Modify: `windows/src/style.css` only if a new class is needed

**Interfaces:**
- Consumes: the wire field `silent?: "hooksNotInstalled" | "folderMissing" | "hooksNotReaching"` (Task 2).
- Produces: `export type SilentReason = "hooksNotInstalled" | "folderMissing" | "hooksNotReaching"` and `silent?: SilentReason` on `SessionView` in `bridge.ts`; `export function silentText(reason: SilentReason): { short: string; help: string }` in `views/sessions.ts`.

There is no front-end test framework (`package.json` has `tsc`, `vite`, `tauri` only); the checks are `npx tsc --noEmit` and `npx vite build`. Do not add a test runner.

- [ ] **Step 1: Types**

`bridge.ts`: add before `SessionView`:

```ts
export type SilentReason = "hooksNotInstalled" | "folderMissing" | "hooksNotReaching";
```

and add `silent?: SilentReason;` as the last member of `SessionView`.

- [ ] **Step 2: The texts and the row**

`views/sessions.ts` (export, near `phaseLabel`):

```ts
/** Texts for a Codex session that writes its rollout while no hook arrives (silent-codex spec §3.1, §4). */
export function silentText(reason: SilentReason): { short: string; help: string } {
  switch (reason) {
    case "hooksNotInstalled":
      return { short: "Mayak hooks are not installed", help: "Install the Codex hooks below, then trust them in Codex." };
    case "folderMissing":
      return {
        short: "the thread's folder is gone",
        help: "Codex cannot start hooks in a folder that no longer exists. Start a new thread in an existing folder.",
      };
    case "hooksNotReaching":
      return {
        short: "hooks do not reach Mayak",
        help: "The hooks are installed but nothing arrives. Check «Hook statistics» in Codex and that the hooks are trusted.",
      };
  }
}
```

In `row(s, dim)`: when `s.silent` is set, build the row **without** the elapsed-time clock and without the `onmousedown` open action's phase wording: phase text `no hooks`, detail `silentText(s.silent).short`, class `silent` added (muted, reusing the existing `dim` look: `cls.push("dim", "silent")`), no `clocks.push`. Keep `openSession(s.key)` on press only if `session_open` can resolve a `silent:<id>` key; it cannot (the key is not a real session), so for silent rows do **not** attach `onmousedown`.

- [ ] **Step 3: Settings**

`settings/main.ts`: in `codexSection`, add a row that draws on each snapshot:

```ts
  const silentRow = h("div", { class: "hint" });
  const drawSilent = (s: SessionsSnapshot | null) => {
    const reasons = new Set<SilentReason>();
    for (const p of s?.projects ?? []) for (const x of p.sessions) if (x.silent) reasons.add(x.silent);
    silentRow.textContent = [...reasons].map((r) => silentText(r).help).join(" ");
    silentRow.style.display = reasons.size ? "" : "none";
  };
  drawSilent(snap);
  snapshotViews.push(drawSilent);
```

and add `silentRow` to the section's children after the `Events` row. Import `SilentReason` from `../core/bridge` and `silentText` from `../views/sessions`.

- [ ] **Step 4: Verify**

Run: `cd /c/Users/HP/source/repos/mayak/windows && npx tsc --noEmit && npx vite build 2>&1 | tail -3`
Expected: both exit 0 (`npm ci` first if `node_modules` is missing). Then `cargo test --workspace` stays green (nothing Rust changed).

- [ ] **Step 5: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src && git commit -m "feat(island): show a silent Codex session as a muted row and explain it in Settings"
```

---

### Task 5: Acceptance and documentation

**Files:**
- Modify: `docs/superpowers/specs/2026-10-06-mayak-silent-codex-design.md`
- Modify: `.git/mayak-pr-notes.md` (local)

**Interfaces:** consumes the branch after Tasks 1-4; produces the filled acceptance table.

- [ ] **Step 1: Align the spec with what was built**

In the spec: §3 host rule — `codex_vscode` gives `Unknown` (VS Code and Cursor both write it and the rollout has no extension path); §4 — silent rows have no click action (the key is not a session); §5 — the cap of 64 rows and 256 watched files. Commit: `docs: align the silent Codex spec with the implementation`.

- [ ] **Step 2: Build and start the app outside the Claude package**

Use the pattern of sub-project 2 (WMI `Win32_Process.Create`, `CARGO_TARGET_DIR=C:\tmp\mayak-target`, `npm run tauri dev` with CDP on :9222, log `C:\tmp\mayak-spike\dev3.log`); stop only processes this task started.

- [ ] **Step 3: Run the four acceptance items of spec §6 with the user**

(1) a Codex Desktop thread whose folder was deleted → a row `… Codex · no hooks: the thread's folder is gone` within seconds of the first message, gone 2 minutes after the last write; (2) hooks not installed (Settings → Codex → Remove, then a new thread) → `Mayak hooks are not installed`; reinstall afterwards; (3) a thread with working hooks → no silent row; (4) CPU of the hidden island with no Codex activity below 0.100 % (`C:\tmp\mayak-accept\sample-cpu.ps1`). Record each as PASS/FAIL with the evidence in `.git/mayak-pr-notes.md`.

- [ ] **Step 4: Full verification (local only)**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|warning"` and `npm run build`.
Expected: 0 failed, 0 warnings, build OK.

- [ ] **Step 5: Ask before publishing**

Ask the user, verbatim: «Запушить ветку `feature/silent-codex` в `origin` и открыть PR (базой — ветка подпроекта 2)? Проверки на GitHub отключены, проверка локальная.» Only on an explicit yes: `git push -u origin feature/silent-codex` and `gh pr create --repo mahach666/mayak --base feature/window-switching --head feature/silent-codex --title "Mayak: show a silent Codex session (no hooks) with its reason" --body-file "$(git rev-parse --git-dir)/mayak-pr3-body.md"` (the body from the spec §6 table and the acceptance results; nothing in braces may remain).

---

## Self-Review

- **Spec coverage:** §1 goal → Tasks 1-4; §2 detection (watcher, first line once, filters, subagents, `.zst`, tracked paths) → Task 3; §3 model (map, TTL at snapshot time, replacement by a hook, host, project, reasons) → Tasks 1-2; §3.1 reasons → Task 1 (`reason_of`) + Task 2; §4 snapshot/island/Settings → Tasks 2 and 4; §5 limits → Tasks 2-3 (caps) and Task 5 Step 1; §6 tests and acceptance → Tasks 1-3 and Task 5.
- **Placeholder scan:** none; the one test that depends on the harness (`a_rollout_that_a_hook_already_tracks_is_never_silent`) states exactly what it must assert and that it must fail without the `rollouts` check.
- **Type consistency:** `SilentReason` (Task 1) is the type of `SessionView.silent` (Task 2) and the TypeScript union (Task 4); `Signal::CodexSilent { id, project, host, folder_exists, written_ms }` (Task 2) is exactly what Task 3 sends; `parse_session_meta`/`host_of` (Task 1) are what Task 3 calls.
- **Review Focus:** 1 → Task 3 test 2; 2 → Task 1 test 2 and Task 3 test 3; 3 → Task 1 and Task 2 tests (120 000 / 120 001); 4 → Task 2 test 4; 5 → Task 1 test 3 and Task 3 tests 3 and 5.
