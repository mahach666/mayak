# Mayak sub-project 1 — shared interface contract

Source of truth: `docs/superpowers/specs/2026-10-01-mayak-multi-session-core-design.md` (the spec, Russian).
This contract fixes names, types, signatures, ownership and task boundaries for T1..T20. It does not
contain task steps. Where the spec is silent, the choice is recorded in §7 "Decisions not in the spec"
(referenced as `D<n>`). If this contract and the spec disagree, the spec wins and this file is wrong.

All paths below are relative to `windows/` unless they start with `.github/` or `docs/`.

Verified toolchain facts used here: rustc 1.98, `windows` 0.61.3, `tauri` 2.12.0 (command args are
camelCase on the JS side by default), `tokio` 1.53 (`ServerOptions::create_with_security_attributes_raw`
exists), `serde_json` with `preserve_order`. No new crates anywhere (spec §4).

---

## 1. File structure map

Legend: NEW = created in sub-project 1. Tasks in order of first touch.

### Rust — shared / relay

| File | Responsibility | Tasks |
|---|---|---|
| `ident.rs` NEW | Product identifiers shared by app and relay (spec §5.1), included via `#[path]`. | T1 |
| `Cargo.lock` | Package renames only (no new crates). | T1 |
| `hook/Cargo.toml` | Package/bin `mayak-hook`; `[lib] name = "mayak_hook"`; windows features. | T1, T3 |
| `hook/src/main.rs` | Process entry only: argv, stdin, wiring of lib calls, exit 0. Includes `ident`. | T1, T3 |
| `hook/src/lib.rs` NEW | Testable relay core: argv parsing, agent detection, BOM strip, enrichment, `talk_to`, decision JSON. | T3 |
| `hook/src/win.rs` | Win32 for the relay: SID, same-user pipe check, process table, ancestors. Module of the lib. | T1 (comments), T3 |
| `hook/src/dump.rs` NEW | Debug-only `--dump` line (length, SHA-256, age, ancestor chain). `#[cfg(debug_assertions)]`. | T3 |

### Rust — app (`src-tauri/`)

| File | Responsibility | Tasks |
|---|---|---|
| `src-tauri/Cargo.toml` | Package `mayak`, lib `mayak_lib`; windows features `Win32_Storage_FileSystem`, `Win32_System_IO` (T6). | T1, T6 |
| `src-tauri/tauri.conf.json` | productName, identifier, window title, bundle resource `mayak-hook.exe`, descriptions. | T1 |
| `src-tauri/capabilities/default.json` | Description string only. | T1 |
| `src-tauri/nsis/hooks.nsh` | Uninstall: `$LOCALAPPDATA\Mayak\{bin,inbox,mayak.log}`; Codex-integration hint. | T1, T14 |
| `src-tauri/src/main.rs` | `mayak_lib::run()`. | T1 |
| `src-tauri/src/lib.rs` | App wiring, Tauri commands, `AppSink`, `IslandPublisher`, `AppPaths`. Includes `ident`. | T1, T4, T12, T13, T14 |
| `src-tauri/src/pipe.rs` | Named-pipe server + router (`route`, `RouterSink`, `serve`), approval Pending/ack/decision. | T1, T4, T15 |
| `src-tauri/src/hooks.rs` | Hook commands dispatch per target; legacy Claude target (PermissionRequest only); `ensure_hook_exe`. | T1, T14 |
| `src-tauri/src/hookfile.rs` NEW | Write protection shared by both targets: exact-bytes diff, format preservation, `PendingPlan`, fingerprint, backup, atomic write. | T14 |
| `src-tauri/src/codex_hooks.rs` NEW | `~/.codex/hooks.json` merge (append/replace/placeholder), command path allow-list, top-level key check. | T14, T15 (`CODEX_EVENTS` and two of its tests, only if S2 rule R1 says reduce) |
| `src-tauri/src/settings.rs` | Settings struct, paths (`Mayak`), one-time Coucou copy; alert prefs. | T1, T14 |
| `src-tauri/src/log.rs` | Log file `mayak.log` (T1). T11: under `cfg(test)` `line` writes to the running test's stderr, never to `mayak.log`; signature unchanged (D77). | T1, T11 |
| `src-tauri/src/secrets.rs` | `SERVICE = ident::BUNDLE_ID`. | T1 |
| `src-tauri/src/tray.rs` | Tray id/tooltip/menu strings. | T1 |
| `src-tauri/src/integrations.rs` | User-Agent string only. | T1 |
| `src-tauri/src/win_user.rs` | Comment wording only. | T1 |
| `src-tauri/src/files.rs` | `ingest_into(source, inbox)` seam; test uses explicit temp base. | T1 (comments), T19 |
| `src-tauri/src/testutil.rs` NEW | `#[cfg(test)]` temp-dir helper shared by app tests. | T1 |
| `src-tauri/src/observe/mod.rs` NEW | Module root. T4: `pub mod` lines only. T13: `start()`, `Handle`, store thread, publication, `approval_context`. | T4, T5..T12 (mod lines), T13 |
| `src-tauri/src/observe/keys.rs` NEW | Identity types: `SessionKey`, `claude_key`, `codex_key` (T4), `ProcId` (T6), `HostKind` (T7). | T4, T6, T7 |
| `src-tauri/src/observe/paths.rs` NEW | `normalize`, `project_of`, `ProjectCache`, `glob_match`, basenames. Pure. | T5 |
| `src-tauri/src/observe/tail.rs` NEW | `JsonlTail`. | T5 |
| `src-tauri/src/observe/win.rs` NEW | `DirWatcher`, `proc::identify`, `proc::Registry`; Windows integration tests. | T6 |
| `src-tauri/src/observe/claude.rs` NEW | T7: session-file parsing, acceptance, host classification (pure). T11: `claude-io` thread. | T7, T11 |
| `src-tauri/src/observe/claude_transcript.rs` NEW | Transcript facts: main chain, tool pairing, labels, titles, `terminal_outcome`. Pure. | T7 |
| `src-tauri/src/observe/codex.rs` NEW | T8: `HookEvent`, rollout record parsing, host classification, index titles (pure). T12: `codex-io` thread. T15: rule updates from S2/S3, each only if its decision rule says so (`classify_host` pattern; paginated branch of `parse_rollout_line`, D70; one more place of the subagent `Meta` keys, D67). | T8, T12, T15 |
| `src-tauri/src/observe/model.rs` NEW | Pure model: types, `Store`, Claude (T9) and Codex (T10) state machines, snapshot. | T9, T10 |
| `src-tauri/src/observe/model_tests.rs` NEW | Table tests of `Store` (child module of `model`). | T9, T10 |
| `src-tauri/src/observe/text.rs` NEW | T7: `collapse`, `tool_label`. T9: `Alert.text` templates. | T7, T9 |
| `src-tauri/src/observe/fixtures/claude/*` NEW | Sanitized session files and transcripts. | T7 |
| `src-tauri/src/observe/fixtures/codex/*` NEW | Sanitized rollout, hook payloads, session index. T15, only if its decision rules say so: `rollout_paginated.jsonl` (D70), `rollout_subagent_meta_s3.jsonl` (D67). | T8, T15 |

### Front end, build, CI

| File | Responsibility | Tasks |
|---|---|---|
| `package.json`, `package-lock.json` | `name: "mayak-windows"`, `predev`/`prebuild` → `-p mayak-hook`. | T1 |
| `scripts/pack.mjs` | Artifact names `Mayak-Windows-<v>-setup.exe`, `Mayak-Windows-setup.exe`. | T1 |
| `.github/workflows/windows.yml` | Artifact/release names follow `pack.mjs` (D1). | T1 |
| `.github/workflows/windows-test.yml` NEW | PR/dispatch CI: `npm ci`, `npm run build`, `cargo test --workspace`. | T19 |
| `index.html`, `settings.html` | `<title>Mayak</title>`, `<title>Settings — Mayak</title>`. | T1 (T18: no further change, D2) |
| `dev/upload-preview.html` | Page title only (`Mayak — upload sequence preview`). | T1 |
| `vite.config.ts` | Shared-sounds plugin id `mayak-shared-sounds` only; `SOUNDS_DIR` unchanged. | T1 |
| `README.md` | Product name, exe and artifact names, folders and log file (Mochi untouched). | T1 |
| `src/core/bridge.ts` | Strings (T1); spec §10 TS types, sessions commands/events, hooks with target (T16). | T1, T16 |
| `src/core/sessions.ts` NEW | Snapshot store with rev guard, row counting, helpers. | T16 |
| `src/core/state.ts` | `Settings` alert fields (T14); remove `integration_claude`, `hooksInstalled`; `alertKey`; `defaultView`; `ApprovalInfo.{sessionKey, projectName}` (T16, D59); `effectiveState`, `CardSubject`, `alertFallback` (T17, D65). | T14, T16, T17 |
| `src/core/layout.ts` | `IslandViewName` + `VIEW_LAYOUTS` entries (T16); T17 Step 0 only checks those entries (D35), then adds `sessionsHeight`, `islandSize(…, sessionRows)`. | T16, T17 |
| `src/island/observer.ts` NEW | Subscribes snapshots (T16) and alerts (T17). | T16, T17 |
| `src/island/island.ts` | `onSnapshot`, Mochi `bodyColor = null` while `Sessions.hasSessions` (T16, D59); `handleAlerts` (D63–D65), `cardRank`, counters, new actions, `setActive` calls in `syncDom` (T17, D57), `greeting.interrupt()` on `coucou → home` (T17, D64). | T16, T17 |
| `src/island/fsm.ts` | `holdVisible`, `setHoldVisible`. | T17 |
| `src/island/hooks.ts` | Product name in the line-264 comment (T1); `PermissionRequest` branch: unconditional `island.alert("approval")`, no `integration_claude` badge/state, `ApprovalInfo` session fields (T16, D59); reduced to the legacy approval card (T17). | T1 (comment), T16, T17 |
| `src/island/integrations.ts` | Drop `integration_claude` configured block. | T16 |
| `src/main.ts` | `registerObserver(island)`. | T16 |
| `src/views/sessions.ts` NEW | Sessions list view, waiting card, session card, elapsed formatting, `colorForProject` (D60), card subject helpers `cardSubject`/`subjectWho`/`outcomeSummary` (D65). | T17 |
| `src/views/views.ts` | String (T1); placeholders + in-island badge + approval caption from `pendingApproval.projectName` (T16, D59); finished/error bound to `alertKey`, header tab (T17). | T1, T16, T17 |
| `src/views/integrations.ts` | Drop `integration_claude` branch. | T16 |
| `src/views/icons.ts` | `ICONS.list` (sessions tab). | T17 |
| `src/style.css` | Product name in the line-1 comment (T1); sessions list, rows, counters, waiting card (T17). | T1 (comment), T17 |
| `src/settings/main.ts` | Strings (T1); Claude status fetched lazily, not in `main()` (T14, D58); fallback `HookStatus` literal gains `target`/`outdated`/`notes` (T16); full rework incl. the focus re-read of D66 (T18). | T1, T14, T16, T18 |
| `src/settings/settings.css` | Styles for new sections. | T18 |

Not files in the repo: S1 result (T2), S2–S5 results (T15), acceptance checklist and PR body (T20) are
recorded in the PR description (D3).

---

## 2. Rust public items per module

Conventions for every module:
- Wire types (anything serialized to the webview) use `#[serde(rename_all = "camelCase")]`.
  Enums use `tag = "kind"` only where spec §10 shows a `kind` field (`Phase`). `Health` and
  `codexHooks` are externally tagged exactly as the TS shapes in §10 require.
- Pure functions take time (`now_ms: i64`), base paths (`&Path`) and probes (`impl Fn(&Path) -> bool`)
  as parameters. Only `lib.rs` reads `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `USERPROFILE`, `APPDATA`,
  `LOCALAPPDATA` for the observer and installer (existing `settings::config_dir/local_dir` keep their
  own env reads for app-owned folders).
- Channels are `std::sync::mpsc` (unbounded). No tokio outside `pipe.rs`.
- Logs never contain payload text, prompts, commands or summaries: only keys, event names, phases,
  counts, lengths, reasons.

### 2.0 Threads and channels (final state after T13)

| Thread (std::thread name) | Owner / spawned by | Receives | Sends to |
|---|---|---|---|
| `store` | `observe::start` (T13) | `Receiver<Signal>` | `Sender<ClaudeInput>`, `Sender<CodexInput>`, `Publisher` |
| `claude-io` | `claude::spawn` (T11) | `Receiver<ClaudeInput>` | `Sender<Signal>` |
| `codex-io` | `codex::spawn` (T12) | `Receiver<CodexInput>` | `Sender<Signal>` |
| `dirwatch-sessions`, `dirwatch-projects` | `DirWatcher::spawn` from claude-io | ReadDirectoryChangesW | `Sender<ClaudeInput>` (via sink closure) |
| `proc-exit-<pid>` | `proc::Registry::watch` | WaitForSingleObject | `Sender<Signal>` (`ProcExit`, via closure) |
| `signals-drain` | `lib.rs` T12 only, removed in T13 | `Receiver<Signal>` | — |
| (tokio tasks) | `pipe::serve` (started with `tauri::async_runtime::spawn`); one `tokio::spawn` task per connection; `approval_context` on `tokio::task::spawn_blocking` (D46) | pipe connections | `RouterSink` |

All three channels are `std::sync::mpsc::channel()` (unbounded, FIFO). Senders: `Sender<Signal>` is
cloned into claude-io, codex-io, every exit waiter and `observe::Handle`; `Sender<CodexInput>` into
`observe::Handle` (router intake) and the store; `Sender<ClaudeInput>` into the watchers and the store.
A failed `send` (receiver gone) is logged once and ignored by the sender; it never blocks.

### 2.1 `ident.rs` (T1)

Exactly spec §5.1:

```rust
pub const APP_SLUG: &str = "mayak";
pub const APP_NAME: &str = "Mayak";
pub const APP_DIR: &str = "Mayak";
pub const HOOK_EXE: &str = "mayak-hook.exe";
pub const PIPE_PREFIX: &str = "mayak-";
pub const MARKER: &str = "mayak-hook";
pub const LEGACY_MARKERS: &[&str] = &["coucou-hook"];
pub const LEGACY_DIR: &str = "Coucou";
pub const BUNDLE_ID: &str = "io.github.mahach666.mayak";
```

Included as `#[allow(dead_code)] #[path = "../../ident.rs"] mod ident;` in `hook/src/main.rs` and
`src-tauri/src/lib.rs` (spec §5). `hook/src/lib.rs` does not include it (the pipe name is a parameter).

### 2.2 `hook/src/lib.rs` (T3)

`hook/Cargo.toml`: `[lib] name = "mayak_hook", path = "src/lib.rs"`; `[[bin]] name = "mayak-hook",
path = "src/main.rs"`; windows features add `Win32_System_Diagnostics_ToolHelp`,
`Win32_Storage_FileSystem`, `Win32_System_IO` (the last two only for the in-test pipe server, D4).

```rust
pub mod win;
#[cfg(debug_assertions)]
pub mod dump;

pub const MAX_FIELD_LEN: usize = 2_000;
pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
pub const CODEX_BUDGET: Duration = Duration::from_secs(1);            // whole run incl. connect
pub const FIRE_AND_FORGET_BUDGET: Duration = Duration::from_secs(2);  // Claude, not waiting (unchanged)
pub const DECISION_BUDGET: Duration = Duration::from_secs(110);       // Claude PermissionRequest (unchanged)
pub const MAX_ANCESTOR_STEPS: usize = 6;
pub const RELAY_VERSION: u64 = 2;
/// (payload key, env var) terminal context, inserted only when absent — unchanged from Coucou.
pub const TERM_CONTEXT: &[(&str, &str)] = &[
    ("term_program", "TERM_PROGRAM"), ("wt_session", "WT_SESSION"),
    ("term_session_id", "TERM_SESSION_ID"), ("vscode_pid", "VSCODE_PID"),
    ("session_pid", "CLAUDE_CODE_SSE_PORT"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent { Claude, Codex }
impl Agent { pub fn as_str(self) -> &'static str }            // "claude" | "codex"

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args { pub agent: Agent, pub event: Option<String>, pub dump: bool }

/// argv without argv[0]. `--agent claude|codex` (missing/unknown value → Claude, value consumed);
/// first non-flag token → event; other tokens ignored; unknown `--flags` ignored;
/// `--dump` sets `dump` only under cfg(debug_assertions) (ignored in release).
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Args;
/// Strips every leading U+FEFF (EF BB BF), not just one.
pub fn strip_boms(raw: &[u8]) -> &[u8];
/// Non-empty `payload.hook_event_name` wins, else `arg_event`, else "".
pub fn resolve_event(payload: &Map<String, Value>, arg_event: Option<&str>) -> String;
/// Codex if `event == "PermissionRequest"` and payload has a non-null `turn_id`; else `arg_agent`.
pub fn detect_agent(arg_agent: Agent, event: &str, payload: &Map<String, Value>) -> Agent;
/// `agent == Claude && event == "PermissionRequest"`.
pub fn waits_for_answer(agent: Agent, event: &str) -> bool;
/// Codex → CODEX_BUDGET; Claude waiting → DECISION_BUDGET; Claude other → FIRE_AND_FORGET_BUDGET.
pub fn budget(agent: Agent, waits: bool) -> Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProc { pub pid: u32, pub created_ft: u64, pub image: Option<String> }

#[derive(Debug, Clone, PartialEq)]
pub struct Enrichment {
    pub agent: Agent,
    pub event: String,
    pub relay_ts: i64,                       // ms since Unix epoch, UTC
    pub cwd_fallback: Option<String>,
    pub term: Vec<(&'static str, String)>,   // resolved TERM_CONTEXT values
    pub claude_pid: Option<u32>,             // from CLAUDE_PID (Claude only)
    pub codex_proc: Option<AgentProc>,       // ancestor codex.exe (Codex only)
}
/// Mutates the payload object:
/// - `hook_event_name` = event; `agent` = "claude"|"codex"; `relay` = 2; `relay_ts` = ms (overwrite);
/// - removes `tool_response` (both); removes `transcript_path` for Claude only (Codex keeps it);
/// - Claude: `agent_pid` = claude_pid or null;
///   Codex: `agent_pid`, `agent_created`, `agent_image` from codex_proc, each null when unknown;
/// - `cwd` missing or empty → cwd_fallback; `term` keys inserted only when absent.
pub fn enrich(payload: &mut Map<String, Value>, e: &Enrichment);
/// Caps every string at MAX_FIELD_LEN bytes on a char boundary + '…' (unchanged behaviour).
pub fn truncate_strings(v: &mut Value);
/// Compact JSON + '\n'.
pub fn to_line(payload: &Map<String, Value>) -> String;
/// Connect (retry only on ERROR_PIPE_BUSY within CONNECT_TIMEOUT), require
/// `win::pipe_server_is_same_user`, write the line; when `waits`, read one reply line.
/// Runs on a worker thread; returns None after `budget` at the latest. Never panics.
/// `budget == Duration::ZERO` → None at once, no connect attempt (D45).
pub fn talk_to(pipe_name: &str, payload: &str, waits: bool, budget: Duration) -> Option<String>;
/// Unchanged shape; deny message is "Denied from Mayak".
pub fn decision_json(decision: &str) -> Option<String>;
/// Text to print on stdout: Some(decision_json) only for Claude + PermissionRequest + recognised reply.
/// Codex → always None.
pub fn stdout_for(agent: Agent, event: &str, reply: Option<&str>) -> Option<String>;
pub fn unix_ms_now() -> i64;
```

