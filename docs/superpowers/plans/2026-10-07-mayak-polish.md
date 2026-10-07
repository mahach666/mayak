# Mayak Polish (Rust texts localized, Settings title, reload, review minors) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish localization, so that texts built in Rust are translated too. Localize the Settings window title. Make a language reload seamless: no greeting, and a pending phase card survives. Fix the minors left over from the batch reviews.

**Architecture:**
- Rust returns `{ code, params, fallback }` for user-visible messages instead of English sentences. The TS side translates each code through `t()` and falls back to `fallback` for an unknown code.
- The reload marker and the phase card ride in the same `sessionStorage` entry that already carries the pomodoro.

**Tech Stack:** Tauri 2 (Rust `windows/src-tauri`), TypeScript (`windows/src`).

**Spec:** `docs/superpowers/specs/2026-10-07-mayak-batch-design.md` §F (and §C for i18n).

## Global Constraints

- No new dependencies.
- Never log texts, titles or paths.
- Keep existing views as they are.
- All new keys go into all seven dictionaries (`en ru de fr es zh ja`) with identical `{var}` sets. `tsc` enforces completeness, and `C:/tmp/mayak-spike/i18n-keys-check.mjs` enforces placeholder parity.
- Commit messages: one line, no trailers, no `Co-Authored-By`, never mention Claude or AI. Stage by explicit path.
- Rust: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target`.
- Checks:
  - `cd windows && cargo test --workspace` (0 warnings);
  - `npx tsc --noEmit`;
  - `npx vite build`;
  - the Node checks in `C:/tmp/mayak-spike/` (pomodoro, activity, i18n, i18n-keys).
- `windows/target/release/mayak-hook.exe` must exist for Rust builds (the tauri resource). If it is missing, build it with `cargo build -p mayak-hook --release` (same `CARGO_TARGET_DIR`) and copy it there.

## Review Focus

1. **An unknown code must still read sensibly.** A Rust message whose code the front end does not know shows `fallback` (English), never a raw key (Task 1: a TS helper with a Node check).
2. **The OS error tail stays verbatim.** Placeholders hold paths and OS errors; those values are never translated or dropped (Task 1 test: Rust carries params unchanged).
3. **The window title.** A failed `setTitle` (permission, older build) must not break the Settings window (Task 2: try/catch with a fixed log string).
4. **The reload marker is one-shot.** It is consumed at boot, so a later real launch still greets (Task 2).
5. **Cover content types.** `image/JPEG` is accepted; `image/`, `image/png,<x>` and `Image/` with a bad subtype are rejected (Task 3 Rust tests).

---

### Task 1: Rust messages as codes, translated in TS

**Files:**
- Modify: `windows/src-tauri/src/hookfile.rs`, `windows/src-tauri/src/hooks.rs`, `windows/src-tauri/src/codex_hooks.rs` (user-visible `Err(String)` and notes)
- Modify: `windows/src-tauri/src/hotkey.rs` (`HotkeyStatus.error`)
- Modify: `windows/src-tauri/src/apps.rs` (`open` errors), and the Health error in `windows/src-tauri/src/observe/` (find where `Health.error` is set)
- Create: `windows/src-tauri/src/message.rs`
- Create: `windows/src/core/message.ts` (pure, Node-importable)
- Modify: the TS consumers: `windows/src/settings/main.ts` (hook previews, apply results, notes, hotkey status, health) and `windows/src/core/bridge.ts` (types)
- Modify: the dictionaries `windows/src/core/i18n/*.ts` (keys `msg.*`)
- Test: Rust unit tests; `C:/tmp/mayak-spike/message-check.mjs`

**Interfaces:**
- Rust `message.rs`:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
  pub struct Message { pub code: &'static str, pub params: BTreeMap<&'static str, String>, pub fallback: String }
  impl Message { pub fn new(code: &'static str, fallback: impl Into<String>) -> Self; pub fn with(mut self, key: &'static str, value: impl Into<String>) -> Self; }
  impl std::fmt::Display for Message { /* writes fallback */ }
  ```
  Serialized as `{ "code": "...", "params": { ... }, "fallback": "..." }`.
- Every user-visible error or note in the listed Rust files becomes a `Message`. The fallback is exactly today's English sentence, so English output is unchanged. Codes are `msg.<area>.<name>`, for example `msg.hook.notObject`, `msg.hook.readFailed`, `msg.codex.hooksNotObject`, `msg.hotkey.invalid`, `msg.apps.notInstalled`, `msg.health.unreadable`.
- Where a command returns `Result<_, String>` to TS, return `Result<_, Message>` instead (serde serializes the Err). Where a struct carries `error: Option<String>` / `notes: Vec<String>` to TS, change it to `Option<Message>` / `Vec<Message>`. Update the TS types in `core/bridge.ts` accordingly: `interface RustMessage { code: string; params: Record<string, string>; fallback: string }`.
- Errors that only go to the log, and never to the UI, stay `String`. Never log a `Message`'s params.
- TS `core/message.ts`:
  ```ts
  export interface RustMessage { code: string; params: Record<string, string>; fallback: string }
  /** The text of a Rust message: the translation when the dictionary knows the code, else its English fallback. */
  export function messageText(m: RustMessage, tr: (key: string, vars: Record<string, string>) => string | null): string
  ```
  `tr` returns null for an unknown key. In the app, pass a wrapper around `t` that checks the key exists in `en`.

- [ ] **Step 1: Inventory.** List every user-visible `Err(...)`/note string in the listed files, and decide for each whether it reaches the UI. Trace the commands and structs to `settings/main.ts`. Put the inventory table in the report.
- [ ] **Step 2: Failing tests.**
  - Rust: a `Message` serializes to the agreed JSON shape, and `Display` is the fallback.
  - Rust: for two representative call sites (a hookfile "not a JSON object" and a hotkey "invalid"), the returned `Message` has the expected code and params, and the fallback equals the old sentence.
  - Update the existing tests that asserted English strings, so they assert `code` plus the fallback text.
  - Node `message-check.mjs`: a known code is translated with params; an unknown code falls back to `fallback`; params are substituted.
- [ ] **Step 3: Implement** Rust and TS, and add every `msg.*` key to all seven dictionaries with natural translations. A placeholder like `{path}` or `{err}` stays verbatim.
- [ ] **Step 4: Checks** (all of them, as in Global Constraints) plus a look at Settings → Agent sessions and Codex in `de` in the browser (`http://localhost:1420/settings.html`; a dev server may be running; do not stop it).
- [ ] **Step 5: Commit:** `feat(i18n): texts built in Rust are translated too`.

