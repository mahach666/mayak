# Mayak: context fill Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every session row of the island shows how full its context is: a percent and a 24×3 px bar for Codex (its rollout writes the window size), a token count (`612k`) for Claude, read only from numbers already present in the rollout and transcript tails Mayak reads today.

**Architecture:** The Codex rollout parser gets a `RolloutRecord::Tokens { used, window }` record from `event_msg` `token_count`; the Claude transcript module gets a pure `context_used` / `last_context_used` over `message.usage`. The model keeps the last value in `Session.context: Option<ContextFill>`, updated in `codex_rollout` and `claude_transcript` (dropped on a Claude `sessionId` change), and copies it into `SessionView.context` (omitted from the JSON when absent). The front end adds a pure `contextLabel` / `formatTokens` next to `silentText` and one new `.sess-ctx` element before the row's time; colours come from the existing `--amber` / `--red` / `--dim-3` tokens.

**Tech Stack:** Rust (serde, serde_json; no new crates), TypeScript (no new packages), Tauri 2.

**Spec:** `docs/superpowers/specs/2026-10-06-mayak-context-fill-design.md` (Russian; new spec text in Russian, LF line endings).

## Global Constraints

- Values from the spec, verbatim:
  - Codex: «процент и тонкая полоска»; Claude: «только число токенов (`612k`)», «угадывать его по модели мы не будем».
  - «**Только заполненность.** Расхода за сессию нет ни в строке, ни в подсказке.»
  - Colour (Codex only): «серый до 80 %», «жёлтый с 80 %», «красный с 95 %»; «Без звука, без уведомлений, без влияния на счётчики и тревоги.»
  - Codex fill: `info.last_token_usage.total_tokens`; window: `info.model_context_window`, «если это положительное целое»; «`info` может быть `null` (до первого ответа). Такая запись не меняет текущее значение.»
  - Claude fill: «`input_tokens + cache_read_input_tokens + cache_creation_input_tokens + output_tokens`. Отсутствующее поле считается за 0. Если нет ни одного из четырёх полей, значения нет.»
  - «Значение — последнее прочитанное. Оно заменяет предыдущее, даже если новое меньше»; «На диск не сохраняется.»
  - `ContextFill { used: u64, window: Option<u64> }`; `#[serde(skip_serializing_if = "Option::is_none")]`; JSON `{"used": 612000, "window": 1000000}`, «при отсутствии окна поле `window` опускается»; «У «немых» строк (`silent:<id>`) поля нет.»
  - Row: label «**перед временем**»; «`62%` + полоска 24×3 px, заполненная на процент»; Claude «`612k`, `1.2M`, `950`», «серый, без полоски».
  - Number format: «меньше 1000 — как есть»; «меньше миллиона — `Nk` (округление вниз до целого)»; «миллион и больше — `N.NM` (одна цифра после точки, округление вниз)».
  - «**Процент** — `floor(used / window × 100)`, не больше 100.»
  - «Метка не кликается и не меняет клик по строке.»; «Новые стили только добавляются (`.sess-ctx`), существующие не меняются.»
- Only numbers are read. Message text is never read for this feature, never stored and never logged. No `last_agent_message`-style copy is added anywhere.
- No new crates, no new packages. No new polling, no timers, no new threads: the values ride on the rollout and transcript reads that already happen.
- Do not restyle existing views: only new CSS selectors; existing rules and elements stay as they are.
- Windows is the only platform touched. Keep the bundle identifier and every pill ID.
- Build environment: always `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target` before cargo. Never touch `windows/target/release`.
- Baseline measured on `feature/context-fill` at `281647f` (2026-10-06): `cargo test --workspace` = lib **409 passed, 1 ignored**; hook **32 passed**; **0 warnings**.
- Front-end checks: `npx tsc --noEmit`, `npx vite build` (there is no front-end test framework; do not add one).
- Keep each file's line endings: every source file touched here is CRLF (checked with `file`); the spec is LF. Edit in place, never rewrite whole files.
- A dev Mayak runs under `tauri dev` from this working tree and rebuilds by itself after each task. Never stop it, never start a second one.
- Commits: conventional, one line, **no trailers** (no Co-Authored-By, no «Generated with»); never mention an AI assistant anywhere (code, comments, docs, PR text). Never push without the user's explicit yes (Task 4 asks). Tests run locally only.
- Never open `*.key`, `ide/*.lock`, `auth.json`, `claude_desktop_config.json`; never edit `~/.codex` or `~/.claude`.

## Review Focus