`hook/src/main.rs` (T3) does, in order: `let started = Instant::now()` (first statement) → `parse_args`
→ read stdin (empty → exit 0) → `strip_boms` → (debug + `--dump`: `dump::append`) → parse JSON object
(else exit 0) → `resolve_event` → `detect_agent` → Codex: `win::find_codex_ancestor(std::process::id())`
→ `enrich` → `truncate_strings` → `talk_to(&pipe_path(), …, budget(..).saturating_sub(started.elapsed()))`
→ print `stdout_for(..)` if Some → `exit(0)`. The budget is a deadline measured from `started`, so stdin,
the ancestor walk and enrichment count against the Codex 1 s (spec §5.2, D45).
`pipe_path() = format!(r"\\.\pipe\{}{}", ident::PIPE_PREFIX, sid)` (SID fallback `USERNAME`, unchanged).
Dump path: `%LOCALAPPDATA%\<ident::APP_DIR>\dump.log`.

### 2.3 `hook/src/win.rs` (T3)

```rust
pub fn current_user_sid() -> Option<String>;                 // existing
pub fn pipe_server_is_same_user(handle: HANDLE) -> bool;      // existing

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcRow { pub pid: u32, pub parent_pid: u32, pub exe_name: String /* szExeFile */ }

/// Toolhelp32 snapshot (TH32CS_SNAPPROCESS). Empty Vec on failure.
pub fn process_table() -> Vec<ProcRow>;
/// OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION) + GetProcessTimes → (high << 32) | low.
pub fn creation_ft(pid: u32) -> Option<u64>;
/// QueryFullProcessImageNameW(PROCESS_NAME_WIN32).
pub fn image_path(pid: u32) -> Option<String>;
/// Pure. Starts at `pid` (its row and created(pid) must exist, else empty). Each step: parent row
/// must exist, created(parent) must be Some and strictly < created(child); otherwise stop (reused PID).
/// At most `max` steps. Nearest ancestor first, each with its creation FILETIME.
pub fn ancestors(rows: &[ProcRow], created: impl Fn(u32) -> Option<u64>, pid: u32, max: usize)
    -> Vec<(ProcRow, u64)>;
/// ancestors(process_table(), creation_ft, pid, MAX_ANCESTOR_STEPS) → first exe_name equal to
/// "codex.exe" ignoring ASCII case → AgentProc { pid, created_ft, image: image_path(pid) }.
pub fn find_codex_ancestor(pid: u32) -> Option<crate::AgentProc>;
```

### 2.4 `hook/src/dump.rs` (T3, `#[cfg(debug_assertions)]` only)

```rust
pub fn sha256(data: &[u8]) -> [u8; 32];          // hand-written, FIPS 180-4 (no crate)
pub fn hex(bytes: &[u8]) -> String;              // lowercase
/// "<now_ms> len=<n> sha256=<hex> age_ms=<age> chain=<a>b>c> shell=<chain[0] or '-'>"
/// chain = ancestor exe names nearest first (≤ 6). stdin content is never written.
pub fn dump_line(stdin: &[u8], age_ms: i64, chain: &[String], now_ms: i64) -> String;
pub fn append(path: &Path, line: &str) -> std::io::Result<()>;   // creates parent dir
```

### 2.5 `src-tauri/src/pipe.rs` (T4; T15 changes only `create_instance`)

```rust
pub const MAX_PAYLOAD: usize = 1 << 20;
const ACK_TIMEOUT: Duration = Duration::from_millis(800);    // unchanged
const DECISION_TIMEOUT: Duration = Duration::from_secs(108); // unchanged

pub enum Reply { Ack, Decision(String), Decline }            // unchanged
#[derive(Default)]
pub struct Pending(pub Mutex<HashMap<String, tokio::sync::mpsc::Sender<Reply>>>); // plain struct

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route { Observe, Approval, Ignore }
/// `agent` is payload["agent"]; a missing field means "claude" (pre-Mayak relay, D5).
/// "codex" + any → Observe; "claude" + "PermissionRequest" → Approval; everything else → Ignore.
pub fn route(agent: &str, event: &str) -> Route;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApprovalContext { pub session_key: Option<SessionKey>, pub project_name: Option<String> }
/// Sets payload["request_id"], ["session_key"], ["project_name"] (string or null).
pub fn annotate_approval(payload: &mut Value, request_id: &str, ctx: &ApprovalContext);

#[derive(Debug, PartialEq)]
pub enum LineError { Oversize(usize), Invalid { len: usize, reason: String } }
/// Line = bytes before the first '\n' (or all). len > MAX_PAYLOAD → Oversize; not JSON or not an
/// object → Invalid. Caller logs `len` + reason and drops explicitly.
pub fn parse_line(buf: &[u8]) -> Result<Value, LineError>;
/// "hook <agent> <Event> s=<first 8 chars of session_id or '-'>"
pub fn log_line_for(agent: &str, event: &str, session_id: Option<&str>) -> String;

pub trait RouterSink: Send + Sync + 'static {
    fn pending(&self) -> &Pending;
    /// Route::Observe. Must not block.
    fn observe(&self, payload: Value);
    /// Route::Approval: data for `annotate_approval`. May block (Win32 + file probes); `serve`
    /// always calls it through `tokio::task::spawn_blocking` (D46).
    fn approval_context(&self, payload: &Value) -> ApprovalContext;
    /// Route::Approval: annotated payload to show (the app emits `hook` to "island").
    fn show_approval(&self, payload: Value);
}

/// `\\.\pipe\mayak-<SID>` (ident::PIPE_PREFIX).
pub fn pipe_name() -> String;
/// The only place a pipe instance is created (`first` → first_pipe_instance(true)). T15 adds the
/// current-SID DACL here, or documents why the default DACL stays.
pub fn create_instance(name: &str, first: bool) -> std::io::Result<NamedPipeServer>;
/// Accept loop; one task per connection; Approval keeps the existing Pending → ack (800 ms) →
/// decision (108 s) flow. Err only if the first instance cannot be created.
/// Runtime rules (D46): every `create_instance` call happens inside `serve` (so inside the runtime
/// that polls it); connection tasks are spawned with `tokio::spawn` (never
/// `tauri::async_runtime::spawn`); on Route::Approval the connection task calls
/// `tokio::task::spawn_blocking(move || sink.approval_context(&payload)).await` (owned clones of the
/// `Arc<S>` and payload; a JoinError is logged and gives `ApprovalContext::default()`), then
/// `annotate_approval`, then `show_approval`. `lib.rs` starts it with
/// `tauri::async_runtime::spawn(pipe::serve(name, sink))`, so in the app all tasks run on Tauri's
/// runtime and in tests on the test's current_thread runtime.
pub async fn serve<S: RouterSink>(name: String, sink: Arc<S>) -> std::io::Result<()>;

pub fn acknowledge(pending: &Pending, request_id: &str);
pub fn decline(pending: &Pending, request_id: &str);
pub fn answer(pending: &Pending, request_id: &str, decision: &str);  // "allow"|"always" → allow, else deny
```

`lib.rs` owns `Arc<Pending>` (managed as Tauri state) and `struct AppSink` (private):
- T4: `AppSink { app: AppHandle, pending: Arc<Pending> }`; `observe` logs
  `codex event dropped — observer not running` and drops; `approval_context` returns
  `ApprovalContext::default()`; `show_approval` = `app.emit_to("island", "hook", payload)`.
- T12: adds `codex: Sender<CodexInput>`; `observe` = `codex.send(CodexInput::Hook(payload))`.
- T13: replaces `codex` with `observe: observe::Handle`; `observe` = `handle.codex_hook(payload)`;
  `approval_context` = `observe::approval_context(payload)`.

### 2.6 `observe/mod.rs`

T4..T12 add only `pub mod <name>;` lines (keys T4; paths, tail T5; win T6; claude, claude_transcript,
text T7; codex T8; model T9). T13 adds:

```rust
pub struct ObserveConfig {
    pub claude_dir: PathBuf,     // $CLAUDE_CONFIG_DIR or %USERPROFILE%\.claude
    pub codex_home: PathBuf,     // $CODEX_HOME or %USERPROFILE%\.codex
    pub appdata: PathBuf,        // %APPDATA% (host classification)
    pub localappdata: PathBuf,   // %LOCALAPPDATA% (host classification)
}

pub trait Publisher: Send + 'static {
    fn alerts(&self, alerts: &[Alert]);              // "session-alert"
    fn snapshot(&self, snapshot: &SessionsSnapshot); // "sessions"
}

#[derive(Clone)]
pub struct Handle { /* signals: Sender<Signal>, codex: Sender<CodexInput>,
                       shared: Arc<Mutex<SessionsSnapshot>>,
                       signals_closed / codex_closed: Arc<AtomicBool>, so a failed send is
                       logged once per channel (D81) */ }
impl Handle {
    pub fn send(&self, s: Signal);                 // never blocks; send errors are logged
    pub fn codex_hook(&self, payload: Value);      // → codex-io CodexInput::Hook
    pub fn snapshot(&self) -> SessionsSnapshot;    // clone of the published snapshot
}

/// Spawns threads "store", "claude-io", "codex-io" with one shared proc::Registry.
/// A source that fails to spawn reports SourceHealth Error and the others still run.
pub fn start(cfg: ObserveConfig, publisher: Box<dyn Publisher>) -> Handle;

/// Real router annotations: payload["agent_pid"] → proc::identify → claude_key(pid, created_ft);
/// payload["cwd"] → paths::project_of(cwd, git_probe) → basename(root). Each part None on failure.
pub fn approval_context(payload: &Value) -> crate::pipe::ApprovalContext;

/// Store loop body, exposed for tests. Blocks on `rx`; each batch = first message + everything
/// available via try_recv. Per batch: Store::apply each signal with `now()`; dispatch effects
/// (TrackTranscript/UntrackTranscript → claude, UntrackRollout/CodexTicker → codex);
/// on Visible(true) also send ClaudeInput::Retry; write `shared`; publish per `should_emit`.
/// Returns when `rx` is disconnected.
pub(crate) fn run_store(rx: Receiver<Signal>, publisher: Box<dyn Publisher>,
                        claude: Sender<ClaudeInput>, codex: Sender<CodexInput>,
                        shared: Arc<Mutex<SessionsSnapshot>>, now: fn() -> i64);

/// Pure publication rule (spec §9):
/// became_visible → true; visible → next.rev != prev.rev; hidden → had_alerts ||
/// counts/attention/hold/aggregate differ from prev. prev None → true.
/// `prev` is the last snapshot *emitted*, not the last one computed (D81).
pub(crate) fn should_emit(prev: Option<&SessionsSnapshot>, next: &SessionsSnapshot,
                          visible: bool, became_visible: bool, had_alerts: bool) -> bool;
pub(crate) fn now_ms() -> i64;   // SystemTime → ms since Unix epoch
```

Alerts are emitted (when non-empty) before the snapshot of the same batch. The store thread tracks
`visible` from Visible signals; initial value false; `lib.rs` sends `Visible(true)` right after
`start()` because the island launches expanded (D6).

### 2.7 `observe/keys.rs` (T4, T6, T7)

```rust
pub type SessionKey = String;                                  // T4 (D7)
pub fn claude_key(pid: u32, created_ft: u64) -> SessionKey;    // "claude:<pid>:<created_ft decimal>"
pub fn codex_key(session_id: &str) -> SessionKey;              // "codex:<session_id>"

#[derive(Debug, Clone, PartialEq, Eq, Hash)]                   // T6
pub struct ProcId { pub pid: u32, pub created_ft: u64, pub image: String }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]  // T7
#[serde(rename_all = "camelCase")]   // "claudeDesktop" | "cursor" | "vsCode" | "codexDesktop" | "unknown"
pub enum HostKind { ClaudeDesktop, Cursor, VsCode, CodexDesktop, Unknown }
```

`model.rs` re-exports these with the explicit list given in §2.15 (no glob re-export), so
`model::claude_key` matches spec §5.4.

### 2.8 `observe/paths.rs` (T5)

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NormPath { pub display: String, pub key: String }
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectRef { pub key: String, pub root: String /* NormPath.display of the root */ }

/// Spec §5.7 order: trim → strip "file:///" (ASCII case-insensitive) → "/c:/"→"c:/" → '/'→'\'
/// → strip "\\?\" → drive letter uppercase → collapse repeated '\' → resolve "." and ".."
/// lexically (".." at root is dropped) → strip trailing '\' except "X:\" (bare "X:" → "X:\")
/// → key = display.to_lowercase(). No percent-decoding, no UNC handling (D8).
pub fn normalize(raw: &str) -> NormPath;
/// First of normalize(raw) and its ancestors (self included) where probe(dir\.git) is true;
/// none → normalize(raw) itself. Root "C:\" probes "C:\.git".
pub fn project_of(raw: &str, probe: impl Fn(&Path) -> bool) -> ProjectRef;
/// Real probe: `p.exists()` (folder or file).
pub fn git_probe(p: &Path) -> bool;

#[derive(Default)]
pub struct ProjectCache { /* HashMap<normalized key of raw, ProjectRef> */ }
impl ProjectCache {
    pub fn new() -> Self;
    /// Lifetime cache owned by one source thread.
    pub fn get(&mut self, raw: &str, probe: impl Fn(&Path) -> bool) -> ProjectRef;
}

/// Segment glob over lowercase '\'-separated keys. Pattern segments: "**" = zero or more
/// segments, "*" = exactly one segment, "abc*" = one segment starting with "abc", else literal.
pub fn glob_match(key: &str, pattern: &str) -> bool;
/// Last component of a display path; "C:\" → "C:".
pub fn basename(display: &str) -> String;
/// basename of the parent; None for a drive root.
pub fn parent_basename(display: &str) -> Option<String>;
```

### 2.9 `observe/tail.rs` (T5)

```rust
pub const WINDOW_BYTES: u64 = 256 * 1024;

pub struct JsonlTail { /* private: path, offset, partial line, skip-first-line flag */ }
impl JsonlTail {
    pub fn new(path: impl Into<PathBuf>) -> Self;               // offset 0
    /// offset = current length. Missing file → offset 0, Ok(0). A last line without '\n'
    /// began before us: its rest is discarded by the next read (D72).
    pub fn start_at_eof(&mut self) -> io::Result<u64>;
    /// offset = max(0, len - bytes); if > 0 the bytes up to and including the first '\n' are
    /// discarded by the next read. Missing file → offset 0.
    pub fn start_with_window(&mut self, bytes: u64) -> io::Result<()>;
    /// Each call: open (share READ|WRITE|DELETE via OpenOptionsExt::share_mode) → read → close.
    /// File shorter than the consumed offset → restart from 0. Missing file → Ok((vec![], 0))
    /// and restart at 0, so a re-created file is read from its start (D72).
    /// Returns complete lines as (start offset of the line, text without '\r\n'); blank lines are
    /// skipped; the incomplete last line is kept and not returned.
    /// Second value = eof (see `eof`).
    pub fn read_new(&mut self) -> io::Result<(Vec<(u64, String)>, u64)>;
    /// First byte not yet consumed (as a returned or blank line, or as skipped pre-existing
    /// bytes); the held incomplete last line is not consumed. It equals the end of
    /// the last complete line, except right after `start_at_eof`/`start_with_window` landed
    /// inside a line and until that line's '\n' is read: then it is a mid-line offset (D73).
    pub fn eof(&self) -> u64;
}
```

There is no `path()` accessor (D71): the private `path` field stays, and callers (T11, T12) keep
their own copy of the path.

### 2.10 `observe/win.rs` (T6)

`src-tauri/Cargo.toml` windows features add `Win32_Storage_FileSystem`, `Win32_System_IO`
(ToolHelp is not needed by the app, D4).

```rust
pub const WATCH_BUFFER: usize = 64 * 1024;
pub const MASK_NAME_WRITE_SIZE: u32;   // FILE_NOTIFY_CHANGE_FILE_NAME | LAST_WRITE | SIZE = 0x19
#[cfg(test)]                           // test-only (D74): mask test + ignored diagnostic
pub const MASK_NAME_WRITE: u32;        // FILE_NAME | LAST_WRITE = 0x11

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction { Added, Removed, Modified, RenamedOld, RenamedNew, Other(u32) }

#[derive(Debug)]
pub enum WatchEvent {
    Changes(Vec<(FileAction, String /* path relative to the watched dir, as reported */)>),
    Overflow,              // 0 bytes returned, or the call failed with ERROR_NOTIFY_ENUM_DIR (1022)
    Error(io::Error),      // any other failure; thread ends after sending this
}

