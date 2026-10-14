//! The manager end to end: real processes, pipes and threads, with fake CLIs. A fake is a
//! PowerShell script behind a `.cmd` shim named like the CLI, so they run on Windows only; the
//! parts that need no process (opening, refusing, choosing) run everywhere. The adapters' real
//! output is checked in their own files against `fixtures/`.

use super::*;
use crate::testutil::TempDir;

use std::collections::HashMap;

#[derive(Default)]
struct FakeHost {
    clis: Mutex<HashMap<String, PathBuf>>,
    changed: Mutex<Vec<Vec<u64>>>,
    asked: Mutex<Vec<(u64, String, String)>>,
    /// Where the conversations are kept; None keeps nothing (most tests).
    saved: Mutex<Option<PathBuf>>,
}

impl Host for FakeHost {
    fn changed(&self, ids: &[u64]) {
        lock(&self.changed).push(ids.to_vec());
    }

    fn permission_asked(&self, conv: u64, request_id: &str, tool: &str) {
        lock(&self.asked).push((conv, request_id.to_string(), tool.to_string()));
    }

    fn find_cli(&self, stem: &str) -> Option<PathBuf> {
        lock(&self.clis).get(stem).cloned()
    }

    fn saved_path(&self) -> Option<PathBuf> {
        lock(&self.saved).clone()
    }
}

fn manager(host: &Arc<FakeHost>) -> ChatManager {
    ChatManager::with_timing(host.clone(), Duration::from_secs(60), Duration::from_millis(400), Duration::from_millis(10))
}

fn open_req(agent: AgentKind, cwd: &Path) -> OpenRequest {
    OpenRequest { agent, cwd: cwd.to_string_lossy().into_owned(), model: String::new(), mode: String::new(), resume: None }
}

/// Polls `cond` for up to 20 s.
fn wait_for(what: &str, cond: impl Fn() -> bool) {
    wait_for_secs(20, what, cond);
}

