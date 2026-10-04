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
            "sources": { "claude": { "degraded": { "unreadable": 1 } }, "codex": "ok", "codexHooks": "not_installed" }
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
    assert_eq!(wire(Health::Error("access denied".into())), json!({ "error": "access denied" }));
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
    assert_eq!(gone, Applied { alerts: vec![], effects: vec![Effect::UntrackTranscript { key: key() }] });
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
