# Mayak Batch (Mochi activity, player cover, apps, icons, localization) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement ideas §21–§25: Mochi's activity animations, a bigger player with album art, an Apps tab that launches installed IDEs and agent apps with Catppuccin-style icons, and a 7-language UI.

**Architecture:** Each part is a vertical slice.
- Rust gains a cover reader in the media thread and a new `apps.rs` (detection and launch, Windows only).
- TypeScript gains `core/i18n.ts` with one dictionary per language, `views/apps.ts`, `views/app-icons.ts`, and an `activity` field on Mochi's engine.
- Localization comes last, so its tasks also cover the new views.

**Tech Stack:** Tauri 2 (Rust `windows/src-tauri`, crate `windows` 0.61), TypeScript with no framework (`windows/src`), Canvas for Mochi.

**Spec:** `docs/superpowers/specs/2026-10-07-mayak-batch-design.md`

## Global Constraints

- 0 % CPU while the island is hidden: animations and ticks run only while it is visible.
- No new npm packages or crates. New features of the `windows` crate are allowed (`Storage_Streams`).
- Existing views stay as they are, except those the spec names: the home player row, the header tabs and Settings → General.
- Never log track titles, artists, UI texts or paths.
- Mochi is drawn in code. Icons are inline SVG strings.
- UI text in English in code. Translations come only through `t()` once Task 6 lands.
- Commit messages: one line, no trailers, no `Co-Authored-By`, never mention Claude or AI. Stage by explicit path.
- Rust: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target` (required; without it the build script fails on an old path).
- Checks:
  - `cd windows && cargo test --workspace` (0 warnings);
  - `npx tsc --noEmit`;
  - `npx vite build`;
  - Node checks: `node --experimental-strip-types C:/tmp/mayak-spike/<name>.mjs`. A module a Node check imports may only use `import type` for local imports.
- Exact values:
  - `PLAYER_ROW_H` = 52;
  - cover 40×40, radius 6;
  - `MAX_COVER_BYTES` = 262144;
  - Catppuccin Mocha colours: surface2 `#585b70`, mauve `#cba6f7`, pink `#f5c2e7`, peach `#fab387`, green `#a6e3a1`, teal `#94e2d5`, blue `#89b4fa`, crust `#11111b`;
  - languages `en ru de fr es zh ja`, setting `language` default `"system"`.

## Review Focus