fn wait_for_secs(secs: u64, what: &str, cond: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

fn state(m: &ChatManager, conv: u64) -> State {
    m.get(conv, None).expect("conversation").state
}

fn assistant_texts(m: &ChatManager, conv: u64) -> Vec<String> {
    m.get(conv, None)
        .unwrap()
        .msgs
        .iter()
        .filter_map(|msg| match &msg.body {
            convo::Body::Assistant { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

// ── No process needed ──

#[test]
fn opening_checks_the_folder_the_mode_the_model_and_that_the_cli_exists() {
    let dir = TempDir::new("chat-open");
    let host = Arc::new(FakeHost::default());
    let m = manager(&host);

    assert_eq!(m.open(open_req(AgentKind::ClaudeCode, &dir.path)).unwrap_err().code, "cliMissing");
    lock(&host.clis).insert("claude".into(), dir.path.join("claude.cmd"));

    let mut bad = open_req(AgentKind::ClaudeCode, &dir.path.join("nope"));
    assert_eq!(m.open(bad.clone()).unwrap_err().code, "badFolder");
    bad.cwd = "relative\\dir".into();
    assert_eq!(m.open(bad).unwrap_err().code, "badFolder");
    for mode in ["bypassPermissions", "yolo", "readOnly"] {
        let mut req = open_req(AgentKind::ClaudeCode, &dir.path);
        req.mode = mode.into();
        assert_eq!(m.open(req).unwrap_err().code, "badMode", "{mode}");
    }
    let mut req = open_req(AgentKind::ClaudeCode, &dir.path);
    req.model = "--dangerously-skip-permissions".into();
    assert_eq!(m.open(req).unwrap_err().code, "badModel");
    let mut req = open_req(AgentKind::ClaudeCode, &dir.path);
    req.resume = Some("a b".into());
    assert_eq!(m.open(req).unwrap_err().code, "badResume");

    let id = m.open(open_req(AgentKind::ClaudeCode, &dir.path)).unwrap();
    let convo = m.get(id, None).unwrap();
    assert_eq!((convo.state, convo.mode.as_str(), convo.agent), (State::Idle, "ask", AgentKind::ClaudeCode));
    // No folder: the home folder.
    let home = m.open(OpenRequest { cwd: String::new(), ..open_req(AgentKind::ClaudeCode, &dir.path) }).unwrap();
    assert_eq!(m.get(home, None).unwrap().cwd, crate::platform::home_dir().to_string_lossy());
    assert_eq!(m.list().len(), 2);
    assert!(m.get(id, Some(convo.rev)).is_none(), "nothing new since that revision");
    m.close(id);
    assert!(m.get(id, None).is_none());
}

#[test]
fn a_message_that_cannot_be_sent_changes_nothing() {
    let dir = TempDir::new("chat-refuse");
    let host = Arc::new(FakeHost::default());
    lock(&host.clis).insert("claude".into(), dir.path.join("claude.cmd"));
    let m = manager(&host);
    let id = m.open(open_req(AgentKind::ClaudeCode, &dir.path)).unwrap();
    assert_eq!(m.send(id, "   ").unwrap_err().code, "badPrompt");
    assert_eq!(m.send(id, &"x".repeat(MAX_PROMPT + 1)).unwrap_err().code, "badPrompt");
    assert_eq!(m.send(99, "hi").unwrap_err().code, "unknownConversation");
    lock(&host.clis).clear();
    assert_eq!(m.send(id, "hi").unwrap_err().code, "cliMissing");
    assert!(m.get(id, None).unwrap().msgs.is_empty(), "no user message was added");
    assert!(!m.answer_permission("1.1", true), "no such request: a click changes nothing");
}

#[test]
fn the_model_and_mode_are_checked_when_they_change() {
    let dir = TempDir::new("chat-choice");
    let host = Arc::new(FakeHost::default());
    lock(&host.clis).insert("codex".into(), dir.path.join("codex.cmd"));
    let m = manager(&host);
    let id = m.open(open_req(AgentKind::Codex, &dir.path)).unwrap();
    assert_eq!(m.get(id, None).unwrap().mode, "readOnly");
    m.set_choice(id, "gpt-6.1-sol".into(), "workspaceWrite".into()).unwrap();
    assert_eq!(m.set_choice(id, String::new(), "danger-full-access".into()).unwrap_err().code, "badMode");
    assert_eq!(m.set_choice(id, "-x".into(), "readOnly".into()).unwrap_err().code, "badModel");
    assert_eq!(m.set_choice(77, String::new(), "readOnly".into()).unwrap_err().code, "unknownConversation");
    let convo = m.get(id, None).unwrap();
    assert_eq!((convo.model.as_str(), convo.mode.as_str()), ("gpt-6.1-sol", "workspaceWrite"));
}

#[test]
fn the_mcp_provider_is_asked_each_time_per_cli_and_can_be_removed() {
    let host = Arc::new(FakeHost::default());
    let m = manager(&host);
    assert_eq!(m.mcp_attach(AgentKind::ClaudeCode), None);
    let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let counter = calls.clone();
    m.set_mcp_provider(Some(Arc::new(move |kind| {
        counter.fetch_add(1, Ordering::SeqCst);
        // The server knows two of the four CLIs.
        matches!(kind, AgentKind::ClaudeCode | AgentKind::Codex)
            .then(|| McpAttach { args: vec!["--mcp-config".into(), "{}".into()], env: vec![("MAYAK_MCP_TOKEN".into(), "t".into())] })
    })));
    assert_eq!(m.mcp_attach(AgentKind::ClaudeCode).map(|a| a.args[0].clone()).as_deref(), Some("--mcp-config"));
    assert_eq!(m.mcp_attach(AgentKind::GeminiCli), None);
    assert_eq!(calls.load(Ordering::SeqCst), 2, "asked each time");
    m.set_mcp_provider(None);
    assert_eq!(m.mcp_attach(AgentKind::ClaudeCode), None);
}

#[test]
fn a_saved_file_that_cannot_be_read_starts_empty() {
    let dir = TempDir::new("chat-bad-file");
    let path = dir.path.join(SAVED_FILE);
    std::fs::write(&path, "{ not json").unwrap();
    let host = Arc::new(FakeHost::default());
    *lock(&host.saved) = Some(path.clone());
    let m = manager(&host);
    assert!(m.list().is_empty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json", "left alone until the next save replaces it");
    assert_eq!(m.conv_for_session(""), None);
    assert!(!m.has_sessions());
}

// ── Fake CLIs ──

#[cfg(windows)]
mod processes {
    use super::*;

    const PRELUDE: &str = r#"
$ErrorActionPreference = 'Stop'
$stdin = [System.IO.StreamReader]::new([Console]::OpenStandardInput(), [System.Text.UTF8Encoding]::new($false))
$stdout = [System.IO.StreamWriter]::new([Console]::OpenStandardOutput(), [System.Text.UTF8Encoding]::new($false))
$stdout.AutoFlush = $true
$argline = (Get-Content -Raw -Path "$PSScriptRoot\args.txt").Trim()
function Send([string]$s) { $stdout.WriteLine($s) }
function J($o) { ConvertTo-Json $o -Compress -Depth 8 }
function Delta([string]$t) { Send (J @{type='stream_event'; event=@{type='content_block_delta'; delta=@{type='text_delta'; text=$t}}}) }
"#;

    /// A persistent Claude-like CLI: one process, many turns.
    const FAKE_CLAUDE: &str = r#"
$sid = '11111111-2222-3333-4444-555555555555'
$turn = 0
while ($true) {
  $line = $stdin.ReadLine()
  if ($line -eq $null) { break }
  $msg = $line | ConvertFrom-Json
  if ($msg.type -ne 'user') { continue }
  $turn++
  $prompt = [string]$msg.message.content
  Send (J @{type='system'; subtype='init'; session_id=$sid; model='fake'; apiKeySource='none'})
  if ($prompt -like 'ask*') {
    Send (J @{type='control_request'; request_id="req-$turn"; request=@{subtype='can_use_tool'; tool_name='Bash'; input=@{command='echo hi'}; tool_use_id='toolu_1'}})
    $resp = $stdin.ReadLine() | ConvertFrom-Json
    Delta ('got ' + $resp.response.response.behavior)
  } elseif ($prompt -like 'slow*' -or $prompt -like 'stuck*') {
    Set-Content -Path "$PSScriptRoot\pid.txt" -Value $PID
    Delta 'working...'
    $pending = $stdin.ReadLineAsync()
    $interrupted = $false
    for ($i = 0; $i -lt 400; $i++) {
      if ($pending.IsCompleted -and $prompt -like 'slow*') {
        $m = $pending.Result | ConvertFrom-Json
        if ($m.request.subtype -eq 'interrupt') {
          Send (J @{type='control_response'; response=@{subtype='success'; request_id=$m.request_id; response=@{still_queued=@()}}})
          $interrupted = $true
          break
        }
      }
      Start-Sleep -Milliseconds 50
    }
    if ($interrupted) {
      Send (J @{type='result'; subtype='error_during_execution'; is_error=$true; terminal_reason='aborted_streaming'; result=$null})
      continue
    }
  } elseif ($prompt -like 'die*') {
    [Console]::Error.WriteLine('boom: the fake CLI crashed')
    exit 3
  } else {
    Delta ("echo: $prompt turn $turn args: " + $argline + " | token: " + $env:FAKE_MCP_TOKEN + " | mcp: " + $env:MAYAK_MCP_TOKEN)
  }
  Send (J @{type='result'; subtype='success'; is_error=$false; result='ok'})
}
"#;

    /// A one-shot Codex-like CLI: reads its prompt to the end of stdin, answers, exits.
    const FAKE_CODEX: &str = r#"
$prompt = $stdin.ReadToEnd()
Send (J @{type='thread.started'; thread_id='01a13655-87f4-73d2-91e3-5ed13cda08cc'})
Send (J @{type='turn.started'})
Send (J @{type='item.completed'; item=@{id='item_0'; type='agent_message'; text=("codex got: " + $prompt.Trim() + " | args: " + $argline)}})
Send (J @{type='turn.completed'})
"#;

    struct Rig {
        dir: TempDir,
        host: Arc<FakeHost>,
    }

    fn write_cli(dir: &Path, stem: &str, body: &str) -> PathBuf {
        std::fs::write(dir.join(format!("{stem}.ps1")), format!("{PRELUDE}{body}")).unwrap();
        let shim = dir.join(format!("{stem}.cmd"));
        // The arguments go to a file the script reads: PowerShell would try to bind `-c` and `-s` itself.
        let body = format!("@echo off\r\necho %*>\"%~dp0args.txt\"\r\npowershell -NoProfile -ExecutionPolicy Bypass -File \"%~dp0{stem}.ps1\"\r\n");
        std::fs::write(&shim, body).unwrap();
        shim
    }

    fn rig() -> Rig {
        let dir = TempDir::new("chat-run");
        let host = Arc::new(FakeHost::default());
        lock(&host.clis).insert("claude".into(), write_cli(&dir.path, "claude", FAKE_CLAUDE));
        lock(&host.clis).insert("codex".into(), write_cli(&dir.path, "codex", FAKE_CODEX));
        Rig { dir, host }
    }

    #[test]
    fn a_persistent_cli_keeps_one_process_for_several_turns() {
        let rig = rig();
        let m = manager(&rig.host);
        let id = m.open(open_req(AgentKind::ClaudeCode, &rig.dir.path)).unwrap();
        m.send(id, "hello").unwrap();
        wait_for("the first answer", || state(&m, id) == State::Idle && !assistant_texts(&m, id).is_empty());
        let first = assistant_texts(&m, id);
        assert!(first[0].starts_with("echo: hello turn 1 args: -p --input-format stream-json"), "{first:?}");
        assert!(first[0].contains("--permission-prompt-tool stdio"), "the verified flags reach the CLI");
        let convo = m.get(id, None).unwrap();
        assert_eq!(convo.session.as_deref(), Some("11111111-2222-3333-4444-555555555555"));
        assert!(m.owns_session("11111111-2222-3333-4444-555555555555"), "the router lets its hook go");
        assert!(!m.owns_session("something-else") && !m.owns_session(""));

        m.send(id, "again").unwrap();
        wait_for("the second answer", || assistant_texts(&m, id).len() == 2 && state(&m, id) == State::Idle);
        assert!(assistant_texts(&m, id)[1].starts_with("echo: again turn 2"), "the same process counted its second turn");
        assert!(!lock(&rig.host.changed).is_empty(), "the island was told");
    }

    #[test]
    fn a_permission_request_waits_for_a_click_and_the_click_answers_the_cli() {
        let rig = rig();
        let m = manager(&rig.host);
        let id = m.open(open_req(AgentKind::ClaudeCode, &rig.dir.path)).unwrap();
        m.send(id, "ask please").unwrap();
        wait_for("the request", || state(&m, id) == State::Waiting);
        let (conv, request_id, tool) = lock(&rig.host.asked)[0].clone();
        assert_eq!((conv, tool.as_str()), (id, "Bash"));

        // Nothing answers by itself: still waiting after a while.
        std::thread::sleep(Duration::from_millis(900));
        assert_eq!(state(&m, id), State::Waiting);
        let msgs = m.get(id, None).unwrap().msgs;
        assert!(msgs.iter().any(|x| matches!(&x.body, convo::Body::Permission { status: convo::PermissionStatus::Pending, summary, .. } if summary == "echo hi")));
        assert!(assistant_texts(&m, id).is_empty());

        assert!(m.answer_permission(&request_id, true));
        wait_for("the turn to go on", || state(&m, id) == State::Idle);
        assert_eq!(assistant_texts(&m, id), ["got allow"]);
        assert!(!m.answer_permission(&request_id, false), "a second click changes nothing");

        // A denial reaches the CLI as a denial.
        m.send(id, "ask again").unwrap();
        wait_for("the second request", || lock(&rig.host.asked).len() == 2);
        let second = lock(&rig.host.asked)[1].1.clone();
        assert_ne!(second, request_id);
        assert!(m.answer_permission(&second, false));
        wait_for("the denial answer", || assistant_texts(&m, id).len() == 2);
        assert_eq!(assistant_texts(&m, id)[1], "got deny");
    }

    #[test]
    fn stopping_a_cli_that_takes_the_interrupt_ends_the_turn_and_keeps_the_process() {
        let rig = rig();
        let m = manager(&rig.host);
        let id = m.open(open_req(AgentKind::ClaudeCode, &rig.dir.path)).unwrap();
        m.send(id, "slow one").unwrap();
        wait_for("the work to start", || assistant_texts(&m, id).first().is_some_and(|t| t == "working..."));
        m.cancel(id).unwrap();
        wait_for("the stop", || state(&m, id) == State::Idle);
        let convo = m.get(id, None).unwrap();
        assert!(convo.msgs.iter().any(|x| matches!(&x.body, convo::Body::Notice { code, .. } if code == "stopped")));
        assert!(m.owns_session("11111111-2222-3333-4444-555555555555"), "the process is still there for the next turn");
        m.send(id, "after the stop").unwrap();
        wait_for("the next turn", || assistant_texts(&m, id).iter().any(|t| t.contains("after the stop turn 2")));
    }

    #[test]
    fn stopping_a_cli_that_ignores_the_interrupt_ends_its_whole_process_tree() {
        let rig = rig();
        let m = manager(&rig.host);
        let id = m.open(open_req(AgentKind::ClaudeCode, &rig.dir.path)).unwrap();
        m.send(id, "stuck forever").unwrap();
        let pid_file = rig.dir.path.join("pid.txt");
        wait_for("the fake to say its pid", || std::fs::read_to_string(&pid_file).is_ok_and(|t| t.trim().parse::<u32>().is_ok()));
        let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        assert!(crate::observe::os::proc::identify(pid).is_ok(), "the CLI runs");

        m.cancel(id).unwrap();
        wait_for("the stop", || state(&m, id) == State::Idle);
        wait_for("the process to be gone", || crate::observe::os::proc::identify(pid).is_err());
        assert!(!m.owns_session("11111111-2222-3333-4444-555555555555"), "no live run any more");
        let convo = m.get(id, None).unwrap();
        assert!(convo.msgs.iter().any(|x| matches!(&x.body, convo::Body::Notice { code, .. } if code == "stopped")), "a stop is not an error");
        assert!(!convo.msgs.iter().any(|x| matches!(&x.body, convo::Body::Error { .. })));
        // The conversation goes on in a new process, resuming the CLI's session.
        m.send(id, "next").unwrap();
        wait_for("the new process", || assistant_texts(&m, id).iter().any(|t| t.contains("next turn 1") && t.contains("--resume 11111111-2222-3333-4444-555555555555")));
    }

    #[test]
    fn a_cli_that_dies_mid_turn_is_an_error_with_its_own_words() {
        let rig = rig();
        let m = manager(&rig.host);
        let id = m.open(open_req(AgentKind::ClaudeCode, &rig.dir.path)).unwrap();
        m.send(id, "die now").unwrap();
        wait_for("the failure", || state(&m, id) == State::Failed);
        let convo = m.get(id, None).unwrap();
        let error = convo.msgs.iter().find_map(|x| if let convo::Body::Error { code, text } = &x.body { Some((code.clone(), text.clone())) } else { None });
        let (code, text) = error.expect("an error message");
        assert_eq!(code, "exited");
        assert!(text.contains("boom: the fake CLI crashed"), "{text}");
        assert!(!m.owns_session("11111111-2222-3333-4444-555555555555"));
        // A failed conversation can be tried again.
        m.send(id, "retry").unwrap();
        wait_for("the retry", || assistant_texts(&m, id).iter().any(|t| t.contains("retry turn 1")));
    }

    #[test]
    fn a_one_shot_cli_gets_the_prompt_on_stdin_and_the_next_turn_resumes_its_thread() {
        let rig = rig();
        let m = manager(&rig.host);
        let id = m.open(open_req(AgentKind::Codex, &rig.dir.path)).unwrap();
        m.send(id, "first\nsecond line \"quoted\"").unwrap();
        wait_for("the first answer", || state(&m, id) == State::Idle && !assistant_texts(&m, id).is_empty());
        let first = assistant_texts(&m, id)[0].clone();
        assert!(first.starts_with("codex got: first\nsecond line \"quoted\" | args: exec --json --skip-git-repo-check"), "{first}");
        assert!(first.contains("-s read-only") && first.ends_with(" -"), "{first}");
        assert_eq!(m.get(id, None).unwrap().session.as_deref(), Some("01a13655-87f4-73d2-91e3-5ed13cda08cc"));

        m.send(id, "more").unwrap();
        wait_for("the second answer", || assistant_texts(&m, id).len() == 2 && state(&m, id) == State::Idle);
        let second = assistant_texts(&m, id)[1].clone();
        assert!(second.contains("args: exec resume --json") && second.contains("01a13655-87f4-73d2-91e3-5ed13cda08cc -"), "{second}");
        // (a .cmd shim sees the quotes of a -c value doubled, the way cmd.exe wants them; codex.exe does not)
        assert!(second.contains("sandbox_mode=") && second.contains("read-only"), "{second}");
    }

    #[test]
    fn a_persistent_process_that_rested_ends_and_the_next_turn_resumes_the_session() {
        let rig = rig();
        let m = ChatManager::with_timing(rig.host.clone(), Duration::from_millis(500), Duration::from_millis(400), Duration::from_millis(10));
        let id = m.open(open_req(AgentKind::ClaudeCode, &rig.dir.path)).unwrap();
        m.send(id, "hello").unwrap();
        wait_for("the answer", || state(&m, id) == State::Idle && !assistant_texts(&m, id).is_empty());
        assert!(m.owns_session("11111111-2222-3333-4444-555555555555"));
        wait_for("the rested process to end", || !m.owns_session("11111111-2222-3333-4444-555555555555"));
        assert_eq!(state(&m, id), State::Idle, "resting is not a failure");
        m.send(id, "back").unwrap();
        wait_for("the new process", || assistant_texts(&m, id).iter().any(|t| t.contains("back turn 1") && t.contains("--resume 11111111")));
    }

    #[test]
    fn changing_the_mode_starts_a_new_process_with_the_new_flags() {
        let rig = rig();
        let m = manager(&rig.host);
        let id = m.open(open_req(AgentKind::ClaudeCode, &rig.dir.path)).unwrap();
        m.send(id, "hello").unwrap();
        wait_for("the answer", || state(&m, id) == State::Idle && !assistant_texts(&m, id).is_empty());
        m.set_choice(id, "haiku".into(), "plan".into()).unwrap();
        m.send(id, "planning").unwrap();
        wait_for("the answer in the new mode", || {
            assistant_texts(&m, id).iter().any(|t| t.contains("planning turn 1") && t.contains("--model haiku --permission-mode plan"))
        });
    }

    /// A provider whose answer the test changes: None is "the MCP switch is off".
    fn switchable(token: &Arc<Mutex<Option<String>>>) -> McpProvider {
        let token = token.clone();
        Arc::new(move |kind| {
            let t = lock(&token).clone()?;
            (kind == AgentKind::ClaudeCode).then(|| McpAttach {
                args: vec!["--mcp-config".into(), "{}".into()],
                env: vec![("MAYAK_MCP_TOKEN".into(), t)],
            })
        })
    }

    /// The rig, keeping its conversations in a file of its folder.
    fn keeping_rig() -> (Rig, PathBuf) {
        let rig = rig();
        let path = rig.dir.path.join("app").join(SAVED_FILE);
        *lock(&rig.host.saved) = Some(path.clone());
        (rig, path)
    }

    #[test]
    fn a_codex_conversation_is_kept_across_a_restart_and_its_next_turn_resumes_the_thread() {
        let (rig, path) = keeping_rig();
        let m = manager(&rig.host);
        let id = m.open(open_req(AgentKind::Codex, &rig.dir.path)).unwrap();
        assert!(!path.exists(), "nothing said yet: nothing kept");
        m.send(id, "hello").unwrap();
        wait_for("the answer", || state(&m, id) == State::Idle && !assistant_texts(&m, id).is_empty());
        wait_for("the file", || path.exists());
        assert_eq!(m.conv_for_session("01a13655-87f4-73d2-91e3-5ed13cda08cc"), Some(id), "the session list finds the chat");
        assert!(m.has_sessions());
        m.shutdown();
        drop(m);

        let again = manager(&rig.host);
        let list = again.list();
        assert_eq!(list.len(), 1, "the conversation came back");
        let back = again.get(list[0].id, None).unwrap();
        assert_eq!((back.title.as_str(), back.state, back.agent), ("hello", State::Idle, AgentKind::Codex));
        assert_eq!(back.session.as_deref(), Some("01a13655-87f4-73d2-91e3-5ed13cda08cc"));
        assert_eq!(assistant_texts(&again, back.id).len(), 1, "with what it showed");
        again.send(back.id, "and now").unwrap();
        wait_for("the resumed answer", || assistant_texts(&again, back.id).len() == 2 && state(&again, back.id) == State::Idle);
        let second = &assistant_texts(&again, back.id)[1];
        assert!(second.contains("args: exec resume ") && second.contains(" 01a13655-87f4-73d2-91e3-5ed13cda08cc -"), "the CLI's own resume: {second}");
    }

    #[test]
    fn a_claude_conversation_comes_back_and_a_new_process_resumes_its_session() {
        let (rig, path) = keeping_rig();
        let m = manager(&rig.host);
        let id = m.open(open_req(AgentKind::ClaudeCode, &rig.dir.path)).unwrap();
        m.send(id, "hello").unwrap();
        wait_for("the answer", || state(&m, id) == State::Idle && !assistant_texts(&m, id).is_empty());
        wait_for("the file", || path.exists());
        m.shutdown();
        drop(m);

        let again = manager(&rig.host);
        let back = again.list()[0].id;
        again.send(back, "again").unwrap();
        wait_for("the answer after the restart", || assistant_texts(&again, back).len() == 2 && state(&again, back) == State::Idle);
        let second = &assistant_texts(&again, back)[1];
        assert!(second.contains("--resume 11111111-2222-3333-4444-555555555555"), "{second}");
        assert!(second.starts_with("echo: again turn 1"), "a new process: {second}");
    }

    #[test]
    fn clearing_the_saved_chats_removes_the_idle_ones_and_the_file() {
        let (rig, path) = keeping_rig();
        let m = manager(&rig.host);
        let id = m.open(open_req(AgentKind::Codex, &rig.dir.path)).unwrap();
        m.send(id, "hello").unwrap();
        wait_for("the answer", || state(&m, id) == State::Idle && !assistant_texts(&m, id).is_empty());
        wait_for("the file", || path.exists());
        let empty = m.open(open_req(AgentKind::Codex, &rig.dir.path)).unwrap();
        assert_eq!(m.saved_count(), 1, "the empty chat is not kept");
        assert_eq!(m.clear_saved(), 2);
        assert!(m.list().is_empty() && m.get(empty, None).is_none());
        std::thread::sleep(SAVE_DELAY * 3);
        assert!(!path.exists(), "the file went with them and nothing wrote it again");
        assert!(lock(&rig.host.changed).iter().any(|ids| ids.contains(&id)), "the island was told");
        drop(m);
        assert!(manager(&rig.host).list().is_empty(), "nothing comes back");
    }

    #[test]
    fn the_mcp_switch_and_a_new_token_take_effect_at_the_next_turn_with_a_new_process() {
        let rig = rig();
        let m = manager(&rig.host);
        let token = Arc::new(Mutex::new(Some("tok-A".to_string())));
        m.set_mcp_provider(Some(switchable(&token)));
        let id = m.open(open_req(AgentKind::ClaudeCode, &rig.dir.path)).unwrap();
        let said = |n: usize| assistant_texts(&m, id).get(n).cloned().unwrap_or_default();
        let turn = |text: &str, n: usize| {
            m.send(id, text).unwrap();
            wait_for("the answer", || state(&m, id) == State::Idle && assistant_texts(&m, id).len() == n);
        };

        turn("one", 1);
        assert!(said(0).contains("one turn 1") && said(0).contains("--mcp-config") && said(0).ends_with("mcp: tok-A"), "{}", said(0));
        // Nothing changed: the same process takes the second turn.
        turn("two", 2);
        assert!(said(1).contains("two turn 2") && said(1).ends_with("mcp: tok-A"), "{}", said(1));
        // A new token: a new process (its first turn) with the new token.
        *lock(&token) = Some("tok-B".into());
        turn("three", 3);
        assert!(said(2).contains("three turn 1") && said(2).ends_with("mcp: tok-B"), "{}", said(2));
        // The switch goes off: a new process with nothing of the server.
        *lock(&token) = None;
        turn("four", 4);
        assert!(said(3).contains("four turn 1") && !said(3).contains("--mcp-config") && said(3).ends_with("mcp: "), "{}", said(3));
        // And on again.
        *lock(&token) = Some("tok-A".into());
        turn("five", 5);
        assert!(said(4).contains("five turn 1") && said(4).contains("--mcp-config") && said(4).ends_with("mcp: tok-A"), "{}", said(4));
    }

    #[test]
    fn what_the_real_attach_of_the_mcp_server_hands_over_reaches_the_cli_without_its_token_in_the_arguments() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let rig = rig();
        let m = manager(&rig.host);
        m.set_mcp_provider(Some(Arc::new(|kind| {
            let cli = match kind {
                AgentKind::ClaudeCode => crate::mcp_server::Cli::Claude,
                AgentKind::Codex => crate::mcp_server::Cli::Codex,
                _ => return None,
            };
            let a = crate::mcp_server::attach(cli, "http://127.0.0.1:47831/mcp", TOKEN);
            Some(McpAttach { args: a.args, env: a.env })
        })));
        let id = m.open(open_req(AgentKind::ClaudeCode, &rig.dir.path)).unwrap();
        m.send(id, "hello").unwrap();
        wait_for("the answer", || state(&m, id) == State::Idle && !assistant_texts(&m, id).is_empty());
        let said = assistant_texts(&m, id)[0].clone();
        let (echoed, rest) = said.split_once(" | token: ").expect("the fake echoes arguments and environment");
        let args = echoed.split_once(" args: ").map(|(_, a)| a).expect("the fake echoes its arguments");
        assert!(args.contains("--mcp-config"), "{args}");
        assert!(args.contains("47831/mcp") && args.contains("270000"), "the config with the url and the timeout is on the command line: {args}");
        assert!(!args.contains(TOKEN), "the token is not on the command line");
        assert!(!args.contains("--allowedTools"), "nothing is pre-approved");
        assert!(rest.ends_with(&format!("mcp: {TOKEN}")), "the token is in the CLI's environment: {rest}");
        // The prompt is on stdin, never an argument after the variadic --mcp-config.
        assert!(args.rfind("--mcp-config").unwrap() > args.rfind("--permission-prompt-tool").unwrap());
        assert!(!args.contains("hello"));
    }

    #[test]
    fn the_mcp_servers_token_reaches_the_cli_in_its_environment_and_never_in_its_arguments() {
        let rig = rig();
        let m = manager(&rig.host);
        m.set_mcp_provider(Some(Arc::new(|kind| {
            (kind == AgentKind::ClaudeCode).then(|| McpAttach {
                args: vec!["--mcp-config".into(), "{}".into()],
                env: vec![("FAKE_MCP_TOKEN".into(), "tok-7f3a".into())],
            })
        })));
        let id = m.open(open_req(AgentKind::ClaudeCode, &rig.dir.path)).unwrap();
        m.send(id, "hello").unwrap();
        wait_for("the answer", || state(&m, id) == State::Idle && !assistant_texts(&m, id).is_empty());
        let said = assistant_texts(&m, id)[0].clone();
        let (args, rest) = said.split_once(" | token: ").expect("the fake echoes both");
        let token = rest.split(" | mcp: ").next().unwrap_or_default();
        assert_eq!(token, "tok-7f3a", "the environment of the CLI has the token");
        // (a .cmd shim sees `{}` quoted for cmd.exe; codex.exe or claude.exe would not)
        assert!(args.ends_with("--mcp-config {}") || args.ends_with("--mcp-config \"{}\""), "the server's arguments are there, last: {args}");
        assert!(!args.contains("tok-7f3a"), "the token is not on the command line");
    }

    #[test]
    fn closing_a_conversation_ends_its_process() {
        let rig = rig();
        let m = manager(&rig.host);
        let id = m.open(open_req(AgentKind::ClaudeCode, &rig.dir.path)).unwrap();
        m.send(id, "slow burn").unwrap();
        let pid_file = rig.dir.path.join("pid.txt");
        wait_for("the pid", || std::fs::read_to_string(&pid_file).is_ok_and(|t| t.trim().parse::<u32>().is_ok()));
        let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        m.close(id);
        wait_for("the process to be gone", || crate::observe::os::proc::identify(pid).is_err());
        assert!(m.get(id, None).is_none());
    }

    #[test]
    fn dropping_the_manager_ends_the_processes_it_started() {
        let rig = rig();
        let pid_file = rig.dir.path.join("pid.txt");
        let pid: u32 = {
            let m = manager(&rig.host);
            let id = m.open(open_req(AgentKind::ClaudeCode, &rig.dir.path)).unwrap();
            m.send(id, "slow burn").unwrap();
            wait_for("the pid", || std::fs::read_to_string(&pid_file).is_ok_and(|t| t.trim().parse::<u32>().is_ok()));
            let pid = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
            m.shutdown();
            pid
        };
        wait_for("the process to be gone", || crate::observe::os::proc::identify(pid).is_err());
    }
}

/// The manager against the real CLIs installed on the machine. Off by default: it uses the
/// user's own subscription logins and starts real agents. Run with
/// `cargo test --lib chat::tests::real -- --ignored --test-threads=1 --nocapture`.
#[cfg(windows)]
mod real {
    use super::*;

    fn real_host(dir: &Path) -> Arc<FakeHost> {
        let host = Arc::new(FakeHost::default());
        // Claude runs without the user's settings files (their hooks would raise cards in a running
        // Mayak for this test's prompts); its login does not depend on them.
        let claude = crate::platform::find_on_path("claude").expect("claude on PATH");
        let shim = dir.join("claude.cmd");
        std::fs::write(&shim, format!("@echo off\r\n\"{}\" %* --setting-sources project\r\n", claude.display())).unwrap();
        lock(&host.clis).insert("claude".into(), shim);
        lock(&host.clis).insert("codex".into(), crate::platform::find_on_path("codex").expect("codex on PATH"));
        host
    }

    /// Waits for the turn to end, at most 90 s. A failed turn panics with the CLI's own words, so a
    /// usage limit or a signed-out CLI says so instead of timing out.
    fn turn_done(m: &ChatManager, id: u64, what: &str) {
        wait_for_secs(90, what, || !matches!(state(m, id), State::Working | State::Waiting));
        let c = m.get(id, None).unwrap();
        if c.state == State::Failed {
            let why = c.msgs.iter().rev().find_map(|x| match &x.body {
                convo::Body::Error { code, text } => Some(format!("{code}: {text}")),
                _ => None,
            });
            panic!("{what}: the turn failed: {}", why.unwrap_or_default());
        }
    }

    /// A conversation kept on disk, the manager gone (the app quit), a new one reading the file: the
    /// next message resumes the CLI's own session, which still knows the first turn.
    fn resumes_after_a_restart(agent: AgentKind, model: &str) {
        let dir = TempDir::new("chat-real-keep");
        let host = real_host(&dir.path);
        *lock(&host.saved) = Some(dir.path.join("app").join(SAVED_FILE));
        let m = ChatManager::new(host.clone());
        let mut req = open_req(agent, &dir.path);
        req.model = model.into();
        let id = m.open(req).unwrap();
        m.send(id, "Remember the word heliotrope. Reply with exactly: ok").unwrap();
        turn_done(&m, id, "the first turn");
        let session = m.get(id, None).unwrap().session.expect("the CLI's session id");
        m.shutdown();
        drop(m);

        let again = ChatManager::new(host);
        let back = again.list().first().expect("the conversation came back").id;
        assert_eq!(again.get(back, None).unwrap().session.as_deref(), Some(session.as_str()));
        again.send(back, "Which word did I ask you to remember? Reply with that word only.").unwrap();
        turn_done(&again, back, "the turn after the restart");
        let answer = assistant_texts(&again, back).last().cloned().unwrap_or_default();
        eprintln!("after the restart: {answer:?}");
        assert!(answer.to_lowercase().contains("heliotrope"), "{answer}");
        again.close(back);
    }

    #[test]
    #[ignore = "runs the real claude CLI"]
    fn claude_code_resumes_a_kept_conversation_after_a_restart() {
        resumes_after_a_restart(AgentKind::ClaudeCode, "haiku");
    }

    #[test]
    #[ignore = "runs the real codex CLI"]
    fn codex_resumes_a_kept_conversation_after_a_restart() {
        resumes_after_a_restart(AgentKind::Codex, "");
    }

    #[test]
    #[ignore = "runs the real claude CLI"]
    fn claude_code_answers_asks_permission_resumes_and_stops() {
        let dir = TempDir::new("chat-real-claude");
        let host = real_host(&dir.path);
        let m = ChatManager::new(host.clone());
        let mut req = open_req(AgentKind::ClaudeCode, &dir.path);
        req.model = "haiku".into();
        let id = m.open(req).unwrap();

        m.send(id, "Reply with exactly: pong").unwrap();
        wait_for("the first answer", || state(&m, id) == State::Idle && !assistant_texts(&m, id).is_empty());
        assert_eq!(assistant_texts(&m, id)[0].trim(), "pong");
        let convo = m.get(id, None).unwrap();
        let session = convo.session.clone().expect("the CLI's session id");
        eprintln!("session {session} model {:?}", convo.used_model);
        assert!(convo.used_model.as_deref().is_some_and(|m| m.contains("haiku")));
        assert!(m.owns_session(&session));

        m.send(id, "Use the Bash tool to run exactly: cmd /c echo real > real_probe.txt . Then say done in two words.").unwrap();
        wait_for("the permission request", || state(&m, id) == State::Waiting);
        let (_, request_id, tool) = lock(&host.asked)[0].clone();
        assert_eq!(tool, "Bash");
        std::thread::sleep(Duration::from_secs(2));
        assert_eq!(state(&m, id), State::Waiting, "nothing answers without a click");
        assert!(!dir.path.join("real_probe.txt").exists(), "the command did not run before the click");
        assert!(m.answer_permission(&request_id, true));
        wait_for("the turn to finish", || state(&m, id) == State::Idle);
        assert!(dir.path.join("real_probe.txt").exists(), "the command ran after the click");

        m.send(id, "Use the Bash tool to run exactly: cmd /c echo no > denied_probe.txt . Then say done in two words.").unwrap();
        wait_for("the second request", || lock(&host.asked).len() == 2);
        assert!(m.answer_permission(&lock(&host.asked)[1].1.clone(), false));
        wait_for("the turn to finish", || state(&m, id) == State::Idle);
        assert!(!dir.path.join("denied_probe.txt").exists(), "a denial keeps the command from running");

        m.send(id, "Count from 1 to 400, one number per line, no other text.").unwrap();
        wait_for("some output", || assistant_texts(&m, id).last().is_some_and(|t| t.len() > 20));
        m.cancel(id).unwrap();
        wait_for("the stop", || state(&m, id) == State::Idle);
        let after_stop = m.get(id, None).unwrap();
        assert!(after_stop.msgs.iter().any(|x| matches!(&x.body, convo::Body::Notice { code, .. } if code == "stopped")));

        m.send(id, "What was the first command I asked you to run? Answer in one short sentence.").unwrap();
        wait_for("the follow-up", || state(&m, id) == State::Idle && assistant_texts(&m, id).last().is_some_and(|t| t.contains("real_probe")));
        assert_eq!(m.get(id, None).unwrap().session.as_deref(), Some(session.as_str()), "one conversation throughout");
        m.close(id);
    }

    #[test]
    #[ignore = "runs the real claude CLI"]
    fn claude_code_takes_what_the_mcp_server_hands_over() {
        let dir = TempDir::new("chat-real-mcp");
        let host = real_host(&dir.path);
        let m = ChatManager::new(host);
        // The shape of mcp_server/attach.rs for Claude Code; nothing listens on that port, so the
        // server shows as failed in the CLI, which must still answer.
        let config = serde_json::json!({ "mcpServers": { "mayak": {
            "type": "http", "url": "http://127.0.0.1:9/mcp",
            "headers": { "Authorization": "Bearer ${MAYAK_MCP_TOKEN}" }, "timeout": 270000
        } } })
        .to_string();
        m.set_mcp_provider(Some(Arc::new(move |kind| {
            (kind == AgentKind::ClaudeCode)
                .then(|| McpAttach { args: vec!["--mcp-config".into(), config.clone()], env: vec![("MAYAK_MCP_TOKEN".into(), "not-a-real-token".into())] })
        })));
        let mut req = open_req(AgentKind::ClaudeCode, &dir.path);
        req.model = "haiku".into();
        let id = m.open(req).unwrap();
        m.send(id, "Reply with exactly: pong").unwrap();
        // With a server passed in --mcp-config Claude Code's first output was often ~20 s here (2.3-3.1 s
        // without one): the Claude debug log shows the slow part is an account connector (claude.ai
        // Webflow, ~16 s), not the server, which connects in ~45 ms. Hence the patient wait.
        wait_for_secs(60, "the answer", || state(&m, id) == State::Idle && !assistant_texts(&m, id).is_empty());
        assert_eq!(assistant_texts(&m, id)[0].trim(), "pong");
        m.close(id);
    }

    #[test]
    #[ignore = "runs the real codex CLI"]
    fn codex_answers_and_resumes_its_thread() {
        let dir = TempDir::new("chat-real-codex");
        let host = real_host(&dir.path);
        let m = ChatManager::new(host);
        let id = m.open(open_req(AgentKind::Codex, &dir.path)).unwrap();
        m.send(id, "Reply with exactly: pong").unwrap();
        turn_done(&m, id, "the first answer");
        assert_eq!(assistant_texts(&m, id)[0].trim(), "pong");
        let thread = m.get(id, None).unwrap().session.expect("the thread id");
        m.send(id, "What did I ask you to reply with before? One short sentence.").unwrap();
        turn_done(&m, id, "the second answer");
        assert!(assistant_texts(&m, id)[1].to_lowercase().contains("pong"), "{:?}", assistant_texts(&m, id));
        assert_eq!(m.get(id, None).unwrap().session.as_deref(), Some(thread.as_str()));
        m.close(id);
    }

    #[test]
    #[ignore = "runs the real claude and codex CLIs"]
    fn the_picker_reads_login_and_models_from_the_real_clis() {
        // The CLIs themselves: the wrapper of `real_host` adds a flag that `auth status` does not take.
        let host = Arc::new(FakeHost::default());
        for stem in ["claude", "codex"] {
            lock(&host.clis).insert(stem.into(), crate::platform::find_on_path(stem).expect("on PATH"));
        }
        let m = ChatManager::new(host);
        let agents = m.agents(true);
        let claude = agents.iter().find(|a| a.kind == AgentKind::ClaudeCode).unwrap();
        let codex = agents.iter().find(|a| a.kind == AgentKind::Codex).unwrap();
        eprintln!("claude {:?}\ncodex {:?} models {}", claude.login, codex.login, codex.models.len());
        assert!(claude.found && claude.login == Login::Ready);
        assert!(codex.found && codex.login == Login::Ready);
        assert!(codex.models.len() > 3, "the catalog came from `codex debug models`");
        assert!(agents.iter().filter(|a| a.kind == AgentKind::GeminiCli || a.kind == AgentKind::CursorAgent).all(|a| !a.found && !a.verified));
    }
}

#[test]
fn a_run_is_claimed_for_a_turn_only_while_it_is_not_ending() {
    fn run() -> Run {
        Run {
            id: 1,
            group: Arc::new(runner::ProcGroup::new().unwrap()),
            stdin: Mutex::new(None),
            cancel: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            mcp: None,
            state: Mutex::new(RunState::default()),
            cv: Condvar::new(),
        }
    }
    let live = run();
    assert!(live.usable() && live.claim_turn());
    assert!(lock(&live.state).turn_open, "claiming opens the turn");
    live.set_turn(false);
    assert!(!lock(&live.state).turn_open && lock(&live.state).last_turn_end.is_some());
    // Once it is retired (the idle watcher, a change of model) no message is given to it.
    live.retire();
    assert!(!live.usable() && !live.claim_turn());
    let ended = run();
    ended.mark_ended();
    assert!(!ended.usable() && !ended.claim_turn());
    assert!(ended.write(b"x").is_err(), "no stdin to write to");
}

/// The shapes the island's TypeScript reads and sends (src/core/agent-chat.ts): a rename here
/// breaks the page without a compile error on either side, so the JSON is pinned.
#[test]
fn the_payloads_have_the_shapes_the_page_expects() {
    let info = agent_info(AgentKind::GeminiCli, true, Login::NotLoggedIn, None);
    let v = serde_json::to_value(&info).unwrap();
    assert_eq!(v["kind"], "geminiCli");
    assert_eq!((v["label"].as_str(), v["found"].as_bool(), v["verified"].as_bool()), (Some("Gemini CLI"), Some(true), Some(false)));
    assert_eq!(v["verifiedWith"], serde_json::Value::Null);
    assert_eq!(v["login"], "notLoggedIn");
    assert_eq!(v["modes"], serde_json::json!(["ask", "plan"]));
    assert_eq!(v["models"][0], serde_json::json!({ "id": "", "label": "" }));
    let claude = serde_json::to_value(agent_info(AgentKind::ClaudeCode, true, Login::Ready, None)).unwrap();
    assert_eq!((claude["verified"].as_bool(), claude["verifiedWith"].as_str()), (Some(true), Some("2.1.296")));

    let summary = Summary { id: 3, agent: AgentKind::CursorAgent, cwd: "p".into(), title: "t".into(), state: State::Waiting, rev: 9 };
    assert_eq!(
        serde_json::to_value(&summary).unwrap(),
        serde_json::json!({ "id": 3, "agent": "cursorAgent", "cwd": "p", "title": "t", "state": "waiting", "rev": 9 })
    );
    assert_eq!(
        serde_json::to_value(ChatError::new("busy", "x")).unwrap(),
        serde_json::json!({ "code": "busy", "message": "x" })
    );

    // What the page sends to open a chat (agent-chat.ts OpenRequest), with and without the optional parts.
    let req: OpenRequest = serde_json::from_value(serde_json::json!({
        "agent": "codex", "cwd": "", "model": "", "mode": "readOnly", "resume": null
    }))
    .unwrap();
    assert_eq!((req.agent, req.cwd.as_str(), req.mode.as_str(), req.resume), (AgentKind::Codex, "", "readOnly", None));
    let bare: OpenRequest = serde_json::from_value(serde_json::json!({ "agent": "claudeCode", "cwd": "C:/x" })).unwrap();
    assert_eq!((bare.model.as_str(), bare.mode.as_str()), ("", ""));
    assert!(serde_json::from_value::<OpenRequest>(serde_json::json!({ "agent": "bash", "cwd": "" })).is_err(), "only the four CLIs");
}
