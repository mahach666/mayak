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
