// Agent Orchestrator (AO): recognising it and reading its sessions, strictly read-only.
//
// Where AO says what it is doing (all of it checked on a real install, see `fixtures/`):
//   • `~/.ao/running.json` (or the file `AO_RUN_FILE` names): pid and port of the daemon.
//   • The daemon's HTTP API on 127.0.0.1:<port>, no authentication; it publishes its own contract at
//     `GET /api/v1/openapi.yaml`. Two GETs are used:
//       /api/v1/sessions?active=true                       every session that is not terminated
//       /api/v1/desktop/sessions/{id}/workspace            the exact folder of one session
//     A worktree is not always `worktrees/<project>/<id>` (an orchestrator's is nested one level
//     deeper), so the folder is asked for, once per session, instead of being guessed.
//   • `ao session ls --json` gives the same list, but costs a 47 MB process per read; the CLI is only
//     looked up on PATH (never run), to tell «installed, not running» from «not installed».
//
// What this module never does: POST, PUT or DELETE anything; run `ao`; write under `~/.ao`; read or
// keep a secret; talk to any host but 127.0.0.1 (the address is a literal, only the port comes from
// the run file, and no system proxy and no redirect is followed); touch an agent or its hooks.
//
// Cost: the island polls only while it is visible (`set_visible`, from `set_collapsed`), every
// `INTERVAL`, right away when it shows, and only while the AO pill is switched on and Mayak is not
// paused. Hidden, the task has no timer at all. A poll reads the ~300 byte run file and checks the
// pid; only a live pid gets one GET (answer capped, `Limits`), plus one small GET for each session
// seen for the first time. AO not installed or not running costs a file read and a PATH search.

pub mod model;

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::LazyLock;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;
use tokio::sync::{watch, Mutex};

use crate::integrations::{self, emit, IntegrationUpdate};
use crate::{log, platform};
use model::{RunFile, RunFileProblem, Status};

/// The pill's id: a stable contract value (settings, island, events).
pub const ID: &str = "integration_ao";

/// Between two polls while the island is visible.
const INTERVAL: Duration = Duration::from_secs(15);
/// The least between two polls: an island folded and opened again in a flash is one poll.
const MIN_GAP: Duration = Duration::from_secs(2);
/// New sessions looked up for their folder per poll, at most.
const MAX_LOOKUPS: usize = 32;
/// The run file is a few hundred bytes; more than this is not one.
const MAX_RUN_FILE: u64 = 64 * 1024;

/// What one request may take and bring back.
#[derive(Clone, Copy)]
pub struct Limits {
    pub timeout: Duration,
    pub max_body: usize,
}

pub const LIMITS: Limits = Limits { timeout: Duration::from_secs(3), max_body: 2 * 1024 * 1024 };

// ── Is AO here? ───────────────────────────────────────────────────────────────

/// `AO_RUN_FILE` when it names one (AO sets it for its own workers), else `~/.ao/running.json`.
pub fn run_file_path(env: Option<std::ffi::OsString>, home: &Path) -> PathBuf {
    match env.filter(|v| !v.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => home.join(".ao").join("running.json"),
    }
}

/// What the run file says about the daemon.
#[derive(Debug, PartialEq, Eq)]
pub enum Daemon {
    /// No run file.
    Absent,
    /// There is a file, and it could not be read.
    Unreadable(std::io::ErrorKind),
    Garbled(RunFileProblem),
    /// The run file names a process that is gone.
    Dead(RunFile),
    Alive(RunFile),
}

pub fn read_daemon(run_file: &Path, alive: impl Fn(u32) -> bool) -> Daemon {
    let mut text = String::new();
    let read = std::fs::File::open(run_file).and_then(|file| file.take(MAX_RUN_FILE).read_to_string(&mut text));
    match read {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Daemon::Absent,
        Err(err) => return Daemon::Unreadable(err.kind()),
    }
    match model::parse_run_file(&text) {
        Err(problem) => Daemon::Garbled(problem),
        Ok(run) if alive(run.pid) => Daemon::Alive(run),
        Ok(run) => Daemon::Dead(run),
    }
}