pub struct DirWatcher;
impl DirWatcher {
    /// Opens `dir` synchronously (CreateFileW FILE_LIST_DIRECTORY, share READ|WRITE|DELETE,
    /// FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED): missing dir → Err(NotFound) (caller
    /// reports Missing). Then the blocking std thread "dirwatch-<dir file name>" keeps one
    /// overlapped ReadDirectoryChangesW request (WATCH_BUFFER buffer) queued, blocks in
    /// GetOverlappedResult(.., bWait = TRUE) until it completes and calls `sink` with the result.
    /// Returns after the first request is queued, so an enumeration done after `spawn` returns
    /// cannot miss a change (D76). `sink` returning false ends the thread (the sink is dropped).
    pub fn spawn(dir: &Path, recursive: bool, mask: u32,
                 sink: impl FnMut(WatchEvent) -> bool + Send + 'static) -> io::Result<()>;
}
```

Implementation notes (verified against `windows` 0.61.3): `Win32::Foundation::HANDLE` is a plain
`*mut c_void` newtype without `Send`, so the directory handle is converted right after `CreateFileW`
into `std::os::windows::io::OwnedHandle` (`OwnedHandle::from_raw_handle(h.0)`), moved into the thread,
and wrapped per call as `HANDLE(owned.as_raw_handle())`; dropping it closes the handle. The
manual-reset completion event (`CreateEventW(None, true, false, ..)`, the `hEvent` of each request's
`OVERLAPPED`) is held the same way. `ReadDirectoryChangesW` takes
`dwnotifyfilter: FILE_NOTIFY_CHANGE`, so the call passes `FILE_NOTIFY_CHANGE(mask)` (the `MASK_*`
constants stay `u32`). A completed request with 0 bytes, or an error whose code is
`ERROR_NOTIFY_ENUM_DIR`, maps to `Overflow` and the loop continues (compare
`err.code() == ERROR_NOTIFY_ENUM_DIR.to_hresult()`); any other error maps to `Error` and the thread
ends (D76).

```rust
pub mod proc {
    #[derive(Debug)]
    pub enum ProcError { NotFound, AccessDenied, Exited, Os(std::io::Error) }
    /// OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION):
    /// ERROR_INVALID_PARAMETER → NotFound, ERROR_ACCESS_DENIED → AccessDenied;
    /// WaitForSingleObject(h, 0) == WAIT_OBJECT_0 → Exited; GetProcessTimes → created_ft;
    /// QueryFullProcessImageNameW → image (failure → Os). The handle is returned for Registry.
    pub fn identify(pid: u32) -> Result<(ProcId, std::os::windows::io::OwnedHandle), ProcError>;

    #[derive(Clone, Default)]
    pub struct Registry { /* Arc<Mutex<HashSet<(u32, u64)>>> */ }
    impl Registry {
        pub fn new() -> Self;
        /// At most one waiter per (pid, created_ft). Returns false (handle dropped) if already
        /// watched, or if the waiter thread could not be started: then the entry is removed
        /// again and `proc-exit thread not started pid=…` is logged, so a later proof registers
        /// again; a caller needs nothing more for either `false` (D75). Thread
        /// "proc-exit-<pid>" waits INFINITE on the given handle, removes the entry, then calls
        /// on_exit(pid, created_ft); a failed wait is logged and does not call on_exit.
        pub fn watch(&self, id: &ProcId, handle: OwnedHandle,
                     on_exit: impl FnOnce(u32, u64) + Send + 'static) -> bool;
        pub fn is_watched(&self, pid: u32, created_ft: u64) -> bool;
    }
}
```

### 2.11 `observe/claude.rs` — parsing part (T7)

```rust
/// Built field by field from a `serde_json::Value` (no `Deserialize` derive, D56): each field is read
/// from its camelCase key; a missing key or a value of the wrong JSON type gives None for that field
/// only. Unknown keys are ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeSessionFile {
    pub pid: Option<u32>,                         // JSON number that fits u32
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub proc_start: Option<String>,               // decimal FILETIME; a JSON string as is, or a JSON
                                                  // u64 number rendered in decimal
    pub version: Option<String>,
    pub kind: Option<String>,
    pub entrypoint: Option<String>,
    pub name: Option<String>,
    pub name_source: Option<String>,
    pub status: Option<String>,
    pub waiting_for: Option<String>,
    pub status_updated_at: Option<i64>,           // ms
    pub updated_at: Option<i64>,
    pub spare: Option<bool>,
    pub parked_job_id: Option<String>,
    pub pid_domain: Option<String>,
}

/// `^[0-9]+\.json$` without a regex crate. "30584.abc.key" → false.
pub fn is_session_file_name(name: &str) -> bool;
/// Empty/whitespace-only → Err("empty"); invalid JSON → Err(reason); valid JSON that is not an
/// object → Err("not an object"). Any JSON object → Ok (wrong-typed fields are None, D56).
/// Never panics.
pub fn parse_session_file(bytes: &[u8]) -> Result<ClaudeSessionFile, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason { Spare, Parked, ForeignDomain, Incomplete }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accept { Track { pid: u32, proc_start: u64 }, Skip(SkipReason) }
/// In order: spare == Some(true) → Spare; parked_job_id non-empty → Parked; pid_domain present and
/// not starting with "win32:" → ForeignDomain; pid missing, proc_start missing or not a u64,
/// session_id or cwd missing/empty → Incomplete (counts as unreadable, D9); else Track.
pub fn accept(f: &ClaudeSessionFile) -> Accept;

/// `image` is raw; compared via paths::normalize(image).key with paths::glob_match:
///   "**\packages\claude_*\localcache\roaming\claude\claude-code\*\claude.exe" or
///   "<normalize(appdata).key>\claude\claude-code\*\claude.exe"
///       → ClaudeDesktop if entrypoint == Some("claude-desktop"), else Unknown;
///   "**\.cursor\extensions\anthropic.claude-code-*\**\claude.exe" → Cursor;
///   "**\.vscode\extensions\anthropic.claude-code-*\**\claude.exe" → VsCode; else Unknown.
pub fn classify_host(image: &str, entrypoint: Option<&str>, appdata: &Path) -> HostKind;
```

### 2.12 `observe/claude_transcript.rs` (T7)

```rust
pub const INTERRUPT_MARKER: &str = "[Request interrupted by user";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUse { pub id: String, pub label: String }   // label = text::tool_label(name, input)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError { pub code: Option<String> /* "error" */, pub status: Option<u16> /* apiErrorStatus */,
                      pub text: String /* collapsed message text */ }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleKind { Custom, Agent, Ai }   // customTitle | agentName | aiTitle

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    /// main-chain `type: user`
    User { interrupted: bool, tool_results: Vec<String /* tool_use_id */> },
    /// main-chain `type: assistant`
    Assistant { stop_reason: Option<String>, api_error: Option<ApiError>,
                text: String /* text blocks joined by ' ', collapse(_, 160) */,
                tool_uses: Vec<ToolUse> },
    /// `type` ∈ {custom-title, agent-name, ai-title} (any chain)
    Title { kind: TitleKind, text: String },
}

/// `isSidechain != true` and `type` ∈ {user, assistant}.
pub fn is_main_chain(v: &Value) -> bool;
/// None for records that are neither main-chain user/assistant nor title records.
/// `interrupted` = string content or a text block that starts (after leading whitespace) with
/// INTERRUPT_MARKER (`text.trim_start().starts_with(INTERRUPT_MARKER)`); tool_result content
/// never counts (D62).
pub fn fact_of(v: &Value) -> Option<Fact>;
/// Pairing: push every tool_use (id, label) of an Assistant fact; remove ids listed in a User
/// fact's tool_results. Other facts: no-op.
pub fn apply_tools(pending: &mut Vec<(String, String)>, fact: &Fact);
/// Unresolved (id, label) in order, from scratch.
pub fn pair_tools<'a>(facts: impl IntoIterator<Item = &'a Fact>) -> Vec<(String, String)>;
/// Label of the last tool_use among `facts`.
pub fn last_tool_label<'a>(facts: impl IntoIterator<Item = &'a Fact>) -> Option<String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome { Pending, Interrupted, Error(ApiError), Finished { summary: String } }
/// Over facts with offset >= after (D10):
/// 1. the last User fact is `interrupted` → Interrupted;
/// 2. let A = the last Assistant fact with stop_reason.is_some() || api_error.is_some();
///    A has api_error → Error;
/// 3. A.stop_reason != "tool_use" and no later fact has tool_uses (Assistant) or tool_results
///    (User) → Finished { summary: A.text, or if empty the text of the nearest earlier Assistant
///    fact in range with non-empty text, else "" };
/// 4. otherwise Pending (incl. no Assistant at all).
/// Second value = offset of the deciding fact (the User fact for Interrupted, A for Error/Finished),
/// None for Pending. The model uses it for the late-final-record rule (D51).
pub fn terminal_outcome(facts: &[(u64, Fact)], after: u64) -> (TurnOutcome, Option<u64>);
```

### 2.13 `observe/codex.rs` — parsing part (T8)

```rust
#[derive(Debug, Clone, PartialEq, Deserialize)]   // snake_case payload keys, unknown keys ignored
pub struct HookEvent {
    pub hook_event_name: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub cwd: String,
    pub transcript_path: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<Value>,
    pub last_assistant_message: Option<String>,
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
    pub relay_ts: i64,
    pub agent_pid: Option<u32>,
    pub agent_created: Option<u64>,
    pub agent_image: Option<String>,
}
pub fn parse_hook(payload: Value) -> Result<HookEvent, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RolloutRecord {
    Meta { id: String, cwd: Option<String>, parent_thread_id: Option<String>, agent_role: Option<String> },
    TaskStarted { turn_id: Option<String> },
    TaskComplete { turn_id: Option<String>, last_agent_message: Option<String>, error: Option<String> },
    TurnAborted { turn_id: Option<String> },
    Call { call_id: String, label: String },
    Other,     // legacy-shaped record that is not interesting
    Unknown,   // not legacy-shaped → source health Degraded
}
/// Legacy mode only (spec §5.6); T15 Step 9a adds a paginated branch only if S3 finds paginated
/// rollouts (D70). Top-level {type, payload}:
///   session_meta → Meta (payload.id, cwd, parent_thread_id, agent_role); each of the last two
///     from the top-level string key of the payload, else from
///     payload.source.subagent.thread_spawn.<key> (T8); T15 Step 9b adds one more place only if
///     S3 finds the keys elsewhere (D67);
///   event_msg + payload.type task_started | task_complete | turn_aborted → the variants;
///     task_complete.error: string → itself; object → its "message" string, else compact JSON (D11);
///   response_item + payload.type function_call | custom_tool_call | local_shell_call → Call
///     (label: function_call → text::tool_label(name, arguments parsed as JSON);
///      custom_tool_call → name; local_shell_call → tool_label("shell", payload.action));
///   other payload types of event_msg/response_item, and top-level types
///     turn_context | world_state | compacted → Other;
///   anything else (unknown top-level type, missing type, non-object) → Unknown.
pub fn parse_rollout_line(v: &Value) -> RolloutRecord;
/// normalize(agent_image).key with glob_match:
///   "**\.cursor\extensions\openai.chatgpt-*\**" → Cursor;
///   "**\.vscode\extensions\openai.chatgpt-*\**" → VsCode;
///   "**\windowsapps\openai.codex_*\**" or "<normalize(localappdata).key>\openai\codex\bin\*\codex.exe"
///     → CodexDesktop; None or anything else → Unknown.
pub fn classify_host(agent_image: Option<&str>, localappdata: &Path) -> HostKind;
/// session_index.jsonl text: the last line whose `id` == session_id and `thread_name` is non-empty.
pub fn index_title(index_text: &str, session_id: &str) -> Option<String>;
/// Non-empty and not ending in ".zst" (ASCII case-insensitive).
pub fn is_readable_rollout(path: &str) -> bool;
```

There is no `is_subagent_meta` (D67): no production code would call it. `RolloutRecord::Meta` still
carries `parent_thread_id` and `agent_role`, and a subagent `session_meta` never becomes a session by
construction (sessions come only from hooks keyed by the root `session_id`; rollout tails start at
EOF; T10 maps `Meta` to a no-op).

### 2.14 `observe/text.rs` (T7 helpers, T9 templates)

```rust
pub const SUMMARY_MAX: usize = 160;
pub const TARGET_FIELDS: &[&str] = &["command", "file_path", "path", "url", "query", "pattern", "prompt"];

/// Whitespace runs (incl. newlines) → one space, trim, keep the first `max_chars` chars. No ellipsis.
pub fn collapse(s: &str, max_chars: usize) -> String;                       // T7
/// "AskUserQuestion" → input.questions[0].question;
/// else "<tool> · <target>" where target = first non-blank string among TARGET_FIELDS
/// (an array of strings is joined with ' '); no target → "<tool>". Result collapse(_, 160).
pub fn tool_label(tool: &str, input: Option<&Value>) -> String;             // T7

pub fn agent_label(a: Source) -> &'static str;                             // "Claude" | "Codex"     T9
/// ClaudeDesktop "Desktop", Cursor "Cursor", VsCode "VS Code", CodexDesktop "Codex app", Unknown None.
pub fn host_label(h: HostKind) -> Option<&'static str>;                     // T9
/// "<code>: <text>" or "<text>".
pub fn error_summary(code: Option<&str>, text: &str) -> String;             // T9
/// "<Agent> · <project>[ (<host>)] needs permission[: <body>]" | "… asks[: <body>]" |
/// "… finished[: <body>]" | "… error[: <body>]"; whole string collapse(_, 160).
/// Host part omitted for Unknown; ": <body>" omitted when body is None or empty.
pub fn alert_text(agent: Source, project: &str, host: HostKind, kind: AttentionKind,
                  body: Option<&str>) -> String;                            // T9
```

### 2.15 `observe/model.rs` (T9 Claude + core, T10 Codex)

Re-exports: `pub use super::keys::{SessionKey, claude_key, codex_key, ProcId, HostKind};`
`pub use super::paths::ProjectRef; pub use super::claude::ClaudeSessionFile;`
`pub use super::codex::HookEvent;`
(T9 writes the keys line as two lines with the same names:
`#[allow(unused_imports)] pub use super::keys::{claude_key, codex_key};` — spec §5.4 names them
`model::claude_key`/`model::codex_key`, while callers use `observe::keys` — and
`pub use super::keys::{HostKind, ProcId, SessionKey};`.)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]                     // "claude" | "codex"
pub enum Source { Claude, Codex }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]                     // "ok" | "missing" | {"error":s} | {"degraded":{"unreadable":n}}
pub enum Health { Ok, Missing, Error(String), Degraded { unreadable: u32 } }

#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    ClaudeTranscript { key: SessionKey, rebind_at: Option<u64>, records: Vec<(u64, Value)>, eof: u64 },
    ClaudeFile { key: SessionKey, proc: ProcId, host: HostKind, project: ProjectRef,
                 file: ClaudeSessionFile, transcript_eof: Option<u64> },
    ClaudeGone { key: SessionKey },
    CodexRollout { key: SessionKey, path: String, records: Vec<(u64, Value)>, eof: u64 },
    CodexHook { key: SessionKey, event: HookEvent, proc: Option<ProcId>, host: HostKind,
                project: ProjectRef, rollout_eof: Option<u64>,
                title: Option<String> /* D12: session_index title, first sight only */ },
    CodexHooksInstalled(bool),
    ProcExit { pid: u32, created_ft: u64 },
    Ack(SessionKey),
    Visible(bool),
    AlertPrefs { waiting: bool, finished: bool, error: bool },
    SourceHealth { source: Source, health: Health },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    TrackTranscript { key: SessionKey, session_id: String },
    UntrackTranscript { key: SessionKey },
    UntrackRollout { key: SessionKey },     // emitted for every removed Codex session (D13)
    CodexTicker(bool),                      // emitted only when the wanted state changes
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied { pub alerts: Vec<Alert>, pub effects: Vec<Effect> }

// ── internal state ──
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitKind { Permission, Input, Other(String) }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase { Working, Waiting { kind: WaitKind }, Idle, Unknown(String) }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    None, Pending,
    Finished { summary: String, at_ms: i64 },
    Error { code: Option<String>, status: Option<u16>, text: String, at_ms: i64 },
    Interrupted { at_ms: i64 },
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptTitles { pub custom: Option<String>, pub agent: Option<String>, pub ai: Option<String> }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertPrefs { pub waiting: bool, pub finished: bool, pub error: bool }  // Default: all true

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub key: SessionKey, pub agent: Source, pub host: HostKind, pub proc: Option<ProcId>,
    pub logical_id: String,              // Claude sessionId; Codex session_id
    pub cwd: String, pub project: ProjectRef, pub title: Option<String>,
    pub phase: Phase, pub since_ms: i64,
    pub activity: Option<String>, pub detail: Option<String>,
    pub outcome: Outcome, pub acked_at: Option<i64>,
    // Claude
    pub file_status_ts: i64, pub turn_start_offset: u64, pub idle_eof: u64, pub transcript_bound: bool,
    pub pending_tools: Vec<(String, String)>,
    pub waiting_for: Option<String>,     // last waitingFor (waiting→waiting rule)
    pub file_name: Option<String>,       // session file `name` (title fallback)
    pub titles: TranscriptTitles,
    pub facts: Vec<(u64, Fact)>,         // facts with offset >= turn_start_offset, last 1024 (D14)
    // Codex
    pub current_turn: Option<String>, pub turn_started_ts: i64,
    pub closed_turns: VecDeque<String>,  // ≤ 8, oldest dropped
    pub wait_key: Option<(String /*turn*/, String /*tool*/, u64 /*fnv1a64(canonical_json(input))*/)>,
}

#[cfg(test)]                                           // test-only (D68)
pub struct Project { pub key: String, pub root: String, pub sessions: Vec<SessionKey> } // derived