---

### Task 2: Settings window title, no greeting after a reload, the phase card kept

**Files:**
- Modify: `windows/src-tauri/capabilities/*.json`: grant `core:window:allow-set-title` to the Settings window only. Use a separate capability file scoped to the `settings` window label if the current one is shared. Find the window label in `lib.rs` `show_settings_window`.
- Modify: `windows/settings.html` and `windows/src/settings/main.ts`: the title through `t()` after `setLanguage`, both `document.title` and `getCurrentWindow().setTitle(...)` from `@tauri-apps/api/window`, inside try/catch with a fixed log string.
- Modify: `windows/src/main.ts`, `windows/src/island/island.ts` (`fsm.launch` call site), `windows/src/island/pomodoro.ts` (saved-run (de)serialization: add the phase card)
- Modify: dictionaries (`set.windowTitle`)
- Test: extend `C:/tmp/mayak-spike/pomodoro-check.mjs` (the saved run with and without a phase card)

- [ ] **Step 1:** Add `set.windowTitle` ("Settings — Mayak", translated, with Mayak untranslated) to the seven dictionaries. Set the title as above. Add the capability.
- [ ] **Step 2: Reload marker.** The saved entry written before a language reload gains `{ reloaded: true, phaseCard?: { title, records }, phaseCardPending?: boolean }` next to the run. Even with no pomodoro running, a language reload writes `{ reloaded: true }`. At boot, the entry is read and removed once (one-shot). When `reloaded` is set, the island skips the launch greeting: find the launch call (`fsm.launch()` or the greeting trigger) and add an option to start without the greeting, going straight to the default state. A pending phase card is restored (`State.phaseCard`, `State.phaseCardPending`) and released through the existing `releaseHeldCard()` once the island shows.
- [ ] **Step 3:** Extend the pure (de)serialization in `pomodoro.ts` and its Node check: round trip with a card; old entries without the new fields still decode; a malformed card is dropped, the run kept.
- [ ] **Step 4: Checks**, plus live in the running dev app over CDP (`C:/tmp/mayak-spike/cdp.mjs`, `shot.mjs`): switch the language (Settings select `de`, then back to `system`). No greeting appears. A pomodoro running with a held card shows the card after the reload. Restore the user's language to `system` at the end.
- [ ] **Step 5: Commit:** `feat(i18n): localized Settings title; a language reload keeps the island quiet and the phase card`.