1. A media session whose thumbnail stream is huge, fails or never completes must never stall the media thread or the row. Bounded wait; the cover is `None` past 256 KiB or on error (Task 1 tests on the pure part).
2. Play/pause flips must not re-read the cover. The cover is cached by (title, artist, app id) (Task 1 test on the cache-key function).
3. Music playing while the island is hidden must not keep the frame loop running. Notes are emitted only from `update()`, which runs only in frames; check that `busy` does not keep a hidden island awake (Task 3: a hidden island's loop uses only geometry settling).
4. A German or French string that is longer than the English one must not break the layout. Ellipsis where needed; the controller checks the de locale live (Task 6/7).
5. An app found by an exe path that was deleted between listing and click must fail quietly: `app_open` returns false and the island stays up (Task 4 test: open of a missing exe returns an error, not a panic).

---

### Task 1: Album art in the media state (Rust)

**Files:**
- Modify: `windows/src-tauri/src/media.rs` (`MediaState`, `Reading`, `into_state`, new pure helpers and tests)
- Modify: `windows/src-tauri/src/media/win.rs` (`reading`, `Subscriptions` cover cache)
- Modify: `windows/src-tauri/Cargo.toml` (feature `Storage_Streams`)
- Modify: `windows/src/core/bridge.ts` (`MediaState.cover: string | null`)

**Interfaces:**
- Produces (Rust, `media.rs`):
  - `pub const MAX_COVER_BYTES: usize = 262_144;`
  - `pub fn base64(bytes: &[u8]) -> String` (standard alphabet, `=` padding);
  - `pub fn cover_data_url(content_type: &str, bytes: &[u8]) -> Option<String>`: `None` when empty, over `MAX_COVER_BYTES`, or when the content type does not start with `image/`; else `format!("data:{content_type};base64,{}", base64(bytes))`;
  - `pub fn cover_key(title: &str, artist: &str, app_id: &str) -> String`;
  - `MediaState { …, cover: Option<String> }` (serialized `cover`, null when none);
  - `Reading { …, cover: Option<String> }`; `into_state` carries it over.
- Produces (TS): `MediaState.cover: string | null`.

- [ ] **Step 1: Failing tests** in the `tests` module of `media.rs`:

```rust
    #[test]
    fn base64_matches_rfc_4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    #[test]
    fn a_cover_is_a_data_url_only_for_a_small_image() {
        assert_eq!(cover_data_url("image/png", b"abc").as_deref(), Some("data:image/png;base64,YWJj"));
        assert_eq!(cover_data_url("image/png", b""), None);
        assert_eq!(cover_data_url("text/plain", b"abc"), None);
        assert_eq!(cover_data_url("image/jpeg", &vec![0u8; MAX_COVER_BYTES + 1]), None);
        assert!(cover_data_url("image/jpeg", &vec![0u8; MAX_COVER_BYTES]).is_some());
    }

    #[test]
    fn the_cover_key_changes_with_the_track_not_with_play_state() {
        assert_eq!(cover_key("Song", "Band", "Spotify"), cover_key("Song", "Band", "Spotify"));
        assert_ne!(cover_key("Song", "Band", "Spotify"), cover_key("Song 2", "Band", "Spotify"));
        assert_ne!(cover_key("a\u{1f}b", "", "x"), cover_key("a", "b", "x"));
    }

    #[test]
    fn the_cover_goes_on_the_wire_as_cover() {
        let r = Reading { title: "T".into(), cover: Some("data:image/png;base64,AA==".into()), ..Default::default() };
        let v = serde_json::to_value(r.into_state().unwrap()).unwrap();
        assert_eq!(v["cover"].as_str(), Some("data:image/png;base64,AA=="));
        let none = Reading { title: "T".into(), ..Default::default() }.into_state().unwrap();
        assert!(serde_json::to_value(none).unwrap()["cover"].is_null());
    }
```

Run: `cd windows && cargo test -p mayak --lib media:: 2>&1 | tail -5`. Expected: compile errors (missing items).

- [ ] **Step 2: Pure helpers** in `media.rs`:

```rust
/// Covers bigger than this are left out: they would cross the IPC on every track change.
pub const MAX_COVER_BYTES: usize = 262_144;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with `=` padding (RFC 4648 §4); no crate for one small image.
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// The cover as the island shows it, or `None`: empty, too big, or not an image.
pub fn cover_data_url(content_type: &str, bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAX_COVER_BYTES || !content_type.starts_with("image/") {
        return None;
    }
    Some(format!("data:{content_type};base64,{}", base64(bytes)))
}

/// One track: the cover is read again only when this changes (spec §B), never on play/pause.
pub fn cover_key(title: &str, artist: &str, app_id: &str) -> String {
    format!("{title}\u{1f}{artist}\u{1f}{app_id}")
}
```

Add `pub cover: Option<String>` to `MediaState` (doc: "The album art as a data URL (spec §B), `None` without one.") and to `Reading`, and carry it in `into_state`. Add `cover: None` wherever a `MediaState` or `Reading` literal exists in tests.

- [ ] **Step 3: WinRT cover read** in `media/win.rs`:
  - Add the feature `"Storage_Streams"` to the `windows` crate features in `Cargo.toml`, in alphabetical order.
  - Add a field `cover_cache: Option<(String, Option<String>)>` to `Subscriptions`, initialized `None`, and reset it in `drop_session`/`drop_all`.
  - `reading` gains the cache: `fn reading(session: &Session, cache: &mut Option<(String, Option<String>)>) -> Result<Reading>`. After reading title, artist and app id, compute `key = cover_key(..)`. If `cache` holds the same key, reuse its cover. Otherwise read it with `read_cover(&properties)` and store `(key, cover)`.
  - `read_cover`:

```rust
/// The thumbnail of the current track as a data URL; `None` without one, on any error, or when it
/// is bigger than MAX_COVER_BYTES. Every async step is bounded like the other WinRT calls.
fn read_cover(properties: &GlobalSystemMediaTransportControlsSessionMediaProperties) -> Option<String> {
    let reference = properties.Thumbnail().ok()?;
    let stream = bounded!("Thumbnail.OpenReadAsync", reference.OpenReadAsync().ok()?).ok()?;
    let size = stream.Size().ok()?;
    if size == 0 || size > MAX_COVER_BYTES as u64 {
        return None;
    }
    let content_type = stream.ContentType().ok()?.to_string_lossy();
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let loaded = bounded!("DataReader.LoadAsync", reader.LoadAsync(size as u32).ok()?).ok()?;
    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    cover_data_url(&content_type, &bytes)
}
```

  - Adapt the `bounded!` macro use to how it is defined in this file. If it returns `Result`, keep `.ok()?`. Import `DataReader` from `::windows::Storage::Streams` and the media properties type from `::windows::Media::Control`.
  - `read()` passes `&mut self.cover_cache`, so `read` takes `&mut self`; update its caller.
  - Log a failure only through the existing `log_error` with a fixed `what`, never with the content type or any title.

- [ ] **Step 4: TS type.** In `windows/src/core/bridge.ts`, interface `MediaState`, add `/** Album art as a data URL (spec §B), null without one. */ cover: string | null;`.

- [ ] **Step 5: Checks**
  - `cd windows && cargo test --workspace 2>&1 | grep -E "^test result|^warning|^error"` → all pass, 0 warnings.
  - `npx tsc --noEmit` → clean.

- [ ] **Step 6: Commit:** `git add windows/src-tauri/src/media.rs windows/src-tauri/src/media/win.rs windows/src-tauri/Cargo.toml windows/src/core/bridge.ts` (and `windows/Cargo.lock` only if it changed), then `git commit -m "feat(media): album art in the player state"`.

---

### Task 2: The bigger player row with the cover (TS)

**Files:**
- Modify: `windows/src/views/usage.ts` (`buildPlayer`)
- Modify: `windows/src/core/layout.ts` (`PLAYER_ROW_H`)
- Modify: `windows/src/style.css` (the player block, around "Home: the player row")

**Interfaces:** Consumes `MediaState.cover` (Task 1).

- [ ] **Step 1:** `PLAYER_ROW_H = 52` in `core/layout.ts`.
- [ ] **Step 2:** Rebuild `buildPlayer`'s DOM:
  - `cover` element: an `img` with class `player-cover` and `alt=""`, plus a fallback `span` with class `player-cover none` and text `♪`. Show one or the other.
  - `player-text` becomes a column: `title` (class `player-title`), then `meta` (class `player-meta`, text `artist · app`, without the leading " — ").
  - Remove the `mark` element.
  - Buttons: icons at size 11 instead of 8.
  - In `show(s)`: `if (s.cover) { img.src = s.cover; img.hidden = false; fallback.hidden = true } else { img.removeAttribute("src"); img.hidden = true; fallback.hidden = false }`. The rest stays as it is.
- [ ] **Step 3: CSS.** Replace the player block with:
  - `.card.with-player .usage { height: calc(100% - 52px); }`
  - `.player { position:absolute; z-index:1; left:116px; right:18px; bottom:8px; height:44px; display:flex; align-items:center; gap:10px; }`
  - `.player-cover { flex:0 0 40px; width:40px; height:40px; border-radius:6px; object-fit:cover; }`
  - `.player-cover.none { display:grid; place-items:center; background:rgba(255,255,255,0.06); color:var(--dim-2); font-size:16px; }`
  - `.player-cover[hidden] { display:none; }`
  - `.player-text { flex:1 1 auto; min-width:0; display:flex; flex-direction:column; gap:2px; }`
  - `.player-title { font:600 12px var(--font); color:var(--ink); white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }`
  - `.player-meta { font:400 11px var(--font); color:var(--dim-3); white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }`
  - `.player-buttons { flex:0 0 auto; display:flex; gap:8px; }`
  - Keep `.player[hidden]` and the disabled button rule.
  - Update the block comment: 52 px, cover.
- [ ] **Step 4: Look.** `npm run dev` may already be served on port 1420. In the browser console, call the player's `show` path by dispatching a fake state: there is no Tauri there, so set `State.media` and call the row's update through a temporary `window.__mediaShow = show` you add for the look and remove before committing. Check that the row fits and the title truncates. Do not leave any debug hook in the commit.
- [ ] **Step 5: Checks:** `npx tsc --noEmit` and `npx vite build`.
- [ ] **Step 6: Commit:** `git add windows/src/views/usage.ts windows/src/core/layout.ts windows/src/style.css`, then `git commit -m "feat(island): a bigger player row with the album art"`.

---

### Task 3: Mochi's activity (headphones for music, notes, laptop, coffee, sleep)

**Files:**
- Create: `windows/src/island/activity.ts` (pure)
- Modify: `windows/src/mochi/engine.ts` (`focusGear` becomes `activity`; drawing; the `note` particle; sleep look)
- Modify: `windows/src/island/island.ts` (`refreshGear` becomes `refreshActivity`; called on media changes too)
- Test: `C:/tmp/mayak-spike/activity-check.mjs`

**Interfaces:**
- Produces (`island/activity.ts`):

```ts
import type { Phase } from "./pomodoro";

export type Prop = "laptop" | "coffee" | null;

export interface ActivityLook {
  headphones: boolean;
  notes: boolean;
  prop: Prop;
  sleep: boolean;
}

/** Spec §A: music → headphones and notes; pomodoro work → laptop, short → coffee, long → sleep. */
export function activityFor(phase: Phase | null, playing: boolean): ActivityLook {
  return {
    headphones: playing,
    notes: playing,
    prop: phase === "work" ? "laptop" : phase === "short" ? "coffee" : null,
    sleep: phase === "long",
  };
}
```

- Produces (engine): `activity: (ActivityLook & { minutes: number | null; minutesColor: RGB }) | null`. It replaces `focusGear`.

- [ ] **Step 1: Node check** `C:/tmp/mayak-spike/activity-check.mjs`:

```js
import assert from "node:assert/strict";
import { activityFor } from "file:///C:/Users/HP/source/repos/mayak/windows/src/island/activity.ts";
assert.deepEqual(activityFor(null, false), { headphones: false, notes: false, prop: null, sleep: false });
assert.deepEqual(activityFor(null, true), { headphones: true, notes: true, prop: null, sleep: false });
assert.deepEqual(activityFor("work", false), { headphones: false, notes: false, prop: "laptop", sleep: false });
assert.deepEqual(activityFor("short", true), { headphones: true, notes: true, prop: "coffee", sleep: false });
assert.deepEqual(activityFor("long", false), { headphones: false, notes: false, prop: null, sleep: true });
console.log("activity-check: all passed");
```

Run it and see it fail. Write `activity.ts`. Run it again and see it pass.

- [ ] **Step 2: Engine.**
  - Replace the `focusGear` field with `activity` (type above; import `type ActivityLook` from `../island/activity`).
  - In `draw()`, inside the body transform after `drawBody`:
    - `if (this.activity?.headphones && !this.isMini) this.drawHeadphones(x, R, rx, ry);`
    - `drawHeadphones` uses fixed colours: band `#585b70`, cups `#cba6f7`. Drop its colour parameter.
  - After `drawParticles`:
    - `if (a?.prop === "laptop" && !this.isMini) this.drawLaptop(x, R, cx, cy);`
    - `if (a?.prop === "coffee" && !this.isMini) this.drawCoffee(x, R, cx, cy);`
    - then the minutes pill as now, using `a.minutes` and `a.minutesColor`.
  - `drawLaptop`: a dark rounded rect (fill `#1e1e2e`, stroke `#45475a`, 1 px) centred at `(cx, cy + R*0.78)`, size `R*1.5 × R*0.5`, radius `R*0.08`; a small dot `#cdd6f4` of radius `R*0.05` in its centre.
  - `drawCoffee`: a cup body (rounded rect fill `#f5e0dc`, size `R*0.42 × R*0.46`) at `(cx + R*1.15, cy + R*0.35)`; a handle (arc stroke `#f5e0dc`, width `R*0.06`) on its right; two short static steam strokes (`rgba(205,214,244,0.6)`, width `R*0.04`) above it.
  - **Notes:**
    - Extend `Particle["type"]` with `"note"`.
    - In `update()`, in the ambient block (every ~1.3 s), add `if (this.activity?.notes && !this.isMini) this.emit("note", 1);`.
    - In `drawParticles`, `case "note"`: fill `#f5c2e7`, font `700 ${sz * 2.1}px ${FONT}`, centred, text `"♪"` or `"♫"`. Pick it by `p.rot > Math.PI ? "♫" : "♪"`, so it stays stable for a particle.
    - In `get busy()`, add `|| (this.activity?.notes === true && !this.isMini)`.
  - **Sleep look:** where the eye shape is chosen in `drawEyes` (the effective shape from `eyeOverride` / `cfg.eye`): when `this.activity?.sleep && (this.state === "idle" || this.state === "finished")`, use `"closed"`. In the ambient block, also emit `"z"` under the same condition. Add the same condition to `busy`.
- [ ] **Step 3: Island.**
  - Rename `refreshGear` to `refreshActivity`.
  - Build `this.engine.activity` from `activityFor(State.pomodoro?.phase ?? null, State.media?.playing === true)`, plus the minutes and the colour as `refreshGear` computed them. Set it to `null` when `activityFor` gives all false/null and no pomodoro runs.
  - The compact `#focus-mins` label stays as it is.
  - Update every caller.
  - **Media changes:** in `build()` (or where the island registers listeners), add `void onMedia(() => this.refreshActivity());` (import `onMedia` from `../core/bridge`). `views/usage.ts` keeps its own listener that sets `State.media`. Rule: the island's listener must read `State.media` after usage.ts updated it. So the island's `refreshActivity` is deferred with `window.setTimeout(() => this.refreshActivity(), 0)` inside the listener.
  - `refreshActivity` calls `ensureRunning()` only while not hidden, as `refreshGear` did. A hidden island never starts frames for notes.
- [ ] **Step 4: Checks:** the Node check, `npx tsc --noEmit`, `npx vite build`.
- [ ] **Step 5: Look** in `npm run dev` (browser): with `State.media = { playing: true, … }` and a running pomodoro, check each phase (set `State.pomodoro.phase` by hand and call a re-render through starting and stopping). Report what you saw. Leave no debug hooks.
- [ ] **Step 6: Commit:** `git add windows/src/island/activity.ts windows/src/mochi/engine.ts windows/src/island/island.ts`, then `git commit -m "feat(island): Mochi wears headphones for music and shows the pomodoro phase"`.

---

### Task 4: Finding and opening installed apps (Rust)

**Files:**
- Create: `windows/src-tauri/src/apps.rs`
- Modify: `windows/src-tauri/src/lib.rs` (`mod apps;`, the two commands, registration in the invoke handler)
- Modify: `windows/src/core/bridge.ts` (`AppEntry`, `appsList()`, `appOpen(id)`)

**Interfaces:**
- Produces (Rust):
  - `#[derive(Serialize, Clone, Debug, PartialEq, Eq)] #[serde(rename_all = "camelCase")] pub struct AppEntry { pub id: &'static str, pub name: &'static str }`;
  - `pub fn found(localappdata: &Path, program_files: &Path, protocol: impl Fn(&str) -> bool, codex_family: Option<String>) -> Vec<AppEntry>`, pure over the given folders;
  - `pub fn list() -> Vec<AppEntry>` (Windows: real folders, `launcher::win::protocol_registered`, `launcher::codex_family`; elsewhere an empty Vec);
  - `pub fn open(id: &str) -> Result<(), String>` (Windows; elsewhere `Err("not supported")`).
- Produces (commands): `apps_list() -> Vec<AppEntry>`, `app_open(id: String) -> bool`. The latter runs `apps::open` on a short-lived thread and returns true when the id is known and its exe or route exists at click time.
- Produces (TS): `interface AppEntry { id: string; name: string }`, `Bridge.appsList(): Promise<AppEntry[] | null>`, `Bridge.appOpen(id: string): Promise<boolean | null>`, following the existing `call<…>` pattern.

- [ ] **Step 1: Failing tests** in `apps.rs`, using `crate::testutil::TempDir` the way other modules do:
  - nothing installed → empty;
  - `Programs\cursor\Cursor.exe` exists → `[cursor]`;
  - VS Code found under Program Files `Microsoft VS Code\Code.exe` → `vscode`;
  - order cursor, vscode, windsurf, zed, claude, codex when all are present (claude via `protocol("claude") == true`, codex via `Some(family)`);
  - a folder without the exe does not count;
  - `exe_for("cursor")` returns the path that `found` checked.
- [ ] **Step 2: Implement** following the spec §D table. A small static table `APPS: [(&str id, &str name, Kind); 6]`, where `Kind` is `Exe(&[&str] relative paths, under LOCALAPPDATA or ProgramFiles, &str exe_name)`, `Protocol("claude")` or `Appx`.
  - `open`, Windows:
    - `Exe`: if `focus::win::app_windows(exe_name)` has a window, `focus::win::raise(first hwnd)`. Otherwise `Command::new(path)` through `platform::no_console`, spawned. Re-check `path.is_file()` first and return Err if it is gone.
    - `Protocol`: `launcher::win::open_uri("claude://")`.
    - `Appx`: `launcher::win::launch_appx(&family)`.
  - Log one line `apps: open id=<id> outcome=<ok|err>`, never the path.
- [ ] **Step 3: Commands** in `lib.rs`, next to `launcher_targets`/`launcher_open`, following their style. `app_open` spawns a thread for `apps::open` and returns whether the id is in `apps::list()`. Register both in the invoke handler.
- [ ] **Step 4: Checks:** `cargo test --workspace` (0 warnings), `npx tsc --noEmit`.
- [ ] **Step 5: Commit:** `git add windows/src-tauri/src/apps.rs windows/src-tauri/src/lib.rs windows/src/core/bridge.ts`, then `git commit -m "feat(apps): find installed IDEs and agent apps and open them"`.

---

### Task 5: The Apps tab and the icons

**Files:**
- Create: `windows/src/views/app-icons.ts`
- Create: `windows/src/views/apps.ts`
- Create: `windows/THIRD_PARTY_NOTICES.md`
- Modify: `windows/src/views/views.ts` (tab and registry), `windows/src/core/layout.ts` (view `apps`), `windows/src/views/icons.ts` (a `grid` icon path for the tab), `windows/src/style.css` (`.apps*` block)

**Interfaces:**
- Consumes (Task 4): `Bridge.appsList`, `Bridge.appOpen`, `AppEntry`.
- Produces: `appIcon(id: string, size: number): HTMLElement`; `buildApps(actions: ViewActions): ViewHost`; view name `"apps"`.

- [ ] **Step 1: `app-icons.ts`.**
  - A header comment: "Cursor and VS Code icons: Catppuccin vscode-icons (Mocha), MIT License, https://github.com/catppuccin/vscode-icons — see windows/THIRD_PARTY_NOTICES.md".
  - SVG strings, verbatim from the repo (viewBox 0 0 16 16):
    - Cursor: two paths with `stroke="#cdd6f4"`, the first `d="m 16.17866,12.334298 -7.0470492,4.068616 -7.0470494,-4.068616 0,-8.1372317 L 9.1316109,0.12845039 16.17866,4.1970663 Z" transform="matrix(.92099 0 0 .92169 -.41 .382)"`, the second `d="M 1.51,4.25 H 14.49 L 8,15.5 V 8 Z"`. Both have `fill="none" stroke-linecap="round" stroke-linejoin="round"`.
    - VS Code: one path, `stroke="#89b4fa"`, `d="M10.5 11 3 4.5h-.5l-1 1V6l9 8.5 4-2v-9l-4-2v13m0-13L5.3 6.41M3.53 8.08 1.5 10v.5l.98 1.1.52-.1 2.17-1.88m1.91-1.66L10.5 5"`, with the same fill and line attributes.
  - Monograms (spec §E): claude "C" on `#fab387`, codex "Cx" on `#a6e3a1`, windsurf "W" on `#94e2d5`, zed "Z" on `#89b4fa`, letters `#11111b`. Each is a rounded square (radius 22 % of size) with the letters centred, `font: 800 (size*0.42)px var(--font)`.
  - `appIcon(id, size)` returns a `span.app-icon` sized `size`×`size`. It contains the SVG (Cursor and VS Code, drawn on a `#1e1e2e` rounded square with 18 % padding) or the monogram. An unknown id gets a monogram of the first letter on `#585b70`.
- [ ] **Step 2: `apps.ts`.** A card like Stats (no Mochi): a grid of `button.apps-tile`, each with `appIcon(id, 28)` and `span.apps-name`.
  - `setActive(true)` calls `Bridge.appsList()` and renders. A late answer after leaving the tab is dropped, with a request counter as in stats.ts.
  - Shows "Looking for apps…" while waiting, and "No apps found" when the list is empty.
  - A click calls `Bridge.appOpen(id)`, then `actions.collapse()` when it returns true. When false, it shows "Couldn't open {name}" in the card for 3 s with one `setTimeout`.
- [ ] **Step 3: Tab and layout.**
  - Add `ICONS.grid` (a 2×2 squares path in the same style as the other icons).
  - Add `tabApps` after `tabFocus` in `buildHeader`, title "Apps", with its `.on` toggle.
  - Register `map.set("apps", buildApps(actions))`.
  - Add `apps: { height: 200, botX: 320, botY: 90, botDiameter: 0, agentMode: "none" }` to `VIEW_LAYOUTS`, and `"apps"` to `IslandViewName`.
- [ ] **Step 4: CSS** `.apps*`:
  - the grid: `display:grid; grid-template-columns: repeat(6, 1fr); gap:10px; padding:14px 18px`;
  - the tile: a column, centred, `gap:6px`, transparent background, hover `rgba(255,255,255,0.06)`, radius 10px, padding 8px;
  - the name: 11px, ellipsis.
- [ ] **Step 5: `THIRD_PARTY_NOTICES.md`:** a heading, "Catppuccin vscode-icons — icons for Cursor and VS Code (windows/src/views/app-icons.ts)", the source URL, and the full MIT license text:

```
MIT License

Copyright (c) 2023 Catppuccin
Copyright (c) 2023 thang-nm

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

  Before writing it, check the license text against `gh api repos/catppuccin/vscode-icons/contents/LICENSE --jq .content | base64 -d`, and use the fetched text if it differs.
- [ ] **Step 6: Checks and look:** `npx tsc --noEmit`, `npx vite build`. In `npm run dev`, the tab shows "No apps found" (no Tauri), and all icons render: check them with a temporary render of all six ids, removed before commit.
- [ ] **Step 7: Commit:** stage the files above, then `git commit -m "feat(island): Apps tab with the installed IDEs and agent apps"`.

---

### Task 6: Localization — the module, the dictionaries, the island

**Files:**
- Create: `windows/src/core/i18n.ts`, and `windows/src/core/i18n/en.ts`, `ru.ts`, `de.ts`, `fr.ts`, `es.ts`, `zh.ts`, `ja.ts`
- Modify: every island-side file with user-visible text: `windows/src/views/*.ts` (views, sessions, usage, usage-format, stats, focus, apps, upload, ticker, integrations, chat), `windows/src/island/*.ts` (pomodoro labels, notes, alerts text), `windows/src/core/state.ts` (setting `language`), `windows/src/main.ts` (`setLanguage` at boot)
- Test: `C:/tmp/mayak-spike/i18n-check.mjs`

**Interfaces:**
- Produces:
  - `type Lang = "en" | "ru" | "de" | "fr" | "es" | "zh" | "ja"`;
  - `const LANGS: readonly Lang[]`;
  - `resolveLanguage(setting: string, navigatorLanguage: string): Lang`;
  - `setLanguage(setting: string): void`;
  - `currentLanguage(): Lang`;
  - `t(key: Key, vars?: Record<string, string | number>): string`;
  - `type Key = keyof typeof en`.
- `en.ts`: `export const en = { … } as const;`. The other files: `export const ru: Record<Key, string> = { … };`, etc.
- Settings: `language: string` in the TS `Settings` and `DEFAULT_SETTINGS` (`"system"`).
- `i18n.ts` keeps the pure part (`resolveLanguage`, `format(template, vars)`) importable under Node: the dictionaries are imported with `import type` only in the pure helpers' file, or the pure helpers live in `core/i18n-core.ts`. **Ruling for the implementer:** put `resolveLanguage` and `format` in `core/i18n-core.ts` (no imports) and the dictionary wiring in `core/i18n.ts`.

- [ ] **Step 1: Node check** `C:/tmp/mayak-spike/i18n-check.mjs`:

```js
import assert from "node:assert/strict";
import { resolveLanguage, format } from "file:///C:/Users/HP/source/repos/mayak/windows/src/core/i18n-core.ts";
assert.equal(resolveLanguage("system", "ru-RU"), "ru");
assert.equal(resolveLanguage("system", "zh-CN"), "zh");
assert.equal(resolveLanguage("system", "pt-BR"), "en");
assert.equal(resolveLanguage("de", "ru-RU"), "de");
assert.equal(resolveLanguage("xx", "ja-JP"), "ja");
assert.equal(resolveLanguage("", ""), "en");
assert.equal(format("{n} finished", { n: 3 }), "3 finished");
assert.equal(format("Couldn't open {name}", { name: "Cursor" }), "Couldn't open Cursor");
assert.equal(format("{a}{b}{a}", { a: 1, b: "x" }), "1x1");
assert.equal(format("no vars {x}", {}), "no vars {x}");
console.log("i18n-check: all passed");
```

  `resolveLanguage(setting, nav)`: if the setting is one of the languages, return it. Otherwise take the part of `nav` before `-` in lower case if it is a language, else `"en"`. An unknown setting such as `"xx"` behaves like `"system"`.
- [ ] **Step 2: Implement** `i18n-core.ts`, `i18n.ts` and `en.ts` with every island string, then translate into `ru`, `de`, `fr`, `es`, `zh` (Simplified) and `ja`.
  - Keys are short dotted names (`home.noData`, `focus.start`, `pomodoro.cardBreak` …).
  - Strings that are built from parts use `{var}`. Plurals: for `ru`, use the simplest correct form for the numbers shown, or a neutral phrasing ("Завершено: 3"). No plural library.
  - Product, agent, model and app names are never translated.
- [ ] **Step 3: Replace** every user-visible string literal in the island files with `t("…")`.
  - That includes `title` attributes, placeholders, `textContent` assignments and the pure label functions in `island/pomodoro.ts`, `island/focus.ts` and `views/usage-format.ts`.
  - Exception: if a pure module is imported by a Node check (`pomodoro.ts`, `focus.ts`), keep it importable. Pass the translated strings in from the caller, or give the label functions a `tr: (key, vars?) => string` parameter defaulting to the English template. Then update `pomodoro-check.mjs` only if a signature changed, keeping its English expectations.
  - Text that overflows in German or French gets `text-overflow: ellipsis` and the full text in `title`.
- [ ] **Step 4: Boot.** In `main.ts`, call `setLanguage(State.settings.language)` right after the boot settings merge and before the island is built. When `settings-changed` brings a different `language`, call `location.reload()`.
- [ ] **Step 5: Checks:**
  - the i18n check, the pomodoro and activity checks;
  - `npx tsc --noEmit` (dictionary completeness);
  - `npx vite build`;
  - `grep -rnE "(text|title|placeholder): \"[A-Za-z]" windows/src/views windows/src/island` must print only names that are not UI copy (agent, app or product names). List the remaining hits in the report with the reason for each.
- [ ] **Step 6: Commit:** stage the new files and every modified island file, then `git commit -m "feat(i18n): island texts in seven languages"`.

---

### Task 7: Localization — Settings window, the language setting, the tray

**Files:**
- Modify: `windows/src/settings/main.ts` (all texts through `t()`, the Language row), the dictionaries `windows/src/core/i18n/*.ts` (new keys)
- Modify: `windows/src-tauri/src/settings.rs` (`language` field), `windows/src-tauri/src/tray.rs` (keep the five items, a `set_texts` function), `windows/src-tauri/src/lib.rs` (command `tray_texts`)
- Modify: `windows/src/main.ts` (send the tray texts after `setLanguage`), `windows/src/core/bridge.ts` (`trayTexts`)

**Interfaces:**
- Consumes (Task 6): `t`, `setLanguage`, `LANGS`, `currentLanguage`.
- Produces:
  - Rust `Settings.language: String`, `#[serde(default = "default_language")]` → `"system"`;
  - `tray::set_texts(app, TrayTexts { open, settings, focus, pause, quit })`;
  - command `tray_texts(texts: TrayTexts)`;
  - TS `Bridge.trayTexts(texts)`.

- [ ] **Step 1: Rust tests.** An older settings file loads with `language == "system"`, and `language` round-trips as camelCase `language`. Run them and see them fail. Add the field and make them pass.
- [ ] **Step 2: Tray.**
  - Keep the five items in managed state, as `PauseItem`/`FocusItem` are (add `OpenItem`, `SettingsItem`, `QuitItem`, or one struct `TrayItems` holding all five).
  - `set_texts` calls `set_text` on each and logs a failure with a fixed string.
  - The command `tray_texts` takes `#[derive(Deserialize)] struct TrayTexts { open: String, settings: String, focus: String, pause: String, quit: String }`, caps each at 64 chars, and calls `set_texts`.
  - Register it.
- [ ] **Step 3: Island side.** In `main.ts`, after `setLanguage(...)`, call `void Bridge.trayTexts({ open: t("tray.open"), settings: t("tray.settings"), focus: t("tray.focus"), pause: t("tray.pause"), quit: t("tray.quit") })`, and add these keys to all seven dictionaries.
- [ ] **Step 4: Settings window.**
  - Call `setLanguage(settings.language)` after boot.
  - Every text through `t()`, in all seven languages.
  - In General, add a "Language" row with a `select` of: System, English, Русский, Deutsch, Français, Español, 中文, 日本語. The language names are not translated; "System" is.
  - A change saves and then calls `location.reload()`. Under the select, the hint "Mayak reloads its windows; a running pomodoro stops." goes through `t()`.
  - When `settings-changed` brings another language, reload.
- [ ] **Step 5: Checks:** `cargo test --workspace` (0 warnings), `npx tsc --noEmit`, `npx vite build`, the Node checks. A grep like Task 6's over `windows/src/settings` lists the remaining literals with reasons.
- [ ] **Step 6: Commit:** stage the files, then `git commit -m "feat(i18n): Settings window, language choice and tray in seven languages"`.

---

### Task 8: Live check and docs (controller)

- [ ] In the running dev app (CDP) or the browser pane, check:
  - the player with a cover (Spotify or a browser video);
  - the notes and headphones;
  - each pomodoro phase prop;
  - the Apps tab tiles and opening one;
  - switching the language to Deutsch and then Русский (the island, Settings and the tray);
  - with the island hidden, no tick and no frame loop (wrap the timers).
- [ ] Update the ideas §21–§25 with "done on feature/batch-2026-10-07", and record the remaining work.
- [ ] Report to the user.