pub struct Store { /* sessions: BTreeMap<SessionKey, Session>, rev: u64, visible: bool,
                      prefs: AlertPrefs, claude_health: Health, codex_health: Health,
                      codex_hooks: CodexHooksView, ticker_on: bool */ }
impl Store {
    pub fn new() -> Self;   // rev 0, visible false, prefs all true, healths Ok, NotInstalled
    pub fn apply(&mut self, s: Signal, now_ms: i64) -> Applied;
    pub fn snapshot(&self, now_ms: i64) -> SessionsSnapshot;
    #[cfg(test)] pub fn session(&self, key: &str) -> Option<&Session>;   // test-only (D68)
    #[cfg(test)] pub fn projects(&self) -> Vec<Project>;                 // test-only (D68)
    #[cfg(test)] pub fn rev(&self) -> u64;                               // test-only (D68)
}

/// Object keys sorted recursively, compact.
pub fn canonical_json(v: &Value) -> String;
pub fn fnv1a64(bytes: &[u8]) -> u64;
/// basename(root); keys whose basenames collide (case-insensitive) → "<parent_basename>\<basename>".
pub fn display_names(projects: &[(String /*key*/, String /*root*/)]) -> HashMap<String, String>;
```

**Wire (snapshot) types**, all `#[derive(Debug, Clone, PartialEq, Serialize)]`:

```rust
#[serde(rename_all = "camelCase")]
pub enum WaitView { Permission, Input, Other }
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PhaseView { Working, Waiting { wait: WaitView, label: String }, Idle, Unknown { label: String } }
// label: Permission → "permission", Input → "input", Other(s) → s; Unknown(label) → label (D15)

#[serde(rename_all = "camelCase")]
pub enum OutcomeKind { Finished, Error, Interrupted }
#[serde(rename_all = "camelCase")]
pub struct OutcomeView { pub kind: OutcomeKind, pub summary: String, pub at: i64 }
// None/Pending → null; Error summary = text::error_summary(code, text); Interrupted summary "".

#[serde(rename_all = "camelCase")]               // also the Alert kind
pub enum AttentionKind { Permission, Input, Error, Finished }
#[serde(rename_all = "camelCase")]
pub enum BotState { Approval, Question, Error, Working, Finished, Idle }

#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub key: String, pub agent: Source, pub host: HostKind, pub title: Option<String>,
    pub phase: PhaseView, pub since: i64, pub activity: Option<String>, pub detail: Option<String>,
    pub outcome: Option<OutcomeView>, pub attention: Option<AttentionKind>, pub acked: bool,
}
#[serde(rename_all = "camelCase")]
pub struct ProjectView { pub key: String, pub name: String, pub root: String, pub sessions: Vec<SessionView> }
#[serde(rename_all = "camelCase")]
pub struct Counts { pub waiting: u32, pub error: u32, pub finished: u32, pub working: u32 }
#[serde(rename_all = "snake_case")]   // "not_installed" | "installed_unseen" | {"receiving":{hosts,since}}
pub enum CodexHooksView { NotInstalled, InstalledUnseen, Receiving { hosts: Vec<HostKind>, since: i64 } }
#[serde(rename_all = "camelCase")]
pub struct Sources { pub claude: Health, pub codex: Health, pub codex_hooks: CodexHooksView }
#[serde(rename_all = "camelCase")]
pub struct SessionsSnapshot {
    pub rev: u64, pub now: i64, pub projects: Vec<ProjectView>, pub attention: Vec<String>,
    pub counts: Counts, pub hold: bool, pub aggregate: BotState, pub sources: Sources,
}
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub key: String, pub kind: AttentionKind, pub rank: u8, pub agent: Source, pub host: HostKind,
    pub project: String /* display name */, pub title: Option<String>, pub detail: Option<String>,
    pub text: String, pub since: i64,
}
```

**Model rules fixed by this contract** (everything else: spec §6, §7 verbatim):
- Episode start: Waiting → `since_ms`; Idle with Finished/Error → `at_ms`; otherwise none.
  `unacked(s)` = episode exists and (`acked_at` is None or `< start`). `SessionView.acked = !unacked`.
- Episode/ack bookkeeping (D49): every new episode start sets `acked_at = None` — entering Waiting
  from any other phase, a new Claude waiting episode (new `statusUpdatedAt` or `waitingFor`), a Codex
  PermissionRequest that is not a `:retry`, and setting Outcome Finished/Error (incl. the Codex
  Finished → Error upgrade). `Ack(key)` sets `acked_at = max(now_ms, episode start)` when an episode
  exists, else `now_ms`. So an ack applies exactly to the episode current when the store processes it,
  whatever the clocks of `statusUpdatedAt`/`relay_ts`; an ack in Working still hides nothing later.
- Outcome time (D50): `at_ms = now_ms` (store clock) for every outcome, Claude and Codex (Finished,
  Error, Interrupted, incl. the Codex Finished → Error upgrade). Waiting `since` stays
  `statusUpdatedAt` (Claude) / `relay_ts` (Codex) per spec §7.
- `attention(s)`: unacked Waiting(Permission) → Permission; unacked Waiting(Input|Other) → Input;
  unacked Idle+Error → Error; unacked Idle+Finished → Finished; else None.
- Rank: 0..3 as above (unacked only), 4 Working, 5 everything else. Order within rank: 0,1 `since`
  asc; 2,3 `at_ms` desc; 4 `since` asc; 5 `since` desc; ties by key asc. `Alert.rank` uses the same
  0..3 numbers; most important = smallest rank (D16).
- `snapshot.attention` = keys with attention != None in global rank order (ranks 0..3 only).
- Sessions inside a project use the same order; projects by best rank asc, then max over sessions of
  `max(since_ms, outcome at_ms)` desc, then key asc.
- `counts.waiting/error/finished` count unacked episodes; `counts.working` counts all Working
  sessions (Working is not an episode, D17). `hold` = any unacked Waiting && `prefs.waiting`.
  `aggregate` = first of approval (unacked Waiting(Permission)) → question (unacked Waiting(other))
  → error (unacked Error) → working (any Working) → finished (unacked Finished) → idle.
- `rev` += 1 whenever any snapshot field other than `rev`/`now` would differ after `apply`.
- Alerts only for live transitions; alerts of a kind disabled in `AlertPrefs` are not returned
  (Permission/Input ← waiting, Finished ← finished, Error ← error) (D18). Alert.text via
  `text::alert_text`, body = detail (permission/input), summary (finished), error_summary (error).
- Claude `since_ms` = max(prev, `statusUpdatedAt` or `now_ms` if missing). A new waiting episode
  with an unchanged `statusUpdatedAt` uses `max(prev_since + 1, statusUpdatedAt)` (D19); it is
  unacked because a new episode clears `acked_at` (D49). Missing `status` → `Unknown("")`.
- Claude facts: every `ClaudeTranscript` record goes through `claude_transcript::fact_of`; Title facts
  update `titles` regardless of offset; title = custom ?? agent ?? ai ?? file `name` (D20).
  A `ClaudeTranscript` with `rebind_at: None` while `transcript_bound == false` is ignored entirely
  (records of a transcript the model already let go, e.g. right after a sessionId change, D52).
  `rebind_at = Some(o)` clears facts, pending_tools and titles, sets idle_eof = turn_start_offset = o,
  `transcript_bound = true`, then processes the records. At a turn start (`turn_start_offset =
  idle_eof`) facts below it are pruned and `pending_tools`/`activity` are rebuilt from the remaining
  facts (`pair_tools`, `last_tool_label`) (D21). `detail` = label of `pending_tools.last()`.
  Outcome evaluation: `terminal_outcome(&facts, turn_start_offset)` mapped to `Outcome` with
  `at_ms = now_ms`; Finished/Error alert, Interrupted none; at most one outcome alert per turn.
  Late final record (D51): when a `ClaudeTranscript` resolves a Pending outcome while the phase is
  Idle, `idle_eof = max(idle_eof, o + 1)` where `o` is the deciding offset returned by
  `terminal_outcome`, so the next turn (`turn_start_offset = idle_eof`) never re-reads that record.
  sessionId change → Untrack + Track effects, transcript_bound false, facts/pending/titles cleared.
  New session with a sessionId → `TrackTranscript`. ClaudeGone/ProcExit → remove + `UntrackTranscript`.
- Codex: unknown key creates the session with `phase = Unknown("")`, `since = relay_ts`, then applies
  the hook as a live transition; `SessionEnd` for an unknown key is ignored (D22). Missing `turn_id`
  (hook or rollout record) means `current_turn`. Rollout-driven transitions use `since =
  max(prev, now_ms)`; `TaskStarted` calls `start_turn(T, now_ms)`. `start_turn` also clears `detail`
  and `activity`. `wait_key` is cleared whenever the phase leaves Waiting. Permission `detail` =
  `tool_label(tool_name, tool_input)`, prefixed `"<agent_type>: "` when `agent_id` is present (D23).
  Stop/TaskComplete summary = `collapse(msg, 160)` ("" when absent). Hook events outside the §7 table
  only update host/proc/cwd/project/title. Every `CodexHook` updates `cwd`, `project`, `host` (when
  not Unknown), `proc` (when Some), `title` (when Some).
- `CodexHooksInstalled(false)` → NotInstalled; `(true)` → InstalledUnseen unless Receiving. Any
  `CodexHook` → Receiving (host appended once, `since` = relay_ts of the first event) (D24).
- `Ack(key)`: `acked_at` per the episode/ack rule above (D49); an Idle session with `proc == None`
  is removed instead (+ `UntrackRollout`). Unknown key → no-op.
- Ticker: after every apply, want = `visible && any Codex session in Working|Waiting`; on change emit
  `CodexTicker(want)`.

### 2.16 `observe/claude.rs` — I/O part (T11)

```rust
pub struct ClaudeConfig {
    pub base: PathBuf,       // the .claude dir (sessions\ and projects\ under it)
    pub appdata: PathBuf,    // host classification
    pub watch: bool,         // false in tests: no DirWatcher, inputs are fed manually
}
#[derive(Debug)]
pub enum ClaudeInput { Sessions(WatchEvent), Projects(WatchEvent), Effect(Effect), Retry }

/// Thread "claude-io". Spawns DirWatcher(sessions\, non-recursive) and DirWatcher(projects\,
/// recursive), both MASK_NAME_WRITE_SIZE, wrapping events into ClaudeInput. Initial enumeration of
/// sessions\ treats every file as new (no alerts by model rules).
pub fn spawn(cfg: ClaudeConfig, signals: Sender<Signal>, registry: proc::Registry)
    -> io::Result<Sender<ClaudeInput>>;
```

Behaviour contract (spec §5.5 plus):
- Session-file events (D47): each `Changes` batch is processed as a whole. Names are filtered by
  `is_session_file_name` and de-duplicated (first-seen order); the action codes only say "look at
  this name", the file system decides. For each name: read the file; `NotFound` → `ClaudeGone` for
  that pid's known key (none known → nothing); `Ok(bytes)` → normal processing (parse, proof,
  D25); any other read error → treated like an unreadable read of a known file (state kept,
  `degraded`). So a temp-file + rename-over write (`[Removed x.json, RenamedNew x.json]` or
  `[RenamedOld x.json, RenamedNew x.json]` in one batch) never yields `ClaudeGone`.
  Proof: `identify(pid)`, created_ft == procStart, else skip
  (NotFound/Exited) or unreadable (AccessDenied). procStart change for a known pid → `ClaudeGone`
  (old key) then new. Registry: `watch(proc, handle, |pid, ft| signals.send(ProcExit{pid, created_ft: ft}))`,
  called only after the session's first `ClaudeFile` was sent; a known pid that is not
  `is_watched` is proved again and, when it is still the same process, watched again (D78).
- `ClaudeFile` is sent only when the parsed file differs from the last one sent for that pid, or
  the session is new (D25). When `status` or `statusUpdatedAt` changed and a transcript is bound:
  drain tail → `ClaudeTranscript{rebind_at: None}` (only if records non-empty) → `ClaudeFile
  {transcript_eof: Some(eof)}`. Otherwise `transcript_eof: None`.
- `TrackTranscript{key, session_id}`: search `projects\*\<session_id>.jsonl`; found →
  `start_with_window(WINDOW_BYTES)`, read, send `ClaudeTranscript{rebind_at: Some(eof)}`;
  not found → expected name `<session_id>.jsonl` (lowercase). Projects events: match by file name
  only; bound name Modified → read, `ClaudeTranscript{rebind_at: None}` if records non-empty.
  Search again on every `ClaudeFile` of a tracked session without a bound transcript.
  **Expected names are new files (D48, spec §7 "для нового файла o = 0"):** once a name is in the
  expected set, every later discovery of it — a projects notification (Added/Modified/RenamedNew),
  the re-search on `ClaudeFile`, the Overflow/Retry re-search — binds with `JsonlTail::new` (offset
  0), reads from 0, sends `ClaudeTranscript{rebind_at: Some(0)}` and removes the name from the set.
  Only the initial search inside `TrackTranscript`, when the file already exists, uses the 256 KiB
  window and `Some(eof)`.
  `UntrackTranscript` drops tail, expected name and tracking.