1. A Claude API error writes a synthetic assistant record (`"model":"<synthetic>"`, `isApiErrorMessage: true`) whose `usage` has all four fields at 0 (41 such main-chain records on the user's machine): it must not turn the label into `0`; the previous value stays (Task 2: `only_main_chain_assistant_records_with_a_real_usage_count`, `api_errors_and_subagent_records_keep_the_last_value`).
2. A subagent record (`isSidechain: true`) carries the subagent's own usage: it must not replace the session's value (Task 2: same two tests).
3. A Codex `token_count` with `info: null`, `info: {}` (the existing fixtures), a `total_tokens` that is negative, fractional or a string, or a `model_context_window` that is 0, negative, fractional, a string or null must neither panic nor become `Unknown` (that would mark the Codex source Degraded); it is `Other`, or `Tokens` with `window: None` (Task 1: the four `token_count` parser tests).
4. A new value changes nothing else (phase, since, outcome, attention, acked, silent, alerts, effects, wake, counts, hold, aggregate), yet it bumps `rev` exactly once, or the island would never redraw the number (Task 1: `token_counts_change_nothing_but_the_context`; Task 2: `claude_usage_changes_nothing_but_the_context`).
5. `/clear` or a resume gives the Claude session a new `sessionId`: the old transcript's number must disappear, and a record of the old transcript still in flight must not bring it back (Task 2: `a_new_session_id_drops_the_old_value_and_stray_old_records_do_not_bring_it_back`).

---

## File Structure

- Modify `windows/src-tauri/src/observe/codex.rs` — `RolloutRecord::Tokens`, its parse arm and helper, parser tests (Task 1).
- Modify `windows/src-tauri/src/observe/model.rs` — `ContextFill`, `Session.context`, `SessionView.context`, `session_view`, `append_silent`, `codex_rollout` (Task 1); `claude_transcript`, `claude_file` (Task 2).
- Modify `windows/src-tauri/src/observe/model_tests.rs` — model tests (Tasks 1-2).
- Modify `windows/src-tauri/src/lib.rs` — the `SessionView` literal of the `session_command_tests` helper (Task 1).
- Modify `windows/src-tauri/src/observe/claude_transcript.rs` — `context_used`, `last_context_used` and their tests (Task 2).
- Modify `windows/src/core/bridge.ts`, `windows/src/views/sessions.ts`, `windows/src/style.css` (Task 3).
- Modify `docs/superpowers/specs/2026-10-06-mayak-context-fill-design.md` — clarifications (Task 4).
- Create `C:\tmp\mayak-spike\ctx.mjs` — acceptance helper outside the repository (Task 4).

**Why four tasks, not the suggested five:** `observe` is a private module (`mod observe;` in `lib.rs`), so rustc's dead-code lint sees the whole crate. A `Tokens` record whose fields nothing reads, or a `Session.context` that no `SessionView` copies, would warn, and the baseline has 0 warnings. So each source is one vertical slice that ends warning-free: Task 1 = Codex parse + the model field and snapshot; Task 2 = Claude parse + its model update. The TS task and the acceptance task are as suggested.

---

### Task 1: Codex `token_count` → `Session.context` → `SessionView.context`

**Files:**
- Modify: `windows/src-tauri/src/observe/codex.rs` (`enum RolloutRecord` ~line 51, `parse_rollout_line` doc ~line 72 and match ~line 107, new helper next to `error_text`, tests module ending ~line 625)
- Modify: `windows/src-tauri/src/observe/model.rs` (wire types after `OutcomeView` ~line 254, `Session` ~line 206, `Session::blank` ~line 844, `SessionView` ~line 293, `append_silent` ~line 709, `session_view` ~line 964, `codex_rollout` ~line 1568)
- Modify: `windows/src-tauri/src/lib.rs` (~line 761)
- Test: `windows/src-tauri/src/observe/codex.rs` (module `tests`), `windows/src-tauri/src/observe/model_tests.rs` (end of file)

**Interfaces:**
- Consumes: `parse_rollout_line(&Value) -> RolloutRecord`; `Signal::CodexRollout { key, path, records: Vec<(u64, Value)>, eof, folder_exists }` (records are raw JSON, parsed in `Store::codex_rollout`); test helpers `run`, `xhook`, `ups`, `xroll`, `xkey`, `event_msg`, `silent`, `silent_rows`, `NOW`.
- Produces (later tasks rely on these exact names):
  - `RolloutRecord::Tokens { used: u64, window: Option<u64> }`
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] pub struct ContextFill { pub used: u64, #[serde(skip_serializing_if = "Option::is_none")] pub window: Option<u64> }` in `observe::model`
  - `Session.context: Option<ContextFill>` (None in `Session::blank`)
  - `SessionView.context: Option<ContextFill>` with `#[serde(skip_serializing_if = "Option::is_none")]`, wire key `context`
  - test helpers in `model_tests.rs`: `fn token_count(used: u64, window: Option<u64>) -> Value`, `fn context_of(st: &Store, key: &str) -> Option<ContextFill>`

- [ ] **Step 1: Write the failing parser tests**

In `codex.rs`, module `tests`, after the test `call_outputs_with_a_string_call_id_are_call_done` (before the module's closing `}`):

```rust
    /// A paginated-mode `token_count` event with the given `info`.
    fn token_count(info: Value) -> RolloutRecord {
        parse_rollout_line(&json!({
            "ordinal": 7, "timestamp": "2026-10-06T10:00:00.000Z", "type": "event_msg",
            "payload": { "type": "token_count", "info": info, "rate_limits": {} }
        }))
    }

    #[test]
    fn a_token_count_with_info_gives_the_fill_and_the_window() {
        // The keys Codex 0.160 writes (numbers generic): only the last request is the fill.
        let info = json!({
            "total_token_usage": { "total_tokens": 900_000 },
            "last_token_usage": {
                "input_tokens": 61_740, "cached_input_tokens": 61_568, "cache_write_input_tokens": 0,
                "output_tokens": 27, "reasoning_output_tokens": 0, "total_tokens": 61_767
            },
            "model_context_window": 258_400
        });
        assert_eq!(token_count(info), RolloutRecord::Tokens { used: 61_767, window: Some(258_400) });
    }

    #[test]
    fn a_token_count_without_info_or_total_tokens_stays_other() {
        assert_eq!(token_count(Value::Null), RolloutRecord::Other, "info null: before the first answer");
        assert_eq!(token_count(json!({})), RolloutRecord::Other, "the empty info of the fixtures");
        assert_eq!(token_count(json!({ "last_token_usage": {}, "model_context_window": 258_400 })), RolloutRecord::Other);
        assert_eq!(
            token_count(json!({ "total_token_usage": { "total_tokens": 5 } })),
            RolloutRecord::Other,
            "the whole-thread total is no fill"
        );
        let absent = parse_rollout_line(&json!({ "type": "event_msg", "payload": { "type": "token_count" } }));
        assert_eq!(absent, RolloutRecord::Other);
    }

    #[test]
    fn a_token_count_window_is_kept_only_when_it_is_a_positive_integer() {
        let with_window = |window: Value| {
            token_count(json!({ "last_token_usage": { "total_tokens": 10 }, "model_context_window": window }))
        };
        let no_window = RolloutRecord::Tokens { used: 10, window: None };
        assert_eq!(token_count(json!({ "last_token_usage": { "total_tokens": 10 } })), no_window);
        for window in [json!(0), json!(-1), json!(1.5), json!("258400"), Value::Null] {
            assert_eq!(with_window(window.clone()), no_window, "window {window}");
        }
        assert_eq!(with_window(json!(1)), RolloutRecord::Tokens { used: 10, window: Some(1) });
    }

    #[test]
    fn a_token_count_whose_total_is_not_an_unsigned_integer_stays_other() {
        for total in [json!(-5), json!(2.5), json!("61767"), Value::Null, json!([1])] {
            let record = token_count(json!({ "last_token_usage": { "total_tokens": total }, "model_context_window": 258_400 }));
            assert_eq!(record, RolloutRecord::Other, "total_tokens {total}");
        }
        assert_eq!(token_count(json!({ "last_token_usage": { "total_tokens": 0 } })), RolloutRecord::Tokens { used: 0, window: None });
    }
```

(The existing `legacy_rollout_records` and `paginated_rollout_records` keep their `Other, // token_count` lines: both fixtures write `"info":{}`.)

- [ ] **Step 2: Write the failing model tests**

At the end of `model_tests.rs`:

```rust
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
```

- [ ] **Step 3: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak --lib token_count 2>&1 | grep -E "^error" | sort -u | head -8`
Expected: compile errors, among them `no variant or associated item named \`Tokens\` found for enum \`RolloutRecord\`` and `cannot find struct, variant or union type \`ContextFill\``.

- [ ] **Step 4: Implement the parser**

`codex.rs`, in `enum RolloutRecord`, after the `UserMessage` variant:

```rust
    /// `event_msg` `token_count` with `info.last_token_usage.total_tokens`: how full the context is
    /// (context-fill spec §2.1). `window` is `info.model_context_window` when it is an integer > 0.
    /// Only these two numbers are read.
    Tokens { used: u64, window: Option<u64> },
```

In the doc comment of `parse_rollout_line`, after the line `///   null or absent → None; any other JSON → compact JSON;` add:

```rust
/// - `event_msg` + `payload.type` token_count with an unsigned integer
///   `info.last_token_usage.total_tokens` → Tokens (`window`: `info.model_context_window` when it is
///   an integer > 0, else None); any other token_count (`info` null or absent, no such number) → Other;
```

In the `match (kind, payload, payload_type)`, after the `("event_msg", Some(p), Some("turn_aborted"))` arm:

```rust
        ("event_msg", Some(p), Some("token_count")) => match context_fill(p) {
            Some((used, window)) => RolloutRecord::Tokens { used, window },
            None => RolloutRecord::Other,
        },
```

After `fn error_text`:

```rust
/// (used, window) of a `token_count` payload: `info.last_token_usage.total_tokens` as a u64 and
/// `info.model_context_window` when it is an integer > 0. None when `info` is null or absent or has
/// no such `total_tokens`.
fn context_fill(p: &Map<String, Value>) -> Option<(u64, Option<u64>)> {
    let info = p.get("info")?.as_object()?;
    let used = info.get("last_token_usage")?.get("total_tokens")?.as_u64()?;
    let window = info.get("model_context_window").and_then(Value::as_u64).filter(|w| *w > 0);
    Some((used, window))
}
```

- [ ] **Step 5: Implement the model**

`model.rs`, in the wire-types section, after `pub struct OutcomeView { .. }`:

```rust
/// How full a session's context is (context-fill spec §3): the tokens of its last request and, for
/// Codex, the size of its window. Only numbers; nothing of the messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ContextFill {
    pub used: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<u64>,
}
```

In `pub struct Session`, after `pub silent_mark: Option<(i64, bool)>,`:

```rust
    /// The last context fill read from the rollout or the transcript; every newer value replaces
    /// it, also a smaller one (compaction). It changes nothing else (context-fill spec §3).
    pub context: Option<ContextFill>,
```

In `Session::blank`, after `silent_mark: None,`:

```rust
            context: None,
```

In `pub struct SessionView`, after the `silent` field (before the `agent_pid` doc comment):

```rust
    /// How full the context is; absent until the first value (context-fill spec §3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextFill>,
```

In `append_silent`, in the `SessionView { .. }` literal, after `silent: Some(reason_of(hooks_not_installed, row.folder_exists)),`:

```rust
                context: None,
```

In `session_view`, after the `silent: s ... .map(|(_, folder_exists)| reason_of(hooks_not_installed, folder_exists)),` field:

```rust
        context: s.context,
```

In `codex_rollout`, replace the last arm of the second `match record`:

```rust
                RolloutRecord::Meta { .. } | RolloutRecord::UserMessage | RolloutRecord::Other | RolloutRecord::Unknown => {}
```

with:

```rust
                RolloutRecord::Tokens { used, window } => s.context = Some(ContextFill { used, window }),
                RolloutRecord::Meta { .. } | RolloutRecord::UserMessage | RolloutRecord::Other | RolloutRecord::Unknown => {}
```

(The first `match &record` of that loop and `TurnScan::feed` in `io_thread` end with `_ => ...`: they need no change.)

`lib.rs`, `session_command_tests::session`, after `silent: None,`:

```rust
            context: None,
```

- [ ] **Step 6: Run the tests and the suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak --lib token_count 2>&1 | grep -E "^test |test result"` then `cargo test -p mayak --lib context 2>&1 | grep -E "^test |test result"` then `cargo test --workspace 2>&1 | grep -E "^test result|^warning|^error"`
Expected: the 4 parser tests and `a_token_count_sets_the_codex_context_and_a_smaller_one_replaces_it`, `token_counts_change_nothing_but_the_context` ok in the first run; `the_context_is_on_the_wire_only_when_known`, `a_silent_row_has_no_context` (and the two above whose names contain `context`) ok in the second; suite: lib **417 passed, 1 ignored**, hook 32 passed, 0 failed, no `warning` line.

- [ ] **Step 7: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/observe/codex.rs windows/src-tauri/src/observe/model.rs windows/src-tauri/src/observe/model_tests.rs windows/src-tauri/src/lib.rs && git commit -m "feat(observe): read the Codex context fill from token_count and put it in the snapshot"
```

---

### Task 2: Claude `message.usage` → `Session.context`

**Files:**
- Modify: `windows/src-tauri/src/observe/claude_transcript.rs` (after `fact_of` ~line 69; tests module ending ~line 518)
- Modify: `windows/src-tauri/src/observe/model.rs` (`claude_file` sessionId-change block ~line 1126, `claude_transcript` ~line 1233)
- Test: `windows/src-tauri/src/observe/claude_transcript.rs` (module `tests`), `windows/src-tauri/src/observe/model_tests.rs` (end of file)

**Interfaces:**
- Consumes: `is_main_chain(&Value) -> bool`; Task 1's `ContextFill`, `Session.context`, `context_of`; test helpers `prelude(Start::Working)`, `ctr`, `cfile`, `sfile`, `reply`, `tool`, `result`, `user`, `api_error`, `key`, `run`.
- Produces:
  - `pub fn context_used(v: &Value) -> Option<u64>` in `observe::claude_transcript`
  - `pub fn last_context_used(records: &[(u64, Value)]) -> Option<u64>` in `observe::claude_transcript`
  - model rule: every `ClaudeTranscript` batch of a bound transcript (also the window read at binding) sets `context = Some(ContextFill { used, window: None })` from `last_context_used`; a `sessionId` change sets `context = None`.

- [ ] **Step 1: Write the failing transcript tests**

In `claude_transcript.rs`, module `tests`, after `only_facts_from_the_turn_start_count` (before the module's closing `}`):

```rust
    fn assistant_with_usage(usage: Value) -> Value {
        json!({ "type": "assistant", "isSidechain": false, "message": { "role": "assistant",
            "content": [{ "type": "text", "text": "ok" }], "stop_reason": "end_turn", "usage": usage } })
    }

    #[test]
    fn the_context_is_the_sum_of_the_four_usage_fields() {
        let usage = json!({
            "input_tokens": 3, "cache_creation_input_tokens": 1_200, "cache_read_input_tokens": 610_000,
            "output_tokens": 797, "cache_creation": { "ephemeral_5m_input_tokens": 1_200 }, "service_tier": "standard"
        });
        assert_eq!(context_used(&assistant_with_usage(usage)), Some(612_000));
    }

    #[test]
    fn a_missing_usage_field_counts_as_zero_and_no_field_gives_nothing() {
        assert_eq!(context_used(&assistant_with_usage(json!({ "input_tokens": 950 }))), Some(950));
        assert_eq!(context_used(&assistant_with_usage(json!({ "cache_read_input_tokens": 10, "output_tokens": 5 }))), Some(15));
        assert_eq!(context_used(&assistant_with_usage(json!({ "service_tier": "standard" }))), None);
        assert_eq!(
            context_used(&assistant_with_usage(json!({ "input_tokens": "950", "output_tokens": null }))),
            None,
            "only integers count"
        );
        assert_eq!(context_used(&assistant_with_usage(json!(42))), None);
        let without = json!({ "type": "assistant", "message": { "role": "assistant", "content": [] } });
        assert_eq!(context_used(&without), None);
    }

    #[test]
    fn only_main_chain_assistant_records_with_a_real_usage_count() {
        let usage = json!({ "input_tokens": 10, "output_tokens": 5 });
        let mut side = assistant_with_usage(usage.clone());
        side["isSidechain"] = json!(true);
        assert_eq!(context_used(&side), None, "a subagent's context is not the session's");
        let mut as_user = assistant_with_usage(usage);
        as_user["type"] = json!("user");
        assert_eq!(context_used(&as_user), None);
        // The synthetic record of an API error carries a usage whose four fields are all 0.
        let zeros = json!({ "input_tokens": 0, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0, "output_tokens": 0 });
        let mut synthetic = assistant_with_usage(zeros);
        synthetic["isApiErrorMessage"] = json!(true);
        synthetic["message"]["model"] = json!("<synthetic>");
        assert_eq!(context_used(&synthetic), None);
    }

    #[test]
    fn the_last_record_with_a_usage_in_the_chunk_counts() {
        let records = vec![
            (0, assistant_with_usage(json!({ "input_tokens": 100 }))),
            (10, assistant_with_usage(json!({ "input_tokens": 200 }))),
            (20, json!({ "type": "user", "message": { "role": "user", "content": "next" } })),
            (30, json!({ "type": "system", "subtype": "stop_hook_summary" })),
        ];
        assert_eq!(last_context_used(&records), Some(200));
        assert_eq!(last_context_used(&records[2..]), None);
        assert_eq!(last_context_used(&[]), None);
    }
```

- [ ] **Step 2: Write the failing model tests**

At the end of `model_tests.rs` (after Task 1's tests):

```rust
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
```

- [ ] **Step 3: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak --lib context 2>&1 | grep -E "^error|^test .*FAILED|test result" | sort -u | head -8`
Expected: compile errors `cannot find function \`context_used\`` and `cannot find function \`last_context_used\`` (the model tests cannot run until the module compiles).

- [ ] **Step 4: Implement the transcript functions**

`claude_transcript.rs`, after `fn fact_of`:

```rust
/// The `message.usage` fields that together are the context of a request (context-fill spec §2.2).
const USAGE_FIELDS: [&str; 4] = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens", "output_tokens"];

/// How full the context was at a main-chain assistant record (context-fill spec §2.2): the sum of
/// the USAGE_FIELDS of `message.usage` that are unsigned integers, a missing one counting 0. None
/// for any other record, for a `usage` with none of the four, and for a sum of 0: the synthetic
/// record of an API error carries all four at 0, and it must not hide the real value. Only these
/// numbers are read.
pub fn context_used(v: &Value) -> Option<u64> {
    if v.get("type").and_then(Value::as_str) != Some("assistant") || !is_main_chain(v) {
        return None;
    }
    let usage = v.pointer("/message/usage")?.as_object()?;
    let mut sum: Option<u64> = None;
    for field in USAGE_FIELDS {
        if let Some(n) = usage.get(field).and_then(Value::as_u64) {
            sum = Some(sum.unwrap_or(0).saturating_add(n));
        }
    }
    sum.filter(|n| *n > 0)
}

/// The context fill of a read chunk: `context_used` of its last record that has one.
pub fn last_context_used(records: &[(u64, Value)]) -> Option<u64> {
    records.iter().rev().find_map(|(_, v)| context_used(v))
}
```

- [ ] **Step 5: Implement the model update**

`model.rs`, `claude_file`, in the `if !session_id.is_empty() && session_id != s.logical_id { .. }` block, after `s.detail = None;`:

```rust
            // a new transcript is a new context (/clear, resume): the old number is no longer true
            s.context = None;
```

`model.rs`, `claude_transcript`, after the `match rebind_at { .. }` block and before `for (offset, record) in records {`:

```rust
        if let Some(used) = transcript::last_context_used(&records) {
            // Claude writes no window size (context-fill spec §2.2): only the token count.
            s.context = Some(ContextFill { used, window: None });
        }
```

(It runs after the `None if !s.transcript_bound => return` arm, so records of a transcript the model let go never set it. It takes every record of the batch, also those before `turn_start_offset`: the facts loop below keeps its own offset filter.)

- [ ] **Step 6: Run the tests and the suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak --lib 2>&1 | grep -E "^test observe::(claude_transcript|model)::tests::(the_context_is_the_sum|a_missing_usage_field|only_main_chain_assistant|the_last_record_with_a_usage|a_claude_reply_sets|api_errors_and_subagent|a_new_session_id_drops|the_window_read_at_binding|claude_usage_changes)"` then `cargo test --workspace 2>&1 | grep -E "^test result|^warning|^error"`
Expected: the first run lists exactly 9 lines, each ending `... ok`; suite: lib **426 passed, 1 ignored**, hook 32 passed, 0 failed, no `warning` line.

- [ ] **Step 7: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/observe/claude_transcript.rs windows/src-tauri/src/observe/model.rs windows/src-tauri/src/observe/model_tests.rs && git commit -m "feat(observe): read the Claude context fill from the transcript usage"
```

---

### Task 3: The row label (bridge type, pure formatting, element, CSS)

**Files:**
- Modify: `windows/src/core/bridge.ts` (~line 162-168: before and inside `SessionView`)
- Modify: `windows/src/views/sessions.ts` (import line 13; after `silentText` ~line 68; after `rowDetail` ~line 107; `row()` ~lines 255-277)
- Modify: `windows/src/style.css` (after the `.sess-time` rule ~line 1595; new selectors only)

**Interfaces:**
- Consumes: the wire field `context?: { used: number; window?: number }` (Task 1, Task 2).
- Produces:
  - `export interface ContextFill { used: number; window?: number }` and `context?: ContextFill` on `SessionView` in `bridge.ts`
  - `export function formatTokens(n: number): string` in `views/sessions.ts`
  - `export type ContextTone = "dim" | "warn" | "full"` in `views/sessions.ts`
  - `export function contextLabel(c: ContextFill): { text: string; percent: number | null; tone: ContextTone }` in `views/sessions.ts`
  - CSS classes `.sess-ctx`, `.sess-ctx.warn`, `.sess-ctx.full`, `.sess-ctx-bar`, `.sess-ctx-fill`

- [ ] **Step 1: Types**

`bridge.ts`, after `export type SilentReason = ...;`:

```ts
/** How full a session's context is (context-fill spec §3); `window` only when the agent writes it (Codex). */
export interface ContextFill { used: number; window?: number }
```

and in `SessionView`, after `silent?: SilentReason;`:

```ts
  context?: ContextFill;
```

- [ ] **Step 2: The pure formatting**

`views/sessions.ts`: change the import line to

```ts
import type { ContextFill, Host, ProjectView, SessionView, SilentReason } from "../core/bridge";
```

and after `silentText`:

```ts
/** Token count of a context label: "950", "612k", "1.2M", always rounded down (context-fill spec §4). */
export function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${Math.floor(n / 1000)}k`;
  return `${(Math.floor(n / 100_000) / 10).toFixed(1)}M`;
}

export type ContextTone = "dim" | "warn" | "full";

/**
 * The context label of a row (context-fill spec §4). With a window: floor(used / window × 100),
 * at most 100, computed on integers (0.29 * 100 is 28.999… in floating point); grey below 80,
 * "warn" from 80, "full" from 95. Without one: the token count, grey, no bar.
 */
export function contextLabel(c: ContextFill): { text: string; percent: number | null; tone: ContextTone } {
  if (c.window !== undefined && c.window > 0) {
    const percent = Math.min(100, Math.floor((c.used * 100) / c.window));
    const tone: ContextTone = percent >= 95 ? "full" : percent >= 80 ? "warn" : "dim";
    return { text: `${percent}%`, percent, tone };
  }
  return { text: formatTokens(c.used), percent: null, tone: "dim" };
}
```

- [ ] **Step 3: The element and the row**

`views/sessions.ts`, after `function rowDetail`:

```ts
/** The row's context label, placed before its time; null without a context. Never clickable (CSS). */
function contextEl(c: ContextFill | undefined): HTMLElement | null {
  if (!c) return null;
  const label = contextLabel(c);
  const el = h("span", { class: label.tone === "dim" ? "sess-ctx" : `sess-ctx ${label.tone}` }, label.text);
  if (label.percent !== null) {
    const fill = h("i", { class: "sess-ctx-fill" });
    fill.style.width = `${label.percent}%`;
    el.append(h("span", { class: "sess-ctx-bar" }, fill));
  }
  return el;
}
```

In `row()`, silent branch: after `h("span", { class: "sess-detail", text: silentText(s.silent).short }),` add

```ts
        contextEl(s.context),
```

(a pure `silent:<id>` row never has a context, so it is unchanged; a known Codex session with a silent mark shows its label at the end, where the time would be.)

In `row()`, normal branch: between `h("span", { class: "sess-detail", text: rowDetail(s) }),` and `time,` add

```ts
      contextEl(s.context),
```

- [ ] **Step 4: CSS (new selectors only)**

`style.css`, after the `.sess-time { .. }` rule:

```css
/* Context fill of a session row (context-fill spec §4): before the time, never clickable,
   so a press on it is a press on the row. Grey by default; Codex warn/full use the tones of
   the waiting and error rows. */
.sess-ctx {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font: 400 11px var(--font);
  color: var(--dim-3);
  font-variant-numeric: tabular-nums;
  pointer-events: none;
}

.sess-ctx.warn { color: var(--amber); }
.sess-ctx.full { color: var(--red); }

.sess-ctx-bar {
  width: 24px;
  height: 3px;
  border-radius: 2px;
  background: rgba(255, 255, 255, 0.1);
  overflow: hidden;
}

.sess-ctx-fill {
  display: block;
  height: 100%;
  background: currentColor;
}
```

- [ ] **Step 5: Verify**

Run: `cd /c/Users/HP/source/repos/mayak/windows && npx tsc --noEmit && npx vite build 2>&1 | tail -3` then `file src/core/bridge.ts src/views/sessions.ts src/style.css`
Expected: `tsc` prints nothing and exits 0; `vite build` ends with `✓ built in …`; all three files still `with CRLF line terminators`. `cargo test --workspace` is unaffected (nothing Rust changed).

- [ ] **Step 6: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src/core/bridge.ts windows/src/views/sessions.ts windows/src/style.css && git commit -m "feat(island): show the context fill before the time of a session row"
```

---

### Task 4: Acceptance with the user, CPU, spec alignment, publishing

**Files:**
- Modify: `docs/superpowers/specs/2026-10-06-mayak-context-fill-design.md` (append)
- Create: `C:\tmp\mayak-spike\ctx.mjs` (outside the repository)
- Modify: `.git/mayak-pr-notes.md`, create `.git/mayak-pr8-body.md` (local, untracked)

**Interfaces:** consumes the branch after Tasks 1-3 (`SessionView.context`, `contextLabel`, `.sess-ctx`); produces the acceptance record.

The controller runs Steps 3-5 **with the user** (real Claude and Codex sessions). The dev Mayak under `tauri dev` is already running the working tree and rebuilt after each task. Do not stop it and do not start a second one.

- [ ] **Step 1: Write the decisions into the spec**

Append at the end of the spec (Russian, LF line endings):

```markdown

## 6. Уточнения при реализации

- **Синтетические записи Claude.** Ошибка API пишет запись ассистента `"model":"<synthetic>"` с `usage`, где все четыре поля равны 0 (у пользователя 41 такая запись в основной цепочке). Сумма 0 считается отсутствием значения: метка сохраняет прежнее число.
- **Подагенты.** Записи с `isSidechain: true` не учитываются: это контекст подагента, а не сессии.
- **Нечисловые поля.** Поле `usage`, которое не является целым ≥ 0, считается отсутствующим. У Codex `total_tokens`, не являющийся целым ≥ 0, даёт `Other`; окно, не являющееся целым > 0, даёт `window: None`.
- **Когда появляется значение.** Claude при привязке существующего транскрипта читает окно 256 КиБ (как и раньше); если в нём есть ответ ассистента с `usage`, число видно сразу. Codex читает rollout с конца: значение появляется со следующей записью `token_count`.
- **Новый `sessionId`** (`/clear`, resume) сбрасывает значение; записи старого транскрипта его не возвращают.
- **«Немая» строка известной сессии Codex** (`codex:<id>` с пометкой) показывает метку в конце строки, так как времени в ней нет. У `silent:<id>` значения нет.
- **Клик.** У метки `pointer-events: none`: нажатие на неё — нажатие на строку.
- **Числа.** Ровно миллион — `1.0M`. Процент считается в целых: `floor(used × 100 / window)`.
- **Цвета.** Серый — `--dim-3`, жёлтый — `--amber` (тон `waiting`), красный — `--red` (тон `error`).
```

Commit:

```bash
cd /c/Users/HP/source/repos/mayak && git add docs/superpowers/specs/2026-10-06-mayak-context-fill-design.md && git commit -m "docs: align the context fill spec with the implementation"
```

- [ ] **Step 2: The acceptance helper and a check of the running dev Mayak**

Create `C:\tmp\mayak-spike\ctx.mjs` (same DevTools pattern as `snap.mjs`):

```js
// Mayak context fill acceptance over the WebView2 DevTools protocol (dev app on :9222).
// Prints agents, hosts, phases, numbers, classes and colours only: never titles, prompts or paths.
// Usage: node ctx.mjs show | labels | colors | restore
const mode = process.argv[2] || "show";
const targets = await (await fetch("http://127.0.0.1:9222/json")).json();
const page = targets.find((t) => t.type === "page" && /index\.html|localhost:1420\/?$/.test(t.url));
if (!page) throw new Error("no island page on :9222");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((ok, err) => { ws.onopen = ok; ws.onerror = err; });
let id = 0;
const call = (method, params) => new Promise((ok, err) => {
  const my = ++id;
  ws.addEventListener("message", function h(ev) {
    const m = JSON.parse(ev.data);
    if (m.id !== my) return;
    ws.removeEventListener("message", h);
    m.error ? err(new Error(JSON.stringify(m.error))) : ok(m.result);
  });
  ws.send(JSON.stringify({ id: my, method, params }));
});
const DOM = `[...document.querySelectorAll(".sess-ctx")].map((e) => ({ text: e.textContent, cls: e.className,
  color: getComputedStyle(e).color, bar: e.querySelector(".sess-ctx-fill")?.style.width ?? "" }))`;
const exprs = {
  show: `(async () => {
    const s = await window.__TAURI_INTERNALS__.invoke("sessions_snapshot");
    const rows = s.projects.flatMap((p) => p.sessions.map((x) => ({ agent: x.agent, host: x.host, phase: x.phase.kind,
      silent: x.silent ?? "", used: x.context?.used ?? "", window: x.context?.window ?? "" })));
    return JSON.stringify({ rev: s.rev, rows, dom: ${DOM} });
  })()`,
  labels: `(async () => {
    const m = await import("/src/views/sessions.ts");
    const cases = [[950], [1000], [612345], [999999], [1000000], [1234567],
      [79999, 100000], [80000, 100000], [94999, 100000], [95000, 100000], [120000, 100000], [29, 100]];
    return JSON.stringify(cases.map(([used, window]) =>
      [used, window ?? null, m.contextLabel(window === undefined ? { used } : { used, window })]));
  })()`,
  colors: `(async () => {
    const { Sessions } = await import("/src/core/sessions.ts");
    const { State } = await import("/src/core/state.ts");
    const real = Sessions.snapshot;
    if (!real) return JSON.stringify({ error: "no snapshot" });
    const fake = structuredClone(real);
    fake.rev = real.rev + 0.5; // the next real snapshot (an integer rev) still replaces it
    const fills = [{ used: 79999, window: 100000 }, { used: 80000, window: 100000 }, { used: 95000, window: 100000 }, { used: 612345 }];
    let n = 0;
    for (const p of fake.projects) for (const x of p.sessions) if (!x.key.startsWith("silent:")) x.context = fills[n++ % fills.length];
    const accepted = Sessions.accept(fake);
    State.notify();
    await new Promise((r) => setTimeout(r, 300));
    const probe = ["sess-ctx", "sess-ctx warn", "sess-ctx full"].map((cls) => {
      const e = document.createElement("span");
      e.className = cls;
      document.body.append(e);
      const color = getComputedStyle(e).color;
      e.remove();
      return { cls, color };
    });
    return JSON.stringify({ accepted, rows: n, dom: ${DOM}, probe });
  })()`,
  restore: `(async () => {
    const { Sessions } = await import("/src/core/sessions.ts");
    const { State } = await import("/src/core/state.ts");
    const s = await window.__TAURI_INTERNALS__.invoke("sessions_snapshot");
    s.rev = (Sessions.snapshot?.rev ?? s.rev) + 0.25;
    const accepted = Sessions.accept(s);
    State.notify();
    return JSON.stringify({ accepted, rev: s.rev });
  })()`,
};
if (!exprs[mode]) throw new Error(`unknown mode ${mode}`);
const r = await call("Runtime.evaluate", { expression: exprs[mode], awaitPromise: true, returnByValue: true });
if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
const out = JSON.parse(r.result.value);
if (out.rows && Array.isArray(out.rows)) console.table(out.rows);
console.log(JSON.stringify({ ...out, rows: Array.isArray(out.rows) ? out.rows.length : out.rows }, null, 1));
ws.close();
```

Run: `curl -s --max-time 2 http://127.0.0.1:9222/json/list | grep -c "localhost:1420"` then `cd /c/tmp/mayak-spike && node ctx.mjs labels`
Expected: the count is at least 1 (the dev island page is up). `labels` prints, in order: `950` dim; `1k`; `612k`; `999k`; `1.0M`; `1.2M`; `79%` dim percent 79; `80%` warn; `94%` warn; `95%` full; `100%` full percent 100; `29%` dim percent 29. If the page is missing, the dev Mayak is not running: ask the user to start it; do not start it yourself.

- [ ] **Step 3: Spec §5 acceptance items 1-3 with the user**

Record each as PASS/FAIL with the evidence (the `ctx.mjs` output, what the user saw) in `.git/mayak-pr-notes.md` under `## Context fill acceptance (2026-10-06)`:
1. Claude: with an active Claude session, the user opens the island on the session list. `node ctx.mjs show`: the Claude row has `used` and no `window`, and `dom` has a grey (`rgb(107, 112, 121)`) entry like `612k` with `bar: ""`. The user sends one message in that session; after the answer `show` again: `used` is larger and the label changed.
2. Codex: the user runs one turn in a Codex session with hooks. `show`: the Codex row has `used` and `window`, and `dom` has `N%` with `bar: "N%"`. The user confirms the percent and the bar in the island.
3. Colours: with the island open on the session list, `node ctx.mjs colors`: `accepted: true`; `dom` shows `79%` `sess-ctx` `rgb(107, 112, 121)` bar `79%`, `80%` `sess-ctx warn` `rgb(245, 165, 36)`, `95%` `sess-ctx full` `rgb(244, 80, 94)`, and `612k` grey with no bar (as many as there are rows); `probe` gives the same three colours. The user confirms them in the island; then `node ctx.mjs restore` (`accepted: true`) and `show` gives the real values again. If `dom` still shows the real values after `colors` (the page holds a different module instance), record the `probe` colours and the Step 2 `labels` table as the evidence instead.
Also check with the user: a press on the label opens the session card exactly like a press elsewhere on the row.

- [ ] **Step 4: CPU (spec §5 item 4)**

With the island hidden and no agent turn running: `powershell -File C:\tmp\mayak-accept\sample-cpu.ps1 -Seconds 60 -OutPath C:\tmp\mayak-accept\cpu-context-idle.log`. Expected: `tree_avg_percent_of_machine` < 0.100, as before this branch (`.git/mayak-pr-notes.md` 5a: 0.010). Record the RESULT line in `.git/mayak-pr-notes.md`.

- [ ] **Step 5: Full verification**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|^warning"` and `npx tsc --noEmit && npx vite build 2>&1 | tail -3` and `cd /c/Users/HP/source/repos/mayak && git status --short`
Expected: lib 426 passed (1 ignored), hook 32 passed, no `warning` line, build OK, `git status --short` empty (the `.git/` notes are not tracked; `dist/` is ignored).

- [ ] **Step 6: Ask before publishing**

Write `.git/mayak-pr8-body.md` from the results of Steps 3-5: What (Codex percent and bar, Claude token count, the model field, numbers only), Checks (local) with the test counts and every acceptance line, and no trailer. Ask the user, verbatim: «Запушить ветку `feature/context-fill` в `origin` и открыть PR в `main`? Проверки на GitHub отключены, проверка локальная.» Only on an explicit yes:

```bash
cd /c/Users/HP/source/repos/mayak && git push -u origin feature/context-fill && gh pr create --repo mahach666/mayak --base main --head feature/context-fill --title "Mayak: show how full each session's context is" --body-file "$(git rev-parse --git-dir)/mayak-pr8-body.md"
```

The merge stays with the user: do not merge unless the user asks for it in a separate, explicit message.

---

## Self-Review

- **Spec coverage:**
  - §1.1 Codex percent and bar, Claude count with no guessed window → Task 1 (`window` from the rollout only), Task 2 (`window: None`), Task 3 (`contextLabel`).
  - §1.2 fill only, no session spend → Global Constraints; only `last_token_usage` is read (Task 1 test 2 rejects `total_token_usage`).
  - §1.3 colours and no sound, alerts or counts → Task 3 (`ContextTone`, CSS), Task 1 test `token_counts_change_nothing_but_the_context`, Task 2 test `claude_usage_changes_nothing_but_the_context`, Task 4 Step 3 item 3.
  - §1 rules (numbers only, no polling, existing views unchanged) → Global Constraints; Tasks 1-2 read only numeric fields of records already tailed; Task 3 adds only new selectors and one element; Task 4 Step 4.
  - §2.1 `Tokens { used, window }`, `info: null` → Other, positive-integer window → Task 1 parser tests and Step 4.
  - §2.2 the four-field sum, missing = 0, no field = no value, last record of the chunk → Task 2 transcript tests and Step 4.
  - §2.3 last value wins also when smaller, memory only, when the value appears → Task 1 and Task 2 model tests; Task 4 Step 1 (the Claude window at binding).
  - §3 `Session.context`, `SessionView.context` with serde, JSON shape, no field on `silent:<id>` → Task 1 Step 5 and the tests `the_context_is_on_the_wire_only_when_known`, `a_silent_row_has_no_context`.
  - §4 label before the time, no label without context, formats, percent floor ≤ 100, not clickable, `.sess-ctx` → Task 3; checked by Task 4 Step 2 (`labels`) and Step 3.
  - §5 Rust tests → Tasks 1-2; TS `tsc` + build and the pure function next to `silentText` → Task 3; live items 1-4 → Task 4 Steps 3-4.
- **Placeholder scan:** none. Every code step carries its code; the only conditional instructions name their exact action (a missing dev page: ask the user; a fake snapshot that does not reach the DOM: use the `probe` colours and the `labels` table).
- **Type consistency:** `RolloutRecord::Tokens { used: u64, window: Option<u64> }` (Task 1 Step 4) is matched in `codex_rollout` (Task 1 Step 5); `ContextFill { used, window }` (Task 1) is built in Task 2 Step 5 and mirrored by the TS `ContextFill { used; window? }` (Task 3); `context_used` / `last_context_used(&[(u64, Value)])` (Task 2 Step 4) match the `ClaudeTranscript.records` type; `context_of`, `token_count`, `with_usage` are defined before their first use in `model_tests.rs`; `contextLabel` / `formatTokens` (Task 3) are the names `ctx.mjs labels` imports (Task 4).
- **Review Focus:** 1 → Task 2 transcript test 3 and model test `api_errors_and_subagent_records_keep_the_last_value`; 2 → the same two; 3 → Task 1 parser tests 2-4; 4 → Task 1 `token_counts_change_nothing_but_the_context` and Task 2 `claude_usage_changes_nothing_but_the_context`; 5 → Task 2 `a_new_session_id_drops_the_old_value_and_stray_old_records_do_not_bring_it_back`.
- **Test counts:** Task 1 adds 4 parser + 4 model tests (409 → 417); Task 2 adds 4 transcript + 5 model tests (417 → 426).