/// Whether the process `pid` exists. A process we may not open still exists.
#[cfg(windows)]
pub fn pid_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, WIN32_ERROR};
    use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    /// GetExitCodeProcess's answer for a process that has not exited.
    const STILL_ACTIVE: u32 = 259;
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(handle) => handle,
        Err(err) => return WIN32_ERROR::from_error(&err) == Some(ERROR_ACCESS_DENIED),
    };
    let mut code = 0u32;
    // A handle kept open elsewhere keeps an exited process findable: its exit code tells.
    let alive = unsafe { GetExitCodeProcess(handle, &mut code) }.is_ok() && code == STILL_ACTIVE;
    let _ = unsafe { CloseHandle(handle) };
    alive
}

#[cfg(unix)]
pub fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else { return false };
    // Signal 0 sends nothing, it only checks; EPERM: the process exists but is not ours.
    unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

/// What this PC has of AO, from the run file and PATH alone.
#[derive(Debug)]
pub struct Detected {
    pub daemon: Daemon,
    /// The `ao` found on PATH (found, never run).
    pub cli: Option<PathBuf>,
}

pub fn detect() -> Detected {
    let run_file = run_file_path(std::env::var_os("AO_RUN_FILE"), &platform::home_dir());
    Detected { daemon: read_daemon(&run_file, pid_alive), cli: platform::find_on_path("ao") }
}

/// Settings → Integrations: the one line about AO, without asking the daemon anything.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    /// `running`, `notRunning`, `notInstalled` or `unsupported` (a run file that cannot be read).
    pub state: &'static str,
    pub cli: Option<String>,
    pub port: Option<u16>,
}

pub fn detection(found: &Detected) -> Detection {
    let (state, port) = match &found.daemon {
        Daemon::Alive(run) => ("running", Some(run.port)),
        Daemon::Dead(_) => ("notRunning", None),
        Daemon::Absent if found.cli.is_some() => ("notRunning", None),
        Daemon::Absent => ("notInstalled", None),
        Daemon::Garbled(_) | Daemon::Unreadable(_) => ("unsupported", None),
    };
    Detection { state, cli: found.cli.as_ref().map(|p| p.display().to_string()), port }
}

// ── Asking the daemon ─────────────────────────────────────────────────────────

#[derive(Debug)]
enum FetchError {
    /// Nothing answered, or not in time.
    Unreachable(String),
    /// Something answered with this status.
    Status(u16),
    TooLarge,
}

fn client(limits: Limits) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(limits.timeout)
        // The daemon is on this PC: no system proxy may carry these requests, no redirect lead them elsewhere.
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