- Records: each line parsed as JSON; unparsable lines skipped (D26).
- Overflow (either watcher): re-enumerate sessions\ (the D47 per-name handling for every name
  present plus every known pid's name, so a vanished file gives `ClaudeGone`),
  re-search all expected names (binding at 0, D48), re-read all tails.
- Health (sent on change): watcher Error → `Error(msg)`; sessions\ or projects\ missing → `Missing`;
  else `Degraded{unreadable: n}` if n > 0, else `Ok`. n = number of pids currently unreadable
  (parse failure of a known file until the next good read, Incomplete files, AccessDenied) (D9).
- `Retry` (store forwards it on `Visible(true)`): re-spawn missing/failed watchers; on success
  behave as Overflow.
- Further T11 rules (D79): a known file that becomes `Spare`/`Parked`/`ForeignDomain` → `ClaudeGone`;
  `ProcError::Os` counts as unreadable; the expected-name re-search runs before every `ClaudeFile` of
  a tracked session without a bound transcript; with `watch: false` `Retry` always rescans and
  `Missing` comes from `is_dir()`; a transcript that exists but cannot be opened stays unbound and is
  later bound with the window at its EOF, never at 0; a session id containing `\`, `/`, `:` or `..`
  is not tracked; a rescan re-checks unreadable names whose files vanished.

### 2.17 `observe/codex.rs` — I/O part (T12)

```rust
pub const TICK: Duration = Duration::from_secs(1);
pub struct CodexConfig { pub codex_home: PathBuf, pub localappdata: PathBuf }
#[derive(Debug)]
pub enum CodexInput { Hook(Value), Effect(Effect) }

/// Thread "codex-io". Ticker = recv_timeout against a 1 s deadline while CodexTicker(true) is in
/// force; plain recv otherwise (no extra thread, D27).
pub fn spawn(cfg: CodexConfig, signals: Sender<Signal>, registry: proc::Registry)
    -> io::Result<Sender<CodexInput>>;
```

Behaviour contract (spec §5.6 plus):
- `Hook(v)`: `parse_hook` (failure → log length + reason, drop). Proof: `identify(agent_pid)` and
  created_ft == agent_created → `proc = Some`, `registry.watch(.., ProcExit)`; else None.
  Host `classify_host(agent_image, localappdata)`; project via own `ProjectCache`.
  First sight of a key in this run → `index_title(read <codex_home>\session_index.jsonl, id)`.
  Main rollout = `transcript_path` of events without `agent_id`, if `is_readable_rollout`; a new path
  gets `JsonlTail::start_at_eof`. Order: drain main rollout → `CodexRollout` (only if records
  non-empty) → `CodexHook{rollout_eof: Some(eof) | None}`. Log
  `codex <Event> s=<8> host=<host> proc=<yes|no>` (no content).
- Tails keyed by normalized path key; `UntrackRollout{key}` drops the key → path mapping, the tail
  if no other key uses it, and the "seen" mark (so a re-created session re-reads its title).
- Tick: drain every tracked tail → `CodexRollout` per key with non-empty records.
- Health: `Degraded{unreadable: n}` where n = tracked rollout paths that produced an `Unknown`
  record; else `Ok`. Sent on change.
- Further T12 rules (D78, D80): `registry.watch` for a proved agent runs after its `CodexHook` was
  sent; a rollout that produced an `Unknown` record stays counted until it is untracked; records
  drained from one path go to every session whose main rollout it is (key order); a
  `transcript_path` becomes a main rollout only when the base name of `normalize(path).key` is
  `rollout-*.jsonl` (else `codex rollout path rejected` is logged and nothing is opened); a rollout
  whose `start_at_eof` fails is not tracked and never read from 0 (the session's next hook tries
  again).

T12 temporary wiring in `lib.rs` (removed in T13): spawn codex-io with a `Sender<Signal>` whose
receiver is drained and dropped by thread "signals-drain".

### 2.18 `settings.rs` (T1, T14)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]          // no deny_unknown_fields: old "hooksInstalled" is ignored
pub struct Settings {
    pub sound_enabled: bool, pub sound_volume: f64, pub auto_close_interval: f64,
    pub absence_interval: f64, pub active_integrations: Vec<String>, pub screen: String,
    pub autostart: bool,
    #[serde(default = "default_model")] pub model: String,
    #[serde(default = "default_true")] pub alert_waiting: bool,     // T14
    #[serde(default = "default_true")] pub alert_finished: bool,    // T14
    #[serde(default = "default_true")] pub alert_error: bool,       // T14
}   // T14 removes `hooks_installed`
fn default_true() -> bool;
pub fn config_dir() -> PathBuf;     // %APPDATA%\Mayak   (T1)
pub fn local_dir() -> PathBuf;      // %LOCALAPPDATA%\Mayak (T1)
pub fn hook_exe_path() -> PathBuf;  // local_dir()\bin\ident::HOOK_EXE (T1)
/// T1. Copies <appdata>\Coucou\settings.json → <appdata>\Mayak\settings.json when the target is
/// absent and the source exists (creates the Mayak dir). Ok(true) if copied. Original untouched.
pub fn copy_legacy_settings(appdata: &Path) -> std::io::Result<bool>;
pub fn load() -> Settings;          // T1: calls copy_legacy_settings(%APPDATA%) first, logs errors
pub fn save(settings: &Settings) -> std::io::Result<()>;
```

### 2.19 `hookfile.rs` (T14)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]          // "claude" | "codex"
pub enum Target { Claude, Codex }
impl Target {
    /// None → Claude; "claude" | "codex"; anything else → Err("unknown target …").
    pub fn parse(raw: Option<&str>) -> Result<Target, String>;
    pub fn as_str(self) -> &'static str;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFormat { pub bom: bool, pub crlf: bool, pub final_newline: bool, pub mixed_eol: bool }
impl TextFormat { pub const NEW_FILE: TextFormat; }   // no BOM, LF, final newline, not mixed (D28)
/// crlf = first line ending is "\r\n"; mixed_eol = the bytes contain both a "\r\n" and a bare "\n"
/// (render writes every line ending in the `crlf` style, D54).
pub fn detect_format(bytes: &[u8]) -> TextFormat;
pub fn strip_bom(bytes: &[u8]) -> &[u8];
/// BOM stripped; empty/whitespace → {}; non-object or invalid JSON → Err (never "empty").
pub fn parse_object(bytes: &[u8], path: &str) -> Result<Value, String>;
/// serde_json pretty (2 spaces, preserve_order), EOL per format, BOM, final newline.
pub fn render(value: &Value, fmt: TextFormat) -> Vec<u8>;
pub fn fingerprint(bytes: &[u8]) -> String;            // FNV-1a 64, 16 hex; missing file = fingerprint(b"")
/// Existing LCS diff, moved unchanged ("No change." when equal).
pub fn unified_diff(before: &str, after: &str) -> String;
pub fn read_config(path: &Path) -> Result<Option<Vec<u8>>, String>;   // NotFound → Ok(None)
pub fn backup_path(config: &Path, stamp: &str) -> PathBuf;  // "<config>.mayak-backup-<stamp>"
pub fn local_stamp() -> String;                             // "YYYYMMDD-HHMMSS" (GetLocalTime)

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPlan {
    pub target: Target, pub install: bool, pub path: PathBuf, pub fingerprint: String,
    pub bytes: Vec<u8>, pub backup_path: PathBuf, pub unchanged: bool,
}
/// Plan + diff text + notes (D53, D54).
/// unchanged = match current { Some(b) => parse_object(b, ..) == Ok(next.clone()),
///                             None => next is an empty object };
///   serde_json `Map` equality ignores object key order and compares arrays positionally (matches
///   positional trust), so a re-install into a file with other indentation/EOL is not a write.
/// unchanged → bytes = current bytes (or empty), diff = "No change.", nothing will be written.
/// Otherwise bytes = render(next, detect_format(current) or NEW_FILE) and
///   diff = unified_diff(lossy(strip_bom(current or "")), lossy(strip_bom(bytes))); a missing file
///   shows every line as "+". notes gets "Line endings will be normalised to CRLF." (or "…to LF.")
///   when the current format is `mixed_eol` — the only byte change the line diff cannot show, since
///   BOM and final newline are preserved.
pub fn make_plan(target: Target, install: bool, path: &Path, current: Option<&[u8]>,
                 next: &Value, stamp: &str) -> (PendingPlan, String, Vec<String>);

#[derive(Default)]
pub struct Plans(Mutex<HashMap<Target, PendingPlan>>);
impl Plans {
    pub fn store(&self, plan: PendingPlan);   // replaces the plan of that target
    /// Removes and returns the plan only if target, install and fingerprint all match; else Err.
    pub fn take(&self, target: Target, install: bool, fingerprint: &str) -> Result<PendingPlan, String>;
}
/// Re-read; fingerprint differs → Err("… changed since the preview …"). `unchanged` →
/// Err("No change: <path> already has this entry; nothing was written.") and nothing written (the
/// UI disables Apply for such a preview, so this is only reached by a direct call, D53).
/// Else: backup = exact re-read bytes written to backup_path (only if the file
/// existed), write bytes to "<path>.<ext>.mayak-<pid>" (with_extension(format!("{ext}.mayak-{pid}"))),
/// rename over path. Ok(backup path), or Ok("") only when there was no previous file.
pub fn write_plan(plan: &PendingPlan) -> Result<String, String>;
```

### 2.20 `codex_hooks.rs` (T14; T15 may change `CODEX_EVENTS`)

```rust
pub const CODEX_EVENTS: &[(&str, u64)] = &[
    ("UserPromptSubmit", 10), ("PermissionRequest", 10), ("Stop", 10), ("Interrupt", 2), ("SessionEnd", 2),
];
pub fn hooks_json_path(codex_home: &Path) -> PathBuf;              // <codex_home>\hooks.json
/// GetShortPathNameW(long). Errors when the file does not exist.
pub fn short_path(long: &Path) -> Result<String, String>;
/// `^[A-Za-z]:(\\[\p{L}\p{N}._~-]+)+$` without regex: char::is_alphabetic / is_numeric (D29).
pub fn check_command_path(path: &str) -> Result<(), String>;
/// short_path(hook_exe) → check_command_path → "<path> --agent codex".
pub fn our_command(hook_exe: &Path) -> Result<String, String>;
/// {"hooks":[{"type":"command","command":<command>,"timeout":<timeout>}]}
pub fn our_group(command: &str, timeout: u64) -> Value;
/// Top level may only hold "description" and "hooks"; "hooks" must be an object; every event
/// value must be an array.
pub fn check_shape(v: &Value) -> Result<(), String>;
/// command (lowercase) contains ident::MARKER, or equals our_command ignoring ASCII case (D30).
pub fn is_ours(command: &str, our_command: &str) -> bool;
pub fn group_is_ours(group: &Value, our_command: &str) -> bool;
pub fn installed(v: &Value, our_command: &str) -> bool;
/// Per CODEX_EVENTS event: first ours group replaced in place (further ours groups → placeholder
/// {"hooks":[]}); none → appended at the end. Every other event key (not in CODEX_EVENTS, e.g.
/// after T15 shrinks the set) gets the `merge_uninstall` treatment for ours groups: last in its
/// array → removed, otherwise replaced by {"hooks":[]}; an array left empty → key removed (D55).
/// Missing file → `current` = {} → new object.
pub fn merge_install(current: &Value, our_command: &str) -> Result<Value, String>;
/// Ours group: last in its array → removed; else replaced by {"hooks":[]}. An array left empty →
/// event key removed; "hooks" kept (possibly {}) (D31).
pub fn merge_uninstall(current: &Value, our_command: &str) -> Result<Value, String>;
/// Read-only status for Signal::CodexHooksInstalled; any read/parse problem → false.
pub fn installed_at(codex_home: &Path, our_command: Option<&str>) -> bool;
```

### 2.21 `hooks.rs` (T1 rename, T14 rework)

```rust
pub const CLAUDE_EVENTS: &[(&str, u64)] = &[("PermissionRequest", 120)];   // legacy target only

#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct HookStatus {
    pub target: Target, pub installed: bool, pub outdated: bool,
    pub settings_path: String,   // config path of the target (settings.json | hooks.json)
    pub hook_path: String, pub hook_ready: bool, pub notes: Vec<String>,
}
#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct HookPreview {
    pub diff: String, pub backup: String, pub settings_path: String,
    pub fingerprint: String, pub notes: Vec<String>,
    pub unchanged: bool,          // additive to spec §8's shape (D53): nothing would be written
}
pub struct HookCtx { pub claude_dir: PathBuf, pub codex_home: PathBuf, pub hook_exe: PathBuf, pub plans: Plans }

pub fn status(ctx: &HookCtx, target: Target) -> HookStatus;
pub fn preview(ctx: &HookCtx, target: Target, install: bool, stamp: &str) -> Result<HookPreview, String>;
pub fn apply(ctx: &HookCtx, target: Target, install: bool, fingerprint: &str) -> Result<String, String>;
/// T1: every candidate path and log message built from ident::HOOK_EXE.
pub fn ensure_hook_exe(app: &AppHandle);
```

Legacy Claude target: config `<claude_dir>\settings.json`; command unchanged in form
(`"\"<exe with '/'>\" PermissionRequest"`); `entry_is_ours` matches `MARKER` or any `LEGACY_MARKERS`;
install = remove all ours/legacy entries from every event, then append the PermissionRequest group;
uninstall = remove all ours/legacy entries (empty arrays/hooks removed as today).
`installed` = any ours/legacy entry; `outdated` = installed && install-merge result != current.
Codex `outdated` = installed && `merge_install(current) != current` (so stale Mayak groups under
events outside `CODEX_EVENTS` also make it outdated, D55). `notes` are Rust-computed
explanations (legacy entries found, unknown top-level keys, invalid JSON, rejected path, relay
missing) plus the `make_plan` notes in `HookPreview` (mixed line endings, D54).
`HookPreview.unchanged` = `PendingPlan.unchanged` (D53).
`settings_path`/`hook_path` keep the existing meaning.

---

## 3. Tauri commands, events, windows

Window labels: `"island"` (`island::WINDOW_LABEL`), `"settings"`. No capability changes.

Managed state (lib.rs): `Shared` (unchanged), `Arc<Pending>` (T4), `Chat`, `observe::Handle` (T13),
`HookCtx` (T14). `AppPaths { claude_dir, codex_home, appdata, localappdata }` + `fn app_paths()` is the
only env reader for observer/installer paths (T12 introduces it, T13/T14 use it).

| Command (Rust) | JS args | Returns | Task |
|---|---|---|---|
| `boot()` | — | `BootInfo` (no longer reads `~/.claude/settings.json`) | T14 |
| `save_settings(settings)` | `{ settings }` | — ; then `observe.send(AlertPrefs{..})` | T14 |
| `set_collapsed(collapsed)` | `{ collapsed }` | — ; then `observe.send(Visible(!collapsed))` | T13 |
| `sessions_snapshot()` | — | `SessionsSnapshot` | T13 |
| `session_ack(key)` | `{ key }` | — (`Signal::Ack`) | T13 |
| `session_open(key)` | `{ key }` | `bool`: host Cursor → `find_on_path("cursor")`, VsCode → `find_on_path("code")`, launched with the project root as one argument, no shell → true; otherwise / launcher missing → `explorer <root>` → false; unknown key → false (D32) | T13 |
| `hooks_status(target?)` | `{ target }` | `Result<HookStatus, String>`; default `"claude"` | T14 |
| `hooks_preview(target?, install)` | `{ target, install }` | `Result<HookPreview, String>` | T14 |
| `hooks_apply(target?, install, fingerprint)` | `{ target, install, fingerprint }` | `Result<String, String>` (backup path, or "" only when there was no previous file; unchanged plan → Err, D53) ; codex → after `hooks::apply` (`Plans::take` + `write_plan`) returns, on every call that got past `Target::parse` — Ok, Err, and a refused `Plans::take` alike (D82) — `observe.send(CodexHooksInstalled(codex_hooks::installed_at(&ctx.codex_home, our_command(&ctx.hook_exe).ok().as_deref())))` — re-read from disk, never the `install` flag | T14 |
| `approval_ack/decline/decision` | unchanged (`requestId`, `decision`) | unchanged; take `State<Arc<Pending>>` | T4 |
| all other existing commands | unchanged | unchanged | — |

Events:

| Event | Target | Payload | Emitted by |
|---|---|---|---|
| `sessions` | `emit_to("island")` | `SessionsSnapshot` | store thread via `Publisher` (T13) |
| `session-alert` | `emit_to("island")` | `Alert[]` | store thread via `Publisher` (T13) |
| `hook` | `emit_to("island")` | approval payload + `request_id`, `session_key`, `project_name` (Claude PermissionRequest only) | `AppSink::show_approval` (T4) |
| `settings-changed` | `emit` (all) | `Settings` | `save_settings` (unchanged; `hooks_apply` no longer emits it) |
| `cursor`, `tray`, `screen-changed`, `integration` | unchanged | unchanged | unchanged |

Setup order in `run()` (T13/T14): tray → settings window → island window → cursor poll → log banner
`--- Mayak <v> started ---` → `ensure_hook_exe` → `observe::start` → `Visible(true)` →
`AlertPrefs` from settings (T14) → `CodexHooksInstalled(installed_at(..))` (T14) → pipe `serve` →
integrations.

---

## 4. TypeScript

### 4.1 `src/core/bridge.ts` (T16) — types mirror spec §10 exactly

```ts
export type Host = "claudeDesktop" | "cursor" | "vsCode" | "codexDesktop" | "unknown";
export type Phase =
  | { kind: "working" }
  | { kind: "waiting"; wait: "permission" | "input" | "other"; label: string }
  | { kind: "idle" }
  | { kind: "unknown"; label: string };
export type OutcomeView = null | { kind: "finished" | "error" | "interrupted"; summary: string; at: number };
export type AttentionKind = null | "permission" | "input" | "error" | "finished";
export type BotState = "approval" | "question" | "error" | "working" | "finished" | "idle";
export type Health = "ok" | "missing" | { error: string } | { degraded: { unreadable: number } };
export type CodexHooksState =
  | "not_installed" | "installed_unseen" | { receiving: { hosts: Host[]; since: number } };
export interface SessionView {
  key: string; agent: "claude" | "codex"; host: Host; title: string | null;
  phase: Phase; since: number; activity: string | null; detail: string | null;
  outcome: OutcomeView; attention: AttentionKind; acked: boolean;
}
export interface ProjectView { key: string; name: string; root: string; sessions: SessionView[] }
export interface SessionsSnapshot {
  rev: number; now: number; projects: ProjectView[]; attention: string[];
  counts: { waiting: number; error: number; finished: number; working: number };
  hold: boolean; aggregate: BotState;
  sources: { claude: Health; codex: Health; codexHooks: CodexHooksState };
}
export interface Alert {
  key: string; kind: "permission" | "input" | "error" | "finished"; rank: number;
  agent: "claude" | "codex"; host: Host; project: string; title: string | null;
  detail: string | null; text: string; since: number;
}
export type HookTarget = "claude" | "codex";
export interface HookStatus {
  target: HookTarget; installed: boolean; outdated: boolean; settingsPath: string;
  hookPath: string; hookReady: boolean; notes: string[];
}
export interface HookPreview {
  diff: string; backup: string; settingsPath: string; fingerprint: string; notes: string[];
  unchanged: boolean;   // D53
}
```

T16 also updates the only `HookStatus` object literal outside the bridge — the fallback in
`src/settings/main.ts` (`?? { installed: false, settingsPath: "", hookPath: "", hookReady: false }`,
moved into the lazy loader by T14, D58) — to add `target: "claude", outdated: false, notes: []`, so
`tsc --noEmit` (`npm run build`) passes from T16 on.

`Bridge` additions/changes:

```ts
sessionsSnapshot: () => call<SessionsSnapshot>("sessions_snapshot"),
sessionAck: (key: string) => call<void>("session_ack", { key }),
sessionOpen: (key: string) => call<boolean>("session_open", { key }),
hooksStatus: (target: HookTarget = "claude") => call<HookStatus>("hooks_status", { target }),
hooksPreview: (install: boolean, target: HookTarget = "claude") =>
  callOrThrow<HookPreview>("hooks_preview", { target, install }),
hooksApply: (install: boolean, fingerprint: string, target: HookTarget = "claude") =>
  callOrThrow<string>("hooks_apply", { target, install, fingerprint }),
```

Free functions: `onSessions(handler: (s: SessionsSnapshot) => void): Promise<() => void>` and
`onSessionAlert(handler: (alerts: Alert[]) => void): Promise<() => void>` (thin wrappers over
`onEvent`). `BridgeEvent` gains `{ name: "sessions"; payload: SessionsSnapshot }` and
`{ name: "session-alert"; payload: Alert[] }`.

### 4.2 `src/core/sessions.ts` (T16)

```ts
export class SessionsStore {
  snapshot: SessionsSnapshot | null;
  readonly expandedIdle: Set<string>;     // project keys whose "+N idle" fold is open
  /** false (ignored) when s.rev <= last accepted rev; initial last rev = -1. */
  accept(s: SessionsSnapshot): boolean;
  get hasSessions(): boolean;             // any project with ≥ 1 session
  find(key: string): { session: SessionView; project: ProjectView } | null;
  /** 24 px lines the sessions view renders: project headers + visible rows + "+N idle" rows.
   *  Fold rule: acked idle sessions of a project > 3 and project not in expandedIdle → one fold row. */
  rowCount(): number;
  subscribe(fn: () => void): () => void;
}
export const Sessions: SessionsStore;
export function botStateName(s: BotState): BotStateName;      // identity mapping onto layout names
export function viewForAttention(k: Exclude<AttentionKind, null>): IslandViewName;
                                                                // permission|input → "waiting", error → "error", finished → "finished"
export const ALERT_SOUND: Record<Alert["kind"], SoundName>;   // permission/input "approval", error "error", finished "finish"
```

### 4.3 `src/island/observer.ts`

```ts
/** Order is fixed: listeners first, then the initial fetch.
 *  T16: `await onSessions(s => { if (Sessions.accept(s)) island.onSnapshot(s) })`, then
 *       `Bridge.sessionsSnapshot()` → the same accept/onSnapshot path (the rev guard drops duplicates,
 *       so a snapshot emitted between listen and fetch is never lost).
 *  T17: registers `onSessionAlert → island.handleAlerts(alerts)` before the initial fetch too. */
