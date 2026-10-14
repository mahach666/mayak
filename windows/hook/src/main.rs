//! mayak-hook — the relay Claude Code, Codex and Antigravity run on hook events.
//!
//! Reads the hook JSON on stdin, adds what Mayak needs, and hands it to Mayak
//! over `\\.\pipe\mayak-<sid>`. The logic lives in `lib.rs`; this file only
//! wires argv, stdin and stdout to it.
//!
//! Hard rule: **never block the agent.** Mayak closed → exit 0 at once with
//! nothing on stdout. Only a permission request (Claude or Codex) and a Claude
//! `AskUserQuestion` wait for an answer; a release or a timeout prints nothing.
//! A foreign `--agent` value, a Claude `PermissionRequest` that is not a real
//! permission request, and a Claude `PreToolUse` that is not a question send
//! nothing.
//!
//! Antigravity is only observed: each event it was installed for gets `{}` on
//! stdout, Mayak running or not, and nothing ever waits (`antigravity.rs`).
//!
//! Usage: `mayak-hook [--agent <name>] [<EventName>]` — `claude` (also an empty
//! or missing name), `codex` or `antigravity`; any other name sends nothing.
//! Debug builds also take `--dump` (spike S2). `mayak-hook --version` prints
//! `mayak-hook <RELAY_VERSION>` and exits without reading stdin.

#[allow(dead_code)]
#[path = "../../ident.rs"]
mod ident;

use std::io::{Read, Write};
use std::time::Instant;

use mayak_hook::{self as relay, antigravity, os, Agent, Enrichment, RawInput, Reply};

fn main() {
    // The budget is a deadline from here: stdin, the ancestor walk and the
    // enrichment all count against it.
    let started = Instant::now();
    let args = relay::parse_args(std::env::args().skip(1));
    // `--version` is only ever passed by Mayak's debug build, never by an agent.
    if args.version {
        exit_with(Some(relay::version_line().as_str()));
    }
    // Antigravity's answer does not depend on Mayak: printed on every way out.
    let fixed = match args.agent {
        Some(Agent::Antigravity) => antigravity::stdout(args.event.as_deref().unwrap_or_default()),
        _ => None,
    };

    let mut raw = Vec::new();
    if std::io::stdin().read_to_end(&mut raw).is_err() || raw.is_empty() {
        exit_with(fixed);
    }
    // A foreign `--agent` value (D86): stdin is drained, so the agent's write
    // never fails; nothing is sent and nothing is printed.
    let Some(arg_agent) = args.agent else { std::process::exit(0) };
    let body = relay::strip_boms(&raw);

    #[cfg(debug_assertions)]
    if args.dump {
        dump(&raw);
    }

    let Ok(serde_json::Value::Object(mut payload)) = serde_json::from_slice(body) else {
        exit_with(fixed);
    };
    if arg_agent == Agent::Antigravity {
        // The event is argv's: Antigravity's payload does not name it.
        let event = args.event.as_deref().unwrap_or_default();
        let proc = os::find_antigravity_ancestor(std::process::id());
        if let Some(mut line) = antigravity::payload(&payload, event, relay::unix_ms_now(), proc.as_ref()) {
            relay::add_ao(&mut line, &relay::ao_context(|name| std::env::var(name).ok()));
            line.values_mut().for_each(relay::truncate_strings);
            let remaining = relay::budget(Agent::Antigravity, false).saturating_sub(started.elapsed());
            let _ = relay::talk_to(&os::pipe_path(), &relay::to_line(&line), false, remaining);
        }
        exit_with(fixed);
    }
    let event = relay::resolve_event(&payload, args.event.as_deref());
    let agent = relay::detect_agent(arg_agent, &event, &payload);
    // Only real Claude Code permission requests (D87) and, of all PreToolUse
    // events, only questions: anything else → Claude asks in its own UI.
    if !relay::relayed(agent, &event, &payload) {
        std::process::exit(0);
    }
    let waits = relay::waits_for_answer(agent, &event, &payload);
    // The replies echo these as stdin had them, before the enrichment cuts strings.
    let raw_input = if waits { RawInput::from_stdin(body) } else { RawInput::default() };
    let codex_proc = match agent {
        Agent::Codex => os::find_codex_ancestor(std::process::id()),
        Agent::Claude | Agent::Antigravity => None,
    };
    let claude_pid = match agent {
        Agent::Claude => std::env::var("CLAUDE_PID").ok().and_then(|v| v.trim().parse().ok()),
        Agent::Codex | Agent::Antigravity => None,
    };
    let enrichment = Enrichment {
        agent,
        event: event.clone(),
        relay_ts: relay::unix_ms_now(),
        cwd_fallback: std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()),
        term: relay::TERM_CONTEXT
            .iter()
            .map(|(key, var)| (*key, std::env::var(var).unwrap_or_default()))
            .collect(),
        ao: relay::ao_context(|name| std::env::var(name).ok()),
        claude_pid,
        codex_proc,
    };
    relay::enrich(&mut payload, &enrichment);
    payload.values_mut().for_each(relay::truncate_strings);

    let remaining = relay::budget(agent, waits).saturating_sub(started.elapsed());
    let reply = relay::talk_to(&os::pipe_path(), &relay::to_line(&payload), waits, remaining);
    let reply = reply.as_deref().map_or(Reply::Release, relay::parse_reply);

    // Only a waiting event ever prints: everything else leaves stdout empty.
    let printed = if waits { relay::reply_json(&event, agent, &reply, &raw_input) } else { None };
    if let Some(json) = printed {
        let mut out = std::io::stdout();
        let _ = writeln!(out, "{json}");
        let _ = out.flush();
    }
    // Nothing printed: the agent asks in its own UI, as if Mayak were not here.
    std::process::exit(0);
}

/// Prints `out` (if any) as one line on stdout and exits 0.
fn exit_with(out: Option<&str>) -> ! {
    if let Some(text) = out {
        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "{text}");
        let _ = stdout.flush();
    }
    std::process::exit(0);
}

/// Spike S2: one measurement line in `%LOCALAPPDATA%\Mayak\dump.log`.
#[cfg(debug_assertions)]
fn dump(stdin: &[u8]) {
    use mayak_hook::dump;

    let Some(base) = std::env::var_os("LOCALAPPDATA") else {
        eprintln!("mayak-hook: --dump needs LOCALAPPDATA");
        return;
    };
    let path = std::path::PathBuf::from(base).join(ident::APP_DIR).join("dump.log");
    let pid = std::process::id();
    let chain: Vec<String> =
        os::ancestors(&os::process_table(), os::creation_ft, pid, relay::MAX_ANCESTOR_STEPS)
            .into_iter()
            .map(|(row, _)| row.exe_name)
            .collect();
    let now = relay::unix_ms_now();
    // FILETIME (100 ns since 1601) → Unix ms; -1 when the creation time is unreadable.
    let age = os::creation_ft(pid)
        .map(|ft| now - ((ft / 10_000) as i64 - 11_644_473_600_000))
        .unwrap_or(-1);
    if let Err(err) = dump::append(&path, &dump::dump_line(stdin, age, &chain, now)) {
        eprintln!("mayak-hook: dump failed: {err}");
    }
}
