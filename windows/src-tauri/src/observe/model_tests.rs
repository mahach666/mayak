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