export function registerObserver(island: Island): Promise<void>;
```

### 4.4 Island, views, layout, FSM, state

`Island` (island.ts):
- T16: `onSnapshot(s: SessionsSnapshot): void` — `State.notify()`, and `animateGeometry(false)` when
  expanded on `"sessions"`. T17 adds `this.fsm.setHoldVisible(s.hold)` and clears an `{ alert }`
  `State.alertFallback` (D65).
- T16 (D59): `drawBot` sets `engine.bodyColor = null` while `Sessions.hasSessions` (Mochi shows the
  sessions aggregate, not the colour of whichever integration became the default focus); otherwise
  the existing `focus?.isIntegration ? hexToRGB(focus.color) : null`.
- T17: `handleAlerts(alerts: Alert[]): void` — rules (D16, D33, D63, D64, D65): paused → return;
  drop `permission` alerts whose key equals `State.pendingApproval?.sessionKey`; empty → return;
  top = smallest `rank`, ties first in batch. At most one sound `ALERT_SOUND[top.kind]` per batch,
  played only when the batch opens or replaces a card or lands on the sessions list (D63):
  - hidden, compact, or expanded on the launch greeting (`State.view === "greeting"`, D64) → sound,
    `State.alertKey = top.key`, `State.alertFallback = { alert: top }`,
    `this.alert(viewForAttention(top.kind))`;
  - expanded on a `waiting`/`finished`/`error` card and `top.rank` < `cardRank(view)` (the card on
    screen) → sound, `alertKey` + `alertFallback` as above, `setView(viewForAttention(top.kind))`;
  - expanded on `"sessions"` → sound, `State.notify()`, the view stays;
  - a card that is not outranked, `"approval"` (never replaced) and every other expanded view →
    `State.notify()` only (counters, pager), no sound.
  Private `cardRank(view)`: `error` 2, `finished` 3, `waiting` from `cardSubject()` (the alert's
  `rank`, else 0 for a permission wait, else 1). The FSM `coucou → home` transition calls
  `greeting.interrupt()`, so the greeting's timed sounds never follow an alert (D64).
- T17: compact counters replace the mini grid when `Sessions.hasSessions`: `!n` amber (waiting),
  `✕n` red (error), `✓n` green (finished), `•n` blue (working), non-zero only (D34).
- `State.effectiveState` (T17): `stateOverride ?? (Sessions.hasSessions ? botStateName(aggregate)
  : focusTask?.state ?? "idle")`.

`ViewActions` (views.ts) additions in T17; `openTerminal` removed:

```ts
openSession(key: string): void;     // row click: State.alertFallback = the clicked
                                    // { session, project } (D65), Bridge.sessionAck(key),
                                    // alertKey = key, open
                                    // "waiting" (Waiting) | "finished"/"error" (outcome unacked
                                    // before the click) | "session" (otherwise)
ackAlert(): void;                   // "OK": Bridge.sessionAck(State.alertKey), collapse()
pageAlert(step: 1 | -1): void;      // pager over snapshot.attention; view by that item's kind
openSessionTarget(): void;          // "↗ Open": Bridge.sessionOpen(State.alertKey)
toggleIdleFold(projectKey: string): void;
```

`ViewHost` gains optional `setActive?(on: boolean): void` (T17, D57). `Island.syncDom` computes, for
every view, `active = State.mode === "expanded" && name === State.view` (independent of the `on`
class toggle, which ignores mode) and calls `setActive(active)` only when that value changed since
the last call for that view. `syncDom` runs after every `State.notify()`, and `setMode` notifies, so
compact/hidden/expanded changes are seen. The sessions view and the `waiting` and `session` cards
use it to run one `window.setInterval(…, 1000)` only while active; the callback rewrites the
elapsed text nodes directly (`formatElapsed(Date.now() - since)`) and never calls `State.notify()`
(which would wake the rAF loop); `setActive(false)` clears the interval.

`IslandViewName` (T16) adds `"sessions" | "waiting" | "session"`. `VIEW_LAYOUTS` (T16):

```ts
sessions: { height: 160, botX: 320, botY: 90, botDiameter: 0, agentMode: "none" },   // height via sessionsHeight
waiting:  { height: 160, botX: 62, botY: null, botDiameter: 56, agentMode: "column" },
session:  { height: 160, botX: 62, botY: null, botDiameter: 58, agentMode: "column" },
```

T16 registers placeholder views for the three names (D35). T17 Step 0 greps these three
`VIEW_LAYOUTS` lines and changes nothing when they match (any other output → fix T16 first); T17 then
replaces the placeholders from `views/sessions.ts`:

```ts
export function buildSessions(actions: ViewActions): ViewHost;
export function buildWaiting(actions: ViewActions): ViewHost;
export function buildSession(actions: ViewActions): ViewHost;
export function formatElapsed(ms: number): string;   // "<60s" → "Ns"; "<60m" → "Nm"; else "Hh MMm"
/** Spec §10 project header dot (D60): FNV-1a 32 over the UTF-16 code units of `projectKey`,
 *  index = hash % PROJECT_PALETTE.length into a fixed palette of 8 hex colours declared in this
 *  file. Depends only on the project key — never on agent or host. */
export function colorForProject(projectKey: string): string;
/** D65: what the card of State.alertKey shows — the `{ alert }` fallback for that key, else
 *  Sessions.find(key), else the `{ session, project }` row fallback for that key, else null. */
export function cardSubject(): CardSubject | null;
/** D65: who-line of a subject; an alert has the project name but no project key, so no dot. */
export function subjectWho(subject: CardSubject, verb?: string): HTMLElement;
/** D65: summary of a finished/error card: the row's outcome summary, or the alert's `detail`. */
export function outcomeSummary(subject: CardSubject | null, kind: "finished" | "error"): string;
```

The waiting/finished/error cards render from `cardSubject()`, the session card from a row subject.

`layout.ts` (T17): `export function sessionsHeight(rows: number): number { return Math.min(300, 72 + 24 * rows); }`;
`islandSize(mode, view, chatCount = 0, sessionRows = 0)` uses it for `"sessions"`;
`Island.targetSize` passes `Sessions.rowCount()`.

`fsm.ts` (T17): `holdVisible = false`; `setHoldVisible(on: boolean): void` — on → off while
`state === "petit"` calls `schedulePetitHide()`; the `petitHide` timer callback does nothing while
`holdVisible`. `forceHidden()` is unchanged and wins over hold; `main.ts` keeps calling
`island.reveal()` after un-pause (covers "reveal when hold is on").

`State` (state.ts):
- T14: `Settings` adds `alertWaiting: boolean; alertFinished: boolean; alertError: boolean`
  (DEFAULT_SETTINGS true). `hooksInstalled` stays in TS until T16 (D36).
- T16: remove `hooksInstalled` (interface + default); remove `integration_claude` from
  `INTEGRATION_AGENTS`, `loadIntegrationTasks` (focus defaults to the first task or null),
  `toggleIntegration`; add `alertKey: string | null = null`; `defaultView()` →
  `Sessions.hasSessions ? "sessions" : (tasks.length === 0 ? "empty" : "overview")`.
- T16 (D59, moved from T17): `ApprovalInfo` adds `sessionKey: string | null; projectName: string`.
- T17: `effectiveState` as above.
- T17 (D65): `export type CardSubject = { alert: Alert } | { session: SessionView; project: ProjectView }`
  (after `ApprovalInfo`) and `alertFallback: CardSubject | null = null` (after `alertKey`).
  `handleAlerts` sets `{ alert: top }` whenever it sets `alertKey`; `onSnapshot` clears an
  `{ alert }` fallback (spec §9 emits a batch's alerts just before its snapshot); `openSession`
  stores the clicked row before `session_ack` (spec §7 removes an Idle Codex session without a
  process on ack).

`hooks.ts`:
- T16 (D59): in the same commit that removes `integration_claude`, the `PermissionRequest` branch
  fills `sessionKey` from `payload.session_key` (null when absent) and `projectName` from
  `payload.project_name`, falling back to the aliased basename of `cwd`; calls
  `island.alert("approval")` unconditionally (the `focused` test and the `CLAUDE_ID`
  `updateTask`/`setPillBadge`/`upsert` calls go, here and in the 110 s timeout callback). Without
  this, removing the pseudo-task would ack every request with no card on screen and leave Claude
  Code waiting ~110 s. One-card/decline/ack/110 s timeout logic is unchanged.
  `buildApproval` (views.ts) takes its caption from `State.pendingApproval?.projectName`
  (`<projectName>` + "needs permission", no dot — the payload carries no project key), no longer
  from `State.focusTask`.
- T17: reduced to that branch: handles only `PermissionRequest` payloads with `request_id`; the other
  event branches (dead since T4 routes them to `Ignore`), `CLAUDE_ID`, `upsert`, `clearSession`,
  `stepLabel` and `TOOL_LABELS` are removed. `island.alert("approval")` replaces a `waiting` card in
  place.

### 4.5 Settings window (T18)

`src/settings/main.ts` exports nothing; internal API:
`integrationSection(target: HookTarget): HTMLElement` (status → preview → apply/remove, shared by
both targets), `codexSection(snapshot: SessionsSnapshot | null)`, `alertsSection()`,
`advancedSection()` (collapsed by default; calls `Bridge.hooksStatus("claude")` only when expanded;
dotfiles warning text), `sourcesSection(snapshot)`. Snapshot fetched with `Bridge.sessionsSnapshot()`
at boot and on the window `"focus"` DOM event (D37). The window `focus` event also re-reads the
status of each installer section that shows its status rows (never under a preview, an error or an
apply result; Claude only while Advanced is open); snapshots with a `rev` not greater than the last
one taken are dropped; the `focus` listener is registered before `main()` awaits anything (D66).
A preview with `unchanged: true` shows "No change." and disables Apply (D53). The preview labels the
backup `Backup (made only if the file already exists) → <path>` (D66). Apply result `""` is shown
as "no previous file"; any other Ok value is the backup path.

---

## 5. Test conventions

- Rust unit tests live in `#[cfg(test)] mod tests` of the module under test; model table tests in
  `observe/model_tests.rs` (`#[cfg(test)] #[path = "model_tests.rs"] mod tests;` inside model.rs).
- Fixtures: `src-tauri/src/observe/fixtures/claude/` (`session_busy.json`, `session_idle.json`,
  `session_waiting_permission.json`, `session_waiting_input.json`, `session_waiting_other.json`,
  `session_spare.json`, `session_parked.json`, `session_linux_domain.json`,
  `session_no_procstart.json`, `session_garbage.json`, `session_mistyped_field.json` (valid
  session whose `waitingFor` is an object and `procStart` a JSON number: parses, `waiting_for` None,
  `proc_start` Some(decimal), `accept` → Track, D56), `turn_end_turn.jsonl`, `turn_refusal.jsonl`,
  `turn_api_error.jsonl`, `turn_interrupted.jsonl`, `turn_tool_use_after_end.jsonl`,
  `turn_no_assistant.jsonl`, `tools_pairing.jsonl` (incl. AskUserQuestion and sidechain records),
  `titles.jsonl`) and `observe/fixtures/codex/` (`rollout_legacy.jsonl`, `rollout_subagent_meta.jsonl`,
  `rollout_unknown.jsonl`, `hook_permission_request.json`, `hook_stop.json`, `session_index.jsonl`;
  T15 adds `rollout_paginated.jsonl` (D70) and `rollout_subagent_meta_s3.jsonl` (D67) only if its S3
  decision rules say so).
  Loaded with `include_str!("fixtures/…")`. Sanitized: generic text, fake UUIDs, paths under
  `C:\Users\dev\…`, no tokens, no real prompts. Host-path cases are string constants in tests.
- Temp dirs (app): `src-tauri/src/testutil.rs` (T1):
  `pub struct TempDir { pub path: PathBuf }`, `TempDir::new(tag: &str)` →
  `%TEMP%\mayak-test-<tag>-<pid>-<n>` (n = static AtomicU32), removed on Drop. Relay tests use a
  private copy of the same helper.
- Unique pipe names: `\\.\pipe\mayak-test-<tag>-<pid>-<n>`. Never `pipe_name()` / `mayak-<SID>`.
  App pipe tests run `serve` on `tokio::runtime::Builder::new_current_thread().enable_all()` and
  connect with `tokio::net::windows::named_pipe::ClientOptions`; because `serve` uses `tokio::spawn`
  and creates every instance itself (D46), all pipe I/O stays on that test runtime. Relay tests
  create the server with `CreateNamedPipeW` + `ConnectNamedPipe` on a std thread.
- Injection: process table → `win::ancestors(rows, created_fn, pid, max)`; `.git` probe →
  `project_of(raw, |p| set.contains(p))`; clocks → `Store::apply(_, now_ms)`, `Enrichment.relay_ts`,
  `run_store(.., now: fn() -> i64)`, `hookfile::make_plan(.., stamp)`; base paths → every function
  takes `&Path`; no test sets environment variables.
- I/O-thread tests (T11/T12) use `ClaudeConfig { watch: false }` and feed `ClaudeInput`/`CodexInput`
  directly; process proof uses the test process itself (`identify(std::process::id())`).
- From T11 on, unit tests never write the user's `mayak.log`: under `cfg(test)` `log::line` writes
  to the running test's stderr (D77).
- Windows integration tests (`#[cfg(windows)]`, in observe/win.rs): `identify` of the current process
  matches `std::process::id()` and `current_exe()` (case-insensitive); `Registry::watch` fires for a
  `cmd /d /c "set /p _="` child with piped stdin (alive until the test drops its stdin) using
  `child.as_handle().try_clone_to_owned()` and a hand-built `ProcId`, and a second `watch` of the
  same `(pid, created)` while the child still runs returns false (D38); `DirWatcher` reports a
  write-then-close and, after the test deletes every file in the watched `TempDir` and then
  `remove_dir`s it, reports `Error` — optionally preceded by one or more `Overflow` events (an empty
  batch can come before the failing call); `Changes` arriving in between are ignored by that
  assertion. The open-handle append diagnostic is `#[ignore]` (MASK_NAME_WRITE, 2 s window, prints
  observations).