/// One GET to the daemon. The only request this module makes.
async fn get(http: &reqwest::Client, port: u16, target: &str, limits: Limits) -> Result<Vec<u8>, FetchError> {
    let unreachable = |err: reqwest::Error| FetchError::Unreachable(err.without_url().to_string());
    let mut response = http.get(format!("http://127.0.0.1:{port}{target}")).send().await.map_err(unreachable)?;
    if !response.status().is_success() {
        return Err(FetchError::Status(response.status().as_u16()));
    }
    if response.content_length().is_some_and(|n| n > limits.max_body as u64) {
        return Err(FetchError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(unreachable)? {
        if body.len() + chunk.len() > limits.max_body {
            return Err(FetchError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The folders of the sessions seen so far, by session id. A session's folder does not change, so it
/// is asked for once; the daemon starting over empties it.
#[derive(Default)]
pub struct Workspaces {
    pid: u32,
    keys: HashMap<String, String>,
}

/// One read of AO: what to show, and what to say about it in the log when it changes.
pub struct Outcome {
    pub status: Status,
    pub data: Value,
    /// Why not `ok`, or what went wrong on the side of an `ok` read; None when all is well.
    pub note: Option<String>,
}

fn outcome(status: Status, cli: bool, note: Option<String>) -> Outcome {
    Outcome { status, data: model::payload(status, cli, &[]), note }
}

/// Reads AO once: the run file and pid, then, for a live daemon, its sessions.
pub async fn read(run_file: &Path, cli: bool, alive: impl Fn(u32) -> bool, cache: &mut Workspaces, limits: Limits) -> Outcome {
    let run = match read_daemon(run_file, alive) {
        Daemon::Absent => return outcome(if cli { Status::NotRunning } else { Status::NotInstalled }, cli, None),
        Daemon::Dead(run) => return outcome(Status::NotRunning, cli, Some(format!("pid {} of the run file is gone", run.pid))),
        Daemon::Unreadable(kind) => return outcome(Status::Unsupported, cli, Some(format!("the run file cannot be read ({kind})"))),
        Daemon::Garbled(problem) => return outcome(Status::Unsupported, cli, Some(problem.to_string())),
        Daemon::Alive(run) => run,
    };
    let http = match client(limits) {
        Ok(http) => http,
        Err(err) => return outcome(Status::Unreachable, cli, Some(format!("no HTTP client: {err}"))),
    };
    let body = match get(&http, run.port, "/api/v1/sessions?active=true", limits).await {
        Ok(body) => body,
        Err(FetchError::Unreachable(why)) => return outcome(Status::Unreachable, cli, Some(format!("port {}: {why}", run.port))),
        Err(FetchError::Status(code)) => return outcome(Status::Unsupported, cli, Some(format!("the session list answered {code}"))),
        Err(FetchError::TooLarge) => return outcome(Status::Unsupported, cli, Some("the session list is too large".into())),
    };
    let mut sessions = match model::parse_sessions(&body) {
        Ok(sessions) => sessions,
        Err(problem) => return outcome(Status::Unsupported, cli, Some(problem.to_string())),
    };

    if cache.pid != run.pid {
        cache.pid = run.pid;
        cache.keys.clear();
    }
    cache.keys.retain(|id, _| sessions.list.iter().any(|s| &s.id == id));
    let mut failed = 0;
    let mut asked = 0;
    for session in &sessions.list {
        if cache.keys.contains_key(&session.id) {
            continue;
        }
        if asked == MAX_LOOKUPS {
            break;
        }
        asked += 1;
        let target = format!("/api/v1/desktop/sessions/{}/workspace", session.id);
        // A session that is still being provisioned has no folder yet: it is asked again next poll.
        match get(&http, run.port, &target, limits).await.ok().and_then(|body| model::parse_workspace(&body, &session.id)) {
            Some(key) => {
                cache.keys.insert(session.id.clone(), key);
            }
            None => failed += 1,
        }
    }
    model::attach_workspaces(&mut sessions.list, &cache.keys);

    let mut notes = Vec::new();
    if sessions.skipped > 0 {
        notes.push(format!("{} session rows could not be read", sessions.skipped));
    }
    if failed > 0 {
        notes.push(format!("the folder of {failed} sessions is unknown"));
    }
    Outcome {
        status: Status::Ok,
        data: model::payload(Status::Ok, cli, &sessions.list),
        note: (!notes.is_empty()).then(|| notes.join("; ")),
    }
}

// ── The poller ────────────────────────────────────────────────────────────────

struct PollState {
    workspaces: Workspaces,
    /// What was last written to the log, so a state that lasts is said once.
    logged: Option<(Status, Option<String>)>,
}

static STATE: LazyLock<Mutex<PollState>> =
    LazyLock::new(|| Mutex::new(PollState { workspaces: Workspaces::default(), logged: None }));

/// Whether the island is on screen: the poll runs only then.
static VISIBLE: LazyLock<watch::Sender<bool>> = LazyLock::new(|| watch::channel(false).0);

/// The island came on screen or was folded away (`set_collapsed`, and the launch).
pub fn set_visible(on: bool) {
    VISIBLE.send_replace(on);
}

/// One poll, emitted to the island. Also the Refresh button's.
pub async fn poll(app: AppHandle) {
    let found = detect();
    let run_file = run_file_path(std::env::var_os("AO_RUN_FILE"), &platform::home_dir());
    let mut state = STATE.lock().await;
    let result = read(&run_file, found.cli.is_some(), pid_alive, &mut state.workspaces, LIMITS).await;
    let now = (result.status, result.note.clone());
    if state.logged.as_ref() != Some(&now) {
        if let (_, Some(note)) = &now {
            log::line(format!("ao: {}: {note}", result.status.code()));
        }
        state.logged = Some(now);
    }
    drop(state);
    remember(crate::observe::now_ms(), &result.data);
    emit(&app, IntegrationUpdate { id: ID, data: result.data, error: None, event: None });
}

/// The last poll's `data` and when it was read (ms since the Unix epoch): what the MCP server's
/// `list_ao_sessions` hands over, so reading AO for an agent costs no request of its own.
static LAST: std::sync::Mutex<Option<(i64, Value)>> = std::sync::Mutex::new(None);

fn remember(at_ms: i64, data: &Value) {
    *LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((at_ms, data.clone()));
}

/// What the last poll read, and when; None before the first poll.
pub fn last() -> Option<(i64, Value)> {
    LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// Spawned once at launch. While the island is hidden the task waits for it to show and nothing else.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut visible = VISIBLE.subscribe();
        loop {
            if !*visible.borrow_and_update() {
                if visible.changed().await.is_err() {
                    return;
                }
                continue;
            }
            if !integrations::PAUSED.load(Ordering::Relaxed) && integrations::enabled(&app, ID) {
                poll(app.clone()).await;
            }
            tokio::time::sleep(MIN_GAP).await;
            if let Ok(Err(_)) = tokio::time::timeout(INTERVAL - MIN_GAP, visible.changed()).await {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::testutil::TempDir;

    const SESSIONS: &str = include_str!("fixtures/sessions.json");

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    fn quick() -> Limits {
        Limits { timeout: Duration::from_millis(400), max_body: 64 * 1024 }
    }

    #[test]
    fn the_last_poll_is_kept_with_its_time_for_whoever_asks_next() {
        let data = model::payload(Status::NotRunning, true, &[]);
        remember(42, &data);
        assert_eq!(last(), Some((42, data)));
        let newer = model::payload(Status::NotInstalled, false, &[]);
        remember(43, &newer);
        assert_eq!(last(), Some((43, newer)));
    }

    // ── The run file ──

    #[test]
    fn the_run_file_is_the_one_ao_names_else_the_one_in_the_home_folder() {
        let home = Path::new("home");
        assert_eq!(run_file_path(None, home), Path::new("home").join(".ao").join("running.json"));
        assert_eq!(run_file_path(Some("".into()), home), Path::new("home").join(".ao").join("running.json"));
        assert_eq!(run_file_path(Some("other.json".into()), home), PathBuf::from("other.json"));
    }

    fn run_file_with(dir: &TempDir, text: &str) -> PathBuf {
        let path = dir.path.join("running.json");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn the_daemon_is_found_dead_alive_garbled_unreadable_or_absent() {
        let dir = TempDir::new("ao-daemon");
        let run = RunFile { pid: 11576, port: 3001 };
        let real = include_str!("fixtures/running.json");
        let path = run_file_with(&dir, real);
        assert_eq!(read_daemon(&path, |pid| pid == 11576), Daemon::Alive(run));
        assert_eq!(read_daemon(&path, |_| false), Daemon::Dead(run));
        assert_eq!(read_daemon(&dir.path.join("none.json"), |_| true), Daemon::Absent);
        let garbled = run_file_with(&dir, "{\"pid\":");
        assert_eq!(read_daemon(&garbled, |_| true), Daemon::Garbled(RunFileProblem::NotJson));
        // A folder where the file should be, and a file of bytes that are no text.
        let folder = dir.path.join("folder");
        std::fs::create_dir(&folder).unwrap();
        assert!(matches!(read_daemon(&folder, |_| true), Daemon::Unreadable(_)));
        let binary = dir.path.join("binary.json");
        std::fs::write(&binary, [0xff, 0xfe, 0x00]).unwrap();
        assert!(matches!(read_daemon(&binary, |_| true), Daemon::Unreadable(_)));
    }

    #[test]
    fn a_run_file_larger_than_any_real_one_is_not_read_to_the_end() {
        let dir = TempDir::new("ao-big");
        let path = run_file_with(&dir, &format!("{{\"pid\":1,\"port\":2,\"pad\":\"{}\"}}", "x".repeat(200_000)));
        assert_eq!(read_daemon(&path, |_| true), Daemon::Garbled(RunFileProblem::NotJson));
    }

    #[test]
    fn this_process_is_alive_and_a_finished_one_is_not() {
        assert!(pid_alive(std::process::id()));
        let mut quick = if cfg!(windows) {
            std::process::Command::new("cmd").args(["/C", "exit"]).spawn().unwrap()
        } else {
            std::process::Command::new("true").spawn().unwrap()
        };
        let pid = quick.id();
        quick.wait().unwrap();
        assert!(!pid_alive(pid), "pid {pid} has exited");
    }

    #[test]
    fn the_settings_line_tells_installed_from_running() {
        let run = RunFile { pid: 1, port: 3001 };
        let cli = Some(PathBuf::from("ao"));
        let state = |daemon, cli| detection(&Detected { daemon, cli }).state;
        assert_eq!(state(Daemon::Alive(run), None), "running");
        assert_eq!(detection(&Detected { daemon: Daemon::Alive(run), cli: cli.clone() }).port, Some(3001));
        assert_eq!(state(Daemon::Dead(run), None), "notRunning");
        assert_eq!(state(Daemon::Absent, cli.clone()), "notRunning");
        assert_eq!(state(Daemon::Absent, None), "notInstalled");
        assert_eq!(state(Daemon::Garbled(RunFileProblem::NoPid), cli.clone()), "unsupported");
        assert_eq!(state(Daemon::Unreadable(std::io::ErrorKind::PermissionDenied), None), "unsupported");
        assert_eq!(detection(&Detected { daemon: Daemon::Absent, cli }).cli.as_deref(), Some("ao"));
    }

    // ── A daemon on a socket: answers from a table, remembers what was asked ──

    enum Reply {
        Body(u16, Vec<u8>),
        /// Accepts, then says nothing.
        Hang,
    }

    /// Serves `routes` (exact request targets) on 127.0.0.1; the request lines it saw are in the second value.
    async fn serve(routes: Vec<(String, Reply)>) -> (u16, Arc<StdMutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let routes = Arc::new(routes);
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let (routes, log) = (routes.clone(), log.clone());
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 8192 {
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let line = String::from_utf8_lossy(&request).lines().next().unwrap_or("").to_string();
                    log.lock().unwrap().push(line.clone());
                    let target = line.split(' ').nth(1).unwrap_or("").to_string();
                    let (status, body) = match routes.iter().find(|(t, _)| *t == target) {
                        Some((_, Reply::Body(status, body))) => (*status, body.clone()),
                        Some((_, Reply::Hang)) => {
                            tokio::time::sleep(Duration::from_secs(30)).await;
                            return;
                        }
                        None => (404, br#"{"error":"not_found"}"#.to_vec()),
                    };
                    let head = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                });
            }
        });
        (port, seen)
    }

    fn list_route(body: &str) -> (String, Reply) {
        ("/api/v1/sessions?active=true".into(), Reply::Body(200, body.as_bytes().to_vec()))
    }

    fn workspace_route(id: &str, file: &str) -> (String, Reply) {
        (format!("/api/v1/desktop/sessions/{id}/workspace"), Reply::Body(200, file.as_bytes().to_vec()))
    }

    /// A run file for the daemon on `port`, with this test process as its pid.
    fn run_file_for(dir: &TempDir, port: u16) -> PathBuf {
        run_file_with(dir, &format!("{{\"pid\":{},\"port\":{port}}}", std::process::id()))
    }

    fn sessions_of(outcome: &Outcome) -> &Vec<Value> {
        outcome.data["sessions"].as_array().unwrap()
    }

    #[test]
    fn a_live_daemon_gives_its_sessions_with_their_folders() {
        runtime().block_on(async {
            let dir = TempDir::new("ao-read");
            let (port, seen) = serve(vec![
                list_route(SESSIONS),
                workspace_route("mayak-6", include_str!("fixtures/workspace-mayak-6.json")),
                workspace_route("mayak-1", include_str!("fixtures/workspace-mayak-1.json")),
                // Another session's answer for this id is no answer.
                workspace_route("mayak-2", include_str!("fixtures/workspace-orbit-1.json")),
            ])
            .await;
            let path = run_file_for(&dir, port);
            let mut cache = Workspaces::default();

            let first = read(&path, true, pid_alive, &mut cache, quick()).await;
            assert_eq!(first.status, Status::Ok);
            assert_eq!(first.data["cli"], true);
            let sessions = sessions_of(&first);
            assert_eq!(sessions.len(), 8);
            let with_folder: Vec<&str> =
                sessions.iter().filter(|s| !s["workspace"].is_null()).map(|s| s["id"].as_str().unwrap()).collect();
            assert_eq!(with_folder, ["mayak-1", "mayak-6"]);
            // 6 of the 8 had no usable answer: said once, not an error.
            assert_eq!(first.note.as_deref(), Some("the folder of 6 sessions is unknown"));

            // What is known is not asked again; what failed is.
            let before = seen.lock().unwrap().len();
            let second = read(&path, true, pid_alive, &mut cache, quick()).await;
            assert_eq!(second.status, Status::Ok);
            let asked: Vec<String> = seen.lock().unwrap()[before..].to_vec();
            assert_eq!(asked.len(), 1 + 6, "the list, and the six unknown folders: {asked:?}");
            assert!(asked.iter().all(|l| !l.contains("mayak-6") && !l.contains("mayak-1/")), "{asked:?}");
        });
    }

    #[test]
    fn nothing_but_get_requests_ever_reach_the_daemon() {
        runtime().block_on(async {
            let dir = TempDir::new("ao-readonly");
            let (port, seen) = serve(vec![list_route(SESSIONS)]).await;
            let path = run_file_for(&dir, port);
            read(&path, false, pid_alive, &mut Workspaces::default(), quick()).await;
            let lines = seen.lock().unwrap().clone();
            assert!(lines.len() >= 2, "{lines:?}");
            assert!(lines.iter().all(|l| l.starts_with("GET /api/v1/")), "{lines:?}");
        });
    }

    #[test]
    fn a_session_that_goes_away_leaves_the_cache_and_a_new_daemon_starts_it_over() {
        runtime().block_on(async {
            let dir = TempDir::new("ao-cache");
            let one = r#"{"sessions":[{"id":"mayak-6","kind":"worker","status":"working"}]}"#;
            let (port, _) = serve(vec![list_route(one), workspace_route("mayak-6", include_str!("fixtures/workspace-mayak-6.json"))]).await;
            let path = run_file_for(&dir, port);
            let mut cache = Workspaces::default();
            let first = read(&path, false, pid_alive, &mut cache, quick()).await;
            assert_eq!(first.note, None);
            assert_eq!(cache.keys.len(), 1);
            // The same folder, though the daemon is another process now.
            cache.pid += 1;
            read(&path, false, pid_alive, &mut cache, quick()).await;
            assert_eq!((cache.pid, cache.keys.len()), (std::process::id(), 1));
            // Its list is empty: nothing stays.
            let (port, _) = serve(vec![list_route(r#"{"sessions":[]}"#)]).await;
            let path = run_file_for(&dir, port);
            let empty = read(&path, false, pid_alive, &mut cache, quick()).await;
            assert_eq!((empty.status, sessions_of(&empty).len(), cache.keys.len()), (Status::Ok, 0, 0));
        });
    }

    #[test]
    fn what_the_daemon_gets_wrong_is_a_status_never_a_crash_or_a_stale_list() {
        runtime().block_on(async {
            let dir = TempDir::new("ao-bad");
            let status_for = |routes: Vec<(String, Reply)>, limits: Limits| {
                let dir = &dir;
                async move {
                    let (port, _) = serve(routes).await;
                    let path = run_file_for(dir, port);
                    read(&path, false, pid_alive, &mut Workspaces::default(), limits).await
                }
            };
            let list = |status: u16, body: &str| ("/api/v1/sessions?active=true".to_string(), Reply::Body(status, body.as_bytes().to_vec()));

            // The route is gone, the daemon is failing, the answer is no JSON or no list.
            for (route, why) in [
                (list(404, "{}"), "answered 404"),
                (list(500, "oops"), "answered 500"),
                (list(200, "<html>"), "not JSON"),
                (list(200, r#"{"data":[]}"#), "no `sessions` list"),
                (list(200, r#"{"sessions":[{"id":1}]}"#), "could be read"),
            ] {
                let got = status_for(vec![route], quick()).await;
                assert_eq!(got.status, Status::Unsupported, "{why}");
                assert!(got.note.as_deref().unwrap().contains(why), "{why}: {:?}", got.note);
                assert_eq!(got.data["sessions"].as_array().unwrap().len(), 0, "no stale sessions on a failure");
            }

            // An answer over the cap, announced or not.
            let big = format!(r#"{{"sessions":[],"pad":"{}"}}"#, "x".repeat(1000));
            let small = Limits { max_body: 100, ..quick() };
            let got = status_for(vec![list(200, &big)], small).await;
            assert_eq!((got.status, got.note.as_deref()), (Status::Unsupported, Some("the session list is too large")));

            // Accepted and never answered: the timeout, not a hang.
            let started = std::time::Instant::now();
            let got = status_for(vec![("/api/v1/sessions?active=true".into(), Reply::Hang)], quick()).await;
            assert_eq!(got.status, Status::Unreachable);
            assert!(started.elapsed() < Duration::from_secs(5));

            // The port is closed: a stale run file whose pid happens to be alive.
            let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = closed.local_addr().unwrap().port();
            drop(closed);
            let path = run_file_for(&dir, port);
            let got = read(&path, false, pid_alive, &mut Workspaces::default(), quick()).await;
            assert_eq!(got.status, Status::Unreachable);
        });
    }

    /// The AO on this PC, read the way the island reads it: `cargo test --lib ao::tests::real -- --ignored --nocapture`.
    /// Skipped unless asked, since it needs a running AO.
    #[test]
    #[ignore = "needs a running Agent Orchestrator"]
    fn real_daemon_on_this_pc() {
        runtime().block_on(async {
            let found = detect();
            eprintln!("detected: {found:?} -> {:?}", detection(&found));
            let run_file = run_file_path(std::env::var_os("AO_RUN_FILE"), &platform::home_dir());
            let got = read(&run_file, found.cli.is_some(), pid_alive, &mut Workspaces::default(), LIMITS).await;
            eprintln!("status {:?}, note {:?}", got.status, got.note);
            assert_eq!(got.status, Status::Ok);
            for s in sessions_of(&got) {
                eprintln!("{} | {} | {} | {} | {} | prs {}/{} ci {} | {}", s["id"], s["project"], s["role"], s["status"], s["activity"], s["prsOpen"], s["prs"], s["ci"], s["workspace"]);
                assert!(s["workspace"].is_string(), "{} has no folder", s["id"]);
            }
        });
    }

    #[test]
    fn without_a_daemon_it_is_a_file_read_and_no_request() {
        runtime().block_on(async {
            let dir = TempDir::new("ao-none");
            let mut cache = Workspaces::default();
            let absent = dir.path.join("none.json");
            let none = read(&absent, false, pid_alive, &mut cache, quick()).await;
            assert_eq!((none.status, none.note), (Status::NotInstalled, None));
            assert_eq!(none.data["status"], "notInstalled");
            let cli_only = read(&absent, true, pid_alive, &mut cache, quick()).await;
            assert_eq!((cli_only.status, cli_only.data["cli"].as_bool()), (Status::NotRunning, Some(true)));

            let dead = run_file_with(&dir, r#"{"pid":4000000000,"port":3001}"#);
            let got = read(&dead, true, |_| false, &mut cache, quick()).await;
            assert_eq!(got.status, Status::NotRunning);
            assert!(got.note.unwrap().contains("4000000000"));

            let garbled = run_file_with(&dir, "not json");
            let got = read(&garbled, true, pid_alive, &mut cache, quick()).await;
            assert_eq!((got.status, got.note.as_deref()), (Status::Unsupported, Some("the run file is not JSON")));
            assert_eq!(cache.keys.len(), 0);
        });
    }
}
