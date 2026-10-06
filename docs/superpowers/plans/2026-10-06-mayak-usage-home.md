# Mayak: plan limits on the home page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The island's home page (view `overview`) shows how much of the Claude and Codex plan limits is used and when each window resets, read only from what Claude Code (`~/.claude.json`, key `cachedUsageUtilization`) and Codex (rollout `token_count.rate_limits`) already wrote to disk. The integration card and pills are no longer rendered there.

**Architecture:** The Codex rollout parser's `RolloutRecord::Tokens` becomes `Tokens { context, limits }`: the existing context fill plus the `codex` bucket's `primary`/`secondary` windows. `Store::codex_rollout` keeps the batch's last windows in a new `Store.usage.codex` (store clock as `seenAtMs`), also for a key it no longer knows. A new pure module `observe/claude_usage.rs` deserializes only `cachedUsageUtilization.{fetchedAtMs, utilization.five_hour, utilization.seven_day}` and parses RFC 3339 dates by hand; the **claude-io thread** reads the file at its start, on every `Retry` (which `Visible(true)` already sends) and on a new `ClaudeInput::ReadUsage` (Tauri command `usage_refresh`, called when the home view becomes active), and sends `Signal::ClaudeUsage` only when the value changed. `SessionsSnapshot.usage` is part of the store's `View`, so a change bumps `rev` and nothing else. The front end gets a DOM-free `views/usage-format.ts` (checked with Node's type stripping) and `views/usage.ts`, which reuses the context label's `.sess-ctx` + `contextRing` and runs a 60 s text refresh only between `setActive(true)` and `setActive(false)`.

**Tech Stack:** Rust (serde, serde_json; no new crates), TypeScript (no new packages), Tauri 2, Node 22 `--experimental-strip-types` for the out-of-repo check script.

**Spec:** `docs/superpowers/specs/2026-10-06-mayak-usage-home-design.md` (Russian; new spec text in Russian, LF line endings).

## Global Constraints

- Values from the spec, verbatim:
  - Colour: «**Цвет как у метки контекста:** серый до 80 %, жёлтый с 80 %, красный с 95 %. Только на домашней странице, без звука и без выдвигания острова.»
  - «запросы в сеть: Mayak не обращается к API Anthropic или OpenAI, только читает то, что агенты уже записали на диск»; «изменение интеграций и их опроса» is out of scope.
  - «Интеграции (карточка слева и пилюли справа, наследство Coucou) с неё убираются. Их код и настройки не удаляются, только перестают показываться на домашней странице.»
  - Claude fields: `fetchedAtMs`; `utilization.five_hour` and `utilization.seven_day`, each `utilization` (number) and `resets_at` (ISO 8601). «Остальные ключи `utilization.*` и весь остальной файл **не читаются**.» «Ничего из файла не логируется.»
  - File: «`%USERPROFILE%\.claude.json`. Если задан `CLAUDE_CONFIG_DIR`, берётся `<CLAUDE_CONFIG_DIR>\.claude.json`».
  - «Окно, у которого `utilization` не число, пропускается. Неразбираемая дата сброса даёт окно без времени сброса.»
  - When read: «при запуске, при каждом показе острова (`Visible(true)` …) и при открытии домашней страницы. Чтение маленькое и идёт в отдельном потоке или на потоке наблюдателя, главный поток не блокируется.»
  - Codex: `payload.rate_limits`, «`primary` и `secondary`, каждый либо `null`, либо `{ used_percent: number, window_minutes: number, resets_at: секунды эпохи }`»; «хранится **последнее** прочитанное значение по всем сессиям Codex. Свежесть — время, когда Mayak прочитал запись.»; «Новых чтений файлов нет.»
  - Wire: `usage: { claude: { fetchedAtMs, windows: [{ id: "5h"|"7d", percent, resetsAtMs? }] } | null, codex: { seenAtMs, windows: [{ minutes, percent, resetsAtMs? }] } | null }`.
  - «Изменение `usage` меняет `rev`, так же как контекст. На счётчики, тревоги и hold оно не влияет.» «Если источник отсутствует или пуст, поле — `null`.»
  - Row: «слева кольцо 12 px и процент — тот же кольцевой индикатор, что у метки контекста, с теми же порогами цвета»; names `5h`, `7d`; «300 даёт `5h`, 10080 — `7d`; иначе меньше 1440 минут даёт `Nh`, а от 1440 — `Nd`»; «справа «resets in 2h 14m» (`in 3d 4h`, `in 12m`, `soon` если меньше минуты); если время сброса уже прошло, показывается `reset`.»
  - «если данным больше 10 минут, под блоком мелким текстом «as of 7h ago»»; «No data yet»; hints «Claude Code updates it while it runs» and «Codex reports it with each answer».
  - «при каждой отрисовке и раз в минуту, **только пока домашняя страница на экране**. Пока остров свёрнут или спрятан, таймера нет.»
  - «Код `renderIntegrationCard` и пилюль остаётся в репозитории.»
- No new crates, no new packages. No polling or file watcher for `.claude.json`; the only new timer is the 60 s text refresh of the home view while it is active.
- Do not restyle other views: new CSS selectors only (`.usage*`); the session row's `.sess-ctx` rules stay as they are and are reused, not changed.
- Privacy: never `cat` or print `.claude.json` while working; the acceptance helper prints only numbers, window names, classes, colours and Mayak's own texts. Never open `*.key`, `ide/*.lock`, `auth.json`, `claude_desktop_config.json`; never edit `~/.codex` or `~/.claude`/`~/.claude.json`.
- Windows is the only platform touched. Keep the bundle identifier and every pill ID.
- Build environment: always `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target` before cargo. Never touch `windows/target/release`.
- Baseline measured on `feature/usage-home` at `dcd4833` (2026-10-06): `cargo test --workspace` = lib **446 passed, 1 ignored**; hook **32 passed**; **0 warnings**.
- Front-end checks: `npx tsc --noEmit`, `npx vite build` (there is no front-end test framework; do not add one; the Node check script lives outside the repo).
- Keep each file's line endings: every source file touched here is CRLF in the working tree (`git ls-files --eol`: `i/lf w/crlf`, `core.autocrlf=true`); the spec is LF. Edit in place; a new source file is converted with `unix2dos` before its commit.
- A dev Mayak runs under `tauri dev` from this working tree and rebuilds by itself after each task. Never stop it, never start a second one.
- Commits: conventional, one line, **no trailers** (no Co-Authored-By, no «Generated with»); never mention an AI assistant anywhere (code, comments, docs, PR text). Never push without the user's explicit yes (Task 4 asks). Tests run locally only.

## Review Focus

1. Codex writes several `rate_limits` buckets into the same rollout (on this machine: `limit_id` `codex` 12 157 records, `codex_bengalfox` "GPT-5.3-Codex-Spark" 5 093, `premium` with both windows null 1): "last value wins" over all of them would flip the home page between the account's limits and a model's. Only `limit_id` `"codex"` or no `limit_id` counts (Task 1: `only_the_codex_limit_bucket_counts`, `a_token_count_sets_the_codex_usage_and_the_last_value_wins`).
2. Claude Code rewrites `.claude.json` while Mayak reads it: a torn or empty file must keep the last value instead of flashing «No data yet», and only a missing file gives `null` (Task 2: `a_torn_file_is_an_error_that_names_no_text_of_it`, `the_usage_is_read_at_start_on_retry_and_on_request_and_sent_on_change_only`).
3. `.claude.json` holds account data (`oauthAccount`, and `accountUuid` even inside `cachedUsageUtilization`): nothing but the named fields may come out of the parser, and an error must name only its category, never text of the file (Task 2: `the_full_object_gives_both_windows_and_nothing_else` compares the whole result; the torn-file test compares the error strings exactly).
4. A new value changes nothing else (alerts, effects, wake, counts, hold, aggregate, attention, rows), yet bumps `rev`, or the island never redraws it (Task 1: `codex_usage_changes_nothing_but_the_usage`; Task 2: `claude_usage_changes_nothing_but_the_usage`).
5. `resets_at` comes as `…T…:…:….123456+00:00`, `…+03:00`, `…Z` or garbage: each valid form gives the same instant in ms, every invalid one gives a window without a reset time, never a panic and never a wrong time (Task 2: `rfc3339_dates_become_epoch_milliseconds`, `anything_else_is_no_date`, `an_unparsable_reset_date_gives_a_window_without_it`).

---

## Decisions (where the spec leaves a choice)

- **Where the Claude read happens: claude-io.** It already owns the Claude paths (`ClaudeConfig`), already runs `start()` once and already receives `ClaudeInput::Retry` from `run_store` on every `Visible(true)` (`observe/mod.rs` lines 100-106), so two of the three triggers cost no new wiring and the read never runs on the main thread or the tokio runtime. The third trigger is a new `ClaudeInput::ReadUsage`, sent by `Handle::claude_usage()` from the Tauri command `usage_refresh`, which the home view calls in `setActive(true)`. claude-io remembers the last value it sent and sends `Signal::ClaudeUsage` only on change: no extra store batch per island show, and the existing claude-io tests that compare exact signal lists stay green (their rigs have no `.claude.json`, and the remembered value starts at `None` like the store). A spawned thread per read was rejected: `run_store` would need the path and its tests would read the real profile.
- **Codex limits are taken before the session lookup** in `Store::codex_rollout`: they belong to the account, so a stray batch of a removed session still counts.
- **`seenAtMs` is the store clock of every batch with limits**, also when the percents are unchanged (the freshness line must move); that is one `rev` change per such batch while the island is visible.
- **The home page is always `overview`:** the home tab and `State.defaultView()` no longer fall back to `empty` when no integration is on (otherwise a user without integrations never sees the limits). `buildOverview`, `renderIntegrationCard` and the pills stay in the code (exported, so `noUnusedLocals` keeps compiling) but are not registered.
- **Strictness of the inputs** (spec says nothing): Claude needs an integer `fetchedAtMs` and object-shaped `cachedUsageUtilization`/`utilization`, else `null`; a JSON syntax/EOF error or an I/O error keeps the last value; Codex windows need a number `used_percent` and an integer `window_minutes` > 0, `resets_at` an integer; a record with no such window changes nothing.
- **Display:** percent rounded down and held within 0-100 (text, ring and tone); a Codex window under 60 minutes is `Nm`; all divisions round down.

## File Structure

- Modify `windows/src-tauri/src/observe/codex.rs` — `RolloutRecord::Tokens { context, limits }`, `rate_limits` / `codex_window` parse, parser tests (Task 1).
- Modify `windows/src-tauri/src/observe/model.rs` — wire types `CodexWindow`, `CodexUsage`, `Usage` (Task 1), `ClaudeWindowId`, `ClaudeWindow`, `ClaudeUsage`, `Signal::ClaudeUsage` (Task 2); `Store.usage`, `View.usage`, `SessionsSnapshot.usage`, `codex_rollout`.
- Modify `windows/src-tauri/src/observe/model_tests.rs` — model tests (Tasks 1-2).
- Create `windows/src-tauri/src/observe/claude_usage.rs` — pure parse of `.claude.json`, RFC 3339 parser, one-shot read (Task 2).
- Modify `windows/src-tauri/src/observe/claude.rs` — `ClaudeConfig.usage_file`, `ClaudeInput::ReadUsage`, `ClaudeIo.usage`, `read_usage` (Task 2).
- Modify `windows/src-tauri/src/observe/mod.rs` — `pub mod claude_usage`, `ObserveConfig.claude_json`, `Handle.claude` + `claude_usage()` (Task 2).
- Modify `windows/src-tauri/src/lib.rs` — `AppPaths.claude_json`, command `usage_refresh` (Task 2).
- Modify `windows/src/core/bridge.ts`, `windows/src/views/sessions.ts`, `windows/src/views/views.ts`, `windows/src/core/state.ts`, `windows/src/style.css`; create `windows/src/views/usage-format.ts`, `windows/src/views/usage.ts` (Task 3).
- Create `C:\tmp\mayak-spike\usage-format-check.mjs` (Task 3) and `C:\tmp\mayak-spike\usage.mjs` (Task 4), outside the repository.
- Modify `docs/superpowers/specs/2026-10-06-mayak-usage-home-design.md` — clarifications (Task 4).

**Why four tasks, as suggested:** `observe` is a private module, so a wire type nobody constructs warns (dead code) and the baseline has 0 warnings. Each Rust task is therefore one source end to end: Task 1 = Codex parse + `Usage { codex }` + snapshot; Task 2 = Claude read + `Usage.claude` + its three triggers (the command is the only consumer of `ReadUsage`, so it lands in the same task). Task 3 is the whole front end; Task 4 the live acceptance.

---

### Task 1: Codex `rate_limits` → `Store.usage.codex` → `SessionsSnapshot.usage`

**Files:**
- Modify: `windows/src-tauri/src/observe/codex.rs` (imports ~line 8; `enum RolloutRecord` ~line 50-74; `parse_rollout_line` doc ~line 84-86 and match arm ~line 131-134; `context_fill` ~line 219-227; tests module ending ~line 703)
- Modify: `windows/src-tauri/src/observe/model.rs` (wire types after `ContextFill` ~line 266; `SessionsSnapshot` ~line 350; `View` ~line 468; `Store` ~line 495; `Store::new` ~line 519; `snapshot` ~line 577; `view` ~line 791; `codex_rollout` ~line 1515 and ~line 1592)
- Test: `windows/src-tauri/src/observe/codex.rs` (module `tests`), `windows/src-tauri/src/observe/model_tests.rs` (`snapshot_serializes_to_the_spec_wire_shape` ~line 438, end of file)

**Interfaces:**
- Consumes: `parse_rollout_line(&Value) -> RolloutRecord`; `ContextFill { used: u64, window: Option<u64> }` (`observe::model`); `Signal::CodexRollout { key, path, records: Vec<(u64, Value)>, eof, folder_exists }`; test helpers `run`, `xhook`, `ups`, `xroll`, `xkey`, `event_msg`, `token_count(used, window)`, `NOW` (= 9 000).
- Produces (later tasks rely on these exact names):
  - `RolloutRecord::Tokens { context: Option<ContextFill>, limits: Option<Vec<CodexWindow>> }` (at least one Some); `RolloutRecord` derives `Debug, Clone, PartialEq` (no `Eq`: `f64` inside)
  - `#[derive(Debug, Clone, Copy, PartialEq, Serialize)] #[serde(rename_all = "camelCase")] pub struct CodexWindow { pub minutes: u64, pub percent: f64, #[serde(skip_serializing_if = "Option::is_none")] pub resets_at_ms: Option<i64> }`
  - `#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")] pub struct CodexUsage { pub seen_at_ms: i64, pub windows: Vec<CodexWindow> }`
  - `#[derive(Debug, Clone, Default, PartialEq, Serialize)] pub struct Usage { pub codex: Option<CodexUsage> }` (Task 2 adds `claude` before `codex`)
  - `SessionsSnapshot.usage: Usage` (wire key `usage`); private `Store.usage: Usage`, `View.usage: Usage`
  - test helpers in `model_tests.rs`: `fn limits_event(rate_limits: Value) -> Value`, `fn limit_window(used_percent: f64, window_minutes: u64, resets_at: i64) -> Value`, `fn codex_usage(st: &Store) -> Option<CodexUsage>`

- [ ] **Step 1: Write the failing parser tests and adapt the three context-fill parser tests**

In `codex.rs`, module `tests`, add after `fn token_count(info: Value) -> RolloutRecord { .. }`:

```rust
    /// The record of a token_count with this context fill and no limits.
    fn fill(used: u64, window: Option<u64>) -> RolloutRecord {
        RolloutRecord::Tokens { context: Some(ContextFill { used, window }), limits: None }
    }
```

In `a_token_count_with_info_gives_the_fill_and_the_window`, replace

```rust
        assert_eq!(token_count(info), RolloutRecord::Tokens { used: 61_767, window: Some(258_400) });
```

with

```rust
        assert_eq!(token_count(info), fill(61_767, Some(258_400)));
```

In `a_token_count_window_is_kept_only_when_it_is_a_positive_integer`, replace

```rust
        let no_window = RolloutRecord::Tokens { used: 10, window: None };
```

with

```rust
        let no_window = fill(10, None);
```

and

```rust
        assert_eq!(with_window(json!(1)), RolloutRecord::Tokens { used: 10, window: Some(1) });
```

with

```rust
        assert_eq!(with_window(json!(1)), fill(10, Some(1)));
```

In `a_token_count_whose_total_is_not_an_unsigned_integer_stays_other`, replace

```rust
        assert_eq!(token_count(json!({ "last_token_usage": { "total_tokens": 0 } })), RolloutRecord::Tokens { used: 0, window: None });
```

with

```rust
        assert_eq!(token_count(json!({ "last_token_usage": { "total_tokens": 0 } })), fill(0, None));
```

Then, after `a_token_count_whose_total_is_not_an_unsigned_integer_stays_other` (before the module's closing `}`):

```rust
    /// A paginated-mode `token_count` event before the first answer, with the given `rate_limits`.
    fn limits(rate_limits: Value) -> RolloutRecord {
        parse_rollout_line(&json!({
            "ordinal": 8, "timestamp": "2026-10-06T10:00:00.000Z", "type": "event_msg",
            "payload": { "type": "token_count", "info": null, "rate_limits": rate_limits }
        }))
    }

    fn window(minutes: u64, percent: f64, resets_at_ms: Option<i64>) -> CodexWindow {
        CodexWindow { minutes, percent, resets_at_ms }
    }

    #[test]
    fn a_token_count_gives_the_codex_limit_windows() {
        // The keys Codex 0.160 writes (values generic); only primary and secondary are read.
        let rate_limits = json!({
            "limit_id": "codex", "limit_name": null,
            "primary": { "used_percent": 12, "window_minutes": 300, "resets_at": 1_791_300_000 },
            "secondary": { "used_percent": 37.5, "window_minutes": 10_080, "resets_at": 1_791_800_000 },
            "credits": { "has_credits": true, "unlimited": false, "balance": "5" },
            "individual_limit": null, "spend_control_reached": null, "plan_type": "plus", "rate_limit_reached_type": null
        });
        let expected = vec![window(300, 12.0, Some(1_791_300_000_000)), window(10_080, 37.5, Some(1_791_800_000_000))];
        assert_eq!(limits(rate_limits.clone()), RolloutRecord::Tokens { context: None, limits: Some(expected.clone()) });
        // with a context fill the record carries both
        let both = parse_rollout_line(&json!({ "type": "event_msg", "payload": {
            "type": "token_count",
            "info": { "last_token_usage": { "total_tokens": 61_767 }, "model_context_window": 258_400 },
            "rate_limits": rate_limits
        } }));
        assert_eq!(
            both,
            RolloutRecord::Tokens { context: Some(ContextFill { used: 61_767, window: Some(258_400) }), limits: Some(expected) }
        );
    }

    #[test]
    fn a_null_or_malformed_limit_window_is_skipped() {
        let week = json!({ "used_percent": 40, "window_minutes": 10_080, "resets_at": 1_791_800_000 });
        let only_week = RolloutRecord::Tokens { context: None, limits: Some(vec![window(10_080, 40.0, Some(1_791_800_000_000))]) };
        assert_eq!(limits(json!({ "primary": null, "secondary": week })), only_week);
        for bad in [
            json!({ "used_percent": "12", "window_minutes": 300 }),
            json!({ "window_minutes": 300 }),
            json!({ "used_percent": 12, "window_minutes": 0 }),
            json!({ "used_percent": 12, "window_minutes": -300 }),
            json!({ "used_percent": 12, "window_minutes": 1.5 }),
            json!({ "used_percent": 12 }),
            json!(12),
        ] {
            assert_eq!(limits(json!({ "primary": bad, "secondary": week })), only_week, "primary {bad}");
        }
        for resets_at in [json!("1791800000"), json!(1.5), Value::Null] {
            let w = json!({ "used_percent": 40, "window_minutes": 10_080, "resets_at": resets_at });
            let no_reset = RolloutRecord::Tokens { context: None, limits: Some(vec![window(10_080, 40.0, None)]) };
            assert_eq!(limits(json!({ "primary": w })), no_reset, "resets_at {resets_at}");
        }
        // no usable window: a token_count before the first answer stays Other
        assert_eq!(limits(json!({ "primary": null, "secondary": null })), RolloutRecord::Other);
        assert_eq!(limits(json!({})), RolloutRecord::Other);
        assert_eq!(limits(Value::Null), RolloutRecord::Other);
        let absent = parse_rollout_line(&json!({ "type": "event_msg", "payload": { "type": "token_count", "info": null } }));
        assert_eq!(absent, RolloutRecord::Other, "no rate_limits");
    }

    #[test]
    fn only_the_codex_limit_bucket_counts() {
        // Codex also writes per-model buckets (`limit_id` "codex_bengalfox") and empty ones ("premium")
        // into the same rollout; only the account's `codex` bucket, or a Codex that writes no
        // `limit_id`, is the home page's.
        let primary = json!({ "used_percent": 12, "window_minutes": 300, "resets_at": 1_791_300_000 });
        let kept = RolloutRecord::Tokens { context: None, limits: Some(vec![window(300, 12.0, Some(1_791_300_000_000))]) };
        assert_eq!(limits(json!({ "limit_id": "codex", "primary": primary })), kept);
        assert_eq!(limits(json!({ "primary": primary })), kept, "no limit_id");
        assert_eq!(limits(json!({ "limit_id": null, "primary": primary })), kept, "limit_id null");
        for other in ["codex_bengalfox", "premium", "Codex", ""] {
            let record = limits(json!({ "limit_id": other, "limit_name": "GPT-5.3-Codex-Spark", "primary": primary }));
            assert_eq!(record, RolloutRecord::Other, "limit_id {other}");
        }
    }
```

- [ ] **Step 2: Write the failing model tests and extend the wire-shape test**

`model_tests.rs`, in `snapshot_serializes_to_the_spec_wire_shape`, replace

```rust
            "sources": { "claude": { "degraded": { "unreadable": 1 } }, "codex": "ok", "codexHooks": "not_installed" }
```

with

```rust
            "sources": { "claude": { "degraded": { "unreadable": 1 } }, "codex": "ok", "codexHooks": "not_installed" },
            "usage": { "codex": null }
```

At the end of `model_tests.rs`:

```rust
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
```

- [ ] **Step 3: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak --lib limit 2>&1 | grep -E "^error" | sort -u | head -8`
Expected: compile errors, among them `cannot find type \`CodexWindow\` in this scope` (or `cannot find struct, variant or union type \`CodexWindow\``) and `variant \`RolloutRecord::Tokens\` does not have a field named \`context\``.

- [ ] **Step 4: Implement the parser**

`codex.rs`, after `use serde_json::{Map, Value};`:

```rust

use super::model::{CodexWindow, ContextFill};
```

Change the derive above `pub enum RolloutRecord` from

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RolloutRecord {
```

to

```rust
#[derive(Debug, Clone, PartialEq)] // no Eq: CodexWindow carries an f64 percent
pub enum RolloutRecord {
```

Replace the `Tokens` variant and its doc comment

```rust
    /// `event_msg` `token_count` with `info.last_token_usage.total_tokens`: how full the context is
    /// (context-fill spec §2.1). `window` is `info.model_context_window` when it is an integer > 0.
    /// Only these two numbers are read.
    Tokens { used: u64, window: Option<u64> },
```

with

```rust
    /// `event_msg` `token_count`: how full the context is (`info.last_token_usage.total_tokens`,
    /// `window` = `info.model_context_window` when it is an integer > 0; context-fill spec §2.1) and
    /// the account's limit windows (`rate_limits.primary` / `secondary` of the `codex` bucket;
    /// usage-home spec §2.2). At least one of the two is Some. Only these numbers are read.
    Tokens { context: Option<ContextFill>, limits: Option<Vec<CodexWindow>> },
```

In the doc comment of `parse_rollout_line`, replace

```rust
/// - `event_msg` + `payload.type` token_count with an unsigned integer
///   `info.last_token_usage.total_tokens` → Tokens (`window`: `info.model_context_window` when it is
///   an integer > 0, else None); any other token_count (`info` null or absent, no such number) → Other;
```

with

```rust
/// - `event_msg` + `payload.type` token_count → Tokens when it has a context fill (an unsigned integer
///   `info.last_token_usage.total_tokens`; `window`: `info.model_context_window` when it is an integer
///   > 0) or at least one limit window (`rate_limits`); any other token_count → Other;
```

Replace the match arm

```rust
        ("event_msg", Some(p), Some("token_count")) => match context_fill(p) {
            Some((used, window)) => RolloutRecord::Tokens { used, window },
            None => RolloutRecord::Other,
        },
```

with

```rust
        ("event_msg", Some(p), Some("token_count")) => match (context_fill(p), rate_limits(p)) {
            (None, None) => RolloutRecord::Other,
            (context, limits) => RolloutRecord::Tokens { context, limits },
        },
```

Replace `fn context_fill` and its doc comment with:

```rust
/// The context fill of a `token_count` payload: `info.last_token_usage.total_tokens` as a u64 and
/// `info.model_context_window` when it is an integer > 0. None when `info` is null or absent or has
/// no such `total_tokens`.
fn context_fill(p: &Map<String, Value>) -> Option<ContextFill> {
    let info = p.get("info")?.as_object()?;
    let used = info.get("last_token_usage")?.get("total_tokens")?.as_u64()?;
    let window = info.get("model_context_window").and_then(Value::as_u64).filter(|w| *w > 0);
    Some(ContextFill { used, window })
}

/// The `rate_limits.limit_id` of the account's own limits. Codex also writes per-model buckets
/// (for example "codex_bengalfox") and empty ones ("premium") into the same rollout.
const CODEX_LIMIT_ID: &str = "codex";

/// The account's limit windows of a `token_count` payload (usage-home spec §2.2): `primary`, then
/// `secondary`, each kept by `codex_window`. None when `rate_limits` is no object, when its
/// `limit_id` is a string other than CODEX_LIMIT_ID, or when no window is kept. Only `limit_id`,
/// `primary` and `secondary` are looked at.
fn rate_limits(p: &Map<String, Value>) -> Option<Vec<CodexWindow>> {
    let limits = p.get("rate_limits")?.as_object()?;
    if limits.get("limit_id").and_then(Value::as_str).is_some_and(|id| id != CODEX_LIMIT_ID) {
        return None;
    }
    let windows: Vec<CodexWindow> =
        ["primary", "secondary"].into_iter().filter_map(|k| limits.get(k)).filter_map(codex_window).collect();
    (!windows.is_empty()).then_some(windows)
}

/// One window: a number `used_percent` and an integer `window_minutes` > 0, else None; `resets_at`
/// (seconds since the epoch) becomes milliseconds when it is an integer.
fn codex_window(v: &Value) -> Option<CodexWindow> {
    let percent = v.get("used_percent")?.as_f64()?;
    let minutes = v.get("window_minutes")?.as_u64().filter(|m| *m > 0)?;
    let resets_at_ms = v.get("resets_at").and_then(Value::as_i64).and_then(|s| s.checked_mul(1_000));
    Some(CodexWindow { minutes, percent, resets_at_ms })
}
```

(`TurnScan::feed` and `turn_record` in `io_thread` end with `_ => ...` and need no change.)

- [ ] **Step 5: Implement the model**

`model.rs`, after `pub struct ContextFill { .. }`:

```rust

/// One limit window of the Codex account (usage-home spec §2.2, §3): its length in minutes, the
/// percent used and, when Codex wrote it, the reset time (ms since the epoch).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWindow {
    pub minutes: u64,
    pub percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<i64>,
}

/// The last Codex limits read, from any session (usage-home spec §2.2); `seen_at_ms` is the store
/// clock when Mayak read them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUsage {
    pub seen_at_ms: i64,
    pub windows: Vec<CodexWindow>,
}

/// The plan limits of the home page (usage-home spec §3); a side is null until its source gave a
/// value. A change bumps `rev` and nothing else.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Usage {
    pub codex: Option<CodexUsage>,
}
```

In `pub struct SessionsSnapshot`, after `pub sources: Sources,`:

```rust
    pub usage: Usage,
```

In `struct View`, after `sources: Sources,`:

```rust
    usage: Usage,
```

In `pub struct Store`, after `codex_hooks: CodexHooksView,`:

```rust
    /// The plan limits of the home page (usage-home spec §3).
    usage: Usage,
```

In `Store::new`, after `codex_hooks: CodexHooksView::NotInstalled,`:

```rust
            usage: Usage::default(),
```

In `snapshot`, after `sources: v.sources,`:

```rust
            usage: v.usage,
```

In `view`, in the final `View { .. }` literal, after the `sources: Sources { .. },` field:

```rust
            usage: self.usage.clone(),
```

In `codex_rollout`, replace

```rust
        let Some(s) = self.sessions.get_mut(key) else { return false };
        let records: Vec<RolloutRecord> = records.iter().map(|(_, record)| parse_rollout_line(record)).collect();
```

with

```rust
        let records: Vec<RolloutRecord> = records.iter().map(|(_, record)| parse_rollout_line(record)).collect();
        // The account's limits (usage-home spec §2.2): the batch's last value wins, read at the store
        // clock, whether or not the store still knows this session.
        let limits = records.iter().rev().find_map(|r| match r {
            RolloutRecord::Tokens { limits: Some(windows), .. } => Some(windows.clone()),
            _ => None,
        });
        if let Some(windows) = limits {
            self.usage.codex = Some(CodexUsage { seen_at_ms: now_ms, windows });
        }
        let Some(s) = self.sessions.get_mut(key) else { return false };
```

and, in the second `match record` of the loop, replace

```rust
                RolloutRecord::Tokens { used, window } => s.context = Some(ContextFill { used, window }),
                RolloutRecord::Meta { .. } | RolloutRecord::UserMessage | RolloutRecord::Other | RolloutRecord::Unknown => {}
```

with

```rust
                RolloutRecord::Tokens { context: Some(fill), .. } => s.context = Some(fill),
                RolloutRecord::Tokens { context: None, .. }
                | RolloutRecord::Meta { .. }
                | RolloutRecord::UserMessage
                | RolloutRecord::Other
                | RolloutRecord::Unknown => {}
```

- [ ] **Step 6: Run the tests and the suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak --lib 2>&1 | grep -E "^test .*(limit|codex_usage|token_count|snapshot_serializes).* \.\.\. "` then `cargo test --workspace 2>&1 | grep -E "^test result|^warning|^error"`
Expected: every listed line ends `... ok`, among them `a_token_count_gives_the_codex_limit_windows`, `a_null_or_malformed_limit_window_is_skipped`, `only_the_codex_limit_bucket_counts`, `a_token_count_sets_the_codex_usage_and_the_last_value_wins`, `a_rollout_batch_of_an_unknown_session_still_sets_the_usage`, `codex_usage_changes_nothing_but_the_usage`, `the_codex_usage_is_on_the_wire`, the four context-fill `token_count` parser tests and `snapshot_serializes_to_the_spec_wire_shape`; suite: lib **453 passed, 1 ignored**, hook 32 passed, 0 failed, no `warning` line.

- [ ] **Step 7: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/observe/codex.rs windows/src-tauri/src/observe/model.rs windows/src-tauri/src/observe/model_tests.rs && git commit -m "feat(observe): read the Codex plan limits from token_count and put them in the snapshot"
```

---

### Task 2: Claude `cachedUsageUtilization` → `Signal::ClaudeUsage` → `Store.usage.claude`, and its three triggers

**Files:**
- Create: `windows/src-tauri/src/observe/claude_usage.rs`
- Modify: `windows/src-tauri/src/observe/model.rs` (wire types after `Usage`; `Usage`; `Signal` ~line 47-86; `apply` ~line 538-563)
- Modify: `windows/src-tauri/src/observe/claude.rs` (`io_thread` imports ~line 360-370; `ClaudeConfig` ~line 420; `ClaudeInput` ~line 430; `ClaudeIo` ~line 466-506; `start` ~line 524; `handle` ~line 552; tests: `rig` ~line 1101, `missing_dirs_report_missing` ~line 1284, `the_spawned_thread_serves_inputs` ~line 1763, new test at the module's end)
- Modify: `windows/src-tauri/src/observe/mod.rs` (module list ~line 6; `ObserveConfig` ~line 137; `Handle` ~line 149-172; `start` ~line 176-214; tests ~line 507-544)
- Modify: `windows/src-tauri/src/lib.rs` (`AppPaths` ~line 224-248; new command after `session_open` ~line 292; `generate_handler!` ~line 599; `ObserveConfig` literal ~line 642; `observe_wiring_tests` ~line 712-740)
- Test: `windows/src-tauri/src/observe/claude_usage.rs` (module `tests`), `windows/src-tauri/src/observe/claude.rs` (module `io_thread::tests`), `windows/src-tauri/src/observe/model_tests.rs` (end of file, wire-shape test)

**Interfaces:**
- Consumes: Task 1's `Usage`, `SessionsSnapshot.usage`, `Store.usage`; test helpers `run`, `prelude(Start::Working)`, `NOW`; `crate::testutil::TempDir { path }`; claude-io `Rig { dir, io, rx, .. }`, `rig(tag)`, `Rig::signals()`.
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] pub enum ClaudeWindowId { #[serde(rename = "5h")] FiveHour, #[serde(rename = "7d")] SevenDay }`
  - `#[derive(Debug, Clone, Copy, PartialEq, Serialize)] #[serde(rename_all = "camelCase")] pub struct ClaudeWindow { pub id: ClaudeWindowId, pub percent: f64, #[serde(skip_serializing_if = "Option::is_none")] pub resets_at_ms: Option<i64> }`
  - `#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")] pub struct ClaudeUsage { pub fetched_at_ms: i64, pub windows: Vec<ClaudeWindow> }`
  - `Usage { pub claude: Option<ClaudeUsage>, pub codex: Option<CodexUsage> }`; `Signal::ClaudeUsage(Option<ClaudeUsage>)`
  - `observe::claude_usage::{parse_claude_usage(bytes: &[u8]) -> Result<Option<ClaudeUsage>, String>, parse_rfc3339_ms(text: &str) -> Option<i64>, read_claude_usage(path: &Path) -> Result<Option<ClaudeUsage>, String>}`
  - `ClaudeConfig.usage_file: PathBuf`; `ClaudeInput::ReadUsage`; `ObserveConfig.claude_json: PathBuf`; `Handle::claude_usage(&self)`
  - Tauri command `usage_refresh` (no arguments, returns nothing) — Task 3 calls it as `Bridge.usageRefresh()`.

- [ ] **Step 1: Write the module with its failing tests**

Create `windows/src-tauri/src/observe/claude_usage.rs` with the tests and stubs that compile but fail:

```rust
// Claude plan limits (usage-home spec §2.1). Claude Code caches its answer about the plan limits in
// `.claude.json`, key `cachedUsageUtilization`. Only `fetchedAtMs` and the `utilization` and
// `resets_at` of `utilization.five_hour` / `utilization.seven_day` are read: every other key of the
// file is skipped by the deserializer, never held, logged or stored. Pure, except `read_claude_usage`.

use std::io;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::model::{ClaudeUsage, ClaudeWindow, ClaudeWindowId};

pub fn parse_claude_usage(_bytes: &[u8]) -> Result<Option<ClaudeUsage>, String> {
    Err("unimplemented".into())
}

pub fn parse_rfc3339_ms(_text: &str) -> Option<i64> {
    None
}

pub fn read_claude_usage(_path: &Path) -> Result<Option<ClaudeUsage>, String> {
    Err("unimplemented".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use serde_json::json;

    const RESETS: &str = "2026-10-06T14:20:00.123456+00:00";
    /// `RESETS` as ms since the epoch (Node: Date.parse).
    const RESETS_MS: i64 = 1_791_296_400_123;

    /// A `.claude.json` with keys Claude Code writes around the cached limits (values generic).
    fn file(cached: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "numStartups": 12,
            "oauthAccount": { "emailAddress": "dev@example.com", "accountUuid": "00000000-0000-4000-8000-000000000000" },
            "projects": { "C:/work/app": { "allowedTools": [] } },
            "cachedUsageUtilization": cached
        }))
        .unwrap()
    }

    fn window(utilization: Value, resets_at: Value) -> Value {
        json!({ "utilization": utilization, "resets_at": resets_at, "limit_dollars": null, "used_dollars": null })
    }

    #[test]
    fn the_full_object_gives_both_windows_and_nothing_else() {
        let cached = json!({
            "fetchedAtMs": 1_791_290_000_000i64,
            "accountUuid": "00000000-0000-4000-8000-000000000000",
            "utilization": {
                "five_hour": window(json!(37), json!(RESETS)),
                "seven_day": window(json!(81.5), json!("2026-10-09T17:20:00+03:00")),
                "seven_day_opus": null,
                "iguana_necktie": window(json!(99), json!(RESETS)),
                "extra_usage": { "is_enabled": false, "utilization": null },
                "spend": { "percent": 4 }
            }
        });
        assert_eq!(
            parse_claude_usage(&file(cached)),
            Ok(Some(ClaudeUsage {
                fetched_at_ms: 1_791_290_000_000,
                windows: vec![
                    ClaudeWindow { id: ClaudeWindowId::FiveHour, percent: 37.0, resets_at_ms: Some(RESETS_MS) },
                    ClaudeWindow { id: ClaudeWindowId::SevenDay, percent: 81.5, resets_at_ms: Some(1_791_555_600_000) },
                ],
            }))
        );
    }

    #[test]
    fn no_cached_object_gives_none() {
        let without = serde_json::to_vec(&json!({ "numStartups": 12, "oauthAccount": { "emailAddress": "dev@example.com" } })).unwrap();
        assert_eq!(parse_claude_usage(&without), Ok(None));
        assert_eq!(parse_claude_usage(&file(Value::Null)), Ok(None), "null");
        assert_eq!(parse_claude_usage(&file(json!("cached"))), Ok(None), "not an object");
        assert_eq!(parse_claude_usage(&file(json!({ "fetchedAtMs": 5, "utilization": 3 }))), Ok(None), "utilization not an object");
        assert_eq!(parse_claude_usage(&file(json!({ "fetchedAtMs": 5 }))), Ok(None), "no utilization");
        assert_eq!(parse_claude_usage(b"[1, 2]"), Ok(None), "not an object at the top");
    }

    #[test]
    fn a_window_whose_utilization_is_no_number_is_skipped() {
        let cached =
            |five: Value, seven: Value| file(json!({ "fetchedAtMs": 1_000, "utilization": { "five_hour": five, "seven_day": seven } }));
        let only_week = parse_claude_usage(&cached(window(json!("37"), json!(RESETS)), window(json!(12), Value::Null)));
        assert_eq!(
            only_week.unwrap().unwrap().windows,
            [ClaudeWindow { id: ClaudeWindowId::SevenDay, percent: 12.0, resets_at_ms: None }]
        );
        for bad in [json!(null), json!(true), json!([37]), json!({ "v": 37 })] {
            assert_eq!(parse_claude_usage(&cached(window(bad.clone(), json!(RESETS)), Value::Null)), Ok(None), "utilization {bad}");
        }
        assert_eq!(parse_claude_usage(&cached(json!(37), json!("x"))), Ok(None), "windows that are no objects");
    }

    #[test]
    fn an_unparsable_reset_date_gives_a_window_without_it() {
        for resets_at in [json!("soon"), json!("2026-10-06"), json!("2026-10-06T14:20:00"), json!(1_791_296_400), Value::Null] {
            let bytes = file(json!({ "fetchedAtMs": 1_000, "utilization": { "five_hour": window(json!(5), resets_at.clone()) } }));
            let usage = parse_claude_usage(&bytes).unwrap().unwrap();
            assert_eq!(
                usage.windows,
                [ClaudeWindow { id: ClaudeWindowId::FiveHour, percent: 5.0, resets_at_ms: None }],
                "resets_at {resets_at}"
            );
        }
    }

    #[test]
    fn fetched_at_must_be_an_integer() {
        for fetched in [Value::Null, json!("1791290000000"), json!(1.5)] {
            let bytes = file(json!({ "fetchedAtMs": fetched, "utilization": { "five_hour": window(json!(5), json!(RESETS)) } }));
            assert_eq!(parse_claude_usage(&bytes), Ok(None), "fetchedAtMs {fetched}");
        }
        let missing = file(json!({ "utilization": { "five_hour": window(json!(5), json!(RESETS)) } }));
        assert_eq!(parse_claude_usage(&missing), Ok(None));
    }

    #[test]
    fn a_torn_file_is_an_error_that_names_no_text_of_it() {
        // Claude Code rewrites the file while Mayak may read it: the caller keeps the last value.
        let whole = file(json!({ "fetchedAtMs": 1_000, "accountUuid": "secret-account-uuid", "utilization": {} }));
        assert_eq!(parse_claude_usage(&whole[..whole.len() - 30]), Err("Eof".to_string()));
        assert_eq!(parse_claude_usage(b""), Err("Eof".to_string()));
        let broken = br#"{"oauthAccount": {"emailAddress": "dev@example.com"} "cachedUsageUtilization": {}}"#;
        assert_eq!(parse_claude_usage(broken), Err("Syntax".to_string()));
    }

    #[test]
    fn a_missing_file_gives_none_and_an_existing_one_is_read() {
        let dir = TempDir::new("claude-usage");
        let path = dir.path.join(".claude.json");
        assert_eq!(read_claude_usage(&path), Ok(None));
        let bytes = file(json!({ "fetchedAtMs": 1_000, "utilization": { "seven_day": window(json!(3), json!(RESETS)) } }));
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(
            read_claude_usage(&path).unwrap().unwrap().windows,
            [ClaudeWindow { id: ClaudeWindowId::SevenDay, percent: 3.0, resets_at_ms: Some(RESETS_MS) }]
        );
        assert!(read_claude_usage(&dir.path).is_err(), "a folder is no file");
    }

    #[test]
    fn rfc3339_dates_become_epoch_milliseconds() {
        let at = 1_791_296_400_000; // 2026-10-06T14:20:00Z (Node: Date.parse)
        assert_eq!(parse_rfc3339_ms("2026-10-06T14:20:00Z"), Some(at));
        assert_eq!(parse_rfc3339_ms("2026-10-06T14:20:00.123456+00:00"), Some(at + 123), "fraction rounded down to ms");
        assert_eq!(parse_rfc3339_ms("2026-10-06T14:20:00.9Z"), Some(at + 900));
        assert_eq!(parse_rfc3339_ms("2026-10-06T17:20:00+03:00"), Some(at));
        assert_eq!(parse_rfc3339_ms("2026-10-06T09:20:00-05:00"), Some(at));
        assert_eq!(parse_rfc3339_ms("2024-02-29T00:00:00Z"), Some(1_709_164_800_000), "leap day");
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_ms("1969-12-31T23:59:59Z"), Some(-1_000));
    }

    #[test]
    fn anything_else_is_no_date() {
        for text in [
            "", "soon", "2026-10-06", "2026-10-06T14:20:00", "2026-10-06 14:20:00Z", "2026-10-06t14:20:00z",
            "2026-13-01T00:00:00Z", "2026-02-29T00:00:00Z", "2026-10-32T00:00:00Z", "2026-10-06T24:00:00Z",
            "2026-10-06T14:60:00Z", "2026-10-06T14:20:60Z", "2026-10-06T14:20:00.Z", "2026-10-06T14:20:00.1234567890Z",
            "2026-10-06T14:20:00+0300", "2026-10-06T14:20:00+24:00", "2026-10-06T14:20:00Z ", "２０２６-10-06T14:20:00Z",
            "+2026-10-06T14:20:00Z",
        ] {
            assert_eq!(parse_rfc3339_ms(text), None, "{text:?}");
        }
    }
}
```

`observe/mod.rs`, after `pub mod claude;`:

```rust
pub mod claude_usage;
```

`model.rs`, after `pub struct ContextFill { .. }` and before `pub struct CodexWindow` (Task 1):

```rust

/// Which Claude limit window (usage-home spec §3): `utilization.five_hour` is "5h", `seven_day` "7d".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ClaudeWindowId {
    #[serde(rename = "5h")]
    FiveHour,
    #[serde(rename = "7d")]
    SevenDay,
}

/// One Claude limit window: the percent used and, when its date parsed, the reset time (ms).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeWindow {
    pub id: ClaudeWindowId,
    pub percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<i64>,
}

/// What Claude Code cached about the plan limits (usage-home spec §2.1); `fetched_at_ms` is when
/// Claude Code got them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeUsage {
    pub fetched_at_ms: i64,
    pub windows: Vec<ClaudeWindow>,
}
```

- [ ] **Step 2: Write the failing model and claude-io tests**

`model_tests.rs`, in `snapshot_serializes_to_the_spec_wire_shape`, replace

```rust
            "usage": { "codex": null }
```

with

```rust
            "usage": { "claude": null, "codex": null }
```

At the end of `model_tests.rs`:

```rust
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
            "codex": null
        })
    );
}
```

`claude.rs`, module `io_thread::tests`: after `use std::time::Duration;` add

```rust
        use crate::observe::model::{ClaudeWindow, ClaudeWindowId};
```

and at the end of the module (after `the_spawned_thread_serves_inputs`, before the module's closing `}`):

```rust
        /// A `.claude.json` with the plan limits cached at `percent`, and keys around them.
        fn usage_json(percent: u32) -> String {
            serde_json::json!({
                "numStartups": 3,
                "oauthAccount": { "emailAddress": "dev@example.com" },
                "cachedUsageUtilization": {
                    "fetchedAtMs": 1_791_290_000_000i64,
                    "utilization": {
                        "five_hour": { "utilization": percent, "resets_at": "2026-10-06T14:20:00Z" },
                        "seven_day": null
                    }
                }
            })
            .to_string()
        }

        fn usage(percent: u32) -> ClaudeUsage {
            ClaudeUsage {
                fetched_at_ms: 1_791_290_000_000,
                windows: vec![ClaudeWindow {
                    id: ClaudeWindowId::FiveHour,
                    percent: f64::from(percent),
                    resets_at_ms: Some(1_791_296_400_000),
                }],
            }
        }

        fn usage_signals(signals: &[Signal]) -> Vec<Option<ClaudeUsage>> {
            signals
                .iter()
                .filter_map(|s| match s {
                    Signal::ClaudeUsage(u) => Some(u.clone()),
                    _ => None,
                })
                .collect()
        }

        #[test]
        fn the_usage_is_read_at_start_on_retry_and_on_request_and_sent_on_change_only() {
            let mut rig = rig("cio-usage");
            let file = rig.dir.path.join(".claude.json");
            std::fs::write(&file, usage_json(12)).unwrap();
            rig.io.start();
            assert_eq!(usage_signals(&rig.signals()), [Some(usage(12))], "read at start");
            rig.io.handle(ClaudeInput::Retry);
            assert_eq!(usage_signals(&rig.signals()), [], "unchanged: not sent again");
            std::fs::write(&file, usage_json(30)).unwrap();
            rig.io.handle(ClaudeInput::ReadUsage);
            assert_eq!(usage_signals(&rig.signals()), [Some(usage(30))], "the home view opened");
            let torn = usage_json(31);
            std::fs::write(&file, &torn[..torn.len() / 2]).unwrap();
            rig.io.handle(ClaudeInput::Retry);
            assert_eq!(usage_signals(&rig.signals()), [], "a file being written keeps the last value");
            std::fs::remove_file(&file).unwrap();
            rig.io.handle(ClaudeInput::Retry);
            assert_eq!(usage_signals(&rig.signals()), [None], "no file: null");
        }
```

- [ ] **Step 3: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak --lib usage 2>&1 | grep -E "^error" | sort -u | head -8`
Expected: compile errors, among them `no variant or associated item named \`ClaudeUsage\` found for enum \`Signal\``, `no variant or associated item named \`ReadUsage\` found for enum \`ClaudeInput\`` and `no field \`claude\` on type \`Usage\``.

- [ ] **Step 4: Implement the parser and the reader**

In `claude_usage.rs`, replace the three stubs (from `pub fn parse_claude_usage(_bytes` to the end of the `read_claude_usage` stub) with:

```rust
/// The only keys of `.claude.json` the deserializer keeps; serde skips every other key.
#[derive(Deserialize)]
struct ClaudeJson {
    #[serde(rename = "cachedUsageUtilization", default)]
    cached: Option<Cached>,
}

#[derive(Deserialize)]
struct Cached {
    #[serde(rename = "fetchedAtMs", default)]
    fetched_at_ms: Option<Value>,
    #[serde(default)]
    utilization: Option<Windows>,
}

#[derive(Deserialize)]
struct Windows {
    #[serde(default)]
    five_hour: Option<Value>,
    #[serde(default)]
    seven_day: Option<Value>,
}

/// What `.claude.json` says about the plan limits (usage-home spec §2.1). Ok(None): no
/// `cachedUsageUtilization` (or one, or a `utilization`, of another shape), no integer
/// `fetchedAtMs`, or no window with a number `utilization`. Err: the bytes are no complete JSON
/// document (Claude Code may be writing the file); the error is the serde_json category only
/// ("Syntax", "Eof", "Io"), never any text of the file.
pub fn parse_claude_usage(bytes: &[u8]) -> Result<Option<ClaudeUsage>, String> {
    let file: ClaudeJson = match serde_json::from_slice(bytes) {
        Ok(file) => file,
        Err(err) if err.classify() == serde_json::error::Category::Data => return Ok(None),
        Err(err) => return Err(format!("{:?}", err.classify())),
    };
    let Some(cached) = file.cached else { return Ok(None) };
    let Some(fetched_at_ms) = cached.fetched_at_ms.as_ref().and_then(Value::as_i64) else { return Ok(None) };
    let Some(utilization) = cached.utilization else { return Ok(None) };
    let windows: Vec<ClaudeWindow> = [(ClaudeWindowId::FiveHour, utilization.five_hour), (ClaudeWindowId::SevenDay, utilization.seven_day)]
        .into_iter()
        .filter_map(|(id, v)| claude_window(id, &v?))
        .collect();
    Ok((!windows.is_empty()).then_some(ClaudeUsage { fetched_at_ms, windows }))
}

/// A window with a number `utilization`, else None; `resets_at` kept when it parses.
fn claude_window(id: ClaudeWindowId, v: &Value) -> Option<ClaudeWindow> {
    let percent = v.get("utilization")?.as_f64()?;
    let resets_at_ms = v.get("resets_at").and_then(Value::as_str).and_then(parse_rfc3339_ms);
    Some(ClaudeWindow { id, percent, resets_at_ms })
}

/// `.claude.json` read once (usage-home spec §2.1). Ok(None) when the file does not exist; Err with
/// the I/O kind or the JSON category when it cannot be read or parsed right now.
pub fn read_claude_usage(path: &Path) -> Result<Option<ClaudeUsage>, String> {
    match std::fs::read(path) {
        Ok(bytes) => parse_claude_usage(&bytes),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("{:?}", err.kind())),
    }
}

/// Milliseconds since the Unix epoch of an RFC 3339 date-time: `YYYY-MM-DDTHH:MM:SS`, an optional
/// `.` with 1 to 9 digits (rounded down to the millisecond), then `Z` or `±HH:MM`. None for
/// anything else, also for a date that does not exist (2026-02-29) or a second 60.
pub fn parse_rfc3339_ms(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        b.get(from..from + len)?
            .iter()
            .try_fold(0_i64, |acc, d| d.is_ascii_digit().then(|| acc * 10 + i64::from(d - b'0')))
    };
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hour, minute, second) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut i = 19;
    let mut millis = 0_i64;
    if b[i] == b'.' {
        let digits = b[i + 1..].iter().take_while(|d| d.is_ascii_digit()).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let kept = digits.min(3);
        millis = b[i + 1..i + 1 + kept].iter().fold(0, |acc, d| acc * 10 + i64::from(d - b'0'));
        millis *= 10_i64.pow(3 - kept as u32);
        i += 1 + digits;
    }
    let offset_minutes = match &b[i..] {
        [b'Z'] => 0,
        [sign @ (b'+' | b'-'), _, _, b':', _, _] => {
            let (hours, minutes) = (num(i + 1, 2)?, num(i + 4, 2)?);
            if hours > 23 || minutes > 59 {
                return None;
            }
            let total = hours * 60 + minutes;
            if *sign == b'-' { -total } else { total }
        }
        _ => return None,
    };
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second - offset_minutes * 60;
    Some(seconds * 1_000 + millis)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to a proleptic Gregorian date (H. Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12; // March = 0
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
```

(`b[i]` at `i = 19` is safe: the length check guarantees at least 20 bytes.)

- [ ] **Step 5: Implement the model, claude-io, the handle and the command**

`model.rs`, in `pub struct Usage`, before `pub codex: Option<CodexUsage>,`:

```rust
    pub claude: Option<ClaudeUsage>,
```

In `pub enum Signal`, after the `ClaudeGone { key: SessionKey },` variant:

```rust
    /// claude-io: what `.claude.json` says about the plan limits (usage-home spec §2.1); sent at
    /// its start, on Retry and on ReadUsage when it differs from the last value sent; None when the
    /// file or the key is absent.
    ClaudeUsage(Option<ClaudeUsage>),
```

In `apply`, after `Signal::ClaudeGone { key } => self.remove_session(&key, &mut fx),`:

```rust
            Signal::ClaudeUsage(usage) => self.usage.claude = usage,
```

`claude.rs`, `io_thread` imports: replace

```rust
    use crate::log;
    use crate::observe::keys::{claude_key, ProcId, SessionKey};
    use crate::observe::model::{Effect, Health, Signal, Source};
```

with

```rust
    use crate::log;
    use crate::observe::claude_usage::read_claude_usage;
    use crate::observe::keys::{claude_key, ProcId, SessionKey};
    use crate::observe::model::{ClaudeUsage, Effect, Health, Signal, Source};
```

In `pub struct ClaudeConfig`, after the `base` field:

```rust
        /// `.claude.json`: `<CLAUDE_CONFIG_DIR>\.claude.json`, else `%USERPROFILE%\.claude.json`
        /// (usage-home spec §2.1). Only its cached plan limits are read.
        pub usage_file: PathBuf,
```

In `pub enum ClaudeInput`, after `Retry,`:

```rust
        /// The home view opened: read the cached plan limits again (usage-home spec §2.1).
        ReadUsage,
```

In `struct ClaudeIo`, after `health: Health,`:

```rust
        /// Last plan limits sent; None at start, like the store. Sent again only on change.
        usage: Option<ClaudeUsage>,
```

In `ClaudeIo::new`, after `health: Health::Ok,`:

```rust
                usage: None,
```

Replace `fn start`:

```rust
        fn start(&mut self) {
            self.ensure_watchers();
            self.rescan();
            self.report_health();
        }
```

with

```rust
        fn start(&mut self) {
            self.ensure_watchers();
            self.rescan();
            self.read_usage();
            self.report_health();
        }
```

In `handle`, replace

```rust
                ClaudeInput::Retry => self.retry(),
```

with

```rust
                ClaudeInput::Retry => {
                    self.retry();
                    // Visible(true) sends Retry: the island was shown (usage-home spec §2.1).
                    self.read_usage();
                }
                ClaudeInput::ReadUsage => self.read_usage(),
```

After `fn report_health` (inside the same `impl ClaudeIo` block, before its closing `}`):

```rust

        /// Reads the plan limits Claude Code cached (usage-home spec §2.1) and sends them when they
        /// differ from the last value sent. A file that cannot be read or parsed now keeps the last
        /// value; the log names only the error kind, never anything of the file.
        fn read_usage(&mut self) {
            match read_claude_usage(&self.cfg.usage_file) {
                Ok(usage) if usage != self.usage => {
                    self.usage = usage.clone();
                    self.emit(Signal::ClaudeUsage(usage));
                }
                Ok(_) => {}
                Err(reason) => log::line(format!("claude usage unreadable: {reason}")),
            }
        }
```

In the claude-io tests, replace the three `ClaudeConfig` literals:

```rust
            let cfg = ClaudeConfig { base: dir.path.clone(), appdata: dir.path.join("appdata"), watch: false };
```

→

```rust
            let cfg = ClaudeConfig {
                base: dir.path.clone(),
                usage_file: dir.path.join(".claude.json"),
                appdata: dir.path.join("appdata"),
                watch: false,
            };
```

```rust
            let cfg = ClaudeConfig { base: dir.path.join("absent"), appdata: dir.path.clone(), watch: false };
```

→

```rust
            let cfg = ClaudeConfig {
                base: dir.path.join("absent"),
                usage_file: dir.path.join("absent.claude.json"),
                appdata: dir.path.clone(),
                watch: false,
            };
```

```rust
            let cfg = ClaudeConfig { base: rig.dir.path.clone(), appdata: rig.dir.path.clone(), watch: false };
```

→

```rust
            let cfg = ClaudeConfig {
                base: rig.dir.path.clone(),
                usage_file: rig.dir.path.join(".claude.json"),
                appdata: rig.dir.path.clone(),
                watch: false,
            };
```

`observe/mod.rs`, in `pub struct ObserveConfig`, after the `claude_dir` field:

```rust
    /// $CLAUDE_CONFIG_DIR\.claude.json or %USERPROFILE%\.claude.json (usage-home spec §2.1)
    pub claude_json: PathBuf,
```

In `pub struct Handle`, replace

```rust
    signals: Sender<Signal>,
    codex: Sender<CodexInput>,
    shared: Arc<Mutex<SessionsSnapshot>>,
    signals_closed: Arc<AtomicBool>,
    codex_closed: Arc<AtomicBool>,
```

with

```rust
    signals: Sender<Signal>,
    claude: Sender<ClaudeInput>,
    codex: Sender<CodexInput>,
    shared: Arc<Mutex<SessionsSnapshot>>,
    signals_closed: Arc<AtomicBool>,
    claude_closed: Arc<AtomicBool>,
    codex_closed: Arc<AtomicBool>,
```

In `impl Handle`, after `pub fn codex_hook(..) { .. }`:

```rust

    /// The home view opened (usage-home spec §2.1): claude-io reads `.claude.json` once more and
    /// sends the result to the store if it changed. Never blocks.
    pub fn claude_usage(&self) {
        send_logged(&self.claude, ClaudeInput::ReadUsage, "claude-io", &self.claude_closed);
    }
```

In `start`, replace

```rust
    let claude_cfg = ClaudeConfig { base: cfg.claude_dir, appdata: cfg.appdata, watch: true };
```

with

```rust
    let claude_cfg = ClaudeConfig { base: cfg.claude_dir, usage_file: cfg.claude_json, appdata: cfg.appdata, watch: true };
```

replace

```rust
    let store_shared = shared.clone();
    let store_codex = codex.clone();
    let spawned = std::thread::Builder::new()
        .name("store".into())
        .spawn(move || run_store(rx, publisher, claude, store_codex, store_shared, now_ms));
```

with

```rust
    let store_shared = shared.clone();
    let store_claude = claude.clone();
    let store_codex = codex.clone();
    let spawned = std::thread::Builder::new()
        .name("store".into())
        .spawn(move || run_store(rx, publisher, store_claude, store_codex, store_shared, now_ms));
```

and replace the returned literal

```rust
    Handle {
        signals,
        codex,
        shared,
        signals_closed: Arc::new(AtomicBool::new(false)),
        codex_closed: Arc::new(AtomicBool::new(false)),
    }
```

with

```rust
    Handle {
        signals,
        claude,
        codex,
        shared,
        signals_closed: Arc::new(AtomicBool::new(false)),
        claude_closed: Arc::new(AtomicBool::new(false)),
        codex_closed: Arc::new(AtomicBool::new(false)),
    }
```

`mod.rs` tests: in `start_reports_a_missing_claude_dir_and_serves_the_snapshot`, after `claude_dir: dir.path.join("no-claude"),` add

```rust
                claude_json: dir.path.join("no-claude.json"),
```

In `handle_sends_never_panic_without_receivers`, replace the body

```rust
        let handle = Handle {
            signals: mpsc::channel().0,
            codex: mpsc::channel().0,
            shared: Arc::new(Mutex::new(Store::new().snapshot(0))),
            signals_closed: Arc::new(AtomicBool::new(false)),
            codex_closed: Arc::new(AtomicBool::new(false)),
        };
        handle.send(Signal::Visible(true));
        handle.send(Signal::Visible(false));
        handle.codex_hook(serde_json::json!({}));
        assert!(handle.signals_closed.load(Ordering::Relaxed));
        assert!(handle.codex_closed.load(Ordering::Relaxed));
        assert_eq!(handle.snapshot().rev, 0);
```

with

```rust
        let handle = Handle {
            signals: mpsc::channel().0,
            claude: mpsc::channel().0,
            codex: mpsc::channel().0,
            shared: Arc::new(Mutex::new(Store::new().snapshot(0))),
            signals_closed: Arc::new(AtomicBool::new(false)),
            claude_closed: Arc::new(AtomicBool::new(false)),
            codex_closed: Arc::new(AtomicBool::new(false)),
        };
        handle.send(Signal::Visible(true));
        handle.send(Signal::Visible(false));
        handle.codex_hook(serde_json::json!({}));
        handle.claude_usage();
        assert!(handle.signals_closed.load(Ordering::Relaxed));
        assert!(handle.claude_closed.load(Ordering::Relaxed));
        assert!(handle.codex_closed.load(Ordering::Relaxed));
        assert_eq!(handle.snapshot().rev, 0);
```

`lib.rs`, in `struct AppPaths`, after `claude_dir: std::path::PathBuf,`:

```rust
    /// `.claude.json`, where Claude Code caches the plan limits (usage-home spec §2.1).
    claude_json: std::path::PathBuf,
```

In `app_paths_from`, replace

```rust
    let home = get(platform::HOME_VAR).unwrap_or_else(platform::home_dir);
    AppPaths {
        claude_dir: get("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
```

with

```rust
    let home = get(platform::HOME_VAR).unwrap_or_else(platform::home_dir);
    let claude_config_dir = get("CLAUDE_CONFIG_DIR");
    AppPaths {
        // `.claude.json` sits inside $CLAUDE_CONFIG_DIR, else next to the `.claude` folder.
        claude_json: claude_config_dir.as_ref().map_or_else(|| home.join(".claude.json"), |dir| dir.join(".claude.json")),
        claude_dir: claude_config_dir.unwrap_or_else(|| home.join(".claude")),
```

After `fn session_open(..) { .. }`:

```rust

/// The home view became active: read Claude Code's cached plan limits again (usage-home spec §2.1).
/// Returns at once; the read runs on claude-io and its result comes with the next snapshot.
#[tauri::command]
fn usage_refresh(observe: State<observe::Handle>) {
    observe.claude_usage();
}
```

In `tauri::generate_handler![..]`, after `session_open,`:

```rust
            usage_refresh,
```

In `setup`, in the `observe::ObserveConfig { .. }` literal, after `claude_dir: paths.claude_dir.clone(),`:

```rust
                    claude_json: paths.claude_json.clone(),
```

In `observe_wiring_tests::app_paths_default_under_the_user_profile`, after `claude_dir: PathBuf::from(r"C:\Users\dev\.claude"),`:

```rust
                claude_json: PathBuf::from(r"C:\Users\dev\.claude.json"),
```

In `app_paths_follow_claude_config_dir_and_codex_home`, after `assert_eq!(paths.claude_dir, PathBuf::from(r"D:\cfg\claude"));`:

```rust
        assert_eq!(paths.claude_json, PathBuf::from(r"D:\cfg\claude\.claude.json"));
```

- [ ] **Step 6: Line endings of the new file, the tests and the suite**

Run: `cd /c/Users/HP/source/repos/mayak/windows && unix2dos src-tauri/src/observe/claude_usage.rs && file src-tauri/src/observe/claude_usage.rs src-tauri/src/observe/claude.rs src-tauri/src/observe/mod.rs src-tauri/src/observe/model.rs src-tauri/src/lib.rs`
Expected: every file `with CRLF line terminators`.

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak --lib 2>&1 | grep -E "^test (observe::claude_usage::|observe::claude::io_thread::tests::the_usage_is_read|observe::model::tests::(a_claude_usage_signal|claude_usage_changes_nothing_but_the_usage|the_claude_usage_is_on_the_wire|snapshot_serializes)|observe::tests::handle_sends|observe_wiring_tests::)"` then `cargo test --workspace 2>&1 | grep -E "^test result|^warning|^error"`
Expected: the first run lists exactly 17 lines (9 `claude_usage` tests, 1 claude-io test, 3 new model tests, `snapshot_serializes_to_the_spec_wire_shape`, `handle_sends_never_panic_without_receivers`, 2 `app_paths_*` tests), each ending `... ok`; suite: lib **466 passed, 1 ignored**, hook 32 passed, 0 failed, no `warning` line.

- [ ] **Step 7: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/observe/claude_usage.rs windows/src-tauri/src/observe/claude.rs windows/src-tauri/src/observe/mod.rs windows/src-tauri/src/observe/model.rs windows/src-tauri/src/observe/model_tests.rs windows/src-tauri/src/lib.rs && git commit -m "feat(observe): read Claude Code's cached plan limits from .claude.json"
```

---

### Task 3: The home view (bridge types, pure texts, view, CSS, minute refresh)

**Files:**
- Modify: `windows/src/core/bridge.ts` (`Bridge` ~line 76; types before `SessionsSnapshot` ~line 172 and inside it ~line 177)
- Create: `windows/src/views/usage-format.ts`, `windows/src/views/usage.ts`
- Modify: `windows/src/views/sessions.ts` (imports ~line 13; `contextLabel` ~line 87; `contextRing` ~line 144-145)
- Modify: `windows/src/views/views.ts` (imports ~line 13; home tab ~line 100; `buildOverview` ~line 149-151; registry ~line 518)
- Modify: `windows/src/core/state.ts` (`defaultView` ~line 260-264)
- Modify: `windows/src/style.css` (append at the end; new selectors only)
- Create: `C:\tmp\mayak-spike\usage-format-check.mjs` (outside the repository)

**Interfaces:**
- Consumes: the wire field `usage` (Tasks 1-2); the command `usage_refresh` (Task 2); `contextRing(percent: number): SVGSVGElement` and the `.sess-ctx` / `.sess-ctx.warn` / `.sess-ctx.full` / `.sess-ctx-ring` CSS of `views/sessions.ts` (context fill); `ViewHost.setActive` (called by `island.ts` `syncDom` only when "expanded and on screen" changes).
- Produces:
  - `bridge.ts`: `export interface ClaudeUsageWindow { id: "5h" | "7d"; percent: number; resetsAtMs?: number }`, `export interface CodexUsageWindow { minutes: number; percent: number; resetsAtMs?: number }`, `export interface Usage { claude: { fetchedAtMs: number; windows: ClaudeUsageWindow[] } | null; codex: { seenAtMs: number; windows: CodexUsageWindow[] } | null }`, `SessionsSnapshot.usage: Usage`, `Bridge.usageRefresh(): Promise<void | null>`
  - `usage-format.ts`: `export type PercentTone = "dim" | "warn" | "full"`, `percentTone(percent: number): PercentTone`, `usagePercent(percent: number): { text: string; ring: number; tone: PercentTone }`, `codexWindowName(minutes: number): string`, `resetsText(resetsAtMs: number, nowMs: number): string`, `STALE_MS = 600_000`, `staleText(atMs: number, nowMs: number): string`, `interface UsageLine { name: string; percent: { text; ring; tone }; resets: string }`, `interface UsageBlock { title: "Claude" | "Codex"; lines: UsageLine[]; note: string; empty: boolean }`, `usageBlocks(usage: Usage | undefined, nowMs: number): UsageBlock[]`
  - `usage.ts`: `export function buildHome(): ViewHost`
  - `sessions.ts`: `export function contextRing` (was module-local)
  - `views.ts`: `export function buildOverview` (kept, no longer registered)
  - CSS classes `.usage`, `.usage-block`, `.usage-name`, `.usage-line`, `.usage-window`, `.usage-reset`, `.usage-empty`, `.usage-note`

- [ ] **Step 1: Bridge types and command**

`bridge.ts`, after `sessionOpen: (key: string) => call<boolean>("session_open", { key }),`:

```ts
  /** The home view opened: Rust reads Claude Code's cached limits again; the result comes with the next snapshot. */
  usageRefresh: () => call<void>("usage_refresh"),
```

After `export interface ContextFill { used: number; window?: number }`:

```ts
/** One window of a plan limit (usage-home spec §3); `resetsAtMs` only when the agent wrote a valid time. */
export interface ClaudeUsageWindow { id: "5h" | "7d"; percent: number; resetsAtMs?: number }
export interface CodexUsageWindow { minutes: number; percent: number; resetsAtMs?: number }
/** The home page's limits; a side is null until its source gave a value. */
export interface Usage {
  claude: { fetchedAtMs: number; windows: ClaudeUsageWindow[] } | null;
  codex: { seenAtMs: number; windows: CodexUsageWindow[] } | null;
}
```

In `SessionsSnapshot`, after `  sources: { claude: Health; codex: Health; codexHooks: CodexHooksState };`:

```ts
  usage: Usage;
```

- [ ] **Step 2: The pure texts and their check (write the check first)**

Create `C:\tmp\mayak-spike\usage-format-check.mjs`:

```js
// Checks the pure texts of the Mayak home page (usage-home spec §4, §5 "TS") with Node's type stripping.
// Run: node --experimental-strip-types C:/tmp/mayak-spike/usage-format-check.mjs
import assert from "node:assert/strict";

const m = await import("file:///C:/Users/HP/source/repos/mayak/windows/src/views/usage-format.ts");
const MIN = 60_000, HOUR = 60 * MIN, DAY = 24 * HOUR;
const now = 1_791_296_400_000;

assert.deepEqual([79, 80, 94, 95, 100].map((p) => m.percentTone(p)), ["dim", "warn", "warn", "full", "full"]);
assert.deepEqual(m.usagePercent(62.7), { text: "62%", ring: 62, tone: "dim" });
assert.deepEqual(m.usagePercent(130), { text: "100%", ring: 100, tone: "full" });
assert.deepEqual(m.usagePercent(-5), { text: "0%", ring: 0, tone: "dim" });
assert.deepEqual(
  [300, 10080, 60, 90, 1439, 1440, 2880, 45].map((n) => m.codexWindowName(n)),
  ["5h", "7d", "1h", "1h", "23h", "1d", "2d", "45m"],
);
assert.equal(m.resetsText(now + 2 * HOUR + 14 * MIN + 30_000, now), "resets in 2h 14m");
assert.equal(m.resetsText(now + 3 * DAY + 4 * HOUR + 5 * MIN, now), "resets in 3d 4h");
assert.equal(m.resetsText(now + 12 * MIN, now), "resets in 12m");
assert.equal(m.resetsText(now + MIN, now), "resets in 1m");
assert.equal(m.resetsText(now + 59_999, now), "resets soon");
assert.equal(m.resetsText(now, now), "reset");
assert.equal(m.resetsText(now - 1, now), "reset");
assert.equal(m.resetsText(now + DAY, now), "resets in 1d 0h");
assert.equal(m.STALE_MS, 10 * MIN);
assert.equal(m.staleText(now - 10 * MIN, now), "");
assert.equal(m.staleText(now - 10 * MIN - 1, now), "as of 10m ago");
assert.equal(m.staleText(now - 7 * HOUR, now), "as of 7h ago");
assert.equal(m.staleText(now - 3 * DAY, now), "as of 3d ago");
assert.equal(m.staleText(now + HOUR, now), "");
assert.deepEqual(m.usageBlocks(undefined, now), [
  { title: "Claude", lines: [], note: "Claude Code updates it while it runs", empty: true },
  { title: "Codex", lines: [], note: "Codex reports it with each answer", empty: true },
]);
const usage = {
  claude: {
    fetchedAtMs: now - 7 * HOUR,
    windows: [{ id: "5h", percent: 82, resetsAtMs: now + 2 * HOUR + 14 * MIN }, { id: "7d", percent: 40 }],
  },
  codex: { seenAtMs: now - MIN, windows: [{ minutes: 300, percent: 96, resetsAtMs: now - 1 }] },
};
assert.deepEqual(m.usageBlocks(usage, now), [
  { title: "Claude", empty: false, note: "as of 7h ago", lines: [
    { name: "5h", percent: { text: "82%", ring: 82, tone: "warn" }, resets: "resets in 2h 14m" },
    { name: "7d", percent: { text: "40%", ring: 40, tone: "dim" }, resets: "" },
  ] },
  { title: "Codex", empty: false, note: "", lines: [
    { name: "5h", percent: { text: "96%", ring: 96, tone: "full" }, resets: "reset" },
  ] },
]);
assert.equal(m.usageBlocks({ claude: { fetchedAtMs: now, windows: [] }, codex: null }, now)[0].empty, true);
console.log("usage-format: all checks passed");
```

Run: `node --experimental-strip-types C:/tmp/mayak-spike/usage-format-check.mjs`
Expected: FAIL with `ERR_MODULE_NOT_FOUND` (the module does not exist yet).

Create `windows/src/views/usage-format.ts`:

```ts
// Pure texts of the home page's plan limits (usage-home spec §4). No DOM and no runtime imports,
// so Node can run these functions as they are (C:\tmp\mayak-spike\usage-format-check.mjs).

import type { Usage } from "../core/bridge";

export type PercentTone = "dim" | "warn" | "full";

/** Colour of a percent: grey below 80, "warn" from 80, "full" from 95 (context-fill spec §4, usage-home spec §1). */
export function percentTone(percent: number): PercentTone {
  return percent >= 95 ? "full" : percent >= 80 ? "warn" : "dim";
}

/** A limit's label: the percent rounded down and held within 0-100, for the text, the ring and the colour. */
export function usagePercent(percent: number): { text: string; ring: number; tone: PercentTone } {
  const shown = Math.max(0, Math.min(100, Math.floor(percent)));
  return { text: `${shown}%`, ring: shown, tone: percentTone(shown) };
}

/** Codex window name from `window_minutes`, rounded down: under 60 "Nm", under 1440 "Nh", else "Nd" (300 → "5h", 10080 → "7d"). */
export function codexWindowName(minutes: number): string {
  if (minutes < 60) return `${minutes}m`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)}h`;
  return `${Math.floor(minutes / 1440)}d`;
}

/** "resets in 2h 14m", "resets in 3d 4h", "resets in 12m", "resets soon" under a minute, "reset" once the time has passed. */
export function resetsText(resetsAtMs: number, nowMs: number): string {
  const left = resetsAtMs - nowMs;
  if (left <= 0) return "reset";
  const minutes = Math.floor(left / 60_000);
  if (minutes < 1) return "resets soon";
  if (minutes < 60) return `resets in ${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `resets in ${hours}h ${minutes % 60}m`;
  return `resets in ${Math.floor(hours / 24)}d ${hours % 24}h`;
}

/** Data older than this get an "as of … ago" line (usage-home spec §4). */
export const STALE_MS = 10 * 60_000;

/** "as of 7h ago" when `atMs` lies more than STALE_MS before `nowMs`, else "". */
export function staleText(atMs: number, nowMs: number): string {
  const age = nowMs - atMs;
  if (age <= STALE_MS) return "";
  const minutes = Math.floor(age / 60_000);
  if (minutes < 60) return `as of ${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `as of ${hours}h ago`;
  return `as of ${Math.floor(hours / 24)}d ago`;
}

export interface UsageLine {
  name: string;
  percent: { text: string; ring: number; tone: PercentTone };
  resets: string;
}

export interface UsageBlock {
  title: "Claude" | "Codex";
  lines: UsageLine[];
  /** The freshness line ("" when fresh), or the hint of an empty block. */
  note: string;
  empty: boolean;
}

const EMPTY_HINT = {
  Claude: "Claude Code updates it while it runs",
  Codex: "Codex reports it with each answer",
} as const;

/** The two blocks of the home page: Claude (dated by its cache) and Codex (dated by the last answer read). */
export function usageBlocks(usage: Usage | undefined, nowMs: number): UsageBlock[] {
  const claude = usage?.claude ?? null;
  const codex = usage?.codex ?? null;
  return [
    block(
      "Claude",
      claude && claude.windows.map((w) => ({ name: w.id, percent: w.percent, resetsAtMs: w.resetsAtMs })),
      claude?.fetchedAtMs ?? 0,
      nowMs,
    ),
    block(
      "Codex",
      codex && codex.windows.map((w) => ({ name: codexWindowName(w.minutes), percent: w.percent, resetsAtMs: w.resetsAtMs })),
      codex?.seenAtMs ?? 0,
      nowMs,
    ),
  ];
}

function block(
  title: "Claude" | "Codex",
  windows: { name: string; percent: number; resetsAtMs?: number }[] | null,
  atMs: number,
  nowMs: number,
): UsageBlock {
  if (!windows || windows.length === 0) return { title, lines: [], note: EMPTY_HINT[title], empty: true };
  const lines = windows.map((w) => ({
    name: w.name,
    percent: usagePercent(w.percent),
    resets: w.resetsAtMs === undefined ? "" : resetsText(w.resetsAtMs, nowMs),
  }));
  return { title, lines, note: staleText(atMs, nowMs), empty: false };
}
```

Run: `node --experimental-strip-types C:/tmp/mayak-spike/usage-format-check.mjs`
Expected: an `ExperimentalWarning: Type Stripping is an experimental feature` line on stderr, then `usage-format: all checks passed`, exit code 0.

- [ ] **Step 3: Share the ring and the thresholds with the session rows**

`views/sessions.ts`, after `import type { ContextFill, Host, ProjectView, SessionView, SilentReason } from "../core/bridge";`:

```ts
import { percentTone } from "./usage-format";
```

In `contextLabel`, replace

```ts
    const tone: ContextTone = percent >= 95 ? "full" : percent >= 80 ? "warn" : "dim";
```

with

```ts
    const tone: ContextTone = percentTone(percent);
```

Replace

```ts
/** A 12 px ring filled clockwise from the top to `percent`, in the label's colour. */
function contextRing(percent: number): SVGSVGElement {
```

with

```ts
/** A 12 px ring filled clockwise from the top to `percent`, in the label's colour. Also the home page's limits (views/usage.ts). */
export function contextRing(percent: number): SVGSVGElement {
```

- [ ] **Step 4: The home view**

Create `windows/src/views/usage.ts`:

```ts
// The home page ("overview", usage-home spec §4): how much of the Claude and Codex plan limits is
// used and when they reset. Everything comes from the Rust snapshot. The texts are rewritten on
// each render and once a minute, only while the page is on screen (setActive), never through
// State.notify(), so the frame loop stays asleep.

import { h, clear } from "./dom";
import { Bridge } from "../core/bridge";
import { Sessions } from "../core/sessions";
import { contextRing } from "./sessions";
import { usageBlocks, type UsageBlock } from "./usage-format";
import type { ViewHost } from "./views";

/** The refresh of "resets in …" and "as of …" while the page is on screen. */
const REFRESH_MS = 60_000;

function blockEl(b: UsageBlock): HTMLElement {
  const el = h("div", { class: "usage-block" }, h("div", { class: "usage-name", text: b.title }));
  if (b.empty) el.append(h("div", { class: "usage-empty", text: "No data yet" }));
  for (const line of b.lines) {
    // The context label of a session row: the same ring, colours and thresholds.
    const label = h("span", { class: line.percent.tone === "dim" ? "sess-ctx" : `sess-ctx ${line.percent.tone}` }, line.percent.text);
    label.prepend(contextRing(line.percent.ring));
    el.append(h(
      "div",
      { class: "usage-line" },
      label,
      h("span", { class: "usage-window", text: line.name }),
      h("span", { class: "usage-reset", text: line.resets }),
    ));
  }
  if (b.note) el.append(h("div", { class: "usage-note", text: b.note }));
  return el;
}

export function buildHome(): ViewHost {
  const body = h("div", { class: "usage" });
  const el = h("div", { class: "view" }, h("div", { class: "card" }, body));
  let renderedKey = "";
  let timer: number | null = null;

  function render() {
    clear(body);
    for (const b of usageBlocks(Sessions.snapshot?.usage, Date.now())) body.append(blockEl(b));
  }

  return {
    el,
    sync() {
      const key = JSON.stringify(Sessions.snapshot?.usage ?? null);
      if (key === renderedKey) return;
      renderedKey = key;
      render();
    },
    setActive(on: boolean) {
      if (on && timer == null) {
        // Opening the home page reads Claude's cache again (usage-home spec §2.1).
        void Bridge.usageRefresh();
        render();
        timer = window.setInterval(render, REFRESH_MS);
      } else if (!on && timer != null) {
        window.clearInterval(timer);
        timer = null;
      }
    },
  };
}
```

`views/views.ts`, after `import { renderIntegrationCard, type IntegrationCardHooks } from "./integrations";`:

```ts
import { buildHome } from "./usage";
```

Replace

```ts
    { class: "tab", title: "Overview", onclick: () => go(State.tasks.length === 0 ? "empty" : "overview") },
```

with

```ts
    { class: "tab", title: "Overview", onclick: () => go("overview") },
```

Replace

```ts
function buildOverview(actions: ViewActions): ViewHost {
```

with

```ts
/**
 * The integrations home inherited from Coucou (integration card and pills). Not registered since
 * the home page shows the plan limits (usage-home spec §1.1, §4); kept unchanged with
 * renderIntegrationCard and the pills.
 */
export function buildOverview(actions: ViewActions): ViewHost {
```

In `buildViews`, replace

```ts
  map.set("overview", buildOverview(actions));
```

with

```ts
  map.set("overview", buildHome());
```

`core/state.ts`, replace

```ts
  /** Agent sessions first (spec §10), then the integrations overview. */
  defaultView(): IslandViewName {
    if (Sessions.hasRows) return "sessions";
    return this.tasks.length === 0 ? "empty" : "overview";
  }
```

with

```ts
  /** Agent sessions first (spec §10), then the home page with the plan limits (usage-home spec §4). */
  defaultView(): IslandViewName {
    return Sessions.hasRows ? "sessions" : "overview";
  }
```

- [ ] **Step 5: CSS (new selectors only)**

Append at the end of `style.css`:

```css

/* ── Home: plan limits (usage-home spec §4) ─────────────────────────────── */
/* Mochi keeps its place on the left (VIEW_LAYOUTS.overview); the percent label is the session
   row's .sess-ctx with its ring, unchanged. */

.usage {
  position: relative;
  z-index: 1;
  height: 100%;
  display: flex;
  gap: 24px;
  padding: 14px 18px 12px 116px;
}

.usage-block {
  flex: 1 1 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.usage-name {
  font: 600 12px var(--font);
  color: var(--ink);
}

.usage-line {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 18px;
  font: 400 11px var(--font);
  color: var(--dim-3);
}

.usage-line .sess-ctx {
  flex: 0 0 44px;
}

.usage-window {
  flex: 0 0 auto;
  color: var(--dim-2);
  font-variant-numeric: tabular-nums;
}

.usage-reset {
  margin-left: auto;
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}

.usage-empty {
  font: 400 12px var(--font);
  color: var(--dim-2);
}

.usage-note {
  font: 400 10.5px var(--font);
  color: var(--dim-3);
}
```

- [ ] **Step 6: Verify**

Run: `cd /c/Users/HP/source/repos/mayak/windows && unix2dos src/views/usage-format.ts src/views/usage.ts && npx tsc --noEmit && npx vite build 2>&1 | tail -3` then `node --experimental-strip-types C:/tmp/mayak-spike/usage-format-check.mjs` then `file src/core/bridge.ts src/core/state.ts src/views/sessions.ts src/views/views.ts src/views/usage-format.ts src/views/usage.ts src/style.css`
Expected: `unix2dos` reports two converted files; `tsc` prints nothing and exits 0; `vite build` ends with `✓ built in …`; the check prints `usage-format: all checks passed`; all seven files `with CRLF line terminators`. `cargo test --workspace` is unaffected (nothing Rust changed).

- [ ] **Step 7: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src/core/bridge.ts windows/src/core/state.ts windows/src/views/sessions.ts windows/src/views/views.ts windows/src/views/usage-format.ts windows/src/views/usage.ts windows/src/style.css && git commit -m "feat(island): show the Claude and Codex plan limits on the home page"
```

---

### Task 4: Acceptance with the user, CPU, spec alignment, publishing

**Files:**
- Modify: `docs/superpowers/specs/2026-10-06-mayak-usage-home-design.md` (append)
- Create: `C:\tmp\mayak-spike\usage.mjs` (outside the repository)
- Modify: `.git/mayak-pr-notes.md`, create `.git/mayak-pr10-body.md` (local, untracked)

**Interfaces:** consumes the branch after Tasks 1-3 (`SessionsSnapshot.usage`, `usage_refresh`, `.usage*`, `buildHome`); produces the acceptance record.

The controller runs Steps 3-5 **with the user** (real Claude and Codex sessions). The dev Mayak under `tauri dev` is already running the working tree and rebuilt after each task. Do not stop it and do not start a second one.

- [ ] **Step 1: Write the decisions into the spec**

Append at the end of the spec (Russian, LF line endings):

```markdown

## 6. Уточнения при реализации

- **Корзины лимитов Codex.** В одном rollout Codex пишет несколько `rate_limits` с разным `limit_id`: `codex` (лимиты аккаунта), отдельные корзины моделей (например, `codex_bengalfox`, «GPT-5.3-Codex-Spark») и пустую `premium`. Берётся только `limit_id: "codex"` или запись без `limit_id` (старые версии Codex); остальные значение не меняют. Из `rate_limits` читаются только `limit_id`, `primary` и `secondary`.
- **Окно Codex** берётся, если `used_percent` — число, а `window_minutes` — целое > 0; `resets_at` — целое число секунд, иначе окно без времени сброса. Запись без годных окон значение не меняет.
- **Сессия, которой уже нет.** Лимиты относятся к аккаунту: пакет rollout сессии, которую модель уже удалила, тоже обновляет значение. «Немые» rollout (без хуков) лимитов не дают: новых чтений нет.
- **Свежесть Codex** (`seenAtMs`) обновляется с каждой записью с лимитами, даже если проценты те же; это меняет `rev`.
- **Claude.** Нужны объекты `cachedUsageUtilization` и `utilization` и целое `fetchedAtMs`; иначе `claude: null` (неверный тип этих объектов тоже). Окна без числового `utilization` пропускаются; если не осталось ни одного, `null`.
- **Недописанный файл.** Если `.claude.json` не разбирается как JSON (Claude Code пишет его в этот момент) или не читается, прежнее значение сохраняется; в лог попадает только вид ошибки (`Eof`, `Syntax` или вид ошибки ввода-вывода). Отсутствие файла даёт `null`.
- **Где читается.** На потоке claude-io: при его запуске, на `Retry` (его шлёт `Visible(true)`) и по команде `usage_refresh`, которую домашняя страница вызывает, когда становится видна. В модель значение уходит только при изменении.
- **Дата сброса** — RFC 3339: `YYYY-MM-DDTHH:MM:SS`, необязательная дробная часть до 9 цифр (округляется вниз до миллисекунд), затем `Z` или `±HH:MM`.
- **Проценты** показываются округлёнными вниз и ограниченными 0–100; цвет — по этому числу.
- **Названия окон Codex:** меньше 60 минут — `Nm`; деление округляется вниз.
- **Домашняя страница всегда `overview`:** вкладка «дом» и вид по умолчанию без сессий открывают её и без включённых интеграций; вид `empty` остаётся в коде, но больше не открывается. Код интеграций (`buildOverview`, `renderIntegrationCard`, пилюли) не удалён, их опрос не менялся.
- **Метка процента** — тот же элемент, что метка контекста (`.sess-ctx` и кольцо `contextRing`); пороги в общей функции `percentTone`.
```

Commit:

```bash
cd /c/Users/HP/source/repos/mayak && git add docs/superpowers/specs/2026-10-06-mayak-usage-home-design.md && git commit -m "docs: align the usage home spec with the implementation"
```

- [ ] **Step 2: The acceptance helper and a check of the running dev Mayak**

Create `C:\tmp\mayak-spike\usage.mjs`:

```js
// Mayak home-page limits acceptance over the WebView2 DevTools protocol (dev app on :9222).
// Prints numbers, window names, classes, colours and Mayak's own texts only: never anything else of
// .claude.json or of the rollouts. Usage: node usage.mjs show | fake | restore
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
const DOM = `({
  homeOn: document.querySelector(".view.on .usage") !== null,
  integrationsOnScreen: document.querySelectorAll(".view.on .pills, .view.on .left-body").length,
  blocks: [...document.querySelectorAll(".usage-block")].map((b) => ({
    name: b.querySelector(".usage-name")?.textContent ?? "",
    empty: b.querySelector(".usage-empty")?.textContent ?? "",
    note: b.querySelector(".usage-note")?.textContent ?? "",
    lines: [...b.querySelectorAll(".usage-line")].map((l) => ({
      text: [...l.children].map((c) => c.textContent).join(" "),
      cls: l.querySelector(".sess-ctx")?.className ?? "",
      color: getComputedStyle(l.querySelector(".sess-ctx")).color,
      ring: l.querySelector(".sess-ctx-ring .fill")?.getAttribute("stroke-dasharray") ?? "none",
    })),
  })),
})`;
const exprs = {
  show: `(async () => {
    const s = await window.__TAURI_INTERNALS__.invoke("sessions_snapshot");
    const now = Date.now();
    const u = s.usage ?? { claude: null, codex: null };
    const mins = (ms) => (ms === undefined ? "" : Math.round(ms / 60000));
    const rows = [
      ...(u.claude?.windows ?? []).map((w) => ({ agent: "claude", window: w.id, percent: w.percent,
        resetsInMin: mins(w.resetsAtMs === undefined ? undefined : w.resetsAtMs - now), ageMin: mins(now - u.claude.fetchedAtMs) })),
      ...(u.codex?.windows ?? []).map((w) => ({ agent: "codex", window: w.minutes + "min", percent: w.percent,
        resetsInMin: mins(w.resetsAtMs === undefined ? undefined : w.resetsAtMs - now), ageMin: mins(now - u.codex.seenAtMs) })),
    ];
    return JSON.stringify({ rev: s.rev, claude: u.claude ? "set" : null, codex: u.codex ? "set" : null, rows, dom: ${DOM} });
  })()`,
  fake: `(async () => {
    const { Sessions } = await import("/src/core/sessions.ts");
    const { State } = await import("/src/core/state.ts");
    const real = Sessions.snapshot;
    if (!real) return JSON.stringify({ error: "no snapshot" });
    const fake = structuredClone(real);
    fake.rev = real.rev + 0.5; // the next real snapshot (an integer rev) still replaces it
    const now = Date.now(), MIN = 60000, HOUR = 60 * MIN, DAY = 24 * HOUR;
    fake.usage = {
      claude: { fetchedAtMs: now - 7 * HOUR, windows: [
        { id: "5h", percent: 79, resetsAtMs: now + 2 * HOUR + 14 * MIN + 30000 },
        { id: "7d", percent: 80, resetsAtMs: now + 3 * DAY + 4 * HOUR + 30000 } ] },
      codex: { seenAtMs: now - MIN, windows: [
        { minutes: 300, percent: 95, resetsAtMs: now - 1000 }, { minutes: 10080, percent: 10 } ] },
    };
    const accepted = Sessions.accept(fake);
    State.notify();
    await new Promise((r) => setTimeout(r, 300));
    return JSON.stringify({ accepted, dom: ${DOM} });
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
if (Array.isArray(out.rows)) console.table(out.rows);
console.log(JSON.stringify({ ...out, rows: Array.isArray(out.rows) ? out.rows.length : undefined }, null, 1));
ws.close();
```

Run: `curl -s --max-time 2 http://127.0.0.1:9222/json/list | grep -c "localhost:1420"` then `cd /c/tmp/mayak-spike && node usage.mjs show`
Expected: the count is at least 1 (the dev island page is up); `show` prints a `rev` and `claude`/`codex` as `"set"` or `null`. If the page is missing, the dev Mayak is not running: ask the user to start it; do not start it yourself.

- [ ] **Step 3: Spec §5 live items 1-4 with the user**

Record each as PASS/FAIL with the evidence (the `usage.mjs` output, what the user saw) in `.git/mayak-pr-notes.md` under `## Usage home acceptance (2026-10-06)`:
1. Claude: the user runs `/usage` in Claude Code, then opens the island's home tab. `node usage.mjs show`: `claude: "set"`, a `5h` and a `7d` row whose `percent` equals the `/usage` numbers and whose `resetsInMin` matches its reset times; `dom.homeOn: true` and the Claude block's lines read like `37% 5h resets in 2h 14m`. The user confirms the numbers against `/usage`.
2. Codex: note `ageMin` of the codex rows (or `codex: null`); the user runs one turn in a Codex session with hooks; `show` again with the home tab open: `codex: "set"`, `ageMin` 0 or 1, windows named from `window_minutes`; the user confirms the Codex block.
3. Freshness and colours: with the home tab open, `node usage.mjs fake`: `accepted: true`; Claude block lines `79% 5h resets in 2h 14m` (`sess-ctx`, `rgb(107, 112, 121)`) and `80% 7d resets in 3d 4h` (`sess-ctx warn`, `rgb(245, 165, 36)`), note `as of 7h ago`; Codex lines `95% 5h reset` (`sess-ctx full`, `rgb(244, 80, 94)`) and `10% 7d ` (no reset text), note empty. The user confirms them in the island; then `node usage.mjs restore` (`accepted: true`) and `show` gives the real values again. If the real Claude cache is itself older than 10 minutes, `show` already has the Claude note `as of …`: record it too.
4. No integrations: `show` gives `integrationsOnScreen: 0` with the home tab open, and the home tab opens the limits also with every integration switched off in Settings (the user toggles them off, checks, and restores them).
Also check: `grep -n "claude usage" "$LOCALAPPDATA/Mayak/mayak.log" | tail -5` prints nothing or only lines ending `claude usage unreadable: <Kind>` (no file text).

- [ ] **Step 4: CPU (spec §5 item 5)**

With the island hidden and no agent turn running: `powershell -File C:\tmp\mayak-accept\sample-cpu.ps1 -Seconds 60 -OutPath C:\tmp\mayak-accept\cpu-usage-idle.log`. Expected: `tree_avg_percent_of_machine` < 0.100, as before this branch (`.git/mayak-pr-notes.md` 5a: 0.010). Record the RESULT line in `.git/mayak-pr-notes.md`.

- [ ] **Step 5: Full verification**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|^warning"` and `npx tsc --noEmit && npx vite build 2>&1 | tail -3` and `node --experimental-strip-types C:/tmp/mayak-spike/usage-format-check.mjs` and `cd /c/Users/HP/source/repos/mayak && git status --short`
Expected: lib 466 passed (1 ignored), hook 32 passed, no `warning` line, build OK, `usage-format: all checks passed`, `git status --short` empty (the `.git/` notes are not tracked; `dist/` is ignored).

- [ ] **Step 6: Ask before publishing**

Write `.git/mayak-pr10-body.md` from the results of Steps 3-5: What (home page = Claude and Codex plan limits, the Codex bucket rule, the Claude read on claude-io, integrations no longer on home), Checks (local) with the test counts and every acceptance line, and no trailer. Ask the user, verbatim: «Запушить ветку `feature/usage-home` в `origin` и открыть PR в `main`? Проверки на GitHub отключены, проверка локальная.» Only on an explicit yes:

```bash
cd /c/Users/HP/source/repos/mayak && git push -u origin feature/usage-home && gh pr create --repo mahach666/mayak --base main --head feature/usage-home --title "Mayak: plan limits on the home page" --body-file "$(git rev-parse --git-dir)/mayak-pr10-body.md"
```

The merge stays with the user: do not merge unless the user asks for it in a separate, explicit message.

---

## Self-Review

- **Spec coverage:**
  - §1.1 home given to usage, integrations hidden but kept → Task 3 Step 4 (`buildHome` registered, `buildOverview` exported and unregistered, home tab and `defaultView`); Task 4 Step 3 item 4.
  - §1.2 colours, home only, no sound or wake → `percentTone` (Task 3), `Applied::default()` / no alerts in `codex_usage_changes_nothing_but_the_usage` and `claude_usage_changes_nothing_but_the_usage`; Task 4 Step 3 item 3.
  - §1 out of scope (no network, integrations unchanged) and rules (named fields only, no hidden polling, other views unchanged, app style) → Global Constraints; `claude_usage.rs` typed deserializer; only `.usage*` CSS added; the timer lives in `setActive`; Task 4 Step 4.
  - §2.1 file location incl. `CLAUDE_CONFIG_DIR`, fields, skipped windows, unparsable date, triggers, off the main thread → Task 2 (`AppPaths.claude_json` + its two tests, `parse_claude_usage` tests, `read_usage` on start / Retry / ReadUsage, `usage_refresh`).
  - §2.2 `rate_limits` primary/secondary/null, last value across sessions, read time as freshness, no new reads → Task 1 parser tests and `codex_rollout` (limits taken from the records already parsed).
  - §3 `usage` shape, `rev`, no effect on counts/alerts/hold, `null` when absent or empty → Task 1 / Task 2 model and wire tests.
  - §4 ring and percent, `5h`/`7d`/`Nh`/`Nd`, reset texts, freshness > 10 min, «No data yet» and both hints, refresh on render and once a minute while on screen, integrations gone → Task 3 (`usage-format.ts` + check script, `usage.ts`, CSS); Task 4 Step 3.
  - §5 Rust tests (Claude: full, no key, non-number, bad date, extra keys, no file; Codex: primary+secondary, one null, no `rate_limits`; model: last wins, `rev`, counters/alerts unchanged) → Tasks 1-2; TS pure functions + `tsc` + build → Task 3; live items 1-5 → Task 4 Steps 3-4.
- **Placeholder scan:** none. Every code step carries its code and every command its expected output; the conditional instructions name their action (no dev page: ask the user; a stale real cache: record its note as well).
- **Type consistency:** `RolloutRecord::Tokens { context: Option<ContextFill>, limits: Option<Vec<CodexWindow>> }` (Task 1 Step 4) is matched in `codex_rollout` (Task 1 Step 5); `CodexWindow { minutes, percent, resets_at_ms }`, `CodexUsage { seen_at_ms, windows }`, `ClaudeWindow { id, percent, resets_at_ms }`, `ClaudeUsage { fetched_at_ms, windows }`, `ClaudeWindowId::{FiveHour, SevenDay}` serialize to the camelCase keys the TS `Usage` (Task 3 Step 1) reads (`seenAtMs`, `fetchedAtMs`, `resetsAtMs`, `minutes`, `percent`, `id` "5h"/"7d"); `Signal::ClaudeUsage(Option<ClaudeUsage>)` is sent by `read_usage` and applied in `apply`; `ClaudeInput::ReadUsage` ← `Handle::claude_usage` ← `usage_refresh` ← `Bridge.usageRefresh` ← `buildHome().setActive(true)`; `contextRing`, `percentTone`, `usageBlocks` are exported where `usage.ts` imports them; the model test helpers (`limits_event`, `limit_window`, `codex_usage`, `claude_usage`) do not collide with existing names (`token_count`, `with_usage`, `context_of`).
- **Review Focus:** 1 → Task 1 `only_the_codex_limit_bucket_counts` and `a_token_count_sets_the_codex_usage_and_the_last_value_wins`; 2 → Task 2 `a_torn_file_is_an_error_that_names_no_text_of_it` and `the_usage_is_read_at_start_on_retry_and_on_request_and_sent_on_change_only`; 3 → Task 2 `the_full_object_gives_both_windows_and_nothing_else` and the torn-file test; 4 → Task 1 `codex_usage_changes_nothing_but_the_usage`, Task 2 `claude_usage_changes_nothing_but_the_usage`; 5 → Task 2 `rfc3339_dates_become_epoch_milliseconds`, `anything_else_is_no_date`, `an_unparsable_reset_date_gives_a_window_without_it`.
- **Test counts:** Task 1 adds 3 parser + 4 model tests (446 → 453); Task 2 adds 9 `claude_usage` + 1 claude-io + 3 model tests (453 → 466); existing tests edited, not added: the four context-fill parser tests, `snapshot_serializes_to_the_spec_wire_shape`, `handle_sends_never_panic_without_receivers`, `start_reports_a_missing_claude_dir_and_serves_the_snapshot`, the two `app_paths_*` tests.
