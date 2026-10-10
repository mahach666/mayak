// Table tests of the session model (spec §12 "model.rs"). Child module of `model`, so tests may
// build `Session`s directly when a scenario is about snapshot rules rather than transitions.

use super::*;
use serde_json::json;

// ── pure helpers ──

#[test]
fn canonical_json_sorts_keys_recursively_and_is_compact() {
    let v = json!({ "b": 1, "a": { "d": [{ "z": 1, "y": [2, { "q": null, "p": true }] }], "c": "x" } });
    assert_eq!(canonical_json(&v), r#"{"a":{"c":"x","d":[{"y":[2,{"p":true,"q":null}],"z":1}]},"b":1}"#);
}

#[test]
fn canonical_json_escapes_strings_like_serde() {
    let v = json!({ "k": "a\"b\nc", "é": "ü" });
    assert_eq!(canonical_json(&v), "{\"k\":\"a\\\"b\\nc\",\"é\":\"ü\"}");
}

#[test]
fn canonical_json_is_independent_of_insertion_order() {
    let a = json!({ "command": "npm test", "cwd": "C:\\x", "env": { "B": "2", "A": "1" } });
    let b = json!({ "env": { "A": "1", "B": "2" }, "cwd": "C:\\x", "command": "npm test" });
    assert_eq!(canonical_json(&a), canonical_json(&b));
    assert_eq!(fnv1a64(canonical_json(&a).as_bytes()), fnv1a64(canonical_json(&b).as_bytes()));
}

#[test]
fn fnv1a64_matches_reference_vectors() {
    assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
}

#[test]
fn display_names_table() {
    let pairs = |rows: &[(&str, &str)]| -> Vec<(String, String)> {
        rows.iter().map(|(k, r)| (k.to_string(), r.to_string())).collect()
    };
    // (input projects, expected (key, name) pairs)
    let cases: Vec<(Vec<(String, String)>, Vec<(&str, &str)>)> = vec![
        (pairs(&[(r"c:\work\solo", r"C:\work\solo")]), vec![(r"c:\work\solo", "solo")]),
        (
            pairs(&[(r"c:\work\app", r"C:\work\app"), (r"d:\other\app", r"D:\other\App"), (r"c:\x\solo", r"C:\x\solo")]),
            vec![(r"c:\work\app", r"work\app"), (r"d:\other\app", r"other\App"), (r"c:\x\solo", "solo")],
        ),
        (pairs(&[(r"c:\", r"C:\")]), vec![(r"c:\", "C:")]),
        (
            pairs(&[(r"c:\a\мой-проект", r"C:\a\Мой-проект"), (r"c:\b\мой-проект", r"C:\b\мой-проект")]),
            vec![(r"c:\a\мой-проект", r"a\Мой-проект"), (r"c:\b\мой-проект", r"b\мой-проект")],
        ),
        // the same key listed twice is not a collision
        (pairs(&[(r"c:\w\api", r"C:\w\api"), (r"c:\w\api", r"C:\w\api")]), vec![(r"c:\w\api", "api")]),
    ];
    for (input, expected) in cases {
        let names = display_names(&input);
        for (key, name) in expected {
            assert_eq!(names.get(key).map(String::as_str), Some(name), "input {input:?}");
        }
    }
}

// ── fixtures shared by the store tests ──

/// Store clock used by every `apply` unless a test needs another one.
const NOW: i64 = 9_000;

fn project(name: &str) -> ProjectRef {
    ProjectRef { key: format!(r"c:\users\dev\{}", name.to_lowercase()), root: format!(r"C:\Users\dev\{name}") }
}

/// A session for snapshot-rule tests, inserted directly (no transition involved).
fn put(store: &mut Store, key: &str, project: ProjectRef, phase: Phase, since: i64, outcome: Outcome, acked_at: Option<i64>) {
    let mut s = Session::blank(
        key.to_string(),
        Source::Claude,
        HostKind::ClaudeDesktop,
        None,
        project,
        "s".into(),
        r"C:\Users\dev".into(),
        since,
    );
    s.phase = phase;
    s.outcome = outcome;
    s.acked_at = acked_at;
    store.sessions.insert(key.to_string(), s);
}

fn waiting(kind: WaitKind) -> Phase {
    Phase::Waiting { kind }
}

fn finished(at_ms: i64) -> Outcome {
    Outcome::Finished { summary: "done".into(), at_ms }
}

fn failed(at_ms: i64) -> Outcome {
    Outcome::Error { code: Some("rate_limit".into()), status: Some(429), text: "limit".into(), at_ms }
}

// ── snapshot rules (spec §6, §10) ──

#[test]
fn attention_and_session_order_follow_ranks() {
    let mut st = Store::new();
    put(&mut st, "k-a", project("P"), waiting(WaitKind::Permission), 300, Outcome::None, None);
    put(&mut st, "k-b", project("P"), waiting(WaitKind::Permission), 100, Outcome::None, None);
    put(&mut st, "k-m", project("P"), waiting(WaitKind::Permission), 100, Outcome::None, None); // tie with k-b → key
    put(&mut st, "k-c", project("P"), waiting(WaitKind::Input), 50, Outcome::None, None);
    put(&mut st, "k-d", project("P"), waiting(WaitKind::Other("plan approval".into())), 60, Outcome::None, None);
    put(&mut st, "k-e", project("P"), Phase::Idle, 10, failed(500), None);
    put(&mut st, "k-f", project("P"), Phase::Idle, 10, failed(700), None);
    put(&mut st, "k-g", project("P"), Phase::Idle, 10, finished(800), None);
    put(&mut st, "k-h", project("P"), Phase::Working, 10, Outcome::None, None);
    put(&mut st, "k-i", project("P"), Phase::Working, 5, Outcome::None, None);
    put(&mut st, "k-j", project("P"), Phase::Idle, 900, Outcome::None, None);
    put(&mut st, "k-k", project("P"), Phase::Idle, 950, Outcome::None, None);
    put(&mut st, "k-l", project("P"), waiting(WaitKind::Permission), 100, Outcome::None, Some(100)); // acked → rank 5

    let snap = st.snapshot(NOW);
    assert_eq!(snap.attention, ["k-b", "k-m", "k-a", "k-c", "k-d", "k-f", "k-e", "k-g"]);
    let order: Vec<&str> = snap.projects[0].sessions.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(order, ["k-b", "k-m", "k-a", "k-c", "k-d", "k-f", "k-e", "k-g", "k-i", "k-h", "k-k", "k-j", "k-l"]);
    let attention: Vec<Option<AttentionKind>> = snap.projects[0].sessions.iter().map(|s| s.attention).collect();
    use AttentionKind::*;
    assert_eq!(
        attention,
        [Some(Permission), Some(Permission), Some(Permission), Some(Input), Some(Input), Some(Error), Some(Error),
         Some(Finished), None, None, None, None, None]
    );
}

#[test]
fn projects_sort_by_best_rank_then_latest_activity_then_key() {
    let mut st = Store::new();
    put(&mut st, "a1", project("Alpha"), Phase::Idle, 100, finished(2_000), None); // rank 3, latest 2000
    put(&mut st, "b1", project("Beta"), waiting(WaitKind::Input), 50, Outcome::None, None); // rank 1
    put(&mut st, "b2", project("Beta"), Phase::Working, 10, Outcome::None, None);
    put(&mut st, "c1", project("Gamma"), Phase::Idle, 100, finished(3_000), None); // rank 3, latest 3000
    put(&mut st, "d1", project("Delta"), Phase::Idle, 4_000, Outcome::None, None); // rank 5
    put(&mut st, "e1", project("Epsilon"), Phase::Idle, 100, finished(2_000), None); // ties Alpha → key asc

    let snap = st.snapshot(NOW);
    let names: Vec<&str> = snap.projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Beta", "Gamma", "Alpha", "Epsilon", "Delta"]);
    let projects = st.projects();
    let keys: Vec<&str> = projects.iter().map(|p| p.key.as_str()).collect();
    let snap_keys: Vec<&str> = snap.projects.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, snap_keys);
    assert_eq!(projects[0].sessions, ["b1", "b2"]);
    assert_eq!(projects[0].root, r"C:\Users\dev\Beta");
}

#[test]
fn snapshot_names_colliding_projects_with_their_parent() {
    let mut st = Store::new();
    let work_app = ProjectRef { key: r"c:\work\app".into(), root: r"C:\work\app".into() };
    let other_app = ProjectRef { key: r"d:\other\app".into(), root: r"D:\other\App".into() };
    put(&mut st, "x1", work_app, Phase::Working, 10, Outcome::None, None);
    put(&mut st, "x2", other_app, Phase::Working, 20, Outcome::None, None);
    put(&mut st, "x3", project("Solo"), Phase::Working, 30, Outcome::None, None);

    let snap = st.snapshot(NOW);
    let names: HashMap<&str, &str> = snap.projects.iter().map(|p| (p.key.as_str(), p.name.as_str())).collect();
    assert_eq!(names[r"c:\work\app"], r"work\app");
    assert_eq!(names[r"d:\other\app"], r"other\App");
    assert_eq!(names[r"c:\users\dev\solo"], "Solo");
}

#[test]
fn counts_hold_and_aggregate_use_unacked_episodes_only() {
    struct Case {
        name: &'static str,
        sessions: Vec<(Phase, Outcome, Option<i64>)>,
        counts: Counts,
        hold: bool,
        aggregate: BotState,
    }
    let counts = |waiting, error, finished, working| Counts { waiting, error, finished, working };
    let cases = vec![
        Case { name: "empty", sessions: vec![], counts: counts(0, 0, 0, 0), hold: false, aggregate: BotState::Idle },
        Case {
            name: "unacked permission",
            sessions: vec![(waiting(WaitKind::Permission), Outcome::None, None)],
            counts: counts(1, 0, 0, 0),
            hold: true,
            aggregate: BotState::Approval,
        },
        Case {
            name: "acked permission",
            sessions: vec![(waiting(WaitKind::Permission), Outcome::None, Some(100))],
            counts: counts(0, 0, 0, 0),
            hold: false,
            aggregate: BotState::Idle,
        },
        Case {
            name: "unacked input is a question",
            sessions: vec![(waiting(WaitKind::Input), Outcome::None, None)],
            counts: counts(1, 0, 0, 0),
            hold: true,
            aggregate: BotState::Question,
        },
        Case {
            name: "unacked other wait is a question",
            sessions: vec![(waiting(WaitKind::Other("plan approval".into())), Outcome::None, None)],
            counts: counts(1, 0, 0, 0),
            hold: true,
            aggregate: BotState::Question,
        },
        Case {
            name: "permission beats question",
            sessions: vec![
                (waiting(WaitKind::Input), Outcome::None, None),
                (waiting(WaitKind::Permission), Outcome::None, None),
            ],
            counts: counts(2, 0, 0, 0),
            hold: true,
            aggregate: BotState::Approval,
        },
        Case {
            name: "error beats working",
            sessions: vec![(Phase::Idle, failed(200), None), (Phase::Working, Outcome::None, None)],
            counts: counts(0, 1, 0, 1),
            hold: false,
            aggregate: BotState::Error,
        },
        Case {
            name: "working beats finished",
            sessions: vec![(Phase::Idle, finished(200), None), (Phase::Working, Outcome::None, None)],
            counts: counts(0, 0, 1, 1),
            hold: false,
            aggregate: BotState::Working,
        },
        Case {
            name: "unacked finished",
            sessions: vec![(Phase::Idle, finished(200), None)],
            counts: counts(0, 0, 1, 0),
            hold: false,
            aggregate: BotState::Finished,
        },
        Case {
            name: "acked finished",
            sessions: vec![(Phase::Idle, finished(200), Some(200))],
            counts: counts(0, 0, 0, 0),
            hold: false,
            aggregate: BotState::Idle,
        },
        Case {
            name: "acked error",
            sessions: vec![(Phase::Idle, failed(200), Some(250))],
            counts: counts(0, 0, 0, 0),
            hold: false,
            aggregate: BotState::Idle,
        },
        Case {
            name: "an ack older than the episode does not count",
            sessions: vec![(Phase::Idle, finished(200), Some(150))],
            counts: counts(0, 0, 1, 0),
            hold: false,
            aggregate: BotState::Finished,
        },
        Case {
            name: "working counts even when acked (no episode)",
            sessions: vec![(Phase::Working, Outcome::None, Some(100))],
            counts: counts(0, 0, 0, 1),
            hold: false,
            aggregate: BotState::Working,
        },
        Case {
            name: "interrupted is not an episode",
            sessions: vec![(Phase::Idle, Outcome::Interrupted { at_ms: 200 }, None)],
            counts: counts(0, 0, 0, 0),
            hold: false,
            aggregate: BotState::Idle,
        },
        Case {
            name: "finished outside Idle is not an episode",
            sessions: vec![(Phase::Unknown("compacting".into()), finished(200), None)],
            counts: counts(0, 0, 0, 0),
            hold: false,
            aggregate: BotState::Idle,
        },
    ];
    for case in cases {
        let mut st = Store::new();
        for (i, (phase, outcome, acked)) in case.sessions.into_iter().enumerate() {
            put(&mut st, &format!("s{i}"), project("P"), phase, 100, outcome, acked);
        }
        let snap = st.snapshot(NOW);
        assert_eq!(snap.counts, case.counts, "{}: counts", case.name);
        assert_eq!(snap.hold, case.hold, "{}: hold", case.name);
        assert_eq!(snap.aggregate, case.aggregate, "{}: aggregate", case.name);
    }
}

#[test]
fn hold_needs_alert_waiting_on() {
    let mut st = Store::new();
    put(&mut st, "w", project("Alpha"), waiting(WaitKind::Permission), 100, Outcome::None, None);
    assert!(st.snapshot(NOW).hold);
    st.apply(Signal::AlertPrefs { waiting: false, finished: true, error: true }, NOW);
    let snap = st.snapshot(NOW);
    assert!(!snap.hold);
    assert_eq!(snap.counts.waiting, 1);
    assert_eq!(snap.aggregate, BotState::Approval);
    assert_eq!(snap.attention, ["w"]);
}

#[test]
fn ack_marks_the_current_episode_and_never_changes_the_phase() {
    let mut st = Store::new();
    put(&mut st, "w", project("Alpha"), waiting(WaitKind::Permission), 5_000, Outcome::None, None);
    // store clock behind the episode start: acked_at = max(now, start) (D49)
    st.apply(Signal::Ack("w".into()), 4_000);
    let s = st.session("w").unwrap();
    assert_eq!(s.acked_at, Some(5_000));
    assert_eq!(s.phase, waiting(WaitKind::Permission));
    let snap = st.snapshot(NOW);
    assert!(snap.attention.is_empty());
    assert!(snap.projects[0].sessions[0].acked);
    assert_eq!(snap.projects[0].sessions[0].attention, None);
    assert_eq!(snap.projects[0].sessions[0].phase, PhaseView::Waiting { wait: WaitView::Permission, label: "permission".into() });

    // no episode: acked_at = now
    put(&mut st, "busy", project("Alpha"), Phase::Working, 10, Outcome::None, None);
    st.apply(Signal::Ack("busy".into()), 7_000);
    assert_eq!(st.session("busy").unwrap().acked_at, Some(7_000));
    assert_eq!(st.session("busy").unwrap().phase, Phase::Working);

    // unknown key: nothing at all
    let rev = st.rev();
    assert_eq!(st.apply(Signal::Ack("nope".into()), NOW), Applied::default());
    assert_eq!(st.rev(), rev);
}

#[test]
fn source_health_is_published_and_bumps_rev_on_change_only() {
    let mut st = Store::new();
    assert_eq!(st.rev(), 0);
    assert_eq!(
        st.snapshot(NOW).sources,
        Sources { claude: Health::Ok, codex: Health::Ok, codex_hooks: CodexHooksView::NotInstalled }
    );
    st.apply(Signal::SourceHealth { source: Source::Claude, health: Health::Degraded { unreadable: 2 } }, NOW);
    assert_eq!(st.rev(), 1);
    st.apply(Signal::SourceHealth { source: Source::Codex, health: Health::Missing }, NOW);
    assert_eq!(st.rev(), 2);
    st.apply(Signal::SourceHealth { source: Source::Codex, health: Health::Missing }, NOW);
    assert_eq!(st.rev(), 2);
    let snap = st.snapshot(NOW + 1);
    assert_eq!(snap.sources.claude, Health::Degraded { unreadable: 2 });
    assert_eq!(snap.sources.codex, Health::Missing);
    assert_eq!((snap.rev, snap.now), (2, NOW + 1));
}

#[test]
fn visible_and_unchanged_prefs_change_nothing() {
    let mut st = Store::new();
    assert_eq!(st.apply(Signal::Visible(true), NOW), Applied::default());
    assert_eq!(st.apply(Signal::AlertPrefs { waiting: true, finished: true, error: true }, NOW), Applied::default());
    assert_eq!(st.rev(), 0);
}

#[test]
fn phase_and_outcome_views() {
    let phases = [
        (Phase::Working, PhaseView::Working),
        (waiting(WaitKind::Permission), PhaseView::Waiting { wait: WaitView::Permission, label: "permission".into() }),
        (waiting(WaitKind::Input), PhaseView::Waiting { wait: WaitView::Input, label: "input".into() }),
        (
            waiting(WaitKind::Other("plan approval".into())),
            PhaseView::Waiting { wait: WaitView::Other, label: "plan approval".into() },
        ),
        (Phase::Idle, PhaseView::Idle),
        (Phase::Unknown("compacting".into()), PhaseView::Unknown { label: "compacting".into() }),
    ];
    for (phase, view) in phases {
        assert_eq!(phase_view(&phase), view);
    }
    let outcomes = [
        (Outcome::None, None),
        (Outcome::Pending, None),
        (finished(5), Some(OutcomeView { kind: OutcomeKind::Finished, summary: "done".into(), at: 5 })),
        (failed(6), Some(OutcomeView { kind: OutcomeKind::Error, summary: "rate_limit: limit".into(), at: 6 })),
        (
            Outcome::Error { code: None, status: None, text: "stream disconnected".into(), at_ms: 7 },
            Some(OutcomeView { kind: OutcomeKind::Error, summary: "stream disconnected".into(), at: 7 }),
        ),
        (Outcome::Interrupted { at_ms: 8 }, Some(OutcomeView { kind: OutcomeKind::Interrupted, summary: String::new(), at: 8 })),
    ];
    for (outcome, view) in outcomes {
        assert_eq!(outcome_view(&outcome), view);
    }
}

#[test]
fn snapshot_serializes_to_the_spec_wire_shape() {
    let mut st = Store::new();
    put(&mut st, "claude:42:7", project("Alpha"), waiting(WaitKind::Permission), 2_000, Outcome::None, None);
    {
        let s = st.sessions.get_mut("claude:42:7").unwrap();
        s.title = Some("Fix login".into());
        s.activity = Some("Bash · ls".into());
        s.detail = Some("Bash · rm -rf build".into());
    }
    put(&mut st, "codex:t-1", project("Beta"), Phase::Idle, 1_000, failed(3_000), None);
    {
        let s = st.sessions.get_mut("codex:t-1").unwrap();
        s.agent = Source::Codex;
        s.host = HostKind::Cursor;
    }
    st.apply(Signal::SourceHealth { source: Source::Claude, health: Health::Degraded { unreadable: 1 } }, NOW);

    let v = serde_json::to_value(st.snapshot(NOW)).unwrap();
    assert_eq!(
        v,
        json!({
            "rev": 1,
            "now": 9000,
            "projects": [
                { "key": "c:\\users\\dev\\alpha", "name": "Alpha", "root": "C:\\Users\\dev\\Alpha", "sessions": [
                    { "key": "claude:42:7", "agent": "claude", "host": "claudeDesktop", "title": "Fix login",
                      "phase": { "kind": "waiting", "wait": "permission", "label": "permission" }, "since": 2000,
                      "activity": "Bash · ls", "detail": "Bash · rm -rf build", "outcome": null,
                      "attention": "permission", "acked": false } ] },
                { "key": "c:\\users\\dev\\beta", "name": "Beta", "root": "C:\\Users\\dev\\Beta", "sessions": [
                    { "key": "codex:t-1", "agent": "codex", "host": "cursor", "title": null,
                      "phase": { "kind": "idle" }, "since": 1000, "activity": null, "detail": null,
                      "outcome": { "kind": "error", "summary": "rate_limit: limit", "at": 3000 },
                      "attention": "error", "acked": false } ] }
            ],
            "attention": ["claude:42:7", "codex:t-1"],
            "counts": { "waiting": 1, "error": 1, "finished": 0, "working": 0 },
            "hold": true,
            "aggregate": "approval",
            "sources": { "claude": { "degraded": { "unreadable": 1 } }, "codex": "ok", "codexHooks": "not_installed" },
            "usage": { "claude": null, "codex": null, "openrouter": null }
        })
    );
}

#[test]
fn health_and_codex_hooks_serialize_externally_tagged() {
    fn wire<T: Serialize>(v: T) -> Value {
        serde_json::to_value(v).unwrap()
    }
    assert_eq!(wire(Health::Ok), json!("ok"));
    assert_eq!(wire(Health::Missing), json!("missing"));
    let denied = crate::message::Message::new("msg.health.threadFailed", "thread failed to start: access denied")
        .with("err", "access denied");
    assert_eq!(
        wire(Health::Error(denied.clone())),
        json!({ "error": {
            "code": "msg.health.threadFailed",
            "params": { "err": "access denied" },
            "fallback": "thread failed to start: access denied"
        } })
    );
    assert_eq!(Health::Error(denied).log_label(), "Error(msg.health.threadFailed)", "no params in the log");
    assert_eq!(Health::Degraded { unreadable: 2 }.log_label(), "Degraded { unreadable: 2 }");
    assert_eq!(wire(Health::Degraded { unreadable: 3 }), json!({ "degraded": { "unreadable": 3 } }));
    assert_eq!(wire(CodexHooksView::NotInstalled), json!("not_installed"));
    assert_eq!(wire(CodexHooksView::InstalledUnseen), json!("installed_unseen"));
    assert_eq!(
        wire(CodexHooksView::Receiving { hosts: vec![HostKind::Cursor, HostKind::CodexDesktop], since: 5 }),
        json!({ "receiving": { "hosts": ["cursor", "codexDesktop"], "since": 5 } })
    );
    assert_eq!(wire(BotState::Question), json!("question"));
    assert_eq!(wire(AttentionKind::Finished), json!("finished"));
    assert_eq!(wire(Source::Codex), json!("codex"));
}

// ── Claude fixtures ──

const PID: u32 = 4242;
const SID: &str = "0f8b1d2e-0000-4000-8000-000000000001";

fn claude_proc(pid: u32) -> ProcId {
    ProcId {
        pid,
        created_ft: 134_000_000_000_000_000 + u64::from(pid),
        image: r"C:\Users\dev\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\claude-code\2.1.286\claude.exe".into(),
    }
}

fn ckey(pid: u32) -> SessionKey {
    claude_key(pid, claude_proc(pid).created_ft)
}

fn key() -> SessionKey {
    ckey(PID)
}

/// A session file as claude-io parses it (status, waitingFor, statusUpdatedAt vary).
fn sfile(status: &str, waiting_for: Option<&str>, ts: i64) -> ClaudeSessionFile {
    ClaudeSessionFile {
        pid: Some(PID),
        session_id: Some(SID.into()),
        cwd: Some(r"C:\Users\dev\Alpha\src".into()),
        proc_start: Some(claude_proc(PID).created_ft.to_string()),
        version: Some("2.1.286".into()),
        kind: Some("interactive".into()),
        entrypoint: Some("claude-desktop".into()),
        name: None,
        name_source: None,
        status: Some(status.into()),
        waiting_for: waiting_for.map(Into::into),
        status_updated_at: Some(ts),
        updated_at: Some(ts),
        spare: None,
        parked_job_id: None,
        pid_domain: Some(format!("win32:{PID}")),
        host_session_id: None,
    }
}

fn cfile_at(pid: u32, project_ref: ProjectRef, file: ClaudeSessionFile, transcript_eof: Option<u64>) -> Signal {
    Signal::ClaudeFile {
        key: ckey(pid),
        proc: claude_proc(pid),
        host: HostKind::ClaudeDesktop,
        project: project_ref,
        file,
        transcript_eof,
    }
}

fn cfile(file: ClaudeSessionFile, transcript_eof: Option<u64>) -> Signal {
    cfile_at(PID, project("Alpha"), file, transcript_eof)
}

fn ctr(rebind_at: Option<u64>, records: Vec<(u64, Value)>, eof: u64) -> Signal {
    Signal::ClaudeTranscript { key: key(), rebind_at, records, eof }
}

/// Applies the signals in order at `NOW` and collects alerts and effects.
fn run(st: &mut Store, signals: Vec<Signal>) -> Applied {
    let mut all = Applied::default();
    for s in signals {
        let a = st.apply(s, NOW);
        all.alerts.extend(a.alerts);
        all.effects.extend(a.effects);
        all.wake |= a.wake;
    }
    all
}

fn kinds(a: &Applied) -> Vec<AttentionKind> {
    a.alerts.iter().map(|x| x.kind).collect()
}

// Claude Code transcript records (JSONL shape).
fn user(text: &str) -> Value {
    json!({ "type": "user", "isSidechain": false, "message": { "role": "user", "content": text } })
}
fn result(tool_use_id: &str) -> Value {
    json!({ "type": "user", "isSidechain": false, "message": { "role": "user",
        "content": [{ "type": "tool_result", "tool_use_id": tool_use_id, "content": "ok" }] } })
}
fn tool(id: &str, name: &str, input: Value) -> Value {
    json!({ "type": "assistant", "isSidechain": false, "message": { "role": "assistant",
        "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }], "stop_reason": "tool_use" } })
}
fn reply(text: &str, stop_reason: &str) -> Value {
    json!({ "type": "assistant", "isSidechain": false, "message": { "role": "assistant",
        "content": [{ "type": "text", "text": text }], "stop_reason": stop_reason } })
}
fn api_error(code: &str, status: u16, text: &str) -> Value {
    json!({ "type": "assistant", "isSidechain": false, "isApiErrorMessage": true, "error": code, "apiErrorStatus": status,
        "message": { "role": "assistant", "content": [{ "type": "text", "text": text }], "stop_reason": "stop_sequence" } })
}
fn interrupted() -> Value {
    json!({ "type": "user", "isSidechain": false, "message": { "role": "user",
        "content": [{ "type": "text", "text": "[Request interrupted by user]" }] } })
}
fn title_record(kind: &str, field: &str, text: &str) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("type".into(), kind.into());
    m.insert(field.into(), text.into());
    Value::Object(m)
}

/// The state a table row starts from.
#[derive(Debug, Clone, Copy)]
enum Start {
    Fresh,
    Working,
    Waiting,
    Idle,
    Unknown,
}

fn prelude(start: Start) -> Vec<Signal> {
    match start {
        Start::Fresh => vec![],
        // Working since 1000; transcript bound at 0; the turn already ended with "Build finished."
        Start::Working => vec![
            cfile(sfile("busy", None, 1_000), None),
            ctr(
                Some(0),
                vec![
                    (0, user("build it")),
                    (100, tool("t1", "Bash", json!({ "command": "cargo build" }))),
                    (200, result("t1")),
                    (300, reply("Build finished.", "end_turn")),
                ],
                400,
            ),
        ],
        // Waiting(Permission) since 2000 on an unresolved `rm -rf build`
        Start::Waiting => vec![
            cfile(sfile("busy", None, 1_000), None),
            ctr(Some(0), vec![(0, user("clean up")), (100, tool("t1", "Bash", json!({ "command": "rm -rf build" })))], 200),
            cfile(sfile("waiting", Some("permission prompt"), 2_000), Some(200)),
        ],
        // Idle since 1000; existing transcript bound at its EOF 200 (history not replayed)
        Start::Idle => vec![
            cfile(sfile("idle", None, 1_000), None),
            ctr(Some(200), vec![(0, user("old question")), (100, reply("Old answer.", "end_turn"))], 200),
        ],
        Start::Unknown => vec![
            cfile(sfile("compacting", None, 1_000), None),
            ctr(Some(200), vec![(0, user("old question")), (100, reply("Old answer.", "end_turn"))], 200),
        ],
    }
}

struct Row {
    cell: &'static str,
    start: Start,
    step: Vec<Signal>,
    phase: Phase,
    since: i64,
    alerts: Vec<AttentionKind>,
    outcome: Outcome,
    /// (turn_start_offset, idle_eof)
    offsets: (u64, u64),
    detail: Option<&'static str>,
}

fn done(summary: &str) -> Outcome {
    Outcome::Finished { summary: summary.into(), at_ms: NOW }
}

/// Every cell of the spec §7 Claude table (rows: no session, Working, Waiting, Idle, Unknown;
/// columns: busy, waiting, idle, unknown status).
fn claude_rows() -> Vec<Row> {
    use AttentionKind::{Finished, Input, Permission};
    let ask = json!({ "questions": [{ "question": "Which database?" }] });
    vec![
        // ── (no session): first appearance, no alerts ──
        Row { cell: "(none) → busy", start: Start::Fresh, step: vec![cfile(sfile("busy", None, 1_000), None)],
              phase: Phase::Working, since: 1_000, alerts: vec![], outcome: Outcome::None, offsets: (0, 0), detail: None },
        Row { cell: "(none) → waiting permission", start: Start::Fresh,
              step: vec![cfile(sfile("waiting", Some("permission prompt"), 1_000), None)],
              phase: waiting(WaitKind::Permission), since: 1_000, alerts: vec![], outcome: Outcome::None, offsets: (0, 0), detail: None },
        Row { cell: "(none) → waiting input", start: Start::Fresh,
              step: vec![cfile(sfile("waiting", Some("input needed"), 1_000), None)],
              phase: waiting(WaitKind::Input), since: 1_000, alerts: vec![], outcome: Outcome::None, offsets: (0, 0), detail: None },
        Row { cell: "(none) → waiting other", start: Start::Fresh,
              step: vec![cfile(sfile("waiting", Some("plan approval"), 1_000), None)],
              phase: waiting(WaitKind::Other("plan approval".into())), since: 1_000, alerts: vec![], outcome: Outcome::None,
              offsets: (0, 0), detail: None },
        Row { cell: "(none) → idle", start: Start::Fresh, step: vec![cfile(sfile("idle", None, 1_000), Some(500))],
              phase: Phase::Idle, since: 1_000, alerts: vec![], outcome: Outcome::None, offsets: (500, 500), detail: None },
        Row { cell: "(none) → unknown", start: Start::Fresh, step: vec![cfile(sfile("compacting", None, 1_000), None)],
              phase: Phase::Unknown("compacting".into()), since: 1_000, alerts: vec![], outcome: Outcome::None, offsets: (0, 0),
              detail: None },
        // ── Working ──
        Row { cell: "working → busy (activity/detail follow the transcript)", start: Start::Working,
              step: vec![ctr(None, vec![(400, tool("t2", "Read", json!({ "file_path": "src/main.rs" })))], 500),
                         cfile(sfile("busy", None, 1_000), None)],
              phase: Phase::Working, since: 1_000, alerts: vec![], outcome: Outcome::None, offsets: (0, 0),
              detail: Some("Read · src/main.rs") },
        Row { cell: "working → waiting permission", start: Start::Working,
              step: vec![ctr(None, vec![(400, tool("t2", "Bash", json!({ "command": "git push" })))], 500),
                         cfile(sfile("waiting", Some("permission prompt"), 2_000), Some(500))],
              phase: waiting(WaitKind::Permission), since: 2_000, alerts: vec![Permission], outcome: Outcome::None, offsets: (0, 0),
              detail: Some("Bash · git push") },
        Row { cell: "working → waiting input (AskUserQuestion)", start: Start::Working,
              step: vec![ctr(None, vec![(400, tool("t2", "AskUserQuestion", ask.clone()))], 500),
                         cfile(sfile("waiting", Some("input needed"), 2_000), Some(500))],
              phase: waiting(WaitKind::Input), since: 2_000, alerts: vec![Input], outcome: Outcome::None, offsets: (0, 0),
              detail: Some("Which database?") },
        Row { cell: "working → waiting other", start: Start::Working,
              step: vec![cfile(sfile("waiting", Some("plan approval"), 2_000), Some(400))],
              phase: waiting(WaitKind::Other("plan approval".into())), since: 2_000, alerts: vec![Input], outcome: Outcome::None,
              offsets: (0, 0), detail: None },
        Row { cell: "working → idle (Pending → Finished, idle_eof = transcript_eof)", start: Start::Working,
              step: vec![cfile(sfile("idle", None, 2_000), Some(400))],
              phase: Phase::Idle, since: 2_000, alerts: vec![Finished], outcome: done("Build finished."), offsets: (0, 400),
              detail: None },
        Row { cell: "working → unknown", start: Start::Working, step: vec![cfile(sfile("compacting", None, 2_000), Some(400))],
              phase: Phase::Unknown("compacting".into()), since: 2_000, alerts: vec![], outcome: Outcome::None, offsets: (0, 0),
              detail: None },
        // ── Waiting ──
        Row { cell: "waiting → busy", start: Start::Waiting,
              step: vec![ctr(None, vec![(200, result("t1"))], 300), cfile(sfile("busy", None, 3_000), Some(300))],
              phase: Phase::Working, since: 3_000, alerts: vec![], outcome: Outcome::None, offsets: (0, 0), detail: None },
        Row { cell: "waiting → waiting, new statusUpdatedAt = new episode", start: Start::Waiting,
              step: vec![cfile(sfile("waiting", Some("permission prompt"), 3_000), Some(200))],
              phase: waiting(WaitKind::Permission), since: 3_000, alerts: vec![Permission], outcome: Outcome::None, offsets: (0, 0),
              detail: Some("Bash · rm -rf build") },
        Row { cell: "waiting → waiting, new waitingFor, same statusUpdatedAt = new episode (D19)", start: Start::Waiting,
              step: vec![cfile(sfile("waiting", Some("input needed"), 2_000), Some(200))],
              phase: waiting(WaitKind::Input), since: 2_001, alerts: vec![Input], outcome: Outcome::None, offsets: (0, 0),
              detail: Some("Bash · rm -rf build") },
        Row { cell: "waiting → waiting, same episode: detail only", start: Start::Waiting,
              step: vec![ctr(None, vec![(200, tool("t2", "Bash", json!({ "command": "make install" })))], 300),
                         cfile(sfile("waiting", Some("permission prompt"), 2_000), None)],
              phase: waiting(WaitKind::Permission), since: 2_000, alerts: vec![], outcome: Outcome::None, offsets: (0, 0),
              detail: Some("Bash · make install") },
        Row { cell: "waiting → idle", start: Start::Waiting,
              step: vec![ctr(None, vec![(200, result("t1")), (300, reply("Removed the build folder.", "end_turn"))], 400),
                         cfile(sfile("idle", None, 3_000), Some(400))],
              phase: Phase::Idle, since: 3_000, alerts: vec![Finished], outcome: done("Removed the build folder."),
              offsets: (0, 400), detail: None },
        Row { cell: "waiting → unknown", start: Start::Waiting, step: vec![cfile(sfile("compacting", None, 3_000), Some(200))],
              phase: Phase::Unknown("compacting".into()), since: 3_000, alerts: vec![], outcome: Outcome::None, offsets: (0, 0),
              detail: Some("Bash · rm -rf build") },
        // ── Idle ──
        Row { cell: "idle → busy (turn starts at idle_eof)", start: Start::Idle,
              step: vec![ctr(None, vec![(200, user("next task"))], 300), cfile(sfile("busy", None, 3_000), Some(300))],
              phase: Phase::Working, since: 3_000, alerts: vec![], outcome: Outcome::None, offsets: (200, 200), detail: None },
        Row { cell: "idle → busy with an older statusUpdatedAt keeps since", start: Start::Idle,
              step: vec![cfile(sfile("busy", None, 500), None)],
              phase: Phase::Working, since: 1_000, alerts: vec![], outcome: Outcome::None, offsets: (200, 200), detail: None },
        Row { cell: "idle → waiting", start: Start::Idle,
              step: vec![ctr(None, vec![(200, user("pick a db")), (300, tool("t5", "AskUserQuestion", ask.clone()))], 400),
                         cfile(sfile("waiting", Some("input needed"), 3_000), Some(400))],
              phase: waiting(WaitKind::Input), since: 3_000, alerts: vec![Input], outcome: Outcome::None, offsets: (200, 200),
              detail: Some("Which database?") },
        Row { cell: "idle → idle, new statusUpdatedAt = collapsed turn", start: Start::Idle,
              step: vec![ctr(None, vec![(200, user("quick one")), (300, reply("Quick answer.", "end_turn"))], 400),
                         cfile(sfile("idle", None, 3_000), Some(400))],
              phase: Phase::Idle, since: 3_000, alerts: vec![Finished], outcome: done("Quick answer."), offsets: (200, 400),
              detail: None },
        Row { cell: "idle → idle, same statusUpdatedAt: nothing", start: Start::Idle,
              step: vec![cfile(sfile("idle", None, 1_000), None)],
              phase: Phase::Idle, since: 1_000, alerts: vec![], outcome: Outcome::None, offsets: (200, 200), detail: None },
        Row { cell: "idle → unknown", start: Start::Idle, step: vec![cfile(sfile("compacting", None, 3_000), None)],
              phase: Phase::Unknown("compacting".into()), since: 3_000, alerts: vec![], outcome: Outcome::None, offsets: (200, 200),
              detail: None },
        // ── Unknown ──
        Row { cell: "unknown → busy", start: Start::Unknown,
              step: vec![ctr(None, vec![(200, user("next task"))], 300), cfile(sfile("busy", None, 3_000), Some(300))],
              phase: Phase::Working, since: 3_000, alerts: vec![], outcome: Outcome::None, offsets: (200, 200), detail: None },
        Row { cell: "unknown → waiting", start: Start::Unknown,
              step: vec![cfile(sfile("waiting", Some("permission prompt"), 3_000), Some(200))],
              phase: waiting(WaitKind::Permission), since: 3_000, alerts: vec![Permission], outcome: Outcome::None,
              offsets: (200, 200), detail: None },
        Row { cell: "unknown → idle, new statusUpdatedAt = collapsed turn", start: Start::Unknown,
              step: vec![ctr(None, vec![(200, user("quick one")), (300, reply("Quick answer.", "end_turn"))], 400),
                         cfile(sfile("idle", None, 3_000), Some(400))],
              phase: Phase::Idle, since: 3_000, alerts: vec![Finished], outcome: done("Quick answer."), offsets: (200, 400),
              detail: None },
        Row { cell: "unknown → idle, same statusUpdatedAt: Idle, no collapsed turn (status is the phase source)",
              start: Start::Unknown, step: vec![cfile(sfile("idle", None, 1_000), None)],
              phase: Phase::Idle, since: 1_000, alerts: vec![], outcome: Outcome::None,
              offsets: (200, 200), detail: None },
        Row { cell: "unknown → another unknown status", start: Start::Unknown,
              step: vec![cfile(sfile("restarting", None, 3_000), None)],
              phase: Phase::Unknown("restarting".into()), since: 3_000, alerts: vec![], outcome: Outcome::None,
              offsets: (200, 200), detail: None },
        Row { cell: "unknown → the same unknown status: nothing", start: Start::Unknown,
              step: vec![cfile(sfile("compacting", None, 3_000), None)],
              phase: Phase::Unknown("compacting".into()), since: 1_000, alerts: vec![], outcome: Outcome::None,
              offsets: (200, 200), detail: None },
    ]
}

#[test]
fn claude_transition_table() {
    for row in claude_rows() {
        let mut st = Store::new();
        run(&mut st, prelude(row.start));
        let applied = run(&mut st, row.step);
        let s = st.session(&key()).unwrap_or_else(|| panic!("{}: session missing", row.cell));
        assert_eq!(s.phase, row.phase, "{}: phase", row.cell);
        assert_eq!(s.since_ms, row.since, "{}: since", row.cell);
        assert_eq!(kinds(&applied), row.alerts, "{}: alerts", row.cell);
        assert_eq!(s.outcome, row.outcome, "{}: outcome", row.cell);
        assert_eq!((s.turn_start_offset, s.idle_eof), row.offsets, "{}: offsets", row.cell);
        assert_eq!(s.detail.as_deref(), row.detail, "{}: detail", row.cell);
    }
}

#[test]
fn claude_outcome_table() {
    // (case, records of the turn from offset 0, outcome, alerts) — spec §7 "Вычисление итога"
    let cases: Vec<(&str, Vec<Value>, Outcome, Vec<AttentionKind>)> = vec![
        ("end_turn", vec![user("q"), reply("Done.", "end_turn")], done("Done."), vec![AttentionKind::Finished]),
        ("refusal", vec![user("q"), reply("I can't help with that.", "refusal")], done("I can't help with that."),
         vec![AttentionKind::Finished]),
        ("max_tokens", vec![user("q"), reply("Partial answer.", "max_tokens")], done("Partial answer."),
         vec![AttentionKind::Finished]),
        ("stop_sequence + isApiErrorMessage", vec![user("q"), api_error("rate_limit", 429, "API Error: Rate limit reached")],
         Outcome::Error { code: Some("rate_limit".into()), status: Some(429), text: "API Error: Rate limit reached".into(), at_ms: NOW },
         vec![AttentionKind::Error]),
        ("interrupt marker", vec![user("q"), tool("t1", "Bash", json!({ "command": "sleep 100" })), interrupted()],
         Outcome::Interrupted { at_ms: NOW }, vec![]),
        ("tool_use after end_turn", vec![user("q"), reply("Let me check.", "end_turn"), tool("t1", "Bash", json!({ "command": "ls" }))],
         Outcome::Pending, vec![]),
        ("no assistant record (slash command)", vec![user("/compact")], Outcome::Pending, vec![]),
        ("whitespace collapsed", vec![user("q"), reply("Line one.\n\n   Line   two.", "end_turn")], done("Line one. Line two."),
         vec![AttentionKind::Finished]),
    ];
    for (case, records, outcome, alerts) in cases {
        let mut st = Store::new();
        let eof = records.len() as u64 * 100;
        let numbered = records.into_iter().enumerate().map(|(i, r)| (i as u64 * 100, r)).collect();
        run(&mut st, vec![cfile(sfile("busy", None, 1_000), None), ctr(Some(0), numbered, eof)]);
        let applied = run(&mut st, vec![cfile(sfile("idle", None, 2_000), Some(eof))]);
        assert_eq!(st.session(&key()).unwrap().outcome, outcome, "{case}");
        assert_eq!(kinds(&applied), alerts, "{case}");
    }
}

#[test]
fn first_appearance_never_alerts_and_tracks_the_transcript() {
    for status in [("busy", None), ("idle", None), ("waiting", Some("permission prompt")), ("waiting", Some("input needed")), ("x", None)] {
        let mut st = Store::new();
        let applied = run(&mut st, vec![cfile(sfile(status.0, status.1, 1_000), None)]);
        assert!(applied.alerts.is_empty(), "{status:?}");
        assert_eq!(applied.effects, [Effect::TrackTranscript { key: key(), session_id: SID.into() }], "{status:?}");
    }
}

#[test]
fn waiting_before_its_tool_use_gets_the_detail_from_the_next_transcript() {
    let mut st = Store::new();
    run(&mut st, vec![cfile(sfile("busy", None, 1_000), None), ctr(Some(0), vec![(0, user("deploy"))], 100)]);
    let first = run(&mut st, vec![cfile(sfile("waiting", Some("permission prompt"), 2_000), Some(100))]);
    assert_eq!(kinds(&first), [AttentionKind::Permission]);
    assert_eq!(first.alerts[0].detail, None);
    assert_eq!(first.alerts[0].text, "Claude · Alpha (Desktop) needs permission");

    let later = run(&mut st, vec![ctr(None, vec![(100, tool("t1", "Bash", json!({ "command": "npm publish" })))], 200)]);
    assert!(later.alerts.is_empty());
    let snap = st.snapshot(NOW);
    let row = &snap.projects[0].sessions[0];
    assert_eq!(row.detail.as_deref(), Some("Bash · npm publish"));
    assert_eq!(row.attention, Some(AttentionKind::Permission));
}

#[test]
fn transcript_found_after_the_session_file() {
    let mut st = Store::new();
    let first = run(&mut st, vec![cfile(sfile("busy", None, 1_000), None)]);
    assert_eq!(first.effects, [Effect::TrackTranscript { key: key(), session_id: SID.into() }]);

    // records of a transcript that is not bound are ignored (D52)
    run(&mut st, vec![ctr(None, vec![(0, user("stray"))], 50)]);
    assert!(st.session(&key()).unwrap().facts.is_empty());
    assert!(!st.session(&key()).unwrap().transcript_bound);

    // claude-io found the new file: bound at 0, the whole first turn counts
    run(&mut st, vec![ctr(Some(0), vec![(0, user("write tests")), (100, tool("t1", "Write", json!({ "file_path": "tests/a.rs" })))], 200)]);
    let s = st.session(&key()).unwrap();
    assert!(s.transcript_bound);
    assert_eq!(s.detail.as_deref(), Some("Write · tests/a.rs"));
    assert_eq!(s.activity.as_deref(), Some("Write · tests/a.rs"));

    let end = run(&mut st, vec![
        ctr(None, vec![(200, result("t1")), (300, reply("Tests written.", "end_turn"))], 400),
        cfile(sfile("idle", None, 2_000), Some(400)),
    ]);
    assert_eq!(kinds(&end), [AttentionKind::Finished]);
    let s = st.session(&key()).unwrap();
    assert_eq!(s.outcome, done("Tests written."));
    assert_eq!(s.idle_eof, 400);
}

#[test]
fn an_existing_transcript_bound_at_eof_does_not_replay_its_history() {
    let mut st = Store::new();
    run(&mut st, vec![
        cfile(sfile("busy", None, 1_000), None),
        // the 256 KiB window contains a finished reply, but it is before turn_start_offset = 400
        ctr(Some(400), vec![(0, user("q")), (300, reply("Earlier answer.", "end_turn"))], 400),
    ]);
    let s = st.session(&key()).unwrap();
    assert_eq!((s.turn_start_offset, s.idle_eof), (400, 400));
    assert!(s.facts.is_empty());
    let end = run(&mut st, vec![cfile(sfile("idle", None, 2_000), Some(400))]);
    assert!(end.alerts.is_empty());
    assert_eq!(st.session(&key()).unwrap().outcome, Outcome::Pending);
}

#[test]
fn session_id_change_rebinds_and_ignores_old_records() {
    let mut st = Store::new();
    run(&mut st, prelude(Start::Working));
    let mut next = sfile("busy", None, 1_000);
    next.session_id = Some("0f8b1d2e-0000-4000-8000-000000000002".into());
    let changed = run(&mut st, vec![cfile(next, Some(400))]);
    assert_eq!(
        changed.effects,
        [
            Effect::UntrackTranscript { key: key() },
            Effect::TrackTranscript { key: key(), session_id: "0f8b1d2e-0000-4000-8000-000000000002".into() },
        ]
    );
    let before = st.session(&key()).unwrap().clone();
    assert!(!before.transcript_bound);
    assert!(before.facts.is_empty() && before.pending_tools.is_empty());
    assert_eq!(before.logical_id, "0f8b1d2e-0000-4000-8000-000000000002");

    // an old-transcript record still in flight changes nothing (D52)
    let stray = run(&mut st, vec![ctr(None, vec![(400, reply("Old transcript reply.", "end_turn"))], 500)]);
    assert_eq!(stray, Applied::default());
    assert_eq!(st.session(&key()).unwrap(), &before);

    // the new binding sets the offsets
    run(&mut st, vec![ctr(Some(0), vec![(0, user("fresh start"))], 80)]);
    let s = st.session(&key()).unwrap();
    assert!(s.transcript_bound);
    assert_eq!((s.turn_start_offset, s.idle_eof), (0, 0));
    assert_eq!(s.facts.len(), 1);
}

#[test]
fn session_id_change_clears_titles_activity_and_detail() {
    // contract §2.15: a sessionId change clears facts, pending tools and titles
    let mut st = Store::new();
    run(&mut st, prelude(Start::Working));
    run(&mut st, vec![ctr(None, vec![(400, title_record("custom-title", "customTitle", "Old title")),
                                     (500, tool("t2", "Bash", json!({ "command": "make" })))], 600)]);
    let mut next = sfile("busy", None, 1_000);
    next.session_id = Some("0f8b1d2e-0000-4000-8000-000000000002".into());
    run(&mut st, vec![cfile(next, Some(600))]);
    let s = st.session(&key()).unwrap();
    assert_eq!(s.titles, TranscriptTitles::default());
    assert_eq!((s.title.as_deref(), s.activity.as_deref(), s.detail.as_deref()), (None, None, None));
}

#[test]
fn late_final_record_is_not_counted_twice() {
    // D51: idle → late end_turn (Finished) → busy → idle without a stop_reason record → no 2nd Finished
    let mut st = Store::new();
    let mut alerts = Vec::new();
    let mut step = |st: &mut Store, signals: Vec<Signal>| alerts.extend(kinds(&run(st, signals)));
    step(&mut st, vec![
        cfile(sfile("busy", None, 1_000), None),
        ctr(Some(0), vec![(0, user("ship it")), (100, tool("t1", "Bash", json!({ "command": "git push" }))), (200, result("t1"))], 300),
    ]);
    step(&mut st, vec![cfile(sfile("idle", None, 2_000), Some(300))]);
    assert_eq!(st.session(&key()).unwrap().outcome, Outcome::Pending);

    step(&mut st, vec![ctr(None, vec![(300, reply("Pushed.", "end_turn"))], 400)]);
    let s = st.session(&key()).unwrap();
    assert_eq!(s.outcome, done("Pushed."));
    assert_eq!(s.idle_eof, 301);

    step(&mut st, vec![ctr(None, vec![(400, user("/compact"))], 450), cfile(sfile("busy", None, 3_000), Some(450))]);
    assert_eq!(st.session(&key()).unwrap().turn_start_offset, 301);
    step(&mut st, vec![cfile(sfile("idle", None, 4_000), Some(450))]);
    assert_eq!(st.session(&key()).unwrap().outcome, Outcome::Pending);
    assert_eq!(alerts, [AttentionKind::Finished]);
}

#[test]
fn a_new_turn_drops_the_unresolved_tools_of_the_previous_turn() {
    // D21: turn 1 ends interrupted with an unresolved tool_use; turn 2's prompt precedes the busy file
    let mut st = Store::new();
    run(&mut st, vec![
        cfile(sfile("busy", None, 1_000), None),
        ctr(Some(0), vec![(0, user("q")), (100, tool("t1", "Bash", json!({ "command": "sleep 100" }))), (200, interrupted())], 300),
        cfile(sfile("idle", None, 2_000), Some(300)),
    ]);
    assert_eq!(st.session(&key()).unwrap().outcome, Outcome::Interrupted { at_ms: NOW });
    run(&mut st, vec![ctr(None, vec![(300, user("next"))], 400), cfile(sfile("busy", None, 3_000), Some(400))]);
    let s = st.session(&key()).unwrap();
    assert_eq!(s.turn_start_offset, 300);
    assert!(s.pending_tools.is_empty());
    assert_eq!((s.detail.as_deref(), s.activity.as_deref()), (None, None));
    assert_eq!(s.facts.iter().map(|(o, _)| *o).collect::<Vec<_>>(), [300]);
}

#[test]
fn ack_hides_and_a_new_waiting_episode_shows_again() {
    let mut st = Store::new();
    run(&mut st, prelude(Start::Waiting));
    assert_eq!(st.snapshot(NOW).attention, [key()]);
    run(&mut st, vec![Signal::Ack(key())]);
    let snap = st.snapshot(NOW);
    assert!(snap.attention.is_empty());
    assert_eq!((snap.counts.waiting, snap.hold), (0, false));
    assert_eq!(st.session(&key()).unwrap().phase, waiting(WaitKind::Permission));

    let again = run(&mut st, vec![cfile(sfile("waiting", Some("permission prompt"), 3_000), Some(200))]);
    assert_eq!(kinds(&again), [AttentionKind::Permission]);
    let snap = st.snapshot(NOW);
    assert_eq!(snap.attention, [key()]);
    assert!(snap.hold);
}

#[test]
fn ack_then_new_episode_with_the_same_status_ts_is_unacked() {
    // D49 + D19
    let mut st = Store::new();
    run(&mut st, prelude(Start::Waiting));
    run(&mut st, vec![Signal::Ack(key())]);
    assert_eq!(st.session(&key()).unwrap().acked_at, Some(NOW));
    let next = run(&mut st, vec![cfile(sfile("waiting", Some("input needed"), 2_000), Some(200))]);
    assert_eq!(kinds(&next), [AttentionKind::Input]);
    let snap = st.snapshot(NOW);
    assert_eq!(snap.attention, [key()]);
    assert_eq!(snap.projects[0].sessions[0].attention, Some(AttentionKind::Input));
    assert!(snap.hold);
    assert_eq!(snap.counts.waiting, 1);
}

#[test]
fn ack_before_an_older_waiting_file_leaves_the_new_episode_unacked() {
    // D49: the ack (store clock 5000) predates the episode in the store, whatever statusUpdatedAt says
    let mut st = Store::new();
    run(&mut st, prelude(Start::Working));
    st.apply(Signal::Ack(key()), 5_000);
    run(&mut st, vec![cfile(sfile("waiting", Some("permission prompt"), 4_000), Some(400))]);
    let s = st.session(&key()).unwrap();
    assert_eq!(s.since_ms, 4_000);
    assert_eq!(s.acked_at, None);
    let snap = st.snapshot(NOW);
    assert_eq!(snap.attention, [key()]);
    assert!(snap.hold);
}

#[test]
fn ack_in_working_does_not_hide_the_coming_finished() {
    // spec §12 "клик в Working не прячет будущий Finished"; equal clocks prove D49
    let mut st = Store::new();
    run(&mut st, prelude(Start::Working));
    run(&mut st, vec![Signal::Ack(key())]);
    let end = run(&mut st, vec![cfile(sfile("idle", None, 2_000), Some(400))]);
    assert_eq!(kinds(&end), [AttentionKind::Finished]);
    let snap = st.snapshot(NOW);
    assert_eq!(snap.attention, [key()]);
    assert!(!snap.projects[0].sessions[0].acked);
    assert_eq!(snap.counts.finished, 1);
}

#[test]
fn alerts_carry_project_host_title_detail_and_text() {
    let mut st = Store::new();
    let named = |status: &str, waiting_for: Option<&str>, ts: i64| {
        let mut f = sfile(status, waiting_for, ts);
        f.name = Some("Fix the build".into());
        f
    };
    run(&mut st, vec![
        cfile(named("busy", None, 1_000), None),
        ctr(Some(0), vec![(0, user("go")), (100, tool("t1", "Bash", json!({ "command": "git push" })))], 200),
    ]);
    let wait = run(&mut st, vec![cfile(named("waiting", Some("permission prompt"), 2_000), Some(200))]);
    assert_eq!(
        wait.alerts,
        [Alert {
            key: key(),
            kind: AttentionKind::Permission,
            rank: 0,
            agent: Source::Claude,
            host: HostKind::ClaudeDesktop,
            project: "Alpha".into(),
            title: Some("Fix the build".into()),
            detail: Some("Bash · git push".into()),
            text: "Claude · Alpha (Desktop) needs permission: Bash · git push".into(),
            since: 2_000,
        }]
    );
    let end = run(&mut st, vec![
        ctr(None, vec![(200, result("t1")), (300, api_error("rate_limit", 429, "API Error: Rate limit reached"))], 400),
        cfile(named("idle", None, 3_000), Some(400)),
    ]);
    assert_eq!(end.alerts.len(), 1);
    let a = &end.alerts[0];
    assert_eq!((a.kind, a.rank, a.since), (AttentionKind::Error, 2, NOW));
    assert_eq!(a.detail.as_deref(), Some("rate_limit: API Error: Rate limit reached"));
    assert_eq!(a.text, "Claude · Alpha (Desktop) error: rate_limit: API Error: Rate limit reached");
}

#[test]
fn alert_project_uses_the_collision_rule() {
    let mut st = Store::new();
    let work_app = ProjectRef { key: r"c:\work\app".into(), root: r"C:\work\app".into() };
    let other_app = ProjectRef { key: r"d:\other\app".into(), root: r"D:\other\App".into() };
    run(&mut st, vec![
        cfile_at(PID, work_app.clone(), sfile("busy", None, 1_000), None),
        cfile_at(5151, other_app, sfile("busy", None, 1_000), None),
    ]);
    let a = run(&mut st, vec![cfile_at(PID, work_app, sfile("waiting", Some("input needed"), 2_000), None)]);
    assert_eq!(a.alerts[0].project, r"work\app");
    assert_eq!(a.alerts[0].text, r"Claude · work\app (Desktop) asks");
    assert_eq!(a.alerts[0].rank, 1);
}

#[test]
fn titles_follow_precedence_and_ignore_turn_offsets() {
    let mut st = Store::new();
    let named = || {
        let mut f = sfile("idle", None, 1_000);
        f.name = Some("Session file name".into());
        f
    };
    run(&mut st, vec![cfile(named(), None)]);
    assert_eq!(st.session(&key()).unwrap().title.as_deref(), Some("Session file name"));
    run(&mut st, vec![ctr(Some(500), vec![(0, title_record("ai-title", "aiTitle", "AI title")), (100, user("x"))], 500)]);
    assert_eq!(st.session(&key()).unwrap().title.as_deref(), Some("AI title"));
    run(&mut st, vec![ctr(None, vec![(500, title_record("agent-name", "agentName", "Agent name"))], 600)]);
    assert_eq!(st.session(&key()).unwrap().title.as_deref(), Some("Agent name"));
    run(&mut st, vec![ctr(None, vec![(600, title_record("custom-title", "customTitle", "Custom title"))], 700)]);
    assert_eq!(st.session(&key()).unwrap().title.as_deref(), Some("Custom title"));
    run(&mut st, vec![ctr(Some(0), vec![], 0)]);
    assert_eq!(st.session(&key()).unwrap().title.as_deref(), Some("Session file name"));
}

#[test]
fn disabled_alert_kinds_are_dropped_but_rows_still_update() {
    // D18
    let mut st = Store::new();
    run(&mut st, vec![Signal::AlertPrefs { waiting: false, finished: true, error: true }]);
    run(&mut st, prelude(Start::Working));
    let wait = run(&mut st, vec![cfile(sfile("waiting", Some("permission prompt"), 2_000), Some(400))]);
    assert!(wait.alerts.is_empty());
    let snap = st.snapshot(NOW);
    assert_eq!((snap.counts.waiting, snap.hold), (1, false));
    assert_eq!(snap.attention, [key()]);

    let end = run(&mut st, vec![
        ctr(None, vec![(400, reply("Done.", "end_turn"))], 500),
        cfile(sfile("idle", None, 3_000), Some(500)),
    ]);
    assert_eq!(kinds(&end), [AttentionKind::Finished]);

    run(&mut st, vec![Signal::AlertPrefs { waiting: true, finished: false, error: true }]);
    let next = run(&mut st, vec![
        ctr(None, vec![(500, user("again")), (600, reply("Again done.", "end_turn"))], 700),
        cfile(sfile("idle", None, 4_000), Some(700)),
    ]);
    assert!(next.alerts.is_empty());
    assert_eq!(st.session(&key()).unwrap().outcome, done("Again done."));
}

#[test]
fn leaving_idle_drops_pending() {
    let pending_turn = |st: &mut Store| {
        run(st, vec![
            cfile(sfile("busy", None, 1_000), None),
            ctr(Some(0), vec![(0, user("q")), (100, tool("t1", "Bash", json!({ "command": "ls" })))], 200),
            cfile(sfile("idle", None, 2_000), Some(200)),
        ]);
        assert_eq!(st.session(&key()).unwrap().outcome, Outcome::Pending);
    };
    let mut st = Store::new();
    pending_turn(&mut st);
    run(&mut st, vec![cfile(sfile("compacting", None, 3_000), None)]);
    assert_eq!(st.session(&key()).unwrap().outcome, Outcome::None);

    let mut st = Store::new();
    pending_turn(&mut st);
    run(&mut st, vec![cfile(sfile("busy", None, 3_000), Some(200))]);
    assert_eq!(st.session(&key()).unwrap().outcome, Outcome::None);
}

#[test]
fn leaving_a_finished_idle_resets_the_outcome() {
    // spec §7: Idle → busy / waiting sets outcome None
    for (next, phase) in [
        (sfile("busy", None, 3_000), Phase::Working),
        (sfile("waiting", Some("permission prompt"), 3_000), waiting(WaitKind::Permission)),
    ] {
        let mut st = Store::new();
        run(&mut st, prelude(Start::Working));
        run(&mut st, vec![cfile(sfile("idle", None, 2_000), Some(400))]);
        assert_eq!(st.session(&key()).unwrap().outcome, done("Build finished."));
        run(&mut st, vec![cfile(next, Some(400))]);
        let s = st.session(&key()).unwrap();
        assert_eq!((s.phase.clone(), s.outcome.clone()), (phase, Outcome::None));
    }
}

#[test]
fn claude_gone_and_proc_exit_remove_without_alerts() {
    let mut st = Store::new();
    run(&mut st, vec![
        cfile(sfile("busy", None, 1_000), None),
        cfile_at(5151, project("Beta"), sfile("waiting", Some("permission prompt"), 1_000), None),
    ]);
    let gone = run(&mut st, vec![Signal::ClaudeGone { key: key() }]);
    assert_eq!(gone, Applied { alerts: vec![], effects: vec![Effect::UntrackTranscript { key: key() }], wake: false });
    assert!(st.session(&key()).is_none());

    // a reused PID with another creation time is not the same process
    let other = run(&mut st, vec![Signal::ProcExit { pid: 5151, created_ft: 1 }]);
    assert_eq!(other, Applied::default());
    let exit = run(&mut st, vec![Signal::ProcExit { pid: 5151, created_ft: claude_proc(5151).created_ft }]);
    assert_eq!(exit.effects, [Effect::UntrackTranscript { key: ckey(5151) }]);
    assert!(exit.alerts.is_empty());
    assert!(st.snapshot(NOW).projects.is_empty());
}

#[test]
fn signals_for_unknown_sessions_are_ignored() {
    let mut st = Store::new();
    let applied = run(&mut st, vec![
        ctr(Some(0), vec![(0, reply("x", "end_turn"))], 10),
        Signal::ClaudeGone { key: key() },
        Signal::ProcExit { pid: PID, created_ft: claude_proc(PID).created_ft },
    ]);
    assert_eq!(applied, Applied::default());
    assert_eq!(st.rev(), 0);
}

#[test]
fn facts_are_capped() {
    // D14
    let mut st = Store::new();
    let records = (0..1_100u64).map(|i| (i * 10, user("x"))).collect();
    run(&mut st, vec![cfile(sfile("busy", None, 1_000), None), ctr(Some(0), records, 11_000)]);
    let s = st.session(&key()).unwrap();
    assert_eq!(s.facts.len(), 1_024);
    assert_eq!(s.facts[0].0, 760);
}

#[test]
fn rev_counts_snapshot_changes_only() {
    let mut st = Store::new();
    run(&mut st, vec![cfile(sfile("busy", None, 1_000), None)]);
    assert_eq!(st.rev(), 1);
    run(&mut st, vec![cfile(sfile("busy", None, 1_000), None)]);
    assert_eq!(st.rev(), 1);
    run(&mut st, vec![Signal::Ack(key())]); // Working: no episode, the row does not change
    assert_eq!(st.rev(), 1);
    run(&mut st, vec![cfile(sfile("waiting", Some("permission prompt"), 2_000), None)]);
    assert_eq!(st.rev(), 2);
    assert_eq!(st.snapshot(NOW).rev, 2);
}

#[test]
fn desktop_and_cursor_sessions_in_a_repo_and_its_subfolder_share_one_project() {
    // Review focus: Claude Desktop at the repo root, Claude in Cursor at a subfolder spelled in
    // lower case. The source threads resolve each cwd with paths::project_of (T5); NTFS names
    // ignore case, so the probe does too.
    let git = |p: &std::path::Path| p.to_str().is_some_and(|s| s.eq_ignore_ascii_case(r"C:\Users\dev\Mayak\.git"));
    let at_root = paths::project_of(r"C:\Users\dev\Mayak", git);
    let in_sub = paths::project_of(r"c:/users/dev/mayak/windows/src-tauri/", git);
    let cursor = |status: &str, waiting_for: Option<&str>, ts: i64| {
        let mut file = sfile(status, waiting_for, ts);
        file.pid = Some(5151);
        file.proc_start = Some(claude_proc(5151).created_ft.to_string());
        file.cwd = Some(r"c:/users/dev/mayak/windows/src-tauri/".into());
        file.entrypoint = Some("claude-vscode".into());
        Signal::ClaudeFile {
            key: ckey(5151),
            proc: claude_proc(5151),
            host: HostKind::Cursor,
            project: in_sub.clone(),
            file,
            transcript_eof: None,
        }
    };
    let mut st = Store::new();
    run(&mut st, vec![cfile_at(PID, at_root, sfile("busy", None, 1_000), None), cursor("busy", None, 1_000)]);
    let snap = st.snapshot(NOW);
    assert_eq!(snap.projects.len(), 1, "one project card");
    let card = &snap.projects[0];
    assert_eq!(
        (card.key.as_str(), card.name.as_str(), card.root.as_str()),
        (r"c:\users\dev\mayak", "Mayak", r"C:\Users\dev\Mayak")
    );
    let rows: Vec<(&str, HostKind)> = card.sessions.iter().map(|s| (s.key.as_str(), s.host)).collect();
    assert_eq!(rows, [(ckey(PID).as_str(), HostKind::ClaudeDesktop), (ckey(5151).as_str(), HostKind::Cursor)]);
    // The subfolder session's alert names the same project and its own host.
    let asks = run(&mut st, vec![cursor("waiting", Some("input needed"), 2_000)]);
    let texts: Vec<&str> = asks.alerts.iter().map(|a| a.text.as_str()).collect();
    assert_eq!(texts, ["Claude · Mayak (Cursor) asks"]);
    assert_eq!(asks.alerts[0].project, "Mayak");
    assert_eq!(st.snapshot(NOW).projects.len(), 1);
}

// ── Codex fixtures ──

const CSID: &str = "019a8c4e-0000-7000-8000-00000000c0de";
const ROLLOUT: &str = r"C:\Users\dev\.codex\sessions\2026\10\01\rollout-2026-10-01T10-00-00-019a8c4e.jsonl";

fn codex_proc() -> ProcId {
    ProcId {
        pid: 7100,
        created_ft: 134_000_000_000_007_100,
        image: r"c:\Users\dev\.cursor\extensions\openai.chatgpt-0.146.0-win32-x64\bin\windows-x86_64\codex.exe".into(),
    }
}

fn xkey() -> SessionKey {
    codex_key(CSID)
}

fn hook(event: &str, turn: Option<&str>, relay_ts: i64) -> HookEvent {
    HookEvent {
        hook_event_name: event.into(),
        session_id: CSID.into(),
        turn_id: turn.map(Into::into),
        cwd: r"C:\Users\dev\Beta".into(),
        transcript_path: Some(ROLLOUT.into()),
        tool_name: None,
        tool_input: None,
        last_assistant_message: None,
        agent_id: None,
        agent_type: None,
        relay_ts,
        agent_pid: Some(codex_proc().pid),
        agent_created: Some(codex_proc().created_ft),
        agent_image: Some(codex_proc().image),
    }
}

fn permission(turn: Option<&str>, relay_ts: i64, command: &str) -> HookEvent {
    let mut e = hook("PermissionRequest", turn, relay_ts);
    e.tool_name = Some("shell".into());
    e.tool_input = Some(json!({ "command": command, "workdir": r"C:\Users\dev\Beta" }));
    e
}

fn stop(turn: Option<&str>, relay_ts: i64, message: &str) -> HookEvent {
    let mut e = hook("Stop", turn, relay_ts);
    e.last_assistant_message = Some(message.into());
    e
}

fn ups(turn: &str, relay_ts: i64) -> HookEvent {
    hook("UserPromptSubmit", Some(turn), relay_ts)
}

/// A CodexHook as codex-io sends it: proof OK, Cursor host, project Beta.
fn xhook(event: HookEvent) -> Signal {
    Signal::CodexHook {
        key: codex_key(&event.session_id),
        event,
        proc: Some(codex_proc()),
        host: HostKind::Cursor,
        project: project("Beta"),
        rollout_eof: None,
        title: None,
    }
}

fn xroll(records: Vec<Value>) -> Signal {
    xroll_folder(true, records)
}

fn xroll_folder(folder_exists: bool, records: Vec<Value>) -> Signal {
    let eof = records.len() as u64 * 100;
    Signal::CodexRollout {
        key: xkey(),
        path: ROLLOUT.into(),
        records: records.into_iter().enumerate().map(|(i, r)| (i as u64 * 100, r)).collect(),
        eof,
        folder_exists,
    }
}

// Rollout records (legacy mode, spec §5.6).
fn event_msg(payload: Value) -> Value {
    json!({ "timestamp": "2026-10-01T10:00:00.000Z", "type": "event_msg", "payload": payload })
}
fn with_turn(mut payload: Value, turn: Option<&str>) -> Value {
    if let Some(t) = turn {
        payload["turn_id"] = json!(t);
    }
    payload
}
fn task_started(turn: Option<&str>) -> Value {
    event_msg(with_turn(json!({ "type": "task_started" }), turn))
}
fn task_complete(turn: Option<&str>, message: Option<&str>, error: Option<&str>) -> Value {
    let mut p = with_turn(json!({ "type": "task_complete" }), turn);
    if let Some(m) = message {
        p["last_agent_message"] = json!(m);
    }
    if let Some(e) = error {
        p["error"] = json!(e);
    }
    event_msg(p)
}
fn turn_aborted(turn: Option<&str>) -> Value {
    event_msg(with_turn(json!({ "type": "turn_aborted", "reason": "interrupted" }), turn))
}
fn call(name: &str, arguments: Value) -> Value {
    json!({ "timestamp": "2026-10-01T10:00:00.000Z", "type": "response_item",
            "payload": { "type": "function_call", "name": name, "arguments": arguments.to_string(), "call_id": "call_1" } })
}
fn response_item(payload: Value) -> Value {
    json!({ "timestamp": "2026-10-01T10:00:00.000Z", "type": "response_item", "payload": payload })
}
/// The `exec` call of a projectless thread (code mode) that asks the user for filesystem permissions.
fn permission_ask(call_id: &str) -> Value {
    response_item(json!({
        "type": "custom_tool_call", "name": "exec", "call_id": call_id,
        "input": "const r = await tools.request_permissions({ permissions: {} }); text(JSON.stringify(r));"
    }))
}
fn call_output(call_id: &str) -> Value {
    response_item(json!({ "type": "custom_tool_call_output", "call_id": call_id, "output": "ok" }))
}

struct CodexRow {
    case: &'static str,
    prelude: Vec<Signal>,
    step: Vec<Signal>,
    /// None: the session does not exist after the step.
    phase: Option<Phase>,
    outcome: Outcome,
    alerts: Vec<AttentionKind>,
    turn: Option<&'static str>,
    since: i64,
    effects: Vec<Effect>,
}

fn error_at_now(text: &str) -> Outcome {
    Outcome::Error { code: None, status: None, text: text.into(), at_ms: NOW }
}

/// Every row of the spec §7 Codex table plus the creation rule and the wait-release rules.
fn codex_rows() -> Vec<CodexRow> {
    use AttentionKind::{Error, Finished, Permission};
    let t = Some;
    vec![
        // ── creation: the first hook of an unknown key is a live transition ──
        CodexRow { case: "unknown key + PermissionRequest → created, Waiting, alert", prelude: vec![],
                   step: vec![xhook(permission(t("T1"), 1_000, "npm test"))],
                   phase: Some(waiting(WaitKind::Permission)), outcome: Outcome::None, alerts: vec![Permission],
                   turn: t("T1"), since: 1_000, effects: vec![] },
        CodexRow { case: "unknown key + Stop → created, Idle Finished, alert", prelude: vec![],
                   step: vec![xhook(stop(t("T1"), 1_000, "All set."))],
                   phase: Some(Phase::Idle), outcome: done("All set."), alerts: vec![Finished],
                   turn: t("T1"), since: 1_000, effects: vec![] },
        CodexRow { case: "unknown key + UserPromptSubmit → Working, no alert", prelude: vec![],
                   step: vec![xhook(ups("T1", 1_000))],
                   phase: Some(Phase::Working), outcome: Outcome::None, alerts: vec![], turn: t("T1"), since: 1_000,
                   effects: vec![] },
        CodexRow { case: "unknown key + SessionEnd → nothing is created (D22)", prelude: vec![],
                   step: vec![xhook(hook("SessionEnd", None, 1_000))],
                   phase: None, outcome: Outcome::None, alerts: vec![], turn: None, since: 0, effects: vec![] },
        CodexRow { case: "unknown key + another event → Unknown(\"\")", prelude: vec![],
                   step: vec![xhook(hook("SessionStart", None, 1_000))],
                   phase: Some(Phase::Unknown(String::new())), outcome: Outcome::None, alerts: vec![], turn: None,
                   since: 1_000, effects: vec![] },
        // ── UserPromptSubmit / TaskStarted ──
        CodexRow { case: "UserPromptSubmit(T) → start_turn, Working", prelude: vec![xhook(ups("T1", 1_000)), xhook(stop(t("T1"), 2_000, "First."))],
                   step: vec![xhook(ups("T2", 3_000))],
                   phase: Some(Phase::Working), outcome: Outcome::None, alerts: vec![], turn: t("T2"), since: 3_000, effects: vec![] },
        CodexRow { case: "UserPromptSubmit releases Waiting", prelude: vec![xhook(ups("T1", 1_000)), xhook(permission(t("T1"), 1_500, "npm test"))],
                   step: vec![xhook(ups("T2", 2_000))],
                   phase: Some(Phase::Working), outcome: Outcome::None, alerts: vec![], turn: t("T2"), since: 2_000, effects: vec![] },
        CodexRow { case: "rollout TaskStarted(T ≠ current) → start_turn, Working", prelude: vec![xhook(stop(t("T1"), 1_000, "First."))],
                   step: vec![xroll(vec![task_started(t("T2"))])],
                   phase: Some(Phase::Working), outcome: Outcome::None, alerts: vec![], turn: t("T2"), since: NOW, effects: vec![] },
        CodexRow { case: "rollout TaskStarted(current) → nothing", prelude: vec![xhook(ups("T1", 1_000))],
                   step: vec![xroll(vec![task_started(t("T1"))])],
                   phase: Some(Phase::Working), outcome: Outcome::None, alerts: vec![], turn: t("T1"), since: 1_000, effects: vec![] },
        CodexRow { case: "rollout TaskStarted without turn_id → nothing", prelude: vec![xhook(stop(t("T1"), 1_000, "First."))],
                   step: vec![xroll(vec![task_started(None)])],
                   phase: Some(Phase::Idle), outcome: done("First."), alerts: vec![], turn: t("T1"), since: 1_000, effects: vec![] },
        // ── PermissionRequest ──
        CodexRow { case: "PermissionRequest of a new turn starts it", prelude: vec![xhook(stop(t("T1"), 1_000, "First."))],
                   step: vec![xhook(permission(t("T2"), 2_000, "npm test"))],
                   phase: Some(waiting(WaitKind::Permission)), outcome: Outcome::None, alerts: vec![Permission], turn: t("T2"),
                   since: 2_000, effects: vec![] },
        CodexRow { case: "PermissionRequest :retry (same wait_key while Waiting) → nothing",
                   prelude: vec![xhook(ups("T1", 1_000)), xhook(permission(t("T1"), 2_000, "npm test"))],
                   step: vec![xhook(permission(t("T1"), 2_500, "npm test"))],
                   phase: Some(waiting(WaitKind::Permission)), outcome: Outcome::None, alerts: vec![], turn: t("T1"),
                   since: 2_000, effects: vec![] },
        CodexRow { case: "PermissionRequest with another tool_input in the same turn → new episode",
                   prelude: vec![xhook(ups("T1", 1_000)), xhook(permission(t("T1"), 2_000, "npm test"))],
                   step: vec![xhook(permission(t("T1"), 2_500, "npm publish"))],
                   phase: Some(waiting(WaitKind::Permission)), outcome: Outcome::None, alerts: vec![Permission], turn: t("T1"),
                   since: 2_500, effects: vec![] },
        // ── Stop ──
        CodexRow { case: "Stop(T) → Idle, Finished, alert", prelude: vec![xhook(ups("T1", 1_000))],
                   step: vec![xhook(stop(t("T1"), 2_000, "Done."))],
                   phase: Some(Phase::Idle), outcome: done("Done."), alerts: vec![Finished], turn: t("T1"), since: 2_000, effects: vec![] },
        CodexRow { case: "Stop releases Waiting", prelude: vec![xhook(ups("T1", 1_000)), xhook(permission(t("T1"), 1_500, "npm test"))],
                   step: vec![xhook(stop(t("T1"), 2_000, "Done."))],
                   phase: Some(Phase::Idle), outcome: done("Done."), alerts: vec![Finished], turn: t("T1"), since: 2_000, effects: vec![] },
        CodexRow { case: "Stop of a closed turn is ignored", prelude: vec![xhook(ups("T1", 1_000)), xhook(stop(t("T1"), 2_000, "Done."))],
                   step: vec![xhook(stop(t("T1"), 2_100, "Again."))],
                   phase: Some(Phase::Idle), outcome: done("Done."), alerts: vec![], turn: t("T1"), since: 2_000, effects: vec![] },
        CodexRow { case: "Stop of an old turn (relay_ts < turn_started_ts) is ignored",
                   prelude: vec![xhook(ups("T1", 1_000)), xhook(ups("T2", 3_000))],
                   step: vec![xhook(stop(t("T1"), 2_000, "Late."))],
                   phase: Some(Phase::Working), outcome: Outcome::None, alerts: vec![], turn: t("T2"), since: 3_000, effects: vec![] },
        CodexRow { case: "Stop of another, newer turn → current_turn = T, Finished", prelude: vec![xhook(ups("T1", 1_000))],
                   step: vec![xhook(stop(t("T2"), 2_000, "Other turn."))],
                   phase: Some(Phase::Idle), outcome: done("Other turn."), alerts: vec![Finished], turn: t("T2"), since: 2_000,
                   effects: vec![] },
        CodexRow { case: "Stop without turn_id finishes the current turn", prelude: vec![xhook(ups("T1", 1_000))],
                   step: vec![xhook(stop(None, 2_000, "Done."))],
                   phase: Some(Phase::Idle), outcome: done("Done."), alerts: vec![Finished], turn: t("T1"), since: 2_000, effects: vec![] },
        CodexRow { case: "Stop without last_assistant_message → empty summary", prelude: vec![xhook(ups("T1", 1_000))],
                   step: vec![xhook(hook("Stop", t("T1"), 2_000))],
                   phase: Some(Phase::Idle), outcome: done(""), alerts: vec![Finished], turn: t("T1"), since: 2_000, effects: vec![] },
        // ── rollout TaskComplete ──
        CodexRow { case: "TaskComplete(current) → Idle, Finished, alert", prelude: vec![xhook(ups("T1", 1_000))],
                   step: vec![xroll(vec![task_complete(t("T1"), Some("All tests pass."), None)])],
                   phase: Some(Phase::Idle), outcome: done("All tests pass."), alerts: vec![Finished], turn: t("T1"), since: NOW,
                   effects: vec![] },
        CodexRow { case: "TaskComplete(current) with error → Error, alert", prelude: vec![xhook(ups("T1", 1_000))],
                   step: vec![xroll(vec![task_complete(t("T1"), None, Some("stream disconnected"))])],
                   phase: Some(Phase::Idle), outcome: error_at_now("stream disconnected"), alerts: vec![Error], turn: t("T1"),
                   since: NOW, effects: vec![] },
        CodexRow { case: "TaskComplete of another turn is ignored", prelude: vec![xhook(ups("T2", 1_000))],
                   step: vec![xroll(vec![task_complete(t("T1"), Some("Old."), None)])],
                   phase: Some(Phase::Working), outcome: Outcome::None, alerts: vec![], turn: t("T2"), since: 1_000, effects: vec![] },
        CodexRow { case: "TaskComplete while current_turn is None", prelude: vec![xhook(hook("SessionStart", None, 1_000))],
                   step: vec![xroll(vec![task_complete(t("T1"), Some("Done."), None)])],
                   phase: Some(Phase::Idle), outcome: done("Done."), alerts: vec![Finished], turn: t("T1"), since: NOW, effects: vec![] },
        CodexRow { case: "TaskComplete releases Waiting",
                   prelude: vec![xhook(ups("T1", 1_000)), xhook(permission(t("T1"), 1_500, "npm test"))],
                   step: vec![xroll(vec![task_complete(t("T1"), Some("Done."), None)])],
                   phase: Some(Phase::Idle), outcome: done("Done."), alerts: vec![Finished], turn: t("T1"), since: NOW, effects: vec![] },
        CodexRow { case: "Stop + TaskComplete of one turn → one alert", prelude: vec![xhook(ups("T1", 1_000))],
                   step: vec![xhook(stop(t("T1"), 2_000, "Done.")), xroll(vec![task_complete(t("T1"), Some("Done."), None)])],
                   phase: Some(Phase::Idle), outcome: done("Done."), alerts: vec![Finished], turn: t("T1"), since: 2_000, effects: vec![] },
        CodexRow { case: "TaskComplete + Stop of one turn → one alert", prelude: vec![xhook(ups("T1", 1_000))],
                   step: vec![xroll(vec![task_complete(t("T1"), Some("Done."), None)]), xhook(stop(t("T1"), 2_000, "Done."))],
                   phase: Some(Phase::Idle), outcome: done("Done."), alerts: vec![Finished], turn: t("T1"), since: NOW, effects: vec![] },
        CodexRow { case: "TaskComplete with error after Stop → upgrade to Error",
                   prelude: vec![xhook(ups("T1", 1_000)), xhook(stop(t("T1"), 2_000, "Done."))],
                   step: vec![xroll(vec![task_complete(t("T1"), None, Some("stream disconnected"))])],
                   phase: Some(Phase::Idle), outcome: error_at_now("stream disconnected"), alerts: vec![Error], turn: t("T1"),
                   since: 2_000, effects: vec![] },
        // ── Interrupt / TurnAborted ──
        CodexRow { case: "Interrupt(current) → Idle, Interrupted, no alert",
                   prelude: vec![xhook(ups("T1", 1_000)), xhook(permission(t("T1"), 1_500, "npm test"))],
                   step: vec![xhook(hook("Interrupt", t("T1"), 2_000))],
                   phase: Some(Phase::Idle), outcome: Outcome::Interrupted { at_ms: NOW }, alerts: vec![], turn: t("T1"),
                   since: 2_000, effects: vec![] },
        CodexRow { case: "Interrupt of another turn is ignored", prelude: vec![xhook(ups("T2", 1_000))],
                   step: vec![xhook(hook("Interrupt", t("T1"), 2_000))],
                   phase: Some(Phase::Working), outcome: Outcome::None, alerts: vec![], turn: t("T2"), since: 1_000, effects: vec![] },
        CodexRow { case: "TurnAborted without turn_id → current turn interrupted", prelude: vec![xhook(ups("T1", 1_000))],
                   step: vec![xroll(vec![turn_aborted(None)])],
                   phase: Some(Phase::Idle), outcome: Outcome::Interrupted { at_ms: NOW }, alerts: vec![], turn: t("T1"),
                   since: NOW, effects: vec![] },
        CodexRow { case: "TurnAborted(current) releases Waiting",
                   prelude: vec![xhook(ups("T1", 1_000)), xhook(permission(t("T1"), 1_500, "npm test"))],
                   step: vec![xroll(vec![turn_aborted(t("T1"))])],
                   phase: Some(Phase::Idle), outcome: Outcome::Interrupted { at_ms: NOW }, alerts: vec![], turn: t("T1"),
                   since: NOW, effects: vec![] },
        CodexRow { case: "TurnAborted of another turn is ignored", prelude: vec![xhook(ups("T2", 1_000))],
                   step: vec![xroll(vec![turn_aborted(t("T1"))])],
                   phase: Some(Phase::Working), outcome: Outcome::None, alerts: vec![], turn: t("T2"), since: 1_000, effects: vec![] },
        // ── activity never releases Waiting; other hooks only refresh fields ──
        CodexRow { case: "rollout Call keeps Waiting",
                   prelude: vec![xhook(ups("T1", 1_000)), xhook(permission(t("T1"), 1_500, "npm test"))],
                   step: vec![xroll(vec![call("shell", json!({ "command": ["cargo", "test"] }))])],
                   phase: Some(waiting(WaitKind::Permission)), outcome: Outcome::None, alerts: vec![], turn: t("T1"),
                   since: 1_500, effects: vec![] },
        CodexRow { case: "a hook outside the table keeps Waiting",
                   prelude: vec![xhook(ups("T1", 1_000)), xhook(permission(t("T1"), 1_500, "npm test"))],
                   step: vec![xhook(hook("PreToolUse", t("T1"), 1_600))],
                   phase: Some(waiting(WaitKind::Permission)), outcome: Outcome::None, alerts: vec![], turn: t("T1"),
                   since: 1_500, effects: vec![] },
        // ── SessionEnd / unknown keys ──
        CodexRow { case: "SessionEnd → removed, UntrackRollout", prelude: vec![xhook(ups("T1", 1_000))],
                   step: vec![xhook(hook("SessionEnd", None, 2_000))],
                   phase: None, outcome: Outcome::None, alerts: vec![], turn: None, since: 0,
                   effects: vec![Effect::UntrackRollout { key: xkey() }] },
        CodexRow { case: "rollout of an unknown key is ignored", prelude: vec![],
                   step: vec![xroll(vec![task_complete(t("T1"), Some("Done."), None)])],
                   phase: None, outcome: Outcome::None, alerts: vec![], turn: None, since: 0, effects: vec![] },
    ]
}

#[test]
fn codex_transition_table() {
    for row in codex_rows() {
        let mut st = Store::new();
        run(&mut st, row.prelude);
        let applied = run(&mut st, row.step);
        assert_eq!(kinds(&applied), row.alerts, "{}: alerts", row.case);
        // The ticker follows the busy sessions; the ticker tests below own that rule.
        let effects: Vec<Effect> = applied.effects.into_iter().filter(|e| !matches!(e, Effect::CodexTicker(_))).collect();
        assert_eq!(effects, row.effects, "{}: effects", row.case);
        match (st.session(&xkey()), row.phase) {
            (None, None) => {}
            (Some(s), Some(phase)) => {
                assert_eq!(s.phase, phase, "{}: phase", row.case);
                assert_eq!(s.outcome, row.outcome, "{}: outcome", row.case);
                assert_eq!(s.current_turn.as_deref(), row.turn, "{}: current_turn", row.case);
                assert_eq!(s.since_ms, row.since, "{}: since", row.case);
                if !matches!(s.phase, Phase::Waiting { .. }) {
                    assert_eq!(s.wait_key, None, "{}: wait_key outside Waiting", row.case);
                }
            }
            (got, want) => panic!("{}: session {:?}, expected phase {:?}", row.case, got.map(|s| &s.phase), want),
        }
    }
}

#[test]
fn codex_both_hook_sets() {
    // with UserPromptSubmit
    let mut st = Store::new();
    let mut phases = Vec::new();
    let mut alerts = Vec::new();
    for signal in [
        xhook(ups("T1", 1_000)),
        xhook(permission(Some("T1"), 1_500, "npm test")),
        xhook(stop(Some("T1"), 2_000, "Done.")),
    ] {
        alerts.extend(kinds(&run(&mut st, vec![signal])));
        phases.push(st.session(&xkey()).unwrap().phase.clone());
    }
    assert_eq!(phases, [Phase::Working, waiting(WaitKind::Permission), Phase::Idle]);
    assert_eq!(alerts, [AttentionKind::Permission, AttentionKind::Finished]);

    // PermissionRequest + Stop only: Working comes from the rollout TaskStarted
    let mut st = Store::new();
    let mut phases = Vec::new();
    let mut alerts = Vec::new();
    for signal in [
        xhook(stop(Some("T1"), 1_000, "First.")),
        xroll(vec![task_started(Some("T2"))]),
        xhook(permission(Some("T2"), 2_000, "npm test")),
        xroll(vec![task_complete(Some("T2"), Some("Second."), None)]),
        xhook(stop(Some("T2"), 3_000, "Second.")),
    ] {
        alerts.extend(kinds(&run(&mut st, vec![signal])));
        phases.push(st.session(&xkey()).unwrap().phase.clone());
    }
    assert_eq!(phases, [Phase::Idle, Phase::Working, waiting(WaitKind::Permission), Phase::Idle, Phase::Idle]);
    assert_eq!(alerts, [AttentionKind::Finished, AttentionKind::Permission, AttentionKind::Finished]);
}

#[test]
fn codex_retry_compares_canonical_input_and_a_new_input_shows_after_ok() {
    let mut st = Store::new();
    let mut first = permission(Some("T1"), 1_000, "npm test");
    first.tool_input = Some(json!({ "command": "npm test", "workdir": r"C:\Users\dev\Beta" }));
    let mut reordered = permission(Some("T1"), 1_100, "npm test");
    reordered.tool_input = Some(json!({ "workdir": r"C:\Users\dev\Beta", "command": "npm test" }));
    assert_eq!(kinds(&run(&mut st, vec![xhook(first)])), [AttentionKind::Permission]);
    assert!(run(&mut st, vec![xhook(reordered)]).alerts.is_empty());

    // OK on the first request, then another tool_input in the same turn: a new, unacked episode
    run(&mut st, vec![Signal::Ack(xkey())]);
    assert!(st.snapshot(NOW).attention.is_empty());
    let next = run(&mut st, vec![xhook(permission(Some("T1"), 1_200, "npm publish"))]);
    assert_eq!(kinds(&next), [AttentionKind::Permission]);
    assert_eq!(st.snapshot(NOW).attention, [xkey()]);
    assert!(st.snapshot(NOW).hold);
}

#[test]
fn codex_permission_alert_text_and_subagent_label() {
    let mut st = Store::new();
    let a = run(&mut st, vec![xhook(permission(Some("T1"), 1_000, "npm test"))]);
    assert_eq!(
        a.alerts,
        [Alert {
            key: xkey(),
            kind: AttentionKind::Permission,
            rank: 0,
            agent: Source::Codex,
            host: HostKind::Cursor,
            project: "Beta".into(),
            title: None,
            detail: Some("shell · npm test".into()),
            text: "Codex · Beta (Cursor) needs permission: shell · npm test".into(),
            since: 1_000,
        }]
    );
    let mut sub = permission(Some("T1"), 1_100, "cargo clippy");
    sub.agent_id = Some("agent-7".into());
    sub.agent_type = Some("reviewer".into());
    let b = run(&mut st, vec![xhook(sub)]);
    assert_eq!(b.alerts[0].detail.as_deref(), Some("reviewer: shell · cargo clippy"));
    assert_eq!(st.session(&xkey()).unwrap().detail.as_deref(), Some("reviewer: shell · cargo clippy"));
}

#[test]
fn codex_outcomes_use_the_store_clock() {
    // D50: at_ms = now_ms of apply, never relay_ts
    let mut st = Store::new();
    st.apply(xhook(ups("T1", 1_000)), 50_000);
    let a = st.apply(xhook(stop(Some("T1"), 1_500, "Done.")), 77_777);
    assert_eq!(st.session(&xkey()).unwrap().outcome, Outcome::Finished { summary: "Done.".into(), at_ms: 77_777 });
    assert_eq!(a.alerts[0].since, 77_777);
    let b = st.apply(xroll(vec![task_complete(Some("T1"), None, Some("boom"))]), 88_888);
    assert_eq!(
        st.session(&xkey()).unwrap().outcome,
        Outcome::Error { code: None, status: None, text: "boom".into(), at_ms: 88_888 }
    );
    assert_eq!((b.alerts[0].kind, b.alerts[0].since), (AttentionKind::Error, 88_888));
    st.apply(xhook(ups("T2", 2_000)), 90_000);
    st.apply(xhook(hook("Interrupt", Some("T2"), 2_100)), 99_999);
    assert_eq!(st.session(&xkey()).unwrap().outcome, Outcome::Interrupted { at_ms: 99_999 });
}

#[test]
fn codex_since_never_decreases() {
    // spec §7 Codex: «Источник since_ms — relay_ts, не меньше предыдущего»
    let mut st = Store::new();
    run(&mut st, vec![xhook(ups("T1", 5_000)), xhook(permission(Some("T1"), 4_000, "npm test"))]);
    assert_eq!(st.session(&xkey()).unwrap().since_ms, 5_000);
}

#[test]
fn a_late_stop_of_an_older_turn_after_creation_is_ignored() {
    // creation sets turn_started_ts = relay_ts, so the old-turn Stop guard holds from the first hook on
    let mut st = Store::new();
    run(&mut st, vec![xhook(permission(Some("T2"), 3_000, "npm test"))]);
    let late = run(&mut st, vec![xhook(stop(Some("T1"), 2_000, "Old."))]);
    assert!(late.alerts.is_empty());
    assert_eq!(st.session(&xkey()).unwrap().phase, waiting(WaitKind::Permission));
}

#[test]
fn codex_hooks_refresh_host_proc_cwd_project_and_title() {
    let mut st = Store::new();
    run(&mut st, vec![Signal::CodexHook {
        key: xkey(),
        event: ups("T1", 1_000),
        proc: Some(codex_proc()),
        host: HostKind::Cursor,
        project: project("Beta"),
        rollout_eof: None,
        title: Some("Refactor the parser".into()),
    }]);
    let mut moved = hook("PreToolUse", Some("T1"), 1_100);
    moved.cwd = r"C:\Users\dev\Gamma".into();
    run(&mut st, vec![Signal::CodexHook {
        key: xkey(),
        event: moved,
        proc: None,
        host: HostKind::Unknown,
        project: project("Gamma"),
        rollout_eof: Some(0),
        title: None,
    }]);
    let s = st.session(&xkey()).unwrap();
    assert_eq!(s.host, HostKind::Cursor);
    assert_eq!(s.proc, Some(codex_proc()));
    assert_eq!(s.title.as_deref(), Some("Refactor the parser"));
    assert_eq!(s.cwd, r"C:\Users\dev\Gamma");
    assert_eq!(s.project, project("Gamma"));
    assert_eq!(s.logical_id, CSID);

    run(&mut st, vec![Signal::CodexHook {
        key: xkey(),
        event: hook("PreToolUse", Some("T1"), 1_200),
        proc: None,
        host: HostKind::CodexDesktop,
        project: project("Gamma"),
        rollout_eof: None,
        title: Some("Renamed thread".into()),
    }]);
    let s = st.session(&xkey()).unwrap();
    assert_eq!(s.host, HostKind::CodexDesktop);
    assert_eq!(s.title.as_deref(), Some("Renamed thread"));
}

#[test]
fn codex_call_sets_activity_and_a_new_turn_clears_it() {
    let mut st = Store::new();
    run(&mut st, vec![
        xhook(ups("T1", 1_000)),
        xroll(vec![call("shell", json!({ "command": ["cargo", "test"] }))]),
    ]);
    assert_eq!(st.session(&xkey()).unwrap().activity.as_deref(), Some("shell · cargo test"));
    run(&mut st, vec![xhook(permission(Some("T1"), 1_500, "npm test")), xhook(ups("T2", 2_000))]);
    let s = st.session(&xkey()).unwrap();
    assert_eq!((s.activity.as_deref(), s.detail.as_deref()), (None, None));
}

// ── a Codex permission dialog inferred from an unanswered request_permissions call (spec §7) ──

/// A Codex session in Working: turn T1, started by a hook at 1 000.
fn working_codex() -> Store {
    let mut st = Store::new();
    run(&mut st, vec![xhook(ups("T1", 1_000))]);
    st
}

/// The `call_id`s of the unanswered request_permissions calls the current wait was inferred from.
fn pending_calls(s: &Session) -> Vec<&str> {
    s.pending_permission_calls.iter().map(String::as_str).collect()
}

#[test]
fn an_unanswered_request_permissions_call_waits_for_permission() {
    let mut st = working_codex();
    let applied = st.apply(xroll(vec![permission_ask("call_9")]), 5_000);
    let s = st.session(&xkey()).unwrap();
    assert_eq!(s.phase, waiting(WaitKind::Permission));
    assert_eq!(s.since_ms, 5_000);
    assert_eq!(s.detail.as_deref(), Some("request_permissions"));
    assert_eq!(s.activity.as_deref(), Some("exec"));
    assert_eq!(s.wait_key, Some(("T1".into(), "request_permissions".into(), fnv1a64(b"call_9"))));
    assert_eq!(pending_calls(s), ["call_9"]);
    assert_eq!(
        applied.alerts,
        [Alert {
            key: xkey(),
            kind: AttentionKind::Permission,
            rank: 0,
            agent: Source::Codex,
            host: HostKind::Cursor,
            project: "Beta".into(),
            title: None,
            detail: Some("request_permissions".into()),
            text: "Codex · Beta (Cursor) needs permission: request_permissions".into(),
            since: 5_000,
        }]
    );
    assert_eq!(st.snapshot(NOW).attention, [xkey()]);
}

#[test]
fn a_request_permissions_call_answered_in_the_same_batch_never_waits() {
    for batch in [
        vec![permission_ask("call_9"), call_output("call_9")],
        vec![call_output("call_9"), permission_ask("call_9")],
    ] {
        let mut st = working_codex();
        let applied = st.apply(xroll(batch), 5_000);
        let s = st.session(&xkey()).unwrap();
        assert_eq!(s.phase, Phase::Working);
        assert_eq!(s.since_ms, 1_000);
        assert_eq!(s.detail, None);
        assert_eq!(s.activity.as_deref(), Some("exec"));
        assert!(pending_calls(s).is_empty());
        assert!(applied.alerts.is_empty());
    }
}

#[test]
fn the_paired_output_in_a_later_batch_ends_the_wait() {
    let mut st = working_codex();
    st.apply(xroll(vec![permission_ask("call_9")]), 5_000);
    let applied = st.apply(xroll(vec![call_output("call_9")]), 8_000);
    let s = st.session(&xkey()).unwrap();
    assert_eq!(s.phase, Phase::Working);
    assert_eq!(s.since_ms, 8_000);
    assert_eq!(s.detail, None);
    assert_eq!(s.wait_key, None);
    assert!(pending_calls(s).is_empty());
    assert!(applied.alerts.is_empty(), "no second alert");
    assert!(st.snapshot(NOW).attention.is_empty());
    // the next unanswered call is a new episode with its own alert
    let next = st.apply(xroll(vec![permission_ask("call_10")]), 9_000);
    assert_eq!(kinds(&next), [AttentionKind::Permission]);
    let s = st.session(&xkey()).unwrap();
    assert_eq!((s.phase.clone(), s.since_ms), (waiting(WaitKind::Permission), 9_000));
}

#[test]
fn an_output_of_another_call_does_not_resume() {
    let mut st = working_codex();
    st.apply(xroll(vec![permission_ask("call_9")]), 5_000);
    let applied = st.apply(xroll(vec![call_output("call_8")]), 6_000);
    let s = st.session(&xkey()).unwrap();
    assert_eq!(s.phase, waiting(WaitKind::Permission));
    assert_eq!(s.since_ms, 5_000);
    assert_eq!(pending_calls(s), ["call_9"]);
    assert!(applied.alerts.is_empty());
    st.apply(xroll(vec![call_output("call_9")]), 7_000);
    assert_eq!(st.session(&xkey()).unwrap().phase, Phase::Working);
}

#[test]
fn stop_ends_the_inferred_wait_and_forgets_the_call() {
    let mut st = working_codex();
    st.apply(xroll(vec![permission_ask("call_9")]), 5_000);
    let applied = st.apply(xhook(stop(Some("T1"), 6_000, "Done.")), 6_000);
    let s = st.session(&xkey()).unwrap();
    assert_eq!(s.phase, Phase::Idle);
    assert_eq!(s.outcome, Outcome::Finished { summary: "Done.".into(), at_ms: 6_000 });
    assert_eq!(s.wait_key, None);
    assert!(pending_calls(s).is_empty());
    assert_eq!(kinds(&applied), [AttentionKind::Finished]);
    // the late output of the old call changes nothing
    let late = st.apply(xroll(vec![call_output("call_9")]), 7_000);
    assert_eq!(st.session(&xkey()).unwrap().phase, Phase::Idle);
    assert!(late.alerts.is_empty());
}

#[test]
fn the_pending_call_never_outlives_its_turn() {
    let ends = vec![
        ("Interrupt hook", xhook(hook("Interrupt", Some("T1"), 6_000))),
        ("rollout TurnAborted", xroll(vec![turn_aborted(Some("T1"))])),
        ("rollout TaskComplete", xroll(vec![task_complete(Some("T1"), Some("Done."), None)])),
        ("UserPromptSubmit of a new turn", xhook(ups("T2", 6_000))),
        ("rollout TaskStarted of a new turn", xroll(vec![task_started(Some("T2"))])),
    ];
    for (case, end) in ends {
        let mut st = working_codex();
        st.apply(xroll(vec![permission_ask("call_9")]), 5_000);
        st.apply(end, 6_000);
        let s = st.session(&xkey()).unwrap();
        assert_ne!(s.phase, waiting(WaitKind::Permission), "{case}");
        assert_eq!(s.wait_key, None, "{case}");
        assert!(pending_calls(s).is_empty(), "{case}");
    }
}

#[test]
fn a_request_permissions_call_waits_only_from_working() {
    // Idle: only the activity changes
    let mut st = Store::new();
    run(&mut st, vec![xhook(stop(Some("T1"), 1_000, "First."))]);
    let applied = st.apply(xroll(vec![permission_ask("call_9")]), 5_000);
    let s = st.session(&xkey()).unwrap();
    assert_eq!((s.phase.clone(), s.since_ms), (Phase::Idle, 1_000));
    assert_eq!(s.activity.as_deref(), Some("exec"));
    assert!(pending_calls(s).is_empty());
    assert!(applied.alerts.is_empty());

    // already Waiting for a hook permission: only the activity changes
    let mut st = working_codex();
    st.apply(xhook(permission(Some("T1"), 2_000, "npm test")), 2_000);
    let hook_key = st.session(&xkey()).unwrap().wait_key.clone();
    let applied = st.apply(xroll(vec![permission_ask("call_9")]), 5_000);
    let s = st.session(&xkey()).unwrap();
    assert_eq!((s.phase.clone(), s.since_ms), (waiting(WaitKind::Permission), 2_000));
    assert_eq!(s.detail.as_deref(), Some("shell · npm test"));
    assert_eq!(s.wait_key, hook_key);
    assert_eq!(s.activity.as_deref(), Some("exec"));
    assert!(pending_calls(s).is_empty());
    assert!(applied.alerts.is_empty());
}

#[test]
fn the_paired_output_does_not_end_a_wait_that_a_hook_took_over() {
    let mut st = working_codex();
    st.apply(xroll(vec![permission_ask("call_9")]), 5_000);
    let hook = st.apply(xhook(permission(Some("T1"), 5_500, "npm test")), 5_500);
    assert_eq!(kinds(&hook), [AttentionKind::Permission]);
    let applied = st.apply(xroll(vec![call_output("call_9")]), 6_000);
    let s = st.session(&xkey()).unwrap();
    assert_eq!(s.phase, waiting(WaitKind::Permission));
    assert_eq!(s.detail.as_deref(), Some("shell · npm test"));
    assert!(pending_calls(s).is_empty());
    assert!(applied.alerts.is_empty());
}

#[test]
fn several_unanswered_asks_keep_the_wait_until_the_last_is_answered() {
    let mut st = working_codex();
    let first = st.apply(xroll(vec![permission_ask("call_8")]), 5_000);
    assert_eq!(kinds(&first), [AttentionKind::Permission]);
    // A second ask while the first one is open joins the same episode: no second alert.
    let second = st.apply(xroll(vec![permission_ask("call_9")]), 6_000);
    assert!(second.alerts.is_empty(), "no second alert");
    let s = st.session(&xkey()).unwrap();
    assert_eq!((s.phase.clone(), s.since_ms), (waiting(WaitKind::Permission), 5_000));
    assert_eq!(pending_calls(s), ["call_8", "call_9"]);
    assert_eq!(s.wait_key, Some(("T1".into(), "request_permissions".into(), fnv1a64(b"call_8"))));
    // The first answer is not enough…
    let applied = st.apply(xroll(vec![call_output("call_8")]), 7_000);
    let s = st.session(&xkey()).unwrap();
    assert_eq!((s.phase.clone(), s.since_ms), (waiting(WaitKind::Permission), 5_000));
    assert_eq!(s.detail.as_deref(), Some("request_permissions"));
    assert_eq!(pending_calls(s), ["call_9"]);
    assert!(applied.alerts.is_empty());
    assert_eq!(st.snapshot(NOW).attention, [xkey()]);
    // …the last one resumes the turn.
    let applied = st.apply(xroll(vec![call_output("call_9")]), 8_000);
    let s = st.session(&xkey()).unwrap();
    assert_eq!((s.phase.clone(), s.since_ms), (Phase::Working, 8_000));
    assert_eq!((s.detail.as_deref(), s.wait_key.clone()), (None, None));
    assert!(pending_calls(s).is_empty());
    assert!(applied.alerts.is_empty());
    assert!(st.snapshot(NOW).attention.is_empty());
}

#[test]
fn asks_answered_out_of_order_or_in_one_batch_wait_once() {
    // Two asks in one batch: one episode, one alert; answered newest first.
    let mut st = working_codex();
    let applied = st.apply(xroll(vec![permission_ask("call_8"), permission_ask("call_9")]), 5_000);
    assert_eq!(kinds(&applied), [AttentionKind::Permission]);
    assert_eq!(pending_calls(st.session(&xkey()).unwrap()), ["call_8", "call_9"]);
    st.apply(xroll(vec![call_output("call_9")]), 6_000);
    assert_eq!(st.session(&xkey()).unwrap().phase, waiting(WaitKind::Permission));
    st.apply(xroll(vec![call_output("call_8")]), 7_000);
    assert_eq!(st.session(&xkey()).unwrap().phase, Phase::Working);

    // An answer to the first call and a new ask in one batch: the wait goes on with the new ask.
    let mut st = working_codex();
    st.apply(xroll(vec![permission_ask("call_8")]), 5_000);
    let applied = st.apply(xroll(vec![permission_ask("call_9"), call_output("call_8")]), 6_000);
    let s = st.session(&xkey()).unwrap();
    assert_eq!((s.phase.clone(), s.since_ms), (waiting(WaitKind::Permission), 5_000));
    assert_eq!(pending_calls(s), ["call_9"]);
    assert!(applied.alerts.is_empty());
}

#[test]
fn pending_asks_are_a_set_of_at_most_eight_oldest_dropped() {
    let mut st = working_codex();
    for i in 1..=9 {
        st.apply(xroll(vec![permission_ask(&format!("call_{i}"))]), 5_000 + i);
    }
    // the same call read twice is one entry
    st.apply(xroll(vec![permission_ask("call_9")]), 6_000);
    let s = st.session(&xkey()).unwrap();
    assert_eq!(pending_calls(s), ["call_2", "call_3", "call_4", "call_5", "call_6", "call_7", "call_8", "call_9"]);
    assert_eq!(s.phase, waiting(WaitKind::Permission));
    // the forgotten call changes nothing when its output arrives…
    st.apply(xroll(vec![call_output("call_1")]), 7_000);
    assert_eq!(pending_calls(st.session(&xkey()).unwrap()).len(), 8);
    assert_eq!(st.session(&xkey()).unwrap().phase, waiting(WaitKind::Permission));
    // …and the wait ends with the answer of the last remembered one.
    for i in 2..=8 {
        st.apply(xroll(vec![call_output(&format!("call_{i}"))]), 8_000 + i);
        assert_eq!(st.session(&xkey()).unwrap().phase, waiting(WaitKind::Permission), "call_{i} answered");
    }
    st.apply(xroll(vec![call_output("call_9")]), 9_000);
    assert_eq!(st.session(&xkey()).unwrap().phase, Phase::Working);
}

#[test]
fn nine_asks_in_one_batch_raise_one_alert_even_though_the_first_is_forgotten() {
    let mut st = working_codex();
    let batch: Vec<Value> = (1..=9).map(|i| permission_ask(&format!("call_{i}"))).collect();
    let applied = st.apply(xroll(batch), 5_000);
    assert_eq!(kinds(&applied), [AttentionKind::Permission]);
    let s = st.session(&xkey()).unwrap();
    assert_eq!(s.phase, waiting(WaitKind::Permission));
    assert_eq!((pending_calls(s).len(), pending_calls(s)[0]), (8, "call_2"));
}

#[test]
fn an_ask_in_a_batch_that_ends_the_turn_raises_no_permission_alert() {
    let ends: Vec<(&str, Value, Phase, Vec<AttentionKind>)> = vec![
        ("TaskComplete", task_complete(Some("T1"), Some("Done."), None), Phase::Idle, vec![AttentionKind::Finished]),
        ("TurnAborted", turn_aborted(Some("T1")), Phase::Idle, vec![]),
        ("TaskStarted of a new turn", task_started(Some("T2")), Phase::Working, vec![]),
    ];
    for (case, end, phase, alerts) in ends {
        let mut st = working_codex();
        let applied = st.apply(xroll(vec![permission_ask("call_9"), end]), 5_000);
        assert_eq!(kinds(&applied), alerts, "{case}");
        let s = st.session(&xkey()).unwrap();
        assert_eq!(s.phase, phase, "{case}");
        assert_eq!(s.wait_key, None, "{case}");
        assert!(pending_calls(s).is_empty(), "{case}");
        // the output of the old call arrives later: still nothing to resume
        let late = st.apply(xroll(vec![call_output("call_9")]), 6_000);
        assert!(late.alerts.is_empty(), "{case}");
        assert_eq!(st.session(&xkey()).unwrap().phase, phase, "{case}");
    }
}

#[test]
fn a_hook_wait_of_a_new_turn_forgets_the_pending_calls_of_the_old_one() {
    // The PermissionRequest of a NEW turn runs start_turn before it waits. Whatever tool it names, the
    // late output of the old turn's call must neither stay remembered nor end the hook wait.
    for tool in ["shell", "request_permissions"] {
        let mut st = working_codex();
        st.apply(xroll(vec![permission_ask("call_9")]), 5_000);
        assert_eq!(pending_calls(st.session(&xkey()).unwrap()), ["call_9"], "{tool}");

        let mut ask = permission(Some("T2"), 6_000, "npm test");
        ask.tool_name = Some(tool.into());
        let hooked = st.apply(xhook(ask), 6_000);
        assert_eq!(kinds(&hooked), [AttentionKind::Permission], "{tool}");
        let s = st.session(&xkey()).unwrap();
        assert_eq!((s.phase.clone(), s.current_turn.as_deref()), (waiting(WaitKind::Permission), Some("T2")), "{tool}");
        assert_eq!(s.wait_key.as_ref().map(|(turn, name, _)| (turn.as_str(), name.as_str())), Some(("T2", tool)));
        assert!(pending_calls(s).is_empty(), "{tool}: the old ids are gone");

        let late = st.apply(xroll(vec![call_output("call_9")]), 7_000);
        let s = st.session(&xkey()).unwrap();
        assert_eq!(s.phase, waiting(WaitKind::Permission), "{tool}: the hook wait is not resumed");
        assert_eq!(s.since_ms, 6_000, "{tool}");
        assert!(late.alerts.is_empty(), "{tool}");
    }
}

/// The ticker runs while a Codex turn runs, hidden island or not (spec §4, §7): the call and the
/// answer of this dialog exist only in the rollout, so a hidden island is told about the wait by
/// the first tail read.
#[test]
fn an_inferred_wait_raised_while_the_island_is_hidden_alerts_and_waits() {
    let mut st = Store::new(); // hidden, and it stays hidden
    let started = st.apply(xhook(ups("T1", 1_000)), 1_000);
    assert_eq!(ticker_effects(&started), [Effect::CodexTicker(true)], "hidden, yet the tail is read");
    let asked = st.apply(xroll(vec![permission_ask("call_9")]), 3_000);
    assert_eq!(kinds(&asked), [AttentionKind::Permission]);
    assert_eq!(ticker_effects(&asked), [], "Waiting is busy too: the ticker stays on");
    let s = st.session(&xkey()).unwrap();
    assert_eq!((s.phase.clone(), s.since_ms), (waiting(WaitKind::Permission), 3_000));
    assert_eq!(st.snapshot(NOW).attention, [xkey()]);
    // the answer comes through the same ticker
    let answered = st.apply(xroll(vec![call_output("call_9")]), 4_000);
    assert!(answered.alerts.is_empty());
    assert_eq!(st.session(&xkey()).unwrap().phase, Phase::Working);
}

#[test]
fn codex_closed_turns_keep_the_last_eight() {
    let mut st = Store::new();
    for i in 0..10i64 {
        let turn = format!("T{i}");
        run(&mut st, vec![xhook(ups(&turn, 1_000 + i * 10)), xhook(stop(Some(&turn), 1_005 + i * 10, "ok"))]);
    }
    let closed: Vec<&str> = st.session(&xkey()).unwrap().closed_turns.iter().map(String::as_str).collect();
    assert_eq!(closed, ["T2", "T3", "T4", "T5", "T6", "T7", "T8", "T9"]);
}

#[test]
fn codex_hooks_status_follows_installs_and_events() {
    let mut st = Store::new();
    let hooks = |st: &Store| st.snapshot(NOW).sources.codex_hooks;
    assert_eq!(hooks(&st), CodexHooksView::NotInstalled);
    run(&mut st, vec![Signal::CodexHooksInstalled(true)]);
    assert_eq!(hooks(&st), CodexHooksView::InstalledUnseen);
    run(&mut st, vec![xhook(ups("T1", 1_000))]);
    assert_eq!(hooks(&st), CodexHooksView::Receiving { hosts: vec![HostKind::Cursor], since: 1_000 });
    run(&mut st, vec![Signal::CodexHook {
        key: codex_key("other"),
        event: hook("UserPromptSubmit", Some("U1"), 2_000),
        proc: None,
        host: HostKind::CodexDesktop,
        project: project("Beta"),
        rollout_eof: None,
        title: None,
    }]);
    run(&mut st, vec![xhook(stop(Some("T1"), 3_000, "Done."))]);
    let receiving = CodexHooksView::Receiving { hosts: vec![HostKind::Cursor, HostKind::CodexDesktop], since: 1_000 };
    assert_eq!(hooks(&st), receiving);
    run(&mut st, vec![Signal::CodexHooksInstalled(true)]);
    assert_eq!(hooks(&st), receiving);
    run(&mut st, vec![Signal::CodexHooksInstalled(false)]);
    assert_eq!(hooks(&st), CodexHooksView::NotInstalled);
    // any hook, even a SessionEnd for an unknown key, proves delivery (D24)
    let mut end = hook("SessionEnd", None, 4_000);
    end.session_id = "gone".into();
    run(&mut st, vec![xhook(end)]);
    assert_eq!(hooks(&st), CodexHooksView::Receiving { hosts: vec![HostKind::Cursor], since: 4_000 });
}

#[test]
fn proc_exit_removes_every_session_of_the_process() {
    let mut st = Store::new();
    let mut second = ups("U1", 1_000);
    second.session_id = "019a8c4e-0000-7000-8000-0000000000b2".into();
    let mut orphan = ups("V1", 1_000);
    orphan.session_id = "019a8c4e-0000-7000-8000-0000000000b3".into();
    run(&mut st, vec![
        xhook(ups("T1", 1_000)),
        xhook(second),
        Signal::CodexHook {
            key: codex_key("019a8c4e-0000-7000-8000-0000000000b3"),
            event: orphan,
            proc: None,
            host: HostKind::Unknown,
            project: project("Beta"),
            rollout_eof: None,
            title: None,
        },
        cfile(sfile("busy", None, 1_000), None),
    ]);
    let exit = run(&mut st, vec![Signal::ProcExit { pid: codex_proc().pid, created_ft: codex_proc().created_ft }]);
    assert_eq!(
        exit.effects,
        [
            Effect::UntrackRollout { key: codex_key("019a8c4e-0000-7000-8000-0000000000b2") },
            Effect::UntrackRollout { key: xkey() },
        ]
    );
    assert!(exit.alerts.is_empty());
    assert!(st.session(&xkey()).is_none());
    assert!(st.session(&codex_key("019a8c4e-0000-7000-8000-0000000000b3")).is_some());
    assert!(st.session(&key()).is_some());
}

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

#[test]
fn the_snapshot_carries_the_desktop_chat_id_but_not_on_the_wire() {
    let mut st = Store::new();
    let file = ClaudeSessionFile { host_session_id: Some("local_abc-1".into()), ..sfile("idle", None, 1_000) };
    run(&mut st, vec![cfile(file, None)]);
    let snapshot = st.snapshot(NOW);
    let view = &snapshot.projects[0].sessions[0];
    assert_eq!(view.host_session_id.as_deref(), Some("local_abc-1"));
    let wire = serde_json::to_value(view).unwrap();
    assert!(wire.get("hostSessionId").is_none() && wire.get("host_session_id").is_none(), "{wire}");
    assert!(!wire.to_string().contains("local_abc-1"), "{wire}");

    // every file read replaces it: a file without the id clears it
    run(&mut st, vec![cfile(sfile("busy", None, 2_000), None)]);
    assert_eq!(st.snapshot(NOW).projects[0].sessions[0].host_session_id, None);
}

// ── Codex lifecycle: Ack removal, ticker ──

/// A CodexHook whose process proof failed (proc: None).
fn xhook_unproven(event: HookEvent) -> Signal {
    Signal::CodexHook {
        key: codex_key(&event.session_id),
        event,
        proc: None,
        host: HostKind::Unknown,
        project: project("Beta"),
        rollout_eof: None,
        title: None,
    }
}

#[test]
fn ack_removes_an_idle_codex_session_without_a_process_until_its_next_hook() {
    let mut st = Store::new();
    run(&mut st, vec![xhook_unproven(stop(Some("T1"), 1_000, "Done."))]);
    let ack = run(&mut st, vec![Signal::Ack(xkey())]);
    assert_eq!(ack, Applied { alerts: vec![], effects: vec![Effect::UntrackRollout { key: xkey() }], wake: false });
    assert!(st.session(&xkey()).is_none());
    // the next hook brings it back
    run(&mut st, vec![xhook_unproven(ups("T2", 2_000))]);
    assert_eq!(st.session(&xkey()).unwrap().phase, Phase::Working);
}

#[test]
fn ack_only_marks_sessions_with_a_process_or_not_idle() {
    // Idle with a proven process: acked, kept
    let mut st = Store::new();
    run(&mut st, vec![xhook(stop(Some("T1"), 1_000, "Done."))]);
    assert_eq!(run(&mut st, vec![Signal::Ack(xkey())]), Applied::default());
    let s = st.session(&xkey()).unwrap();
    assert_eq!(s.acked_at, Some(NOW));
    assert!(st.snapshot(NOW).attention.is_empty());

    // Waiting without a process: acked, kept
    let mut st = Store::new();
    run(&mut st, vec![xhook_unproven(permission(Some("T1"), 1_000, "npm test"))]);
    assert_eq!(run(&mut st, vec![Signal::Ack(xkey())]), Applied::default());
    assert_eq!(st.session(&xkey()).unwrap().phase, waiting(WaitKind::Permission));
    assert_eq!(st.session(&xkey()).unwrap().acked_at, Some(NOW));

    // the removal rule is Codex-only: an Idle Claude row without a process is acked, kept
    let mut st = Store::new();
    put(&mut st, "c", project("Alpha"), Phase::Idle, 100, finished(200), None);
    assert_eq!(run(&mut st, vec![Signal::Ack("c".into())]), Applied::default());
    assert_eq!(st.session("c").unwrap().acked_at, Some(NOW));
}

fn ticker_effects(a: &Applied) -> Vec<Effect> {
    a.effects.iter().filter(|e| matches!(e, Effect::CodexTicker(_))).cloned().collect()
}

/// A hook of a second Codex session (same process).
fn second_codex(mut event: HookEvent) -> HookEvent {
    event.session_id = "019a8c4e-0000-7000-8000-0000000000b2".into();
    event
}

#[test]
fn the_ticker_follows_the_busy_codex_sessions_not_the_island() {
    let mut st = Store::new(); // the island is hidden all the time
    let mut step = |signal: Signal, expected: Vec<Effect>, what: &str| {
        assert_eq!(ticker_effects(&st.apply(signal, NOW)), expected, "{what}");
    };
    step(xhook(ups("T1", 1_000)), vec![Effect::CodexTicker(true)], "a Codex turn starts while hidden");
    step(xhook(permission(Some("T1"), 1_500, "npm test")), vec![], "Working → Waiting stays busy");
    step(xhook(second_codex(ups("U1", 1_600))), vec![], "a second busy session changes nothing");
    step(xhook(stop(Some("T1"), 2_000, "Done.")), vec![], "the other turn still runs");
    step(cfile(sfile("busy", None, 2_100), None), vec![], "a Claude session never turns the Codex ticker on");
    step(xhook(second_codex(stop(Some("U1"), 2_200, "Done."))), vec![Effect::CodexTicker(false)], "the last turn ends");
    step(xhook(ups("T2", 3_000)), vec![Effect::CodexTicker(true)], "the next turn");
    step(xroll(vec![turn_aborted(Some("T2"))]), vec![Effect::CodexTicker(false)], "an abort read from the rollout ends it");
    step(xhook(ups("T3", 4_000)), vec![Effect::CodexTicker(true)], "another turn");
    step(xhook(hook("SessionEnd", None, 5_000)), vec![Effect::CodexTicker(false)], "SessionEnd of the busy session");
}

#[test]
fn toggling_the_island_never_changes_the_ticker() {
    for busy in [false, true] {
        let mut st = Store::new();
        let first = if busy { ups("T1", 1_000) } else { stop(Some("T1"), 1_000, "Done.") };
        st.apply(xhook(first), 1_000);
        for visible in [true, false, true, false] {
            let applied = st.apply(Signal::Visible(visible), 2_000);
            assert_eq!(ticker_effects(&applied), [], "busy={busy}, Visible({visible})");
        }
    }
}

#[test]
fn without_a_busy_codex_session_there_is_no_ticker() {
    let mut st = Store::new();
    // no session at all, then Claude sessions in every phase
    let claude = [
        cfile(sfile("busy", None, 1_000), None),
        cfile(sfile("waiting", Some("permission prompt"), 2_000), None),
        cfile(sfile("idle", None, 3_000), None),
    ];
    for signal in claude {
        assert_eq!(ticker_effects(&st.apply(signal, NOW)), []);
    }
    // Codex sessions that are only Idle: their turns have finished
    for signal in [xhook(stop(Some("T1"), 4_000, "Done.")), xhook(second_codex(stop(Some("U1"), 4_100, "Done.")))] {
        assert_eq!(ticker_effects(&st.apply(signal, NOW)), []);
    }
    assert!(st.session(&xkey()).is_some_and(|s| s.phase == Phase::Idle));
    assert_eq!(ticker_effects(&st.apply(Signal::Visible(true), NOW)), [], "only Idle sessions: nothing to read");
}

#[test]
fn session_end_effects_are_untrack_then_ticker() {
    let mut st = Store::new();
    run(&mut st, vec![Signal::Visible(true), xhook(ups("T1", 1_000))]);
    let end = st.apply(xhook(hook("SessionEnd", None, 2_000)), NOW);
    assert_eq!(end.effects, [Effect::UntrackRollout { key: xkey() }, Effect::CodexTicker(false)]);
    // Visible is not part of the snapshot
    let rev = st.rev();
    st.apply(Signal::Visible(false), NOW);
    assert_eq!(st.rev(), rev);
}

// ── silent Codex rows (spec silent-codex §3) ──

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
    run(&mut st, vec![Signal::CodexHooksInstalled(true)]);
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
    assert_eq!(
        (snap.counts.clone(), snap.hold, snap.aggregate, snap.attention.clone()),
        (before.counts, before.hold, before.aggregate, before.attention)
    );
}

#[test]
fn the_row_dies_exactly_two_minutes_after_the_last_write() {
    let mut st = Store::new();
    run(&mut st, vec![silent("s-1", NOW)]);
    assert_eq!(silent_rows(&st.snapshot(NOW + 120_000)).len(), 1);
    assert!(silent_rows(&st.snapshot(NOW + 120_001)).is_empty());
    // the next apply drops the expired row from the store, not only from the view
    assert_eq!(st.silent.len(), 1);
    st.apply(Signal::Visible(true), NOW + 120_001);
    assert!(st.silent.is_empty());
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
    run(
        &mut st,
        vec![Signal::CodexSilent {
            id: "s-2".into(),
            project: project("Gamma"),
            host: HostKind::Unknown,
            folder_exists: false,
            written_ms: NOW,
        }],
    );
    let snap = st.snapshot(NOW);
    assert!(silent_rows(&snap).iter().any(|s| s.silent == Some(SilentReason::FolderMissing)));
}

#[test]
fn a_real_hook_replaces_the_silent_row_and_later_silent_signals_are_ignored() {
    let mut st = Store::new();
    run(&mut st, vec![silent(CSID, NOW)]); // CSID is the session id `ups(..)` events carry
    run(&mut st, vec![xhook(ups("T1", 1_000))]);
    let snap = st.snapshot(NOW);
    assert!(silent_rows(&snap).is_empty());
    run(&mut st, vec![silent(CSID, NOW + 1)]);
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

#[test]
fn a_new_silent_row_wakes_the_island_once_and_a_refresh_does_not() {
    let mut st = Store::new();
    let first = run(&mut st, vec![silent("s-1", NOW)]);
    assert!(first.wake);
    assert!(first.alerts.is_empty() && first.effects.is_empty(), "a wake is no alert and no effect");
    assert!(!run(&mut st, vec![silent("s-1", NOW + 5)]).wake, "a refresh");
    assert!(run(&mut st, vec![silent("s-2", NOW + 6)]).wake, "another id is new");
}

#[test]
fn a_silent_row_that_expired_wakes_again() {
    let mut st = Store::new();
    assert!(st.apply(silent("s-1", NOW), NOW).wake);
    assert!(!st.apply(silent("s-1", NOW + 100_000), NOW + 100_000).wake);
    // The row was last written at NOW + 100_000: alive until +120_000 after that.
    assert!(!st.apply(silent("s-1", NOW + 220_000), NOW + 220_000).wake, "exactly 120 000 ms: still alive");
    let late = NOW + 220_000 + 120_001;
    assert!(st.apply(silent("s-1", late), late).wake, "expired, then back");
}

#[test]
fn a_silent_signal_for_a_real_session_does_not_wake() {
    let mut st = Store::new();
    run(&mut st, vec![xhook(ups("T1", 1_000))]);
    assert!(!run(&mut st, vec![silent(CSID, NOW)]).wake);
}

// ── a hooked Codex session whose turn ends without hooks (silent-codex Task 7) ──

/// `silent` of the real row `codex:<CSID>` in the snapshot at `now`.
fn real_silent(st: &Store, now: i64) -> Option<SilentReason> {
    let snap = st.snapshot(now);
    let row = snap.projects.iter().flat_map(|p| p.sessions.iter()).find(|s| s.key == xkey()).cloned();
    row.expect("the real session row").silent
}

/// The user's prompt as Codex writes it in a user turn: a `response_item` message, role user.
fn codex_user() -> Value {
    response_item(json!({ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }))
}

/// A session known from its hooks (T1 done), then a whole user turn T2 that only the rollout shows.
fn hooked_then_silent_turn(st: &mut Store) -> Applied {
    run(st, vec![xhook(ups("T1", 1_000)), xhook(stop(Some("T1"), 2_000, "first"))]);
    run(
        st,
        vec![
            xroll(vec![task_started(Some("T2")), codex_user()]),
            xroll(vec![task_complete(Some("T2"), Some("second"), None)]),
        ],
    )
}

#[test]
fn a_turn_without_a_user_message_is_never_silent() {
    // A Codex-internal task (no user prompt) runs no hook: it is not a silent turn.
    let mut st = Store::new();
    run(&mut st, vec![xhook(ups("T1", 1_000)), xhook(stop(Some("T1"), 2_000, "first"))]);
    let out = run(&mut st, vec![xroll(vec![task_started(Some("T2"))]), xroll(vec![task_complete(Some("T2"), None, None)])]);
    assert!(!out.wake);
    assert_eq!(real_silent(&st, NOW), None);
    let out = run(&mut st, vec![xroll(vec![task_started(Some("T3")), turn_aborted(Some("T3"))])]);
    assert!(!out.wake);
    assert_eq!(real_silent(&st, NOW), None);
    // A user message of the previous turn does not count for the next one.
    let out = run(&mut st, vec![xroll(vec![codex_user(), task_started(Some("T4")), task_complete(Some("T4"), None, None)])]);
    assert!(!out.wake);
    assert_eq!(real_silent(&st, NOW), None);
}

#[test]
fn a_hooked_turn_in_the_real_order_is_never_silent() {
    // Measured order: task_started, then UserPromptSubmit, then Stop, then task_complete.
    let mut st = Store::new();
    run(&mut st, vec![xhook(ups("T1", 1_000)), xhook(stop(Some("T1"), 2_000, "first"))]);
    let out = run(
        &mut st,
        vec![
            xroll(vec![task_started(Some("T2"))]),
            xhook(ups("T2", 3_000)),
            xroll(vec![codex_user()]),
            xhook(stop(Some("T2"), 4_000, "second")),
            xroll(vec![task_complete(Some("T2"), Some("second"), None)]),
        ],
    );
    assert!(!out.wake);
    assert_eq!(real_silent(&st, NOW), None);
    assert_eq!(kinds(&out), [AttentionKind::Finished], "the Stop alert only, the rollout raises no second one");
}

#[test]
fn a_turn_of_a_known_session_that_ends_without_a_hook_is_silent_and_wakes() {
    let mut st = Store::new();
    let out = hooked_then_silent_turn(&mut st);
    assert!(out.wake);
    assert_eq!(real_silent(&st, NOW), Some(SilentReason::HooksNotReaching));
    // The row is the real session: no `silent:<id>` row, and its turn still finished.
    let snap = st.snapshot(NOW);
    assert_eq!(silent_rows(&snap).len(), 1);
    assert_eq!(st.session(&xkey()).unwrap().phase, Phase::Idle);
}

#[test]
fn a_turn_started_and_ended_in_one_batch_without_a_hook_is_silent() {
    let mut st = Store::new();
    run(&mut st, vec![xhook(ups("T1", 1_000)), xhook(stop(Some("T1"), 2_000, "first"))]);
    let out = run(&mut st, vec![xroll(vec![task_started(Some("T2")), codex_user(), turn_aborted(Some("T2"))])]);
    assert!(out.wake);
    assert!(real_silent(&st, NOW).is_some());
}

#[test]
fn a_second_silent_turn_while_the_mark_lives_does_not_wake_again() {
    let mut st = Store::new();
    assert!(hooked_then_silent_turn(&mut st).wake);
    let again = run(&mut st, vec![xroll(vec![task_started(Some("T3")), codex_user(), task_complete(Some("T3"), None, None)])]);
    assert!(!again.wake);
    assert!(real_silent(&st, NOW).is_some());
}

#[test]
fn a_hook_after_the_mark_clears_it() {
    let mut st = Store::new();
    hooked_then_silent_turn(&mut st);
    run(&mut st, vec![xhook(ups("T3", 5_000))]);
    assert_eq!(real_silent(&st, NOW), None);
    // The next silent turn is a new mark: it wakes again.
    let out = run(&mut st, vec![xhook(stop(Some("T3"), 6_000, "third"))]);
    assert!(!out.wake);
    let out = run(&mut st, vec![xroll(vec![task_started(Some("T4")), codex_user(), task_complete(Some("T4"), None, None)])]);
    assert!(out.wake);
}

#[test]
fn the_mark_is_gone_from_the_view_after_two_minutes() {
    let mut st = Store::new();
    hooked_then_silent_turn(&mut st); // marked at NOW
    assert!(real_silent(&st, NOW + 120_000).is_some());
    assert_eq!(real_silent(&st, NOW + 120_001), None);
    // An expired mark wakes again on the next silent turn.
    let late = NOW + 120_001;
    st.apply(xroll(vec![task_started(Some("T3")), codex_user()]), late);
    assert!(st.apply(xroll(vec![task_complete(Some("T3"), None, None)]), late).wake);
}

#[test]
fn a_turn_end_without_a_task_started_seen_is_not_silent() {
    // The records read when the rollout was first bound start after its task_started.
    let mut st = Store::new();
    run(&mut st, vec![xhook(ups("T1", 1_000))]);
    let out = run(&mut st, vec![xroll(vec![codex_user(), task_complete(Some("T1"), Some("done"), None)])]);
    assert!(!out.wake);
    assert_eq!(real_silent(&st, NOW), None);
    let out = run(&mut st, vec![xroll(vec![turn_aborted(None)])]);
    assert!(!out.wake);
    assert_eq!(real_silent(&st, NOW), None);
}

#[test]
fn the_reason_of_a_known_silent_session_follows_the_installer_and_the_folder() {
    let mut st = Store::new();
    hooked_then_silent_turn(&mut st);
    assert_eq!(real_silent(&st, NOW), Some(SilentReason::HooksNotReaching));
    run(&mut st, vec![Signal::CodexHooksInstalled(false)]);
    assert_eq!(real_silent(&st, NOW), Some(SilentReason::HooksNotInstalled));
    run(&mut st, vec![Signal::CodexHooksInstalled(true)]);
    // The folder fact of the batch that ended the turn counts.
    run(&mut st, vec![xroll(vec![task_started(Some("T3")), codex_user()]), xroll_folder(false, vec![task_complete(Some("T3"), None, None)])]);
    assert_eq!(real_silent(&st, NOW), Some(SilentReason::FolderMissing));
    run(&mut st, vec![Signal::CodexHooksInstalled(false)]);
    assert_eq!(real_silent(&st, NOW), Some(SilentReason::HooksNotInstalled), "checked from the top");
}

// ── context fill (spec context-fill §2-§3) ──

/// A rollout `token_count` event: `used` as the last request's total, `window` when given.
fn token_count(used: u64, window: Option<u64>) -> Value {
    let mut info = json!({ "last_token_usage": { "total_tokens": used } });
    if let Some(w) = window {
        info["model_context_window"] = json!(w);
    }
    event_msg(json!({ "type": "token_count", "info": info, "rate_limits": {} }))
}

/// The context of the snapshot row `key`.
fn context_of(st: &Store, key: &str) -> Option<ContextFill> {
    st.snapshot(NOW).projects.iter().flat_map(|p| p.sessions.iter()).find(|s| s.key == key).and_then(|s| s.context)
}

#[test]
fn a_token_count_sets_the_codex_context_and_a_smaller_one_replaces_it() {
    let mut st = Store::new();
    run(&mut st, vec![xhook(ups("T1", 1_000))]);
    assert_eq!(context_of(&st, &xkey()), None, "no value before the first token_count");
    run(&mut st, vec![xroll(vec![token_count(200_000, Some(258_400))])]);
    assert_eq!(context_of(&st, &xkey()), Some(ContextFill { used: 200_000, window: Some(258_400) }));
    // after a compaction the number drops: the newest value wins
    run(&mut st, vec![xroll(vec![token_count(30_000, Some(258_400))])]);
    assert_eq!(context_of(&st, &xkey()), Some(ContextFill { used: 30_000, window: Some(258_400) }));
    // the last token_count of a batch counts; a null info after it changes nothing
    let null_info = event_msg(json!({ "type": "token_count", "info": null, "rate_limits": {} }));
    run(&mut st, vec![xroll(vec![token_count(40_000, None), token_count(41_000, Some(258_400)), null_info])]);
    assert_eq!(context_of(&st, &xkey()), Some(ContextFill { used: 41_000, window: Some(258_400) }));
}

#[test]
fn token_counts_change_nothing_but_the_context() {
    let mut st = Store::new();
    run(&mut st, vec![xhook(ups("T1", 1_000))]);
    let before = st.snapshot(NOW);
    let rev = st.rev();
    let out = run(&mut st, vec![xroll(vec![token_count(120_000, Some(258_400))])]);
    assert!(out.alerts.is_empty() && out.effects.is_empty() && !out.wake, "{out:?}");
    assert_eq!(st.rev(), rev + 1, "a new value alone republishes the snapshot");
    let after = st.snapshot(NOW);
    assert_eq!(
        (after.counts.clone(), after.hold, after.aggregate, after.attention.clone()),
        (before.counts.clone(), before.hold, before.aggregate, before.attention.clone())
    );
    let (b, a) = (&before.projects[0].sessions[0], &after.projects[0].sessions[0]);
    assert_eq!(
        (&a.phase, a.since, &a.outcome, a.attention, a.acked, a.silent),
        (&b.phase, b.since, &b.outcome, b.attention, b.acked, b.silent)
    );
    // the same value again is no change
    run(&mut st, vec![xroll(vec![token_count(120_000, Some(258_400))])]);
    assert_eq!(st.rev(), rev + 1);
}

#[test]
fn the_context_is_on_the_wire_only_when_known() {
    let mut st = Store::new();
    run(&mut st, vec![xhook(ups("T1", 1_000))]);
    let wire = |st: &Store| serde_json::to_value(&st.snapshot(NOW).projects[0].sessions[0]).unwrap();
    assert!(wire(&st).get("context").is_none(), "{}", wire(&st));
    run(&mut st, vec![xroll(vec![token_count(612_000, Some(1_000_000))])]);
    assert_eq!(wire(&st)["context"], json!({ "used": 612_000, "window": 1_000_000 }));
    run(&mut st, vec![xroll(vec![token_count(612_000, None)])]);
    assert_eq!(wire(&st)["context"], json!({ "used": 612_000 }), "no window key without a window");
}

#[test]
fn a_silent_row_has_no_context() {
    let mut st = Store::new();
    run(&mut st, vec![silent("s-1", NOW)]);
    let snap = st.snapshot(NOW);
    let rows = silent_rows(&snap);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].context, None);
    assert!(serde_json::to_value(rows[0]).unwrap().get("context").is_none());
}

/// A transcript record with a `message.usage` whose four counted fields sum to `used` (>= 10).
fn with_usage(mut record: Value, used: u64) -> Value {
    record["message"]["usage"] = json!({
        "input_tokens": 2, "cache_creation_input_tokens": 0, "cache_read_input_tokens": used - 10, "output_tokens": 8
    });
    record
}

#[test]
fn a_claude_reply_sets_the_token_count_without_a_window() {
    let mut st = Store::new();
    run(&mut st, prelude(Start::Working));
    assert_eq!(context_of(&st, &key()), None, "the prelude records carry no usage");
    run(&mut st, vec![ctr(
        None,
        vec![
            (400, with_usage(tool("t2", "Bash", json!({ "command": "ls" })), 600_000)),
            (500, result("t2")),
            (600, with_usage(reply("Done.", "end_turn"), 612_000)),
        ],
        700,
    )]);
    assert_eq!(context_of(&st, &key()), Some(ContextFill { used: 612_000, window: None }));
    // after /compact the number drops: the newest value wins
    run(&mut st, vec![ctr(None, vec![(700, with_usage(reply("Compacted.", "end_turn"), 40_000))], 800)]);
    assert_eq!(context_of(&st, &key()), Some(ContextFill { used: 40_000, window: None }));
}

#[test]
fn api_errors_and_subagent_records_keep_the_last_value() {
    let mut st = Store::new();
    run(&mut st, prelude(Start::Working));
    run(&mut st, vec![ctr(None, vec![(400, with_usage(reply("Step one.", "end_turn"), 300_000))], 500)]);
    let mut error = api_error("rate_limit", 429, "API Error: Rate limit reached.");
    error["message"]["model"] = json!("<synthetic>");
    error["message"]["usage"] =
        json!({ "input_tokens": 0, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0, "output_tokens": 0 });
    let mut side = with_usage(reply("Subagent answer.", "end_turn"), 9_000);
    side["isSidechain"] = json!(true);
    run(&mut st, vec![ctr(None, vec![(500, error), (600, side)], 700)]);
    assert_eq!(context_of(&st, &key()), Some(ContextFill { used: 300_000, window: None }));
}

#[test]
fn a_new_session_id_drops_the_old_value_and_stray_old_records_do_not_bring_it_back() {
    let mut st = Store::new();
    run(&mut st, prelude(Start::Working));
    run(&mut st, vec![ctr(None, vec![(400, with_usage(reply("Old.", "end_turn"), 700_000))], 500)]);
    assert_eq!(context_of(&st, &key()).map(|c| c.used), Some(700_000));
    let mut next = sfile("busy", None, 1_000);
    next.session_id = Some("0f8b1d2e-0000-4000-8000-000000000002".into());
    run(&mut st, vec![cfile(next, Some(500))]);
    assert_eq!(context_of(&st, &key()), None, "/clear starts an empty context");
    // a record of the old transcript still in flight is ignored (D52)
    run(&mut st, vec![ctr(None, vec![(500, with_usage(reply("Old again.", "end_turn"), 710_000))], 600)]);
    assert_eq!(context_of(&st, &key()), None);
    // the new transcript's first answer brings a value back
    run(&mut st, vec![ctr(Some(0), vec![(0, user("fresh start")), (80, with_usage(reply("Hi.", "end_turn"), 21_000))], 160)]);
    assert_eq!(context_of(&st, &key()), Some(ContextFill { used: 21_000, window: None }));
}

#[test]
fn the_window_read_at_binding_gives_the_value_at_once() {
    let mut st = Store::new();
    run(&mut st, vec![
        cfile(sfile("idle", None, 1_000), None),
        // the 256 KiB window of an existing transcript, before turn_start_offset = 400
        ctr(Some(400), vec![(0, user("q")), (300, with_usage(reply("Earlier answer.", "end_turn"), 150_000))], 400),
    ]);
    assert_eq!(context_of(&st, &key()), Some(ContextFill { used: 150_000, window: None }));
    assert!(st.session(&key()).unwrap().facts.is_empty(), "the window still adds no facts");
}

#[test]
fn claude_usage_changes_nothing_but_the_context() {
    let records = |counted: bool| {
        let r = reply("Build finished.", "end_turn");
        vec![(400, if counted { with_usage(r, 90_000) } else { r })]
    };
    let mut plain = Store::new();
    let mut counted = Store::new();
    run(&mut plain, prelude(Start::Working));
    run(&mut counted, prelude(Start::Working));
    let a = run(&mut plain, vec![ctr(None, records(false), 500), cfile(sfile("idle", None, 2_000), Some(500))]);
    let b = run(&mut counted, vec![ctr(None, records(true), 500), cfile(sfile("idle", None, 2_000), Some(500))]);
    assert_eq!(a, b, "the same alerts and effects");
    assert_eq!(counted.rev(), plain.rev() + 1, "the value is one more snapshot change");
    let mut with = counted.snapshot(NOW);
    assert_eq!(with.projects[0].sessions[0].context.take(), Some(ContextFill { used: 90_000, window: None }));
    let mut without = plain.snapshot(NOW);
    without.rev = with.rev;
    assert_eq!(with, without, "everything else is equal");
}

// ── Codex usage (usage-home spec §2.2, §3) ──

/// A rollout `token_count` event before the first answer (`info` null) with these `rate_limits`.
fn limits_event(rate_limits: Value) -> Value {
    event_msg(json!({ "type": "token_count", "info": null, "rate_limits": rate_limits }))
}

/// A limit window as Codex writes it (`resets_at` in seconds).
fn limit_window(used_percent: f64, window_minutes: u64, resets_at: i64) -> Value {
    json!({ "used_percent": used_percent, "window_minutes": window_minutes, "resets_at": resets_at })
}

fn codex_usage(st: &Store) -> Option<CodexUsage> {
    st.snapshot(NOW).usage.codex
}

#[test]
fn a_token_count_sets_the_codex_usage_and_the_last_value_wins() {
    let mut st = Store::new();
    run(&mut st, vec![xhook(ups("T1", 1_000))]);
    assert_eq!(codex_usage(&st), None);
    let both = json!({
        "limit_id": "codex", "plan_type": "plus",
        "primary": limit_window(10.0, 300, 1_800_000_000), "secondary": limit_window(40.0, 10_080, 1_800_500_000)
    });
    run(&mut st, vec![xroll(vec![limits_event(both)])]);
    assert_eq!(
        codex_usage(&st),
        Some(CodexUsage {
            seen_at_ms: NOW,
            windows: vec![
                CodexWindow { minutes: 300, percent: 10.0, resets_at_ms: Some(1_800_000_000_000) },
                CodexWindow { minutes: 10_080, percent: 40.0, resets_at_ms: Some(1_800_500_000_000) },
            ],
        })
    );
    // the last record of a batch wins, also with fewer windows
    let early = json!({ "limit_id": "codex", "primary": limit_window(20.0, 300, 1_800_000_000), "secondary": null });
    let late = json!({ "limit_id": "codex", "primary": limit_window(25.0, 10_080, 1_800_600_000), "secondary": null });
    run(&mut st, vec![xroll(vec![limits_event(early), token_count(5_000, Some(258_400)), limits_event(late)])]);
    let expected = Some(CodexUsage {
        seen_at_ms: NOW,
        windows: vec![CodexWindow { minutes: 10_080, percent: 25.0, resets_at_ms: Some(1_800_600_000_000) }],
    });
    assert_eq!(codex_usage(&st), expected);
    // another bucket, an empty one and a token_count without limits change nothing
    let spark = json!({
        "limit_id": "codex_bengalfox", "limit_name": "GPT-5.3-Codex-Spark",
        "primary": limit_window(90.0, 10_080, 1_800_700_000), "secondary": null
    });
    let empty = json!({ "limit_id": "premium", "primary": null, "secondary": null });
    run(&mut st, vec![xroll(vec![limits_event(spark), limits_event(empty), token_count(6_000, None)])]);
    assert_eq!(codex_usage(&st), expected);
}

#[test]
fn a_rollout_batch_of_an_unknown_session_still_sets_the_usage() {
    // The limits belong to the account: a stray batch of a session the store let go still counts.
    let mut st = Store::new();
    let primary = json!({ "limit_id": "codex", "primary": limit_window(7.0, 300, 1_800_000_000) });
    let out = run(&mut st, vec![xroll(vec![limits_event(primary)])]);
    assert!(st.session(&xkey()).is_none(), "no session is created");
    assert_eq!(out, Applied::default());
    assert_eq!(codex_usage(&st).map(|u| u.windows.len()), Some(1));
}

#[test]
fn codex_usage_changes_nothing_but_the_usage() {
    let mut st = Store::new();
    run(&mut st, vec![xhook(ups("T1", 1_000))]);
    let before = st.snapshot(NOW);
    let limits = json!({ "limit_id": "codex", "primary": limit_window(96.0, 300, 1_800_000_000) });
    let out = run(&mut st, vec![xroll(vec![limits_event(limits.clone())])]);
    assert!(out.alerts.is_empty() && out.effects.is_empty() && !out.wake, "{out:?}");
    assert_eq!(st.rev(), before.rev + 1, "a new value republishes the snapshot");
    let mut after = st.snapshot(NOW);
    assert!(after.usage.codex.take().is_some());
    after.rev = before.rev;
    assert_eq!(after, before, "everything else is equal");
    // the same value read again at the same time is no change; later it is a fresher value
    run(&mut st, vec![xroll(vec![limits_event(limits.clone())])]);
    assert_eq!(st.rev(), before.rev + 1);
    st.apply(xroll(vec![limits_event(limits)]), NOW + 60_000);
    assert_eq!(st.rev(), before.rev + 2, "seenAtMs moved");
    assert_eq!(codex_usage(&st).map(|u| u.seen_at_ms), Some(NOW + 60_000));
}

#[test]
fn the_codex_usage_is_on_the_wire() {
    let mut st = Store::new();
    let wire = |st: &Store| serde_json::to_value(st.snapshot(NOW)).unwrap()["usage"]["codex"].clone();
    assert_eq!(wire(&st), Value::Null);
    let limits = json!({
        "primary": limit_window(10.0, 300, 1_800_000_000),
        "secondary": { "used_percent": 40.0, "window_minutes": 10_080 }
    });
    run(&mut st, vec![xroll(vec![limits_event(limits)])]);
    assert_eq!(
        wire(&st),
        json!({ "seenAtMs": 9000, "windows": [
            { "minutes": 300, "percent": 10.0, "resetsAtMs": 1_800_000_000_000i64 },
            { "minutes": 10080, "percent": 40.0 }
        ] }),
        "no limit_id: an older Codex, kept; no resets_at: no key"
    );
}

fn seed(seen_at_ms: i64, percent: f64) -> CodexUsage {
    CodexUsage { seen_at_ms, windows: vec![CodexWindow { minutes: 300, percent, resets_at_ms: None }] }
}

#[test]
fn a_codex_usage_seed_fills_an_empty_usage_with_its_own_time() {
    let mut st = Store::new();
    let out = run(&mut st, vec![Signal::CodexUsageSeed(seed(4_000, 15.0))]);
    assert_eq!(codex_usage(&st), Some(seed(4_000, 15.0)), "seenAtMs is the seed's, not the store clock");
    assert!(out.alerts.is_empty() && out.effects.is_empty() && !out.wake, "{out:?}");
}

#[test]
fn a_later_live_record_replaces_the_codex_usage_seed() {
    let mut st = Store::new();
    run(&mut st, vec![Signal::CodexUsageSeed(seed(4_000, 15.0))]);
    let live = json!({ "limit_id": "codex", "primary": limit_window(60.0, 300, 1_800_000_000) });
    run(&mut st, vec![xroll(vec![limits_event(live)])]);
    let usage = codex_usage(&st).unwrap();
    assert_eq!((usage.seen_at_ms, usage.windows[0].percent), (NOW, 60.0));
}

#[test]
fn a_codex_usage_seed_never_replaces_a_newer_value() {
    let mut st = Store::new();
    let live = json!({ "limit_id": "codex", "primary": limit_window(60.0, 300, 1_800_000_000) });
    run(&mut st, vec![xroll(vec![limits_event(live)])]);
    let before = st.rev();
    for at in [NOW - 1, NOW] {
        run(&mut st, vec![Signal::CodexUsageSeed(seed(at, 15.0))]);
        assert_eq!(codex_usage(&st).map(|u| u.windows[0].percent), Some(60.0), "seed at {at}");
    }
    assert_eq!(st.rev(), before, "nothing changed");
    // a seed that is newer than what is held is the fresher value
    run(&mut st, vec![Signal::CodexUsageSeed(seed(NOW + 1, 15.0))]);
    assert_eq!(codex_usage(&st), Some(seed(NOW + 1, 15.0)));
}

// ── Claude usage (usage-home spec §2.1, §3) ──

fn claude_usage(percent: f64) -> ClaudeUsage {
    ClaudeUsage {
        fetched_at_ms: 8_000,
        windows: vec![
            ClaudeWindow { id: ClaudeWindowId::FiveHour, percent, resets_at_ms: Some(20_000) },
            ClaudeWindow { id: ClaudeWindowId::SevenDay, percent: 40.0, resets_at_ms: None },
        ],
    }
}

#[test]
fn a_claude_usage_signal_sets_and_clears_the_claude_usage() {
    let mut st = Store::new();
    assert_eq!(st.snapshot(NOW).usage.claude, None);
    run(&mut st, vec![Signal::ClaudeUsage(Some(claude_usage(12.0)))]);
    assert_eq!(st.snapshot(NOW).usage.claude, Some(claude_usage(12.0)));
    assert_eq!(st.rev(), 1);
    run(&mut st, vec![Signal::ClaudeUsage(Some(claude_usage(12.0)))]);
    assert_eq!(st.rev(), 1, "the same value is no change");
    run(&mut st, vec![Signal::ClaudeUsage(Some(claude_usage(30.0)))]);
    assert_eq!(st.rev(), 2, "the newest value wins");
    run(&mut st, vec![Signal::ClaudeUsage(None)]);
    assert_eq!(st.snapshot(NOW).usage.claude, None, "the file or the key is gone");
    assert_eq!(st.rev(), 3);
}

#[test]
fn claude_usage_changes_nothing_but_the_usage() {
    let mut st = Store::new();
    run(&mut st, prelude(Start::Working));
    let before = st.snapshot(NOW);
    let out = run(&mut st, vec![Signal::ClaudeUsage(Some(claude_usage(97.0)))]);
    assert_eq!(out, Applied::default(), "no alert, no effect, no wake");
    assert_eq!(st.rev(), before.rev + 1);
    let mut after = st.snapshot(NOW);
    assert_eq!(after.usage.claude.take(), Some(claude_usage(97.0)));
    after.rev = before.rev;
    assert_eq!(after, before, "everything else is equal");
}

#[test]
fn the_claude_usage_is_on_the_wire() {
    let mut st = Store::new();
    run(&mut st, vec![Signal::ClaudeUsage(Some(claude_usage(12.0)))]);
    let wire = serde_json::to_value(st.snapshot(NOW)).unwrap();
    assert_eq!(
        wire["usage"],
        json!({
            "claude": { "fetchedAtMs": 8000, "windows": [
                { "id": "5h", "percent": 12.0, "resetsAtMs": 20000 },
                { "id": "7d", "percent": 40.0 }
            ] },
            "codex": null,
            "openrouter": null
        })
    );
}

// ── OpenRouter usage ──

fn openrouter_usage(limit: Option<f64>) -> OpenRouterUsage {
    OpenRouterUsage {
        fetched_at_ms: 9_000,
        limit,
        limit_remaining: limit.map(|l| l - 2.5),
        limit_reset: limit.map(|_| "monthly".to_string()),
        usage: 7.5,
        usage_daily: 0.5,
        usage_weekly: 2.5,
        usage_monthly: 2.5,
    }
}

#[test]
fn the_openrouter_usage_is_set_cleared_and_changes_nothing_else() {
    let mut st = Store::new();
    let before = st.snapshot(NOW);
    let out = run(&mut st, vec![Signal::OpenRouterUsage(Some(openrouter_usage(Some(20.0))))]);
    assert_eq!(out, Applied::default(), "no alert, no effect, no wake");
    assert_eq!(st.rev(), before.rev + 1);
    let mut after = st.snapshot(NOW);
    assert_eq!(after.usage.openrouter.take(), Some(openrouter_usage(Some(20.0))));
    after.rev = before.rev;
    assert_eq!(after, before, "everything else is equal");
    run(&mut st, vec![Signal::OpenRouterUsage(None)]);
    assert_eq!(st.snapshot(NOW).usage.openrouter, None, "a removed key takes its block away");
}

#[test]
fn the_openrouter_usage_is_on_the_wire() {
    let mut st = Store::new();
    run(&mut st, vec![Signal::OpenRouterUsage(Some(openrouter_usage(None)))]);
    let wire = serde_json::to_value(st.snapshot(NOW)).unwrap();
    assert_eq!(
        wire["usage"]["openrouter"],
        json!({
            "fetchedAtMs": 9000, "limit": null, "limitRemaining": null, "limitReset": null,
            "usage": 7.5, "usageDaily": 0.5, "usageWeekly": 2.5, "usageMonthly": 2.5
        })
    );
}