- Required regression tests for review fixes (in addition to each task's own list):
  T11 — a sessions batch `[Removed x.json, RenamedNew x.json]` with `x.json` present sends no
  `ClaudeGone` (D47); a transcript that was expected by `TrackTranscript` and then created and found
  by the re-search on the next `ClaudeFile` yields `ClaudeTranscript{rebind_at: Some(0)}` whose
  records start at offset 0 (D48). T9 — idle → late `end_turn` record (Finished alert) → busy →
  idle with no stop_reason record (slash command) gives no second Finished (D51); after a sessionId
  change a `ClaudeTranscript{rebind_at: None}` changes nothing (D52); ack of waiting episode 1, then
  a new waiting episode with the same `statusUpdatedAt` is unacked with attention and `hold` (D49);
  an `Ack` applied before a waiting file whose `statusUpdatedAt` is older than the ack time leaves the
  new episode unacked (D49); ack in Working, then Finished → unacked (D49). T10 — Codex Finished
  `at_ms` equals the `now_ms` passed to `apply` (D50). T14 — `merge_install` removes or placeholders
  Mayak groups under an event outside `CODEX_EVENTS` and `outdated` is true before that (D55);
  `make_plan` re-install into a 4-space-indented CRLF file with the identical entry → `unchanged`,
  diff "No change." and `write_plan` writes nothing (D53); a mixed-EOL current file gives the
  line-ending note (D54). T3 — `talk_to(.., Duration::ZERO)` returns None without connecting (D45).
- Store publication tests drive `run_store` with a fake `Publisher` and drop the sender to end it.
- No TS test runner (spec §14); TS is checked by `tsc --noEmit` in `npm run build`.

---

## 6. Produces / Consumes per task

**T1 Rename to Mayak.** Produces: `ident.rs`; package/crate/bin renames (`mayak`, `mayak_lib`,
`mayak-hook`); `pipe_name()` and relay `pipe_path()` via `PIPE_PREFIX`; `settings::{config_dir,
local_dir, hook_exe_path}` → Mayak; `copy_legacy_settings` + call in `load()`; `log.rs` → `mayak.log`;
`secrets::SERVICE`; tray/strings/titles (§11 list incl. `index.html`, `settings.html`,
`settings/main.ts`, `views.ts:335`, `bridge.ts`, banners, User-Agent, "Denied from Mayak",
`hooks.nsh`, `capabilities` description); `ensure_hook_exe` from `HOOK_EXE`; `hooks.rs` MARKER via
ident + temp suffix `.json.mayak-<pid>` + test dir name; `pack.mjs` + `windows.yml` names; `testutil.rs`.
Remaining product-name strings outside the spec §11 list, also renamed in T1: `README.md`, the
`dev/upload-preview.html` title, the `vite.config.ts` plugin id, and two comments
(`src/island/hooks.ts:264`, `src/style.css:1`).
Consumes: —.

**T2 Spike S1.** Produces: S1 verdict recorded for the PR description (D3); go/no-go for T3+.
Consumes: T1 build (optional). No product code.

**T3 Relay.** Produces: `hook/src/lib.rs` (§2.2), `win.rs` (§2.3), `dump.rs` (§2.4), new `main.rs`
with the deadline budget from `started` (D45), hook features. Consumes: `ident` (T1).

**T4 Pipe & router.** Produces: §2.5 (`Route`, `route`, `RouterSink`, `ApprovalContext`,
`annotate_approval`, `parse_line`, `LineError`, `log_line_for`, `create_instance`, `serve` with the
D46 runtime rules — `tokio::spawn` per connection, `approval_context` via `spawn_blocking` —,
`acknowledge/decline/answer(&Pending, ..)`); `observe/mod.rs` with `pub mod keys;`;
`observe/keys.rs::{SessionKey, claude_key, codex_key}`; `lib.rs` `AppSink` (temporary observe),
`Arc<Pending>` state. Consumes: relay wire fields (T3: `agent`, `relay`, `relay_ts`, `agent_pid`).

**T5 Paths & tail.** Produces: §2.8, §2.9 (no `JsonlTail::path`, D71; tail rules D72, D73).
Consumes: `testutil::TempDir` (T1).

**T6 Win32 primitives.** Produces: §2.10 (incl. its implementation notes: `OwnedHandle` across the
thread, overlapped requests collected with `GetOverlappedResult` (D76), `FILE_NOTIFY_CHANGE(mask)`,
`ERROR_NOTIFY_ENUM_DIR` → Overflow; `MASK_NAME_WRITE` `#[cfg(test)]`, D74; `Registry::watch` false
semantics, D75; the `set /p` Registry test child, D38); `keys::ProcId`; app windows features.
Consumes: `testutil::TempDir` (T1, for the `DirWatcher` integration test).

**T7 Claude parsing.** Produces: §2.11 (lenient field-by-field `ClaudeSessionFile`, D56), §2.12
(`terminal_outcome` returning the deciding offset, D51; `interrupted` only for a text that starts
with the marker, D62), `text::{SUMMARY_MAX, TARGET_FIELDS, collapse,
tool_label}`, `keys::HostKind`, Claude fixtures incl. `session_mistyped_field.json`. Consumes:
`paths::{normalize, glob_match}` (T5).

**T8 Codex parsing.** Produces: §2.13 (no `is_subagent_meta`, D67; `subagent_meta_fields_are_parsed`
asserts `parent_thread_id`/`agent_role` of every line of `rollout_subagent_meta.jsonl`), Codex
fixtures. Consumes: `paths` (T5), `keys::HostKind` (T7), `text::tool_label` (T7).

**T9 Model core + Claude.** Produces: §2.15 types, `Store::{new, apply (Claude signals, Ack, Visible,
AlertPrefs, SourceHealth, ProcExit), snapshot}` and the test-only `#[cfg(test)]`
`Store::{session, projects, rev}` and `Project` (D68), ranks/attention/counts/hold/
aggregate, display names, `canonical_json`, `fnv1a64`, `text::{agent_label, host_label,
error_summary, alert_text}`, `model_tests.rs`; the review rules D49 (episode/ack), D50 (outcome
`at_ms`), D51 (late final record), D52 (unbound transcript records ignored) with their §5 tests.
T9 declares every `Signal`/`Effect` variant; the Codex
variants are no-ops until T10, and `ProcExit` removal emits only `UntrackTranscript` until T10 adds
`UntrackRollout`. Consumes: keys (T4/T6/T7), `ProjectRef` (T5),
`ClaudeSessionFile` (T7), `claude_transcript` (T7), `HookEvent` type (T8, for `Signal`).

**T10 Model Codex.** Produces: Codex branches of `Store::apply` (`CodexHook`, `CodexRollout`,
`CodexHooksInstalled`, Codex `ProcExit`/`Ack`, `CodexTicker` effect), with D49/D50 applied to Codex
episodes and outcomes, Codex table tests. Consumes:
T9 model, `codex::{HookEvent, parse_rollout_line, RolloutRecord}` (T8), `text::tool_label` (T7).

**T11 claude-io.** Produces: §2.16 incl. whole-batch session-file handling (D47) and expected
names binding at 0 (D48), with their §5 tests, and the rules D78 (waiter after the first
`ClaudeFile`, re-watch of a live unwatched process) and D79; `log.rs`: `line` writes to the test's
stderr under `cfg(test)`, signature unchanged (D77). Consumes: `DirWatcher`,
`proc::{identify, Registry}` (T6), `JsonlTail`, `ProjectCache`, `git_probe` (T5), claude parsing
(T7), `Signal`/`Effect`/`Health` (T9), `log::line` (T1).

**T12 codex-io.** Produces: §2.17 incl. the rules D78 (waiter after the `CodexHook`) and D80;
`lib.rs` `AppPaths`/`app_paths()`; `AppSink.observe` → codex-io; temporary "signals-drain".
Consumes: T6, T5, T8 parsing, `Signal`/`Effect` (T9/T10), `AppSink` (T4).

**T13 Store, publication, commands.** Produces: §2.6 (`start`, `Handle` with its log-once flags,
`Publisher`, `run_store`, `should_emit` against the last emitted snapshot (D81), `approval_context`,
`now_ms`); `lib.rs`: `IslandPublisher`, `observe::Handle` state,
commands `sessions_snapshot`, `session_ack`, `session_open`, `set_collapsed` → `Visible`, setup
order; removes "signals-drain"; `AppSink` final form. Consumes: T9–T12, `pipe::ApprovalContext` (T4).

**T14 Installer.** Produces: §2.19, §2.20, §2.21; Settings alert fields (Rust §2.18 and TS
`Settings`/`DEFAULT_SETTINGS`), `hooks_installed` removed from Rust; `boot()` without
`hooks::status()`; commands `hooks_status/preview/apply` with `target`; `HookCtx` state;
`save_settings` → `AlertPrefs`; startup `AlertPrefs` + `CodexHooksInstalled`; codex `hooks_apply`
→ `CodexHooksInstalled(installed_at(..))` re-read after every call past `Target::parse`, also a
refused `Plans::take` (§3, D82); `hooks.nsh` Codex hint;
`unchanged` by structural equality + `HookPreview.unchanged` (D53); `TextFormat.mixed_eol` + note
(D54); `merge_install` cleanup outside `CODEX_EVENTS` (D55); `src/settings/main.ts`: `main()` no
longer calls `Bridge.hooksStatus()` — the Claude section is built on the first window `"focus"`
event (`window.addEventListener("focus", …, { once: true })`; `show_settings_window` calls
`set_focus`), so app start never reads `~/.claude/settings.json` (spec §8, D58);
migrated + new tests (six old `hooks.rs` tests move to hookfile/hooks). Consumes: `observe::Handle`
(T13), `Signal::{AlertPrefs, CodexHooksInstalled}` (T9/T10), `ident` (T1), `AppPaths` (T12).

**T15 Spikes S2–S5 + DACL.** Runs after T14, a deviation from the spec §16 order (D69).
Produces: S2–S5 results (PR description); `pipe::create_instance` with a
current-SID DACL (or a comment with the recorded reason), committed in T15; each only if its decision
rule says so, with tests and committed before T16: `CODEX_EVENTS` reduced to `PermissionRequest` +
`Stop` (S2, Step 12), one more `codex::classify_host` pattern (S2, Step 13), a paginated branch of
`parse_rollout_line` with `rollout_paginated.jsonl` and the tests `paginated_rollout_records` and
`a_paginated_line_with_a_bad_ordinal_is_unknown` (S3, Step 9a, D70), one more place of the subagent
`Meta` keys (`SUBAGENT_OBSERVED`, `observed_subagent`) with `rollout_subagent_meta_s3.jsonl` and the
test `subagent_meta_seen_in_spike_s3` (S3, Step 9b, D67). S3 also spawns one Codex subagent and
reports where its keys appear (keys only, D67). A rollout shape that cannot be determined from keys
alone stops implementation for a spec update with the user; T16 waits (D70). Consumes: debug
relay `--dump` (T3), router (T4), codex-io (T12), `sessions_snapshot` (T13), installer commands and
`CODEX_EVENTS` (T14).

**T16 Front-end data layer.** Produces: §4.1 (incl. `HookPreview.unchanged`), §4.2, `observer.ts`
(snapshots; listener before fetch, §4.3), `Island.onSnapshot`,
`main.ts` wiring, `State` T16 changes (incl. `ApprovalInfo.{sessionKey, projectName}`, D59),
`IslandViewName`/`VIEW_LAYOUTS` entries, placeholder views,
removal of `integration_claude` (state.ts, island.ts, island/integrations.ts, views.ts, views/
integrations.ts), in-island settings badge from `Sessions.snapshot?.sources.claude === "ok"`;
`hooks.ts` `PermissionRequest` branch with unconditional `island.alert("approval")` and session
fields, approval caption from `pendingApproval.projectName`, Mochi `bodyColor = null` while
`Sessions.hasSessions` (all D59); `src/settings/main.ts` fallback `HookStatus` literal gains
`target: "claude", outdated: false, notes: []` (§4.1). Consumes: T13 commands/events, T14
`Settings` fields, approval payload fields (T4/T13).

**T17 Front-end views.** Produces: `views/sessions.ts` (incl. `colorForProject`, D60), `ViewActions`
additions, `ViewHost.setActive` + `syncDom` calls (D57) used by the sessions view and the
`waiting`/`session` cards, `sessionsHeight`/`islandSize` param, `fsm.setHoldVisible`,
`Island.handleAlerts` (sounds per D63, greeting per D64) + `cardRank`, `greeting.interrupt()` on
`coucou → home` (D64), counters, header sessions tab + `ICONS.list`, finished/error/waiting/session
cards bound to `alertKey`, with the D65 fallback (`CardSubject`, `State.alertFallback`,
`cardSubject`, `subjectWho`, `outcomeSummary`), `State.effectiveState`, reduced `hooks.ts` (dead
branches removed), `observer.ts` alert subscription (registered before the initial fetch),
`style.css`. Plan step (D35): Step 0 checks the T16 `VIEW_LAYOUTS` entries against §4.4 (no change
when they match), then the T16 placeholder views are replaced — T17 does not add the view names or
layouts a second time. Consumes: T16 store/types, `session_ack`/`session_open` (T13), `ApprovalInfo`
session fields (T16).

**T18 Settings window.** Produces: §4.5 sections (incl. the `unchanged` → "No change." / disabled
Apply rule and the D66 focus re-read), `settings.css`. Plan step (D2): "check that `settings.html` `<title>` is already
`Settings — Mayak` from T1 — no change". Consumes: `hooks_*` with target (T14), `sessions_snapshot`
(T13), `Settings` alert fields (T14), `HookStatus.notes`, `HookPreview.unchanged` (T14/T16).

**T19 CI.** Produces: `.github/workflows/windows-test.yml` (`pull_request` + `workflow_dispatch`,
`windows-latest`, working dir `windows`, Node 22, `npm ci`, `npm run build`, `cargo test
--workspace`); `files::ingest_into(source: &str, inbox: &Path) -> Result<DroppedFile, String>` with
`ingest(source) = ingest_into(source, &inbox_dir())`; test on `TempDir`. Its "no warnings" check
covers the non-test `mayak_lib`, which holds no test-only item (D67, D68, D71, D74). Consumes: all
tests.

**T20 Acceptance + PR.** Produces: PR description with the spec §12 checklist results and S1–S5
outcomes, and a "Known migration effects" list (D61): API keys stored under the old keyring service
(`fr.louisraille.coucou`) are not visible to Mayak and must be entered again; a copied Coucou `settings.json` with
`autostart: true` shows "Launch at startup" on although Mayak is not registered until the toggle is
switched off and on. Consumes: everything.

---

## 7. Decisions not in the spec

- **D1** `.github/workflows/windows.yml` artifact/release file names change with `pack.mjs`
  (otherwise the tag workflow breaks). Other workflows untouched.
- **D2** `settings.html`/`index.html` `<title>` change in T1 (spec §11); T18 has nothing left there.
  `lib.rs` settings window title `"Settings — Mayak"` also in T1. Where the fixed task list still
  names the `settings.html` title under T18, the plan's T18 has a check-only step ("`<title>` already
  `Settings — Mayak` from T1, no change"); D2 stays the source of truth.
- **D3** Spike results and the acceptance checklist live in the PR description, not in repo files.
- **D4** Relay gets `Win32_System_Diagnostics_ToolHelp` (ancestors) plus `Win32_Storage_FileSystem`,
  `Win32_System_IO` (test pipe server). The app gets `Win32_Storage_FileSystem`, `Win32_System_IO`
  only; ToolHelp is not used by the app.
- **D5** A payload without `agent` is treated as `"claude"` by `route` (an old Coucou-era relay only
  ever relayed Claude).
- **D6** Store starts with `visible = false`; `lib.rs` sends `Visible(true)` after `start()` because
  the island launches expanded (greeting).
- **D7** `SessionKey` is `type SessionKey = String`.
- **D8** `normalize` does no percent-decoding and no UNC handling (local sessions only, spec §2).
- **D9** `Health::Degraded{unreadable}` counts items currently in a bad state (Claude: pids whose
  file is unparsable/incomplete or whose process denies access; Codex: tracked rollouts that yielded
  `Unknown`), so health returns to `Ok` when they clear. Precedence Error > Missing > Degraded > Ok.
- **D10** `terminal_outcome` considers only assistant records that carry a stop_reason or an API
  error (streaming fragments with `null` are skipped), so a later successful retry beats an earlier
  API error; the "no tool_use/tool_result after it" guard applies to the record found.
- **D11** Codex `task_complete.error`: string as is; object → its `message` string, else compact JSON.
- **D12** `Signal::CodexHook` carries `title: Option<String>` (the model cannot read
  `session_index.jsonl`); filled only on the first hook of a key in this run.
- **D13** The model emits `UntrackRollout` whenever it removes a Codex session (SessionEnd, ProcExit,
  Ack removal), not only on SessionEnd.
- **D14** `Session.facts` keeps facts with offset ≥ `turn_start_offset`, at most the last 1024.
- **D15** `PhaseView` labels: Waiting Permission "permission", Input "input", Other → raw
  `waitingFor`; Unknown → raw status.
- **D16** "Highest rank" in spec §10 means most important = smallest number (0 permission … 3
  finished). Ties keep batch order.
- **D17** `counts.working` counts all Working sessions; ack does not apply to Working (no episode).
- **D18** Alerts of a kind disabled by `AlertPrefs` are dropped in Rust; rows/counts unaffected.
  Pause is applied in TS.
- **D19** A new Claude waiting episode whose `statusUpdatedAt` did not advance gets
  `since = max(prev_since + 1, statusUpdatedAt)` so it is a distinct episode. Its unacked state
  comes from D49 (a new episode clears `acked_at`), not from comparing `since` with `acked_at`.
- **D20** Title precedence: customTitle > agentName > aiTitle > session file `name`.
- **D21** At a Claude turn start `pending_tools` and `activity` are rebuilt from facts ≥ the new
  `turn_start_offset` (records of the new turn may arrive before the `busy` file change).
- **D22** Codex: a session created by a hook starts as `Unknown("")`; `SessionEnd` for an unknown key
  creates nothing.
- **D23** Codex subagent permission detail is `"<agent_type>: <tool> · <target>"`.
- **D24** Any `CodexHook` sets `codexHooks` to `receiving` (even if the installer status said
  not installed); `CodexHooksInstalled(false)` resets to `not_installed`.
- **D25** claude-io sends `ClaudeFile` only when the parsed file changed or the session is new.
- **D26** Unparsable transcript lines are skipped silently.
- **D27** The Codex 1 Hz ticker is a `recv_timeout` deadline inside codex-io, not a separate thread.
- **D28** A config file created from scratch is written without BOM, with LF and a final newline.
- **D29** The path allow-list uses `char::is_alphabetic` for `\p{L}` and `char::is_numeric` for
  `\p{N}` (no regex crate).
- **D30** A Codex group is "ours" when a command contains `mayak-hook` (case-insensitive) or equals
  the current command; needed because `GetShortPathNameW` may turn `mayak-hook.exe` into an 8.3 name.
- **D31** Codex uninstall removes an event key whose array becomes empty and keeps `"hooks"` even if
  empty; install never reuses placeholders (always replace-in-place or append).
- **D32** `session_open`: Cursor host → `cursor`, VS Code host → `code`, any other host → Explorer;
  returns true only when an editor was launched.
- **D33** Model `permission` alerts for a session with a live legacy approval are dropped in TS before
  choosing the top alert; an approval card is never replaced by a model alert.
- **D34** Compact counters show non-zero values only.
- **D35** T16 registers placeholder views for `sessions`/`waiting`/`session` so `defaultView()` can
  return `"sessions"` while T17 builds the real views. T16 also adds the `IslandViewName` members
  and `VIEW_LAYOUTS` entries (needed by `tsc` for `defaultView()`). Where the fixed task list names
  "layout.ts views" under T17, the plan's T17 step is "replace the T16 placeholder views and check
  the T16 layouts" (the check is T17 Step 0, a grep of the three `VIEW_LAYOUTS` lines) — nothing is
  added twice; D35 stays the source of truth.
- **D36** TS `hooksInstalled` survives until T16 (Rust ignores the unknown field), so T14 compiles
  without touching views; T16 removes it with its last readers.
- **D37** The settings window refreshes the snapshot on the DOM `focus` event (no new capability).
- **D38** (amended in T6) The `Registry::watch` integration test uses duplicated handles of a
  `cmd /d /c "set /p _="` child with piped stdin and a hand-built `ProcId` instead of `identify`,
  which would race with the child's exit. The child stays alive until the test drops its stdin, so
  the second `watch()` deterministically sees the process running; a `cmd /c exit` child could exit
  before the second call, the first waiter would then remove the entry, and the second call would
  return `true`.
- **D39** Router annotations are `null` from T4 until T13 wires `observe::approval_context`; the
  pre-T16 `hooks.ts` does not read them, and from T16 on (D59) `hooks.ts` falls back to the `cwd`
  basename when `project_name` is null. Annotation uses the basename of the project root, not the
  collision-resolved snapshot name.
- **D40** The approval `project_name`, `Alert.project` and `ProjectView.name` share
  `paths::basename`; only snapshot/alert names apply the collision rule.
- **D41** `pipe::serve` is `async fn serve<S: RouterSink>(name: String, sink: Arc<S>)` (spec writes
  `serve(name: &str, sink: impl RouterSink)`): the accept loop and per-connection tasks need owned,
  shared `'static` values on the tokio runtime.
- **D42** `talk_to` takes an explicit `budget` (4th argument) because Codex (1 s), Claude
  fire-and-forget (2 s, unchanged) and Claude decisions (110 s, unchanged) differ.
- **D43** `ProcId` and `HostKind` live in `observe/keys.rs` (with `SessionKey`, `claude_key`,
  `codex_key`) because T6/T7 need them before `model.rs` exists; `model.rs` re-exports them, so the
  spec's `model::…` paths still resolve.
- **D44** `PendingPlan` also stores `install`, `path` and `unchanged`; `Plans::take` refuses a plan
  whose `install` flag differs from the `hooks_apply` call.
- **D45** The relay budget is a deadline from the first statement of `main` (`started`): `talk_to`
  receives `budget(..).saturating_sub(started.elapsed())`, so stdin, the Codex ancestor walk and
  enrichment count against Codex's 1 s (spec §5.2 "бюджет 1 с"); a zero budget skips the connect.
- **D46** `pipe::serve` spawns connection tasks with `tokio::spawn` and creates every pipe instance
  itself, so tasks and their pipe I/O live on the runtime that polls `serve` (Tauri's in the app, the
  test's current_thread runtime in tests). `RouterSink::approval_context` (OpenProcess,
  GetProcessTimes, QueryFullProcessImageNameW, `.git` probes) runs on `spawn_blocking`, keeping
  blocking Win32/file-system work off the async runtime (spec §4); it stays well inside the 800 ms ack.
- **D47** claude-io processes each session-file `Changes` batch as a whole: distinct names, then the
  file system decides — `NotFound` → `ClaudeGone`, readable → normal processing. Action codes are
  never trusted alone, so a temp-file + rename-over write is an update, not a deletion (spec §5.5
  "Удаление файла → ClaudeGone" means the file is gone).
- **D48** A transcript name that entered the expected set is a new file: every later discovery
  (notification, re-search on `ClaudeFile`, Overflow/Retry) binds at 0 and sends
  `rebind_at: Some(0)` (spec §7 "для нового файла o = 0"). Only the initial `TrackTranscript` search
  of an existing file uses the 256 KiB window and `Some(eof)`.
- **D49** Ack bookkeeping without clock races: a new episode clears `acked_at`; `Ack` sets
  `acked_at = max(now_ms, episode start)` (or `now_ms` without an episode). The spec formula
  `acked_at >= episode_start` and the spec field `acked_at` are kept; what changes is that an ack
  processed before an episode existed in the store can never count for it (the UI can only have seen
  episodes the store already produced). This is the review's "acked_episode" variant expressed with
  the spec's field; it also covers D19 episodes and Codex `relay_ts` episodes, so the
  `max(…, now_ms)` tweak to D19 is not needed.
- **D50** Every outcome (`Finished`/`Error`/`Interrupted`, Claude and Codex) has `at_ms = now_ms`
  (store clock); Waiting `since` keeps the source time (`statusUpdatedAt` / `relay_ts`, spec §7).
- **D51** Late final Claude record: when a `ClaudeTranscript` resolves a Pending outcome while Idle,
  `idle_eof = max(idle_eof, o + 1)` with `o` = offset of the deciding record (second value of
  `terminal_outcome`). Chosen over `idle_eof = max(idle_eof, signal.eof)` because a record of the
  next turn in the same batch (its prompt or an interrupt marker) must stay in the next turn.
- **D52** The model ignores `ClaudeTranscript{rebind_at: None}` while `transcript_bound == false`
  (e.g. old-transcript records still in flight after a sessionId change, before claude-io handles the
  Untrack/Track effects). claude-io only ever sends `None` after `Some(_)` for the same binding, so
  no legitimate record is lost.
- **D53** Hook-file "unchanged" (spec §8 "если запись идентична, файл не пишется") is structural:
  `parse_object(current) == next` (key order ignored, arrays positional); a missing file is
  unchanged when `next` is `{}`. `HookPreview.unchanged` is an additive field to the spec §8 shape;
  the UI disables Apply with "No change.". `write_plan` on an unchanged plan is an `Err`, so `Ok("")`
  means only "there was no previous file".
- **D54** The diff shows lines (BOM and final newline are preserved by `render`), so the one byte
  change it cannot show — normalising mixed line endings — is stated in `notes`
  (`TextFormat.mixed_eol`), keeping the preview honest about the exact bytes (spec §8).
- **D55** `merge_install` also cleans Mayak groups under events outside `CODEX_EVENTS` (uninstall
  rule: last → removed, else placeholder), so shrinking the default set after S2 leaves no stale
  groups, and `outdated` reflects them.
- **D56** `ClaudeSessionFile` is extracted field by field from a `serde_json::Value`; a wrong-typed
  field becomes None instead of failing the file ("терпимый serde", spec §15); `procStart` is
  accepted as a string or a u64 number. `parse_session_file` errs only for empty, invalid or
  non-object JSON.
- **D57** `ViewHost.setActive(on)` uses `on = State.mode === "expanded" && name === State.view`
  (the view-toggle in `syncDom` ignores mode, and `State.view` stays `"sessions"` while compact or
  hidden), re-evaluated on every notify; per-second elapsed refresh runs only while active and
  updates text nodes without `State.notify()` (spec §10 "только пока вид открыт", acceptance "CPU 0%").
- **D58** From T14 the hidden-at-launch settings window builds its Claude section on the first window
  `focus` event instead of in `main()`, so app start never reads `~/.claude/settings.json` between
  T14 and T18 (spec §8); T18 replaces this with the collapsed Advanced block.
- **D59** The legacy approval path is made independent of `integration_claude` in the same task that
  removes it (T16): unconditional `island.alert("approval")`, `ApprovalInfo.{sessionKey,
  projectName}` (moved from T17), approval caption from `projectName`, and Mochi `bodyColor = null`
  while sessions exist. Otherwise T16..T17 would ack approvals with no card (Claude Code blocked
  ~110 s), name an unrelated integration in the caption, and colour Mochi after it.
- **D60** `colorForProject(projectKey)` lives in `views/sessions.ts` (T17): FNV-1a 32 of the key into a
  fixed 8-colour palette; never derived from agent or host.
- **D61** Known migration effects of spec §2/§11 (keyring `SERVICE` = bundle id; copy of Coucou
  `settings.json` only) are recorded in the T20 PR description: stored API keys must be re-entered;
  a copied `autostart: true` is not registered for Mayak until toggled. Mayak does not call
  `autolaunch().enable()` on its own after the copy: spec §2 limits migration to the file copy, and
  doing it at startup would also fire on the first `npm run dev` run and register the debug exe in
  the user's HKCU Run key.

Amendments recorded by the task sections (D62–D66 numbered there; D67–D70 orchestrator decisions;
D71–D82 the remaining amendments of sections C, F and G, numbered in order of appearance):

- **D62** (T7) `fact_of` sets `interrupted` only when the string content or a `text` block, after
  leading whitespace, starts with `INTERRUPT_MARKER` (`text.trim_start().starts_with(..)`), not when
  it merely contains it; `tool_result` content never counts. Spec §7 rule 1 ("Последняя запись
  user — маркер `[Request interrupted by user`") and §3 name a user record that *is* the marker.
  With "contains", a prompt that quotes the marker mid-sentence is the last User fact of a turn
  without tool calls, so `terminal_outcome` would return Interrupted instead of Finished and the
  alert would be lost. Local evidence (counts only, 1412 transcript files, re-counted with 1429): all
  3 real markers in main-chain user text start their text block, none is mid-text; the 2 other
  quotes sit in `tool_result` content, which never counts.
- **D63** (T17) One alert sound per batch, played only when the batch opens or replaces a card or
  lands on the sessions list: hidden, compact or the launch greeting (D64) → sound, then
  `island.alert`; an expanded `waiting`/`finished`/`error` card outranked by the top alert → sound,
  then the card is replaced; expanded `"sessions"` → sound, the view stays. A card that is not
  outranked, `"approval"` and every other expanded view only call `State.notify()` (counters and
  pager), without a sound. This reads spec §10: a sound for hidden/compact ("Один звук на пачку") and
  for `"sessions"` ("звучит звук"); for another view "обновляются только счётчики", for an open card
  that is not outranked "обновляется только счётчик листалки". A sound over the approval card would
  read as being about the request on screen.
- **D64** (T17) The launch greeting (`State.view === "greeting"`, FSM `coucou`) is neither a view the
  user chose nor a card, so `handleAlerts` treats it like hidden/compact: the top alert opens its card
  via `island.alert` (the legacy approval path already interrupts the greeting through `forceHome`).
  The `coucou → home` transition calls `greeting.interrupt()`, so the greeting's timed `greet`/`blip`
  sounds do not follow the alert's sound (nor an approval card or a file drop that cuts the greeting
  short).
- **D65** (T17) Alert cards fall back when the snapshot cannot show `State.alertKey`. `state.ts` gains
  `CardSubject = { alert: Alert } | { session: SessionView; project: ProjectView }` and
  `State.alertFallback: CardSubject | null = null`. `handleAlerts` stores `{ alert: top }` whenever it
  sets `alertKey`; `Island.onSnapshot` clears an `{ alert }` fallback, because spec §9 emits a batch's
  `session-alert` just before the `sessions` snapshot that contains it (cleared by that next snapshot,
  not only once a snapshot shows the key in the alerted state: one batch can carry a session past
  that state, e.g. a Codex `PermissionRequest` and `Stop` in one batch leave it Idle/finished);
  `openSession` stores the clicked `{ session, project }` before `session_ack`, because spec §7
  removes an Idle Codex session without a process on ack. `cardSubject()` returns the alert fallback
  for the key, else `Sessions.find(key)`, else the row fallback for the key; the waiting/finished/error
  cards render from it (`Alert.detail` is the alert body), the session card from a row subject;
  `subjectWho` gives an alert no project dot (no project key, as the approval card, D59);
  `outcomeSummary` reads the row outcome or the alert body; `cardRank` reads the same subject.
- **D66** (T18) The settings window re-reads on every window `focus` both the snapshot (D37) and the
  installer status of each `integrationSection`, so a `hooks.json` changed outside Mayak (by hand, by
  Codex, by a dotfiles sync) and the relay readiness are current when the window is shown (spec §8
  flow Status → Preview → Install). The status is re-read only while that section shows its status
  rows, never under a preview, an error or an apply result, and the Claude section only while the
  Advanced block is open, so `~/.claude/settings.json` is still read only while that block is
  expanded. The `focus` listener is registered before `main()` awaits anything, and a snapshot with
  `rev` ≤ the last one taken is dropped (the spec §9 rule, applied in this window as well). The
  preview prints `Backup (made only if the file already exists) → <path>`: `write_plan` backs up only
  an existing file (§2.19), and the preview cannot tell.
- **D67** (T8, T15) `codex::is_subagent_meta` is removed from production code: no production code
  calls it, so it would leave a permanent `dead_code` warning in the non-test build (T19 Step 2
  expects no warnings). `RolloutRecord::Meta` keeps `parent_thread_id` and `agent_role` (top-level
  string keys, else `payload.source.subagent.thread_spawn.<key>`), and T8's
  `subagent_meta_fields_are_parsed` asserts both fields for every line of
  `rollout_subagent_meta.jsonl`. Spec §5.6 ("сессией он не становится") and §12 ("`session_meta`
  субагента игнорируется") hold by construction: Codex sessions are created only from hooks keyed by
  the root `session_id` (subagents share it), and rollout tails start at EOF (T12), so a subagent
  `Meta` never creates a session; T10 maps `Meta` to a no-op. T15's S3 script also spawns one Codex
  subagent and reports where `parent_thread_id`, `agent_role` and `source.subagent` appear (keys and
  JSON types only). If they are not where T8 parses them, T15 Step 9b updates the
  `RolloutRecord::Meta` extraction in `observe/codex.rs` with a fixture and a test; a key whose JSON
  type is neither string nor null stops for a decision with the user.
- **D68** (T9) `Store::session`, `Store::projects`, `Store::rev` and `Project` are `#[cfg(test)]`. Only
  `model_tests.rs` (T9, T10) calls them; production code reads the store only through `apply` and
  `snapshot`, and no later task calls them. Without the mark the non-test `mayak_lib` would keep a
  permanent `dead_code` warning, and T19 Step 2 expects no warnings. `model_tests.rs` is a child
  module of `model` compiled only under `cfg(test)`, so every test that calls them still compiles.
- **D69** (T15) Spikes S2–S5 all run in T15, after T14 — a deviation from the spec §16 order (S4
  right after the router, S2/S3/S5 right after codex-io) — because T15 reads its observations through
  `sessions_snapshot` (T13; the island draws no session rows before T16/T17), clears a leftover Codex
  entry with T14's `hooks_preview`/`hooks_apply`, and its S2 decision edits T14's `CODEX_EVENTS` and
  its tests. The S4 DACL commit stays inside T15, and every code change the spikes call for (Steps
  9a, 9b, 12, 13 and 18) is committed in T15, before T16 starts.
- **D70** (T15) If S3 shows paginated rollouts (only the top-level key sets
  `[ordinal,payload,timestamp,type]`, `[metadata,ordinal,payload,timestamp,type]` or
  `[payload,timestamp,type]`), T15 Step 9a adds a paginated branch to `codex::parse_rollout_line`
  (after Codex main's `decode_rollout_line`: an `ordinal` that is neither a u64 nor null → `Unknown`)
  with the sanitized fixture `observe/fixtures/codex/rollout_paginated.jsonl` and the tests
  `paginated_rollout_records` and `a_paginated_line_with_a_bad_ordinal_is_unknown`, and commits it
  before T16 (spec §12 S3 «Если paginated — добавить разбор»). If the shape cannot be determined from
  keys alone, T15 finishes its remaining steps and implementation stops for a spec update with the
  user; T16 does not start before that.
- **D71** (T5) `JsonlTail::path()` is removed from §2.9. No task calls it, so it would stay a permanent
  ``method `path` is never used`` warning in the test and the non-test build, contradicting T19
  Step 2 (no warnings). The private `path` field stays; callers keep their own copy of the path.
- **D72** (T5) `JsonlTail::start_at_eof` discards the rest of a trailing partial line (the line began
  before us); `read_new` on `NotFound` returns `Ok((vec![], 0))` and restarts at offset 0, so a
  re-created file is read from its start.
- **D73** (T5) `JsonlTail` `eof` = the first byte not yet consumed, either as a returned (or blank)
  line or as skipped pre-existing bytes; it equals the end of the last complete line except right
  after `start_at_eof`/`start_with_window` has landed inside a line, until that line's `\n` is read.
  The old §2.9 wording "offset just past the last complete line" reads this way.
- **D74** (T6) `MASK_NAME_WRITE` is `#[cfg(test)]`: only `masks_are_the_documented_bits` and the
  `#[ignore]` open-handle diagnostic use it, so in the non-test `mayak_lib` (which
  `cargo test -p mayak` also builds for the `mayak` bin) it would stay a permanent
  ``constant `MASK_NAME_WRITE` is never used`` warning. It stays a plain `pub const` with the same
  value; no `#[allow]`.
- **D75** (T6) `Registry::watch` returns `false` (handle dropped) if `(pid, created_ft)` is already
  watched **or** if the waiter thread could not be started; in that case the entry is removed and
  `proc-exit thread not started pid=…` is logged, so a later proof registers again. Callers need
  nothing more for either `false` (T11 and T12 discard the result).
- **D76** (T6) `DirWatcher` opens the directory with
  `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED`; its blocking thread keeps one overlapped
  `ReadDirectoryChangesW` request on a manual-reset event (`CreateEventW`) queued and collects it
  with `GetOverlappedResult(.., bWait = TRUE)`; `spawn` returns after the first request is queued
  (this replaces the earlier "synchronous ReadDirectoryChangesW loop"). The Overflow/Error mapping
  is unchanged (0 bytes or `ERROR_NOTIFY_ENUM_DIR` → `Overflow`, any other error → `Error`). Reason: with a
  synchronous call a change made between the handshake and the call was lost under load (reviewer
  stress run: 3 and 11 of 640 missed with 32 CPU hogs, 0 of 640 with this design).
- **D77** (T11) Under `cfg(test)` `crate::log::line` writes to the running test's stderr, so no unit
  test writes the user's `%LOCALAPPDATA%\Mayak\mayak.log`; the signature is unchanged. `log.rs` is
  owned by T1 and changed once more by T11.
- **D78** (T11, T12) Liveness of a known session is checked with `Registry::is_watched`, and exit
  waiters are registered only after the session's first signal was sent (`ClaudeFile` / `CodexHook`),
  so `ProcExit` never overtakes the signal that created the session and no ghost session appears
  after `ProcExit`. T11: a known process that is not watched but still identifies with the same
  creation time is watched again and keeps its session.
- **D79** (T11) A known file that becomes `Spare`/`Parked`/`ForeignDomain` → `ClaudeGone`;
  `ProcError::Os` → unreadable; the transcript re-search runs before `ClaudeFile`; `Retry` with
  `watch: false` always rescans and `Missing` with `watch: false` comes from `is_dir()`; a transcript
  that exists but cannot be opened stays unbound and is later bound with the window at its EOF, never
  at 0; session ids with `\`, `/`, `:` or `..` are not tracked; a rescan re-checks vanished unreadable
  names.
- **D80** (T12) `Unknown` rollouts stay counted until untracked; drained records go to every session
  sharing the path; only `rollout-*.jsonl` paths become main rollouts; a rollout whose `start_at_eof`
  fails is not tracked (the next hook tries again).
- **D81** (T13) `prev` in `should_emit` is the last *emitted* snapshot, so hidden-mode comparisons are
  against what the island actually has; `Handle` carries two private "closed" flags so a failed send
  is logged once per channel.
- **D82** (T14) For target `codex`, `hooks_apply` sends `CodexHooksInstalled` after every call that got
  past `Target::parse` — also when `Plans::take` refuses — always from a fresh `installed_at` read of
  the file, as spec §8 says («после каждого hooks_apply»). An extra send after a refused take reports
  the file as it is on disk; when the file did not change, the model state does not change either
  (`(false)` → NotInstalled, `(true)` → InstalledUnseen unless Receiving). This replaces the earlier
  §3 wording "(once a plan was taken)".

## 8. Rejected review findings

None of the 25 review findings was rejected. Applied with a variant (reasons in the D entries):
- Ack/episode race (finding on D19/mixed clocks): applied as D49 — a new episode clears `acked_at`
  and `Ack` stores `max(now_ms, episode start)`; equivalent to the suggested `acked_episode`, keeps
  the spec field and formula, so the `max(…, now_ms)` tweak is unnecessary.
- Late final record: applied as D51 with the deciding record's offset (`o + 1`) instead of
  `signal.eof`, so next-turn records in the same batch are not swallowed.
- Migration effects: the required part (record in the T20 PR) is applied (D61); the optional
  one-time `autolaunch().enable()` is not adopted (reason in D61).
- Lenient session file: applied (D56). Note: spec §15 says format breakage should give «меньше
  строк», а не призраков ("fewer rows, not ghosts"), not "less detail"; the fix is still adopted
  because the same row asks for «Терпимый serde».