---

### Task 3: Review minors

**Files:** `windows/src/views/app-icons.ts`, `windows/src/views/usage.ts`, `windows/src-tauri/src/media.rs`, `windows/src-tauri/src/apps.rs` (tests), `windows/src/core/i18n/fr.ts`, `windows/src/core/i18n/zh.ts`, `windows/src/settings/main.ts`

- [ ] **Icons:** look up `SVG_ICONS` and `MONOGRAMS` with `Object.hasOwn(...)`, or turn them into `Map`s. The monogram span gets `aria-hidden="true"` (the SVG one too).
- [ ] **Cover:** `img.onerror` → hide the img and show the ♪ fallback.
- [ ] **Content type:** in `cover_data_url`, lowercase the type before checking; require `image/` plus a non-empty subtype of allowed chars. Tests: `image/JPEG` → Some (with the data URL using the lowercased type); `image/` → None; `image/png,<x>` → None.
- [ ] **apps tests:** build relative paths with `Path::join` per component (store the paths as slices of components, for example `&["Programs", "cursor", "Cursor.exe"]`) so the tests pass on any OS. Behaviour on Windows is unchanged.
- [ ] **fr and zh count lines:** make `focus.countFinished`, `focus.countError` and `focus.countWaited` follow one pattern and one tense per language, matching how es was fixed ("Label: {n}"). Keep placeholders.
- [ ] **"Error:" prefix:** in `settings/main.ts`, where a caught error is shown inside a translated sentence, use `err instanceof Error ? err.message : String(err)` (lines near the model-list errors).
- [ ] **Checks:** all of them. **Commit:** `fix: review minors (icons, cover fallback, content type, portable tests, wording)`.

---

### Task 4: Bugs shared with upstream (from docs/superpowers/ideas/2026-10-07-upstream-review.md, top 1-3)

**Files:** `windows/src-tauri/src/settings.rs`, `windows/src-tauri/src/lib.rs` (`save_settings` error path), `windows/src-tauri/src/hooks.rs` (`merged` / plan), `windows/src/island/island.ts` (the upload-error reset timer, around the `setTimeout(() => this.setView(State.defaultView()), 2400)`)

- [ ] **Settings load must never lose everything to one bad field** (upstream d8d2939, adapted):
  - strip a UTF-8 BOM (reuse `hookfile::strip_bom` if it fits);
  - if the full parse fails, start from `Settings::default()` and keep every field of the file whose value deserializes on its own; rescue field by field through `serde_json::Value`;
  - if the file is not JSON at all, copy it once next to itself as `settings.json.broken-<unix secs>` before any save, and log a fixed line (never the content);
  - keep the existing migration from the Coucou folder and `migrated()` behaviour.
  - Tests: BOM file loads; one wrong-typed field keeps the others (language, hotkey, pomodoro values); a non-JSON file → defaults plus the broken copy exists; a good file → unchanged.
- [ ] **Atomic save:** write to `settings.json.tmp` then rename over the target (reuse the `write_synced` + rename pattern from `hookfile.rs` if present); `save_settings` logs failures with `log::line` (fixed text plus the error kind), not `eprintln!`. Test: the save leaves no `.tmp` behind and the file round-trips.
- [ ] **Claude hooks shape check:** before planning a merge into `~/.claude/settings.json`, refuse when `"hooks"` exists and is not an object, or when an event value Mayak would touch is not an array. Return the same kind of note/error the Codex writer gives (`codex_hooks::check_shape`); since Task 1, that means a `Message` with code `msg.hook.badShape` and keys in all seven dictionaries. Tests with temp files only: never touch the real `~/.claude/settings.json`.
- [ ] **Stale upload-error timer:** the 2.4 s reset after a failed upload only runs if `State.view === "note"` and the island is still expanded; otherwise it does nothing.
- [ ] **Checks:** all of them. **Commit:** `fix: settings survive a bad field, atomic save, refuse odd Claude hooks shapes, no stale note reset`.

---

### Task 5: Live check and docs (controller)

- [ ] Live, in the running dev app:
  - Settings in `de`: the hook section texts and the window title;
  - a language switch: no greeting, and the card is kept;
  - the Apps tab;
  - a cover, if something plays.
- [ ] Update the ideas "Итог §21–§25" list (what is left), and report.
