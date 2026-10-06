# Mayak: island strip, collapse button and quiet mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On Windows a hidden island is a thin visible line at the top centre that stretches towards an approaching cursor and wakes on the first touch; the island gets a collapse button (home → petit → strip) and a quiet-mode button that is the tray's Pause.

**Architecture:** A new Windows-only module `strip.rs`: a pure part (zone/line/catcher geometry, `in_zone`, `stretch`, `touches`, `is_fullscreen`, the hook's `Filter`, the `Translator` from raw strip messages to island events — all unit-tested) and a Win32 part: one thread per hidden period with its own message loop that owns a `WH_MOUSE_LL` hook (coordinate compare plus a bounded-channel `try_send`, nothing else), a 132×10 native catcher window and two out-of-context WinEvent hooks for fullscreen; a second thread turns the channel into Tauri events. `set_collapsed` starts the thread on hide and stops it on show; the hidden webview window is the 600×150 zone and passes every click. The page draws the line from `strip-cursor`, wakes on `strip-touch`, hides it on `strip-fullscreen`; header buttons drive `fsm.forceCollapse()` and the existing `setPaused`.

**Tech Stack:** Rust (Tauri 2.12, `windows` crate 0.61.3 — three new feature flags, no new crates), TypeScript (no new packages, no test runner).

**Spec:** `docs/superpowers/specs/2026-10-06-mayak-island-strip-design.md` (Russian; new spec text in Russian). Upstream context: `docs/superpowers/ideas/2026-10-06-upstream-review.md` (#28).

## Global Constraints

- Windows only is built (D84). `strip.rs` compiles only under `cfg(windows)`; Linux/macOS keep the invisible 240×6 strip (spec §8).
- FSM state names stay `hidden | petit | home | coucou`; `hidden` now means the visible strip (spec §2).
- Zone `STRIP_ZONE_W × STRIP_ZONE_H` = 600×150 logical px, top centre; line `STRIP_LINE_W × STRIP_LINE_H` = 120×4 px; `STRIP_TOUCH_MARGIN` = 6 px; catcher `(120 + 12) × (4 + 6)` logical px (spec §2, §3, §4.2).
- `t = clamp(1 − d / 150, 0, 1)`; length `120 + t × 200`; thickness `4 + t × 8`; centre shift `t × 0.5 × (x_cursor − x_centre)`, never past the zone (spec §3).
- Line colour `rgba(255,255,255,0.35)` on the island's dark backing, rounded ends; quiet: opacity 0.18, dashed; return to rest is one 150 ms CSS transition (spec §2, §3).
- No new crates. Only `windows` crate features may be added (`Win32_Graphics_Gdi`, `Win32_System_LibraryLoader`, `Win32_UI_Accessibility`).
- No timers or polling while hidden (0 % CPU when the mouse is still and the foreground window does not change). The hook handler only compares coordinates and `try_send`s on a pre-allocated bounded channel, then `CallNextHookEx`; it never calls Tauri, never blocks, never allocates (spec §4.1, §7).
- Do not restyle existing views. Only new elements (`#strip-line`, `#petit-collapse`, two header buttons that reuse the existing `.header-actions button` rule) follow the app style.
- Build environment: always `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target`; **never** touch `windows/target/release` and never stop a `mayak.exe` the user runs. Front-end checks are `npx tsc --noEmit` and `npx vite build` (not `npm run build`: its `prebuild` runs cargo).
  `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace`
- Baseline before this plan (measured 2026-10-06 on `feature/island-strip` at `9771255`): `cargo test --workspace` = **362 lib passed (1 ignored) + 32 hook passed**, 0 failed, 0 warnings.
- Keep each file's existing line endings (working tree is CRLF, `core.autocrlf=true`; check with `file`); edit in place, never rewrite whole files.
- Commit messages: conventional, one line, **no trailers** (no Co-Authored-By, no «Generated with»), never mention an AI assistant anywhere. Never push without the user's explicit yes (Task 6 asks). Tests run locally only.
- Branch: `feature/island-strip` (spec at `b0b18e8`).
- Verified only live (no unit test can reach it): the hook firing on real mouse movement, the catcher catching a real cursor, WinEvent fullscreen detection, click-through of the zone over browser tabs, the tray check mark, CPU. Task 6 runs these with the user.

## Review Focus

1. A monitor left of the primary (negative x) or at 125 %/150 % scale: the zone the hook compares, the catcher and the webview window must be the same rectangle, or touches land off the line — pinned by the three `geometry` tests in Task 1.
2. A «left the zone» or a move the full channel refused must be sent again at the next mouse event, or the line stays stretched (or never wakes) until the next hide — pinned by `a_message_the_channel_refused_is_sent_again_at_the_next_event` in Task 1.
3. A fullscreen video or game with the cursor at the top edge: the invisible strip must neither draw nor wake the island, leaving fullscreen brings it back, and one approach wakes once — pinned by `nothing_reaches_the_island_while_an_app_is_fullscreen` and `a_cursor_on_the_line_wakes_the_island_once_until_it_leaves` in Task 1.
4. Click-through after the poll parks (upstream #28): whichever of `set_collapsed` and the parking poll thread writes last, the hidden Windows window passes every click (browser tabs under the zone) and a visible one takes the mouse — pinned by `parked_the_window_passes_clicks_only_as_the_native_strip` in Task 3.
5. Hiding and showing many times: every hide starts a thread with hooks and a window; every show must end both threads and release them — pinned by `the_strip_thread_reports_its_catcher_and_ends_cleanly_every_time` (two cycles, sink disconnected) in Task 2.

---

## File Structure

- Create `windows/src-tauri/src/strip.rs` — pure strip logic (Task 1) and the Win32 thread, hooks, catcher, `enter`/`leave` (Task 2).
- Modify `windows/src-tauri/src/island.rs` — zone constants, `top_centre` (Task 1); `NATIVE_STRIP`, `collapsed_size`, `parked_ignore`, #28 fix, `strip_geometry` (Task 3).
- Modify `windows/src-tauri/src/lib.rs` — `mod strip;` (Task 1); `place_island`, `set_collapsed`, `reposition`, `save_settings`, `BootInfo.native_strip` (Task 3); `set_paused(app, paused)` (Task 5).
- Modify `windows/src-tauri/Cargo.toml` (+ `windows/Cargo.lock` if it changes) — three `windows` features (Task 2).
- Modify `windows/src-tauri/src/tray.rs` — Pause becomes a check item the island keeps in step (Task 5).
- Create `windows/src/island/strip.ts` — the TS mirror of `line_at` (Task 4).
- Modify `windows/src/core/bridge.ts` (Tasks 3-4), `windows/src/main.ts` (Tasks 3-5), `windows/src/island/island.ts` (Tasks 3-5), `windows/src/island/fsm.ts` (Task 5), `windows/src/views/views.ts` (Task 5), `windows/src/views/icons.ts` (Task 5), `windows/src/style.css` (Tasks 4-5).
- Modify `docs/superpowers/specs/2026-10-06-mayak-island-strip-design.md`, `windows/README.md` (Task 6).

Task shape note: the suggested tasks 2 and 3 are split differently here. The Win32 thread is its own task (Task 2, not wired, testable through its lifecycle test), and the #28 fix lands with the wiring (Task 3), because the correct parked flag depends on whether the native strip exists, and because wiring the click-through zone without the thread and the `strip-touch` listener would leave a commit where a hidden island cannot be woken. `forceCollapse` moves to Task 5 with the buttons, its only callers.

---

### Task 1: Pure strip geometry, touch, fullscreen, filter and translator

**Files:**
- Create: `windows/src-tauri/src/strip.rs`
- Modify: `windows/src-tauri/src/island.rs` (constants after `STRIP_H` at line 22; `apply_geometry` lines 160-164; new test module at the end)
- Modify: `windows/src-tauri/src/lib.rs` (module list, after `mod settings;`)

**Interfaces:**
- Consumes: nothing new.
- Produces (later tasks rely on these exact names):
  `island::STRIP_ZONE_W: f64 = 600.0`, `island::STRIP_ZONE_H: f64 = 150.0`
  `pub fn island::top_centre(mon_x: i32, mon_y: i32, mon_w: u32, logical: (f64, f64), scale: f64) -> (i32, i32, u32, u32)` — `(x, y, physical w, physical h)`, the exact placement `apply_geometry` uses.
  `strip::STRIP_LINE_W/STRIP_LINE_H/STRIP_TOUCH_MARGIN: f64`
  `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct strip::PxRect { pub left: i32, pub top: i32, pub right: i32, pub bottom: i32 }` (physical, right/bottom exclusive)
  `#[derive(Debug, Clone, Copy, PartialEq)] pub struct strip::Rect { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }` (logical, strip window)
  `#[derive(Debug, Clone, Copy, PartialEq)] pub struct strip::StripGeometry { pub zone: PxRect, pub catcher: PxRect, pub monitor: PxRect, pub scale: f64 }`
  `pub fn geometry(monitor: PxRect, scale: f64) -> StripGeometry`
  `pub fn in_zone(p: (i32, i32), zone: PxRect) -> bool`
  `pub fn stretch(d: f64) -> (f64, f64)`, `pub fn pull(d: f64) -> f64`, `pub fn line_at(p: (f64, f64)) -> Rect`
  `pub fn touches(p: (f64, f64), line: Rect, margin: f64) -> bool`
  `pub fn is_fullscreen(window: PxRect, monitor: PxRect, class: &str) -> bool`
  `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Msg { Move(i32, i32), Left, Touch, Fullscreen(bool), Display }`
  `pub struct Filter` with `pub fn new(zone: PxRect, scale: f64) -> Self` and `pub fn feed(&mut self, x: i32, y: i32, send: impl FnOnce(Msg) -> bool)`
  `#[derive(Debug, Clone, Copy, PartialEq)] pub enum Out { Cursor(Option<(f64, f64)>), Touch, Fullscreen(bool), ScreenChanged }`
  `pub struct Translator` with `pub fn new(geo: StripGeometry) -> Self` and `pub fn on(&mut self, msg: Msg) -> Vec<Out>`

- [ ] **Step 1: Island constants and `top_centre` (behaviour-preserving)**

In `island.rs`, after `pub const STRIP_H: f64 = 6.0;` add:

```rust
/// Logical size of the window while a Windows island is hidden: the zone in which the visible
/// strip reacts to the cursor (spec §2). The webview passes every click there; `strip.rs` decides.
pub const STRIP_ZONE_W: f64 = 600.0;
pub const STRIP_ZONE_H: f64 = 150.0;
```

Above `/// Places and sizes the window.` add:

```rust
/// Top-centre placement of a `logical` size on a monitor at (`mon_x`, `mon_y`), `mon_w` physical
/// px wide: `(x, y, physical width, physical height)`. `apply_geometry` and the strip zone
/// (`strip::geometry`) both place through here, so the zone the hook tests is the window.
pub fn top_centre(mon_x: i32, mon_y: i32, mon_w: u32, logical: (f64, f64), scale: f64) -> (i32, i32, u32, u32) {
    let pw = (logical.0 * scale).round().max(1.0) as u32;
    let ph = (logical.1 * scale).round().max(1.0) as u32;
    (mon_x + (mon_w as i32 - pw as i32) / 2, mon_y, pw, ph)
}
```

In `apply_geometry` replace

```rust
    let (lw, lh) = if collapsed { (STRIP_W, STRIP_H) } else { (PANEL_W, PANEL_H) };
    let pw = (lw * scale).round().max(1.0) as u32;
    let ph = (lh * scale).round().max(1.0) as u32;
    let x = mp.x + (ms.width as i32 - pw as i32) / 2;
    let y = mp.y;
```

with

```rust
    let (lw, lh) = if collapsed { (STRIP_W, STRIP_H) } else { (PANEL_W, PANEL_H) };
    let (x, y, pw, ph) = top_centre(mp.x, mp.y, ms.width, (lw, lh), scale);
```

Append at the end of `island.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_centre_places_the_panel_as_before() {
        assert_eq!(top_centre(0, 0, 1920, (PANEL_W, PANEL_H), 1.0), (600, 0, 720, 320));
        assert_eq!(top_centre(-1920, 0, 1920, (PANEL_W, PANEL_H), 1.5), (-1500, 0, 1080, 480));
    }
}
```

In `lib.rs` after `mod settings;` add:

```rust
#[cfg(windows)]
#[allow(dead_code)] // wired into set_collapsed in Task 3; remove this attribute there
mod strip;
```

- [ ] **Step 2: Create `strip.rs` with the tests only**

`windows/src-tauri/src/strip.rs`:

```rust
//! The visible strip of a hidden island, Windows only (spec 2026-10-06-mayak-island-strip-design.md).
//! The pure part: where the zone, the line and the touch catcher are, how the line stretches,
//! when a cursor touches it, what counts as fullscreen, what the mouse hook lets through, and how
//! raw strip messages become island events. The Win32 part (thread, hooks, catcher) follows it.

use crate::island::{top_centre, STRIP_ZONE_H, STRIP_ZONE_W};

#[cfg(test)]
mod tests {
    use super::*;

    const PRIMARY: PxRect = PxRect { left: 0, top: 0, right: 1920, bottom: 1080 };

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn assert_rect(r: Rect, x: f64, y: f64, w: f64, h: f64) {
        assert!(close(r.x, x) && close(r.y, y) && close(r.w, w) && close(r.h, h), "{r:?} != ({x}, {y}, {w}, {h})");
    }

    /// The message the filter tried to send, with the channel accepting it or not.
    fn feed(f: &mut Filter, x: i32, y: i32, accept: bool) -> Option<Msg> {
        let mut sent = None;
        f.feed(x, y, |m| {
            sent = Some(m);
            accept
        });
        sent
    }

    #[test]
    fn the_zone_is_the_strip_window_at_the_top_centre_of_the_monitor() {
        let g = geometry(PRIMARY, 1.0);
        assert_eq!(g.zone, PxRect { left: 660, top: 0, right: 1260, bottom: 150 });
        assert_eq!(g.catcher, PxRect { left: 894, top: 0, right: 1026, bottom: 10 });
        assert_eq!(g.monitor, PRIMARY);
    }

    #[test]
    fn a_scaled_monitor_left_of_the_primary_keeps_zone_and_catcher_aligned() {
        let left = PxRect { left: -1920, top: 0, right: 0, bottom: 1080 };
        let g = geometry(left, 1.5);
        assert_eq!(g.zone, PxRect { left: -1410, top: 0, right: -510, bottom: 225 });
        assert_eq!(g.catcher, PxRect { left: -1059, top: 0, right: -861, bottom: 15 });
        // the window apply_geometry places is the zone
        assert_eq!(top_centre(-1920, 0, 1920, (STRIP_ZONE_W, STRIP_ZONE_H), 1.5), (-1410, 0, 900, 225));
    }

    #[test]
    fn fractional_scales_round_the_way_the_window_does() {
        let g = geometry(PxRect { left: 0, top: 0, right: 2560, bottom: 1440 }, 1.25);
        assert_eq!(g.zone, PxRect { left: 905, top: 0, right: 1655, bottom: 188 });
        assert_eq!(g.catcher, PxRect { left: 1198, top: 0, right: 1363, bottom: 13 });
    }

    #[test]
    fn the_zone_holds_its_top_left_edge_but_not_its_bottom_right() {
        let zone = geometry(PRIMARY, 1.0).zone;
        assert!(in_zone((660, 0), zone));
        assert!(in_zone((1259, 149), zone));
        assert!(!in_zone((659, 0), zone));
        assert!(!in_zone((1260, 0), zone));
        assert!(!in_zone((700, 150), zone));
        assert!(!in_zone((700, -1), zone));
    }

    #[test]
    fn the_line_stretches_from_150_px_away_to_its_full_size() {
        assert_eq!(stretch(0.0), (320.0, 12.0));
        assert_eq!(stretch(75.0), (220.0, 8.0));
        assert_eq!(stretch(150.0), (120.0, 4.0));
        assert_eq!(stretch(400.0), (120.0, 4.0));
    }

    #[test]
    fn the_line_grows_towards_the_cursor_and_stays_put_when_it_is_far() {
        assert_rect(line_at((300.0, 200.0)), 240.0, 0.0, 120.0, 4.0);
        assert_rect(line_at((300.0, 4.0)), 140.0, 0.0, 320.0, 12.0);
        // d = 60 → t = 0.6: 240 long, 8.8 thick, centre pulled 0.6 × 0.5 × 120 = 36 px right
        assert_rect(line_at((420.0, 2.0)), 216.0, 0.0, 240.0, 8.8);
    }

    #[test]
    fn a_touch_is_six_px_around_the_line() {
        let line = Rect { x: 240.0, y: 0.0, w: 120.0, h: 4.0 };
        assert!(touches((234.0, 0.0), line, STRIP_TOUCH_MARGIN));
        assert!(!touches((233.9, 0.0), line, STRIP_TOUCH_MARGIN));
        assert!(touches((366.0, 10.0), line, STRIP_TOUCH_MARGIN));
        assert!(!touches((366.1, 10.0), line, STRIP_TOUCH_MARGIN));
        assert!(!touches((300.0, 10.1), line, STRIP_TOUCH_MARGIN));
        assert!(touches((300.0, -6.0), line, STRIP_TOUCH_MARGIN));
    }

    #[test]
    fn a_window_exactly_over_the_monitor_is_fullscreen() {
        assert!(is_fullscreen(PRIMARY, PRIMARY, "Chrome_WidgetWin_1"));
    }

    #[test]
    fn a_window_one_px_short_or_on_another_monitor_is_not_fullscreen() {
        let short = PxRect { bottom: 1079, ..PRIMARY };
        assert!(!is_fullscreen(short, PRIMARY, "Chrome_WidgetWin_1"));
        let narrow = PxRect { left: 1, ..PRIMARY };
        assert!(!is_fullscreen(narrow, PRIMARY, "Chrome_WidgetWin_1"));
        let second = PxRect { left: 1920, top: 0, right: 3840, bottom: 1080 };
        assert!(!is_fullscreen(second, PRIMARY, "Chrome_WidgetWin_1"));
    }

    #[test]
    fn the_desktop_and_the_taskbar_are_never_fullscreen() {
        for class in ["Progman", "WorkerW", "Shell_TrayWnd"] {
            assert!(!is_fullscreen(PRIMARY, PRIMARY, class), "{class}");
        }
    }

    #[test]
    fn a_maximised_window_overhanging_the_monitor_counts_as_fullscreen() {
        // Spec §7 risk: a maximised window with an auto-hidden taskbar overhangs by its border.
        let maximised = PxRect { left: -8, top: -8, right: 1928, bottom: 1088 };
        assert!(is_fullscreen(maximised, PRIMARY, "CASCADIA_HOSTING_WINDOW_CLASS"));
    }

    #[test]
    fn the_hook_sends_moves_of_at_least_one_logical_px_inside_the_zone() {
        let mut f = Filter::new(geometry(PRIMARY, 1.0).zone, 1.5);
        assert_eq!(feed(&mut f, 100, 100, true), None);
        assert_eq!(feed(&mut f, 700, 10, true), Some(Msg::Move(700, 10)));
        assert_eq!(feed(&mut f, 701, 10, true), None); // 1 physical px < 1 logical px at 150 %
        assert_eq!(feed(&mut f, 702, 11, true), Some(Msg::Move(702, 11)));
        assert_eq!(feed(&mut f, 702, 12, true), None);
    }

    #[test]
    fn leaving_the_zone_is_sent_once() {
        let mut f = Filter::new(geometry(PRIMARY, 1.0).zone, 1.0);
        assert_eq!(feed(&mut f, 700, 10, true), Some(Msg::Move(700, 10)));
        assert_eq!(feed(&mut f, 1260, 10, true), Some(Msg::Left));
        assert_eq!(feed(&mut f, 1300, 10, true), None);
        assert_eq!(feed(&mut f, 700, 10, true), Some(Msg::Move(700, 10)));
    }

    #[test]
    fn a_message_the_channel_refused_is_sent_again_at_the_next_event() {
        let mut f = Filter::new(geometry(PRIMARY, 1.0).zone, 1.0);
        assert_eq!(feed(&mut f, 700, 10, false), Some(Msg::Move(700, 10)));
        assert_eq!(feed(&mut f, 700, 10, true), Some(Msg::Move(700, 10)));
        assert_eq!(feed(&mut f, 100, 100, false), Some(Msg::Left));
        assert_eq!(feed(&mut f, 100, 101, true), Some(Msg::Left));
        assert_eq!(feed(&mut f, 100, 102, true), None);
    }

    #[test]
    fn a_cursor_on_the_line_wakes_the_island_once_until_it_leaves() {
        let mut t = Translator::new(geometry(PRIMARY, 1.0));
        // physical (960, 17) is logical (300, 17): 13 px below the line, inside its stretched margin
        assert_eq!(t.on(Msg::Move(960, 17)), vec![Out::Cursor(Some((300.0, 17.0))), Out::Touch]);
        assert_eq!(t.on(Msg::Move(961, 17)), vec![Out::Cursor(Some((301.0, 17.0)))]);
        assert_eq!(t.on(Msg::Touch), vec![]);
        assert_eq!(t.on(Msg::Left), vec![Out::Cursor(None)]);
        assert_eq!(t.on(Msg::Move(960, 17)), vec![Out::Cursor(Some((300.0, 17.0))), Out::Touch]);
        // one px further down is outside the stretched margin
        let mut fresh = Translator::new(geometry(PRIMARY, 1.0));
        assert_eq!(fresh.on(Msg::Move(960, 18)), vec![Out::Cursor(Some((300.0, 18.0)))]);
    }

    #[test]
    fn nothing_reaches_the_island_while_an_app_is_fullscreen() {
        let mut t = Translator::new(geometry(PRIMARY, 1.0));
        assert_eq!(t.on(Msg::Fullscreen(true)), vec![Out::Fullscreen(true), Out::Cursor(None)]);
        assert_eq!(t.on(Msg::Move(960, 2)), vec![]);
        assert_eq!(t.on(Msg::Touch), vec![]);
        assert_eq!(t.on(Msg::Left), vec![]);
        assert_eq!(t.on(Msg::Fullscreen(false)), vec![Out::Fullscreen(false)]);
        assert_eq!(t.on(Msg::Touch), vec![Out::Touch]);
        assert_eq!(t.on(Msg::Touch), vec![]);
    }

    #[test]
    fn the_cursor_reaches_the_page_in_strip_window_logical_px() {
        // 150 %: the zone starts at (1920 − 900) / 2 = 510
        let mut t = Translator::new(geometry(PRIMARY, 1.5));
        assert_eq!(t.on(Msg::Move(960, 15)), vec![Out::Cursor(Some((300.0, 10.0))), Out::Touch]);
    }

    #[test]
    fn a_display_change_asks_the_page_to_place_the_island_again() {
        let mut t = Translator::new(geometry(PRIMARY, 1.0));
        assert_eq!(t.on(Msg::Display), vec![Out::ScreenChanged]);
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak strip:: 2>&1 | tail -15`
Expected: FAIL to compile with ``error[E0425]: cannot find function `geometry` in this scope`` (and the other missing items: `PxRect`, `Rect`, `in_zone`, `stretch`, `line_at`, `touches`, `is_fullscreen`, `Filter`, `Msg`, `Translator`, `Out`).

- [ ] **Step 4: Implement the pure part**

Insert between the `use` line and `#[cfg(test)]` in `strip.rs`:

```rust
/// Logical size of the line at rest (spec §2).
pub const STRIP_LINE_W: f64 = 120.0;
pub const STRIP_LINE_H: f64 = 4.0;
/// Slack around the line that still counts as touching it, logical px (spec §3).
pub const STRIP_TOUCH_MARGIN: f64 = 6.0;
/// Distance from the line at which the stretch starts, logical px.
const STRETCH_REACH: f64 = 150.0;
/// What a cursor on the line adds to its length and its thickness.
const STRETCH_LEN: f64 = 200.0;
const STRETCH_THICK: f64 = 8.0;
/// Share of the cursor's offset from the centre that the line follows.
const STRETCH_FOLLOW: f64 = 0.5;
/// The touch catcher: the line plus the margin on both sides and below it (spec §4.2).
const CATCHER_W: f64 = STRIP_LINE_W + 2.0 * STRIP_TOUCH_MARGIN;
const CATCHER_H: f64 = STRIP_LINE_H + STRIP_TOUCH_MARGIN;
/// Window classes of the desktop and the taskbar: never «fullscreen» (spec §4.3).
const SHELL_CLASSES: [&str; 3] = ["Progman", "WorkerW", "Shell_TrayWnd"];

/// A rectangle in physical screen px, right and bottom exclusive (like a Win32 `RECT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// A rectangle in logical px of the strip window, origin at its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StripGeometry {
    /// The strip window, which is the zone: exactly where `island::apply_geometry` puts it.
    pub zone: PxRect,
    /// The native touch catcher over the line at rest.
    pub catcher: PxRect,
    /// The monitor of the island, for the fullscreen test.
    pub monitor: PxRect,
    pub scale: f64,
}

/// Zone and catcher of a hidden island on `monitor` at `scale` (spec §2, §4.2).
pub fn geometry(monitor: PxRect, scale: f64) -> StripGeometry {
    let width = (monitor.right - monitor.left) as u32;
    let (x, y, w, h) = top_centre(monitor.left, monitor.top, width, (STRIP_ZONE_W, STRIP_ZONE_H), scale);
    let zone = PxRect { left: x, top: y, right: x + w as i32, bottom: y + h as i32 };
    let px = |v: f64| (v * scale).round() as i32;
    let left = zone.left + px((STRIP_ZONE_W - CATCHER_W) / 2.0);
    let catcher = PxRect { left, top: zone.top, right: left + px(CATCHER_W), bottom: zone.top + px(CATCHER_H) };
    StripGeometry { zone, catcher, monitor, scale }
}

/// The hook's only test: is the cursor (physical px) in the zone?
pub fn in_zone(p: (i32, i32), zone: PxRect) -> bool {
    p.0 >= zone.left && p.0 < zone.right && p.1 >= zone.top && p.1 < zone.bottom
}

fn base_line() -> Rect {
    Rect { x: (STRIP_ZONE_W - STRIP_LINE_W) / 2.0, y: 0.0, w: STRIP_LINE_W, h: STRIP_LINE_H }
}

/// Euclidean distance from `p` to the nearest point of `r`; 0 inside it.
fn distance(p: (f64, f64), r: Rect) -> f64 {
    let dx = (r.x - p.0).max(0.0).max(p.0 - (r.x + r.w));
    let dy = (r.y - p.1).max(0.0).max(p.1 - (r.y + r.h));
    dx.hypot(dy)
}

/// `t = clamp(1 − d / 150, 0, 1)` (spec §3).
pub fn pull(d: f64) -> f64 {
    (1.0 - d / STRETCH_REACH).clamp(0.0, 1.0)
}

/// Length and thickness of the line for a cursor `d` logical px from it (spec §3).
pub fn stretch(d: f64) -> (f64, f64) {
    let t = pull(d);
    (STRIP_LINE_W + t * STRETCH_LEN, STRIP_LINE_H + t * STRETCH_THICK)
}

/// The line as drawn for a cursor at `p` (logical px of the strip window): stretched, its centre
/// pulled towards the cursor's x, never past the zone. `lineAt` in `src/island/strip.ts` is the
/// same formula; the page draws it, this decides the touch.
pub fn line_at(p: (f64, f64)) -> Rect {
    let base = base_line();
    let d = distance(p, base);
    let t = pull(d);
    let (len, thick) = stretch(d);
    let centre = STRIP_ZONE_W / 2.0;
    let pulled = centre + t * STRETCH_FOLLOW * (p.0 - centre);
    let c = pulled.clamp(len / 2.0, STRIP_ZONE_W - len / 2.0);
    Rect { x: c - len / 2.0, y: 0.0, w: len, h: thick }
}

/// The cursor is on `line` give or take `margin`, edges included (spec §3).
pub fn touches(p: (f64, f64), line: Rect, margin: f64) -> bool {
    p.0 >= line.x - margin && p.0 <= line.x + line.w + margin && p.1 >= line.y - margin && p.1 <= line.y + line.h + margin
}

/// The foreground window covers the whole monitor of the island and is not the desktop or the
/// taskbar (spec §4.3). A window larger than the monitor covers it too.
pub fn is_fullscreen(window: PxRect, monitor: PxRect, class: &str) -> bool {
    !SHELL_CLASSES.contains(&class)
        && window.left <= monitor.left
        && window.top <= monitor.top
        && window.right >= monitor.right
        && window.bottom >= monitor.bottom
}

/// What the strip thread hands to the event thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    /// The cursor moved inside the zone, physical screen px.
    Move(i32, i32),
    /// The cursor left the zone.
    Left,
    /// The mouse reached the catcher window.
    Touch,
    /// The foreground window became (or stopped being) fullscreen on the island's monitor.
    Fullscreen(bool),
    /// The display layout changed (`WM_DISPLAYCHANGE`).
    Display,
}

/// The mouse hook's whole decision (spec §4.1): moves inside the zone of at least one logical px,
/// one `Left` when the cursor goes out. `send` is a non-blocking channel send; when it refuses,
/// nothing is remembered, so the same message goes again at the next mouse event.
pub struct Filter {
    zone: PxRect,
    /// One logical px in physical px.
    min_move: f64,
    last: Option<(i32, i32)>,
    inside: bool,
}

impl Filter {
    pub fn new(zone: PxRect, scale: f64) -> Self {
        Self { zone, min_move: scale, last: None, inside: false }
    }

    pub fn feed(&mut self, x: i32, y: i32, send: impl FnOnce(Msg) -> bool) {
        if in_zone((x, y), self.zone) {
            if let Some((lx, ly)) = self.last {
                if f64::from((x - lx).abs()) < self.min_move && f64::from((y - ly).abs()) < self.min_move {
                    return;
                }
            }
            if send(Msg::Move(x, y)) {
                self.last = Some((x, y));
                self.inside = true;
            }
        } else if self.inside && send(Msg::Left) {
            self.inside = false;
            self.last = None;
        }
    }
}

/// What the event thread emits to the island.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Out {
    /// `strip-cursor`: logical px of the strip window, or `null` when the cursor left.
    Cursor(Option<(f64, f64)>),
    /// `strip-touch`.
    Touch,
    /// `strip-fullscreen {on}`.
    Fullscreen(bool),
    /// `screen-changed`: the page asks for `reposition`, which restarts the strip there.
    ScreenChanged,
}

/// Raw strip messages → island events. A touch (of the stretched line or of the catcher) is sent
/// once per approach, again only after the cursor left the zone. While an app is fullscreen the
/// strip is invisible, so it neither draws nor wakes the island (the hook stays, spec §4.3).
pub struct Translator {
    geo: StripGeometry,
    fullscreen: bool,
    touched: bool,
}

impl Translator {
    pub fn new(geo: StripGeometry) -> Self {
        Self { geo, fullscreen: false, touched: false }
    }

    pub fn on(&mut self, msg: Msg) -> Vec<Out> {
        match msg {
            Msg::Move(x, y) => {
                if self.fullscreen {
                    return Vec::new();
                }
                let p = self.logical(x, y);
                let mut out = vec![Out::Cursor(Some(p))];
                if touches(p, line_at(p), STRIP_TOUCH_MARGIN) {
                    out.extend(self.touch());
                }
                out
            }
            Msg::Left => {
                self.touched = false;
                if self.fullscreen { Vec::new() } else { vec![Out::Cursor(None)] }
            }
            Msg::Touch => {
                if self.fullscreen {
                    return Vec::new();
                }
                self.touch().into_iter().collect()
            }
            Msg::Fullscreen(on) => {
                self.fullscreen = on;
                if on { vec![Out::Fullscreen(true), Out::Cursor(None)] } else { vec![Out::Fullscreen(false)] }
            }
            Msg::Display => vec![Out::ScreenChanged],
        }
    }

    fn touch(&mut self) -> Option<Out> {
        if self.touched {
            return None;
        }
        self.touched = true;
        Some(Out::Touch)
    }

    fn logical(&self, x: i32, y: i32) -> (f64, f64) {
        let z = self.geo.zone;
        (f64::from(x - z.left) / self.geo.scale, f64::from(y - z.top) / self.geo.scale)
    }
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak strip:: 2>&1 | grep -E "^test |test result|warning"` and `cargo test -p mayak island::tests 2>&1 | grep -E "test result"`
Expected: 18 `strip::tests` `ok`, `test result: ok. 18 passed`; `island::tests` 1 passed; no warning. (If rustc reports `dead_code` for `STRIP_ZONE_W`/`STRIP_ZONE_H` in `island.rs`, add `#[allow(dead_code)] // used from Task 3` above each and remove both in Task 3.) Then `cargo test --workspace 2>&1 | grep -E "^test result|warning"`: lib **381 passed** (1 ignored), hook 32 passed.

- [ ] **Step 6: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/strip.rs windows/src-tauri/src/island.rs windows/src-tauri/src/lib.rs && git commit -m "feat(strip): geometry, stretch, touch and fullscreen rules of the island strip"
```

---

### Task 2: The strip thread (hook, catcher window, fullscreen WinEvents, events)

**Files:**
- Modify: `windows/src-tauri/src/strip.rs` (Win32 part between the pure part and `#[cfg(test)]`; one test)
- Modify: `windows/src-tauri/Cargo.toml` (+ `windows/Cargo.lock` if it changes)

**Interfaces:**
- Consumes: Task 1 (`StripGeometry`, `PxRect`, `Filter`, `Msg`, `Translator`, `Out`, `is_fullscreen`); `island::{CursorPayload, WINDOW_LABEL}`; `crate::log::line`.
- Produces:
  `pub struct Strip` with `pub fn stop(self)` (posts `WM_QUIT`, joins both threads)
  `pub fn start(geo: StripGeometry, sink: impl FnMut(Msg) + Send + 'static) -> std::io::Result<Strip>` — returns once the hooks and the catcher exist; the first message is always `Msg::Fullscreen(current)`.
  `pub fn enter(app: AppHandle, geo: StripGeometry)` — stops a running strip, starts one whose events go to the island (`strip-cursor`, `strip-touch`, `strip-fullscreen`, `screen-changed`).
  `pub fn leave()` — stops the running strip, if any.

What only the live run verifies (Task 6): the `WH_MOUSE_LL` callback on real movement, the catcher under a real cursor, the WinEvent callbacks. The test below covers start, the catcher's window procedure, the first fullscreen message and a clean stop, twice.

- [ ] **Step 1: Add the features and the failing lifecycle test**

`Cargo.toml`: add to the `windows` feature list, keeping alphabetical order: `"Win32_Graphics_Gdi",` after `"Win32_Foundation",` (`WNDCLASSW`/`RegisterClassW` are gated on it), `"Win32_System_LibraryLoader",` after `"Win32_System_IO",` (`GetModuleHandleW`), `"Win32_UI_Accessibility",` before `"Win32_UI_Input_KeyboardAndMouse",` (`SetWinEventHook`).

Append inside `mod tests` of `strip.rs`:

```rust
    #[test]
    fn the_strip_thread_reports_its_catcher_and_ends_cleanly_every_time() {
        use std::sync::mpsc::{channel, RecvTimeoutError};
        use std::time::Duration;
        use ::windows::Win32::UI::WindowsAndMessaging::SendMessageW;

        // Far outside every screen: no real cursor reaches the zone or the catcher, and no
        // foreground window can cover this «monitor».
        let geo = geometry(PxRect { left: -32000, top: -32000, right: -30080, bottom: -30920 }, 1.0);
        for _ in 0..2 {
            let (tx, rx) = channel();
            let strip = start(geo, move |m| {
                let _ = tx.send(m);
            })
            .expect("the strip thread starts");
            let wait = || rx.recv_timeout(Duration::from_secs(2));
            assert_eq!(wait(), Ok(Msg::Fullscreen(false)));
            unsafe { SendMessageW(strip.catcher(), WM_MOUSEMOVE, None, None) };
            assert_eq!(wait(), Ok(Msg::Touch));
            unsafe { SendMessageW(strip.catcher(), WM_DISPLAYCHANGE, None, None) };
            assert_eq!(wait(), Ok(Msg::Display));
            strip.stop();
            // both threads are gone: the sink (and its sender) was dropped
            assert_eq!(wait(), Err(RecvTimeoutError::Disconnected));
        }
    }
```

- [ ] **Step 2: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak strip::tests::the_strip_thread 2>&1 | tail -12`
Expected: compile errors ``cannot find function `start` `` and ``cannot find value `WM_MOUSEMOVE` `` / `WM_DISPLAYCHANGE`.

- [ ] **Step 3: Implement the Win32 part**

Insert in `strip.rs` after the `impl Translator` block (before `#[cfg(test)]`), and extend the top `use` line to `use crate::island::{top_centre, CursorPayload, STRIP_ZONE_H, STRIP_ZONE_W, WINDOW_LABEL};`. If rustc reports a signature difference in windows 0.61.3 (for example `GetModuleHandleW(None)`), fix it the way `platform/windows.rs` and `focus/win.rs` do (`PCWSTR::null()`, `HWND(raw as *mut _)`).

```rust
// ── Win32: the strip thread ───────────────────────────────────────────────────
//
// One thread per hidden period, with its own message loop (spec §4.1). It owns the WH_MOUSE_LL
// hook, the catcher window and both WinEvent hooks: all their callbacks run on it, inside
// GetMessageW, so their shared state is a thread-local, never a lock. A second thread turns the
// bounded channel into island events; it is the only one that calls Tauri.

use std::cell::RefCell;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Mutex, Once};
use std::thread::JoinHandle;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use ::windows::core::{w, PCWSTR};
use ::windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use ::windows::Win32::System::LibraryLoader::GetModuleHandleW;
use ::windows::Win32::System::Threading::GetCurrentThreadId;
use ::windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use ::windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClassNameW,
    GetForegroundWindow, GetMessageW, GetWindowRect, LoadCursorW, PostThreadMessageW, RegisterClassW,
    SetLayeredWindowAttributes, SetWindowsHookExW, ShowWindow, TranslateMessage, UnhookWindowsHookEx,
    CHILDID_SELF, EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND, IDC_ARROW, LWA_ALPHA, MA_NOACTIVATE,
    MSG, MSLLHOOKSTRUCT, OBJID_WINDOW, SW_HIDE, SW_SHOWNOACTIVATE, WH_MOUSE_LL, WINEVENT_OUTOFCONTEXT,
    WM_DISPLAYCHANGE, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_QUIT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

/// Room for a burst of mouse moves; the channel is allocated once, so `try_send` never allocates.
const CHANNEL_CAP: usize = 64;
const CLASS_NAME: PCWSTR = w!("MayakStripCatcher");
static CLASS: Once = Once::new();

/// What the hooks and the catcher of this thread share.
struct Ctx {
    tx: SyncSender<Msg>,
    filter: Filter,
    catcher: HWND,
    monitor: PxRect,
    /// The window of the last EVENT_SYSTEM_FOREGROUND: LOCATIONCHANGE of any other is dropped.
    foreground: HWND,
    fullscreen: bool,
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

impl Ctx {
    fn recheck_fullscreen(&mut self) {
        let Some(on) = foreground_is_fullscreen(self.foreground, self.monitor) else { return };
        // A refused send changes nothing, so the next WinEvent tries again.
        if on == self.fullscreen || self.tx.try_send(Msg::Fullscreen(on)).is_err() {
            return;
        }
        self.fullscreen = on;
        // The catcher hides with the line; the mouse hook stays (spec §4.3).
        unsafe {
            let _ = ShowWindow(self.catcher, if on { SW_HIDE } else { SW_SHOWNOACTIVATE });
        }
    }
}

fn foreground_is_fullscreen(hwnd: HWND, monitor: PxRect) -> Option<bool> {
    if hwnd.is_invalid() {
        return None;
    }
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r) }.ok()?;
    let mut name = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut name) };
    let class = String::from_utf16_lossy(&name[..len.max(0) as usize]);
    Some(is_fullscreen(PxRect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }, monitor, &class))
}

/// The WH_MOUSE_LL handler: one compare in `Filter::feed`, one non-blocking send, then the next
/// hook. No Tauri, no lock, no allocation: Windows drops a hook that is slow to answer.
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let pt = unsafe { (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt };
        CTX.with(|cell| {
            let Ok(mut guard) = cell.try_borrow_mut() else { return };
            let Some(Ctx { tx, filter, .. }) = guard.as_mut() else { return };
            filter.feed(pt.x, pt.y, |msg| tx.try_send(msg).is_ok());
        });
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// EVENT_SYSTEM_FOREGROUND and EVENT_OBJECT_LOCATIONCHANGE, out of context (spec §4.3).
unsafe extern "system" fn win_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    CTX.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else { return };
        let Some(ctx) = guard.as_mut() else { return };
        if event == EVENT_SYSTEM_FOREGROUND {
            ctx.foreground = hwnd;
        } else if hwnd != ctx.foreground || id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 {
            return;
        }
        ctx.recheck_fullscreen();
    });
}

/// The catcher (spec §4.2): any mouse over it is a touch, even when Windows has dropped the hook.
/// Being a top-level window it also hears WM_DISPLAYCHANGE while the cursor poll is parked.
unsafe extern "system" fn catcher_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => send_from_window(Msg::Touch),
        WM_DISPLAYCHANGE => send_from_window(Msg::Display),
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Also called while the context is borrowed (ShowWindow inside a WinEvent): then it does
/// nothing, and the next WM_MOUSEMOVE sends the touch.
fn send_from_window(msg: Msg) {
    CTX.with(|cell| {
        let Ok(guard) = cell.try_borrow() else { return };
        if let Some(ctx) = guard.as_ref() {
            let _ = ctx.tx.try_send(msg);
        }
    });
}

fn register_class(instance: HINSTANCE) {
    CLASS.call_once(|| {
        let class = WNDCLASSW {
            lpfnWndProc: Some(catcher_proc),
            hInstance: instance,
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            crate::log::line(format!("strip: RegisterClassW failed: {}", ::windows::core::Error::from_win32()));
        }
    });
}

/// Borderless, topmost, never activated, alpha 1: invisible but hit-testable (spec §4.2).
fn create_catcher(instance: HINSTANCE, r: PxRect) -> ::windows::core::Result<HWND> {
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED,
            CLASS_NAME,
            w!(""),
            WS_POPUP,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            None,
            None,
            Some(instance),
            None,
        )
    }?;
    if let Err(err) = unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA) } {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(err);
    }
    Ok(hwnd)
}

type Ready = Result<(u32, isize), String>;

fn run(geo: StripGeometry, tx: SyncSender<Msg>, ready: SyncSender<Ready>) {
    let instance: HINSTANCE = match unsafe { GetModuleHandleW(None) } {
        Ok(module) => module.into(),
        Err(err) => {
            let _ = ready.send(Err(format!("GetModuleHandleW: {err}")));
            return;
        }
    };
    register_class(instance);
    let catcher = match create_catcher(instance, geo.catcher) {
        Ok(hwnd) => hwnd,
        Err(err) => {
            let _ = ready.send(Err(format!("catcher window: {err}")));
            return;
        }
    };
    let foreground = unsafe { GetForegroundWindow() };
    let fullscreen = foreground_is_fullscreen(foreground, geo.monitor).unwrap_or(false);
    // Always the first message: the island may be hiding over a fullscreen app, and a page that
    // still thinks «fullscreen» from the last hide must learn it is not. The channel is empty
    // here, so the send fails only when the event thread is already gone.
    let _ = tx.try_send(Msg::Fullscreen(fullscreen));
    if !fullscreen {
        unsafe {
            let _ = ShowWindow(catcher, SW_SHOWNOACTIVATE);
        }
    }
    CTX.with(|cell| {
        *cell.borrow_mut() =
            Some(Ctx { tx, filter: Filter::new(geo.zone, geo.scale), catcher, monitor: geo.monitor, foreground, fullscreen });
    });
    // Installed again at every hide (spec §4.1); the catcher alone still wakes the island.
    let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(instance), 0) };
    if let Err(err) = &mouse {
        crate::log::line(format!("strip: no mouse hook, only the catcher wakes the island: {err}"));
    }
    let events = [EVENT_SYSTEM_FOREGROUND, EVENT_OBJECT_LOCATIONCHANGE]
        .map(|event| unsafe { SetWinEventHook(event, event, None, Some(win_event), 0, 0, WINEVENT_OUTOFCONTEXT) });
    if events.iter().any(|h| h.is_invalid()) {
        crate::log::line("strip: a fullscreen WinEvent hook was refused");
    }
    let _ = ready.send(Ok((unsafe { GetCurrentThreadId() }, catcher.0 as isize)));

    let mut msg = MSG::default();
    // 0 on WM_QUIT, -1 on error: both end the strip.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    if let Ok(hook) = mouse {
        if let Err(err) = unsafe { UnhookWindowsHookEx(hook) } {
            crate::log::line(format!("strip: UnhookWindowsHookEx: {err}"));
        }
    }
    for hook in events {
        if !hook.is_invalid() {
            unsafe {
                let _ = UnhookWinEvent(hook);
            }
        }
    }
    if let Err(err) = unsafe { DestroyWindow(catcher) } {
        crate::log::line(format!("strip: DestroyWindow: {err}"));
    }
    // Drops the sender: the event thread ends.
    CTX.with(|cell| *cell.borrow_mut() = None);
}

/// A running strip: its thread (for WM_QUIT) and both join handles.
pub struct Strip {
    thread_id: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    catcher: isize,
    strip: JoinHandle<()>,
    events: JoinHandle<()>,
}

impl Strip {
    /// Ends the strip: unhooks, destroys the catcher, ends the event thread, and waits for both.
    /// When WM_QUIT cannot be posted the thread is left running rather than hanging the caller.
    pub fn stop(self) {
        if let Err(err) = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) } {
            crate::log::line(format!("strip: cannot stop the strip thread: {err}"));
            return;
        }
        if self.strip.join().is_err() {
            crate::log::line("strip: the strip thread panicked");
        }
        if self.events.join().is_err() {
            crate::log::line("strip: the strip event thread panicked");
        }
    }

    #[cfg(test)]
    fn catcher(&self) -> HWND {
        HWND(self.catcher as *mut _)
    }
}

/// Starts the strip for `geo`; every message goes to `sink` on the event thread. Returns once
/// the catcher and the hooks exist (and WM_QUIT can be posted).
pub fn start(geo: StripGeometry, mut sink: impl FnMut(Msg) + Send + 'static) -> std::io::Result<Strip> {
    let (tx, rx) = sync_channel::<Msg>(CHANNEL_CAP);
    let events = std::thread::Builder::new().name("strip-events".into()).spawn(move || {
        for msg in rx {
            sink(msg);
        }
    })?;
    let (ready_tx, ready_rx) = sync_channel::<Ready>(1);
    let strip = std::thread::Builder::new().name("strip".into()).spawn(move || run(geo, tx, ready_tx))?;
    match ready_rx.recv() {
        Ok(Ok((thread_id, catcher))) => Ok(Strip { thread_id, catcher, strip, events }),
        Ok(Err(why)) => {
            let _ = strip.join();
            let _ = events.join();
            Err(std::io::Error::other(why))
        }
        Err(_) => {
            let _ = strip.join();
            let _ = events.join();
            Err(std::io::Error::other("the strip thread ended before it was ready"))
        }
    }
}

#[derive(Serialize, Clone)]
struct FullscreenPayload {
    on: bool,
}

fn emit(app: &AppHandle, out: Out) {
    let result = match out {
        Out::Cursor(p) => app.emit_to(WINDOW_LABEL, "strip-cursor", p.map(|(x, y)| CursorPayload { x, y })),
        Out::Touch => app.emit_to(WINDOW_LABEL, "strip-touch", ()),
        Out::Fullscreen(on) => app.emit_to(WINDOW_LABEL, "strip-fullscreen", FullscreenPayload { on }),
        Out::ScreenChanged => app.emit_to(WINDOW_LABEL, "screen-changed", ()),
    };
    if let Err(err) = result {
        crate::log::line(format!("strip emit failed: {err}"));
    }
}

static CURRENT: Mutex<Option<Strip>> = Mutex::new(None);

/// Hidden island → the strip runs for `geo` (a running one is stopped first: a new monitor,
/// a new scale or simply a new hide).
pub fn enter(app: AppHandle, geo: StripGeometry) {
    let mut current = CURRENT.lock().unwrap();
    if let Some(old) = current.take() {
        old.stop();
    }
    let mut translator = Translator::new(geo);
    let sink = move |msg: Msg| {
        for out in translator.on(msg) {
            emit(&app, out);
        }
    };
    match start(geo, sink) {
        Ok(strip) => *current = Some(strip),
        Err(err) => crate::log::line(format!("strip: cannot start, only events wake the island: {err}")),
    }
}

/// Visible island → no strip thread, no hooks, no catcher.
pub fn leave() {
    let old = CURRENT.lock().unwrap().take();
    if let Some(old) = old {
        old.stop();
    }
}
```

- [ ] **Step 4: Run the test and the suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak strip:: 2>&1 | grep -E "^test |test result|warning|error"`
Expected: 19 tests `ok` (the lifecycle test finishes in well under a second). Then `cargo test --workspace 2>&1 | grep -E "^test result|warning"`: lib **382 passed** (1 ignored), hook 32 passed, no warning (the module is still under `#[allow(dead_code)]` from Task 1).

- [ ] **Step 5: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/strip.rs windows/src-tauri/Cargo.toml windows/Cargo.lock && git commit -m "feat(strip): mouse hook, touch catcher and fullscreen watch on their own thread"
```

(If `windows/Cargo.lock` did not change, `git add` of that path is harmless; add no other file.)

---

### Task 3: Wiring — the zone window, the #28 fix, `set_collapsed`, and the wake

**Files:**
- Modify: `windows/src-tauri/src/island.rs` (constants, `apply_geometry`, `PollGate::forget_ignore_state`, `spawn_cursor_poll` after the inner loop at line 284, `refresh_click_through` lines 294-298, new `strip_geometry`, tests)
- Modify: `windows/src-tauri/src/lib.rs` (remove the Task 1 `#[allow(dead_code)]`; `BootInfo`; `save_settings`; `set_collapsed`; `reposition`; new `place_island`)
- Modify: `windows/src/core/bridge.ts` (`BootInfo`), `windows/src/main.ts`, `windows/src/island/island.ts`

**Interfaces:**
- Consumes: `strip::{geometry, PxRect, StripGeometry, enter, leave}` (Tasks 1-2).
- Produces:
  `pub const island::NATIVE_STRIP: bool = cfg!(windows)`
  `pub fn island::collapsed_size(native_strip: bool) -> (f64, f64)`
  `pub fn island::parked_ignore(collapsed: bool, native_strip: bool) -> bool`
  `#[cfg(windows)] pub fn island::strip_geometry(app: &AppHandle, pref: &str) -> Option<crate::strip::StripGeometry>`
  `fn lib::place_island(app: &AppHandle, pref: &str, collapsed: bool)`
  `BootInfo.native_strip` (wire `nativeStrip: boolean`)
  `Island.onStripTouch(): void` (TS); event `strip-touch` listened in `main.ts`.

After this task the hidden island on Windows is a 600×150 click-through window, the strip thread runs while hidden, and a touch of the (still undrawn) line wakes it; Task 4 draws it.

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` of `island.rs`:

```rust
    #[test]
    fn the_collapsed_window_is_the_zone_only_where_the_native_strip_runs() {
        assert_eq!(collapsed_size(true), (600.0, 150.0));
        assert_eq!(collapsed_size(false), (240.0, 6.0));
        assert_eq!(NATIVE_STRIP, cfg!(windows));
    }

    #[test]
    fn parked_the_window_passes_clicks_only_as_the_native_strip() {
        // upstream #28: the parked flag follows the state, never the poll's last tick
        assert!(parked_ignore(true, true));
        assert!(!parked_ignore(false, true));
        assert!(!parked_ignore(true, false));
        assert!(!parked_ignore(false, false));
    }
```

- [ ] **Step 2: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak island::tests 2>&1 | tail -8`
Expected: compile errors for `collapsed_size`, `NATIVE_STRIP`, `parked_ignore`.

- [ ] **Step 3: Implement `island.rs`**

After the `STRIP_ZONE_H` constant add:

```rust
/// Where a hidden island is the visible strip of `strip.rs` (spec §8: Windows only). Elsewhere it
/// stays the invisible 240×6 wake strip.
pub const NATIVE_STRIP: bool = cfg!(windows);
```

Above `top_centre` add:

```rust
/// Logical size of the window while the island is hidden.
pub fn collapsed_size(native_strip: bool) -> (f64, f64) {
    if native_strip { (STRIP_ZONE_W, STRIP_ZONE_H) } else { (STRIP_W, STRIP_H) }
}

/// The click-through flag of a parked poll: the visible strip passes every click (its catcher
/// and its hook take the touches, spec §2); anything else takes the mouse until the next tick.
pub fn parked_ignore(collapsed: bool, native_strip: bool) -> bool {
    collapsed && native_strip
}
```

In `apply_geometry` replace `let (lw, lh) = if collapsed { (STRIP_W, STRIP_H) } else { (PANEL_W, PANEL_H) };` with:

```rust
    let (lw, lh) = if collapsed { collapsed_size(NATIVE_STRIP) } else { (PANEL_W, PANEL_H) };
```

Replace the method `forget_ignore_state` (lines 81-84) with:

```rust
    /// Records the flag just written to the window, so the next poll tick only calls into the
    /// OS when the flag has to change.
    pub fn remember_ignore(&self, ignoring: bool) {
        self.ignoring.store(ignoring, Ordering::Relaxed);
    }
```

In `spawn_cursor_poll`, right after the closing brace of `while gate.is_active() { … }` (before the outer `loop`'s closing brace) add:

```rust
            // Upstream #28: the last tick may have left the window click-through for a shape it
            // saw then. Parked, the flag is what the collapsed state needs. set_collapsed writes
            // the same value from the same function, so whichever of the two runs last agrees.
            if platform::CURSOR_POLL {
                let ignore = parked_ignore(gate.collapsed.load(Ordering::Relaxed), NATIVE_STRIP);
                if let Some(win) = window(&app) {
                    let _ = win.set_ignore_cursor_events(ignore);
                }
                gate.remember_ignore(ignore);
            }
```

In `refresh_click_through` replace

```rust
    if platform::CURSOR_POLL {
        set_ignore_cursor(app, false);
        gate.forget_ignore_state();
        return;
    }
```

with

```rust
    if platform::CURSOR_POLL {
        let ignore = parked_ignore(gate.collapsed.load(Ordering::Relaxed), NATIVE_STRIP);
        set_ignore_cursor(app, ignore);
        gate.remember_ignore(ignore);
        return;
    }
```

and update its doc comment's second paragraph to: `/// With the cursor poll (Windows) a visible window takes the mouse again and the next tick decides from the cursor; the hidden one, the strip zone, passes every click. Without it (Linux) the input region is set to the island itself, or to the whole wake strip while collapsed.`

After `apply_geometry` add:

```rust
/// Zone and catcher of the strip on the monitor `apply_geometry` uses for `pref`.
#[cfg(windows)]
pub fn strip_geometry(app: &AppHandle, pref: &str) -> Option<crate::strip::StripGeometry> {
    let m = target_monitor(app, pref)?;
    let p = m.position();
    let s = m.size();
    let monitor = crate::strip::PxRect { left: p.x, top: p.y, right: p.x + s.width as i32, bottom: p.y + s.height as i32 };
    Some(crate::strip::geometry(monitor, m.scale_factor()))
}
```

Remove the `#[allow(dead_code)] // used from Task 3` attributes on `STRIP_ZONE_W`/`STRIP_ZONE_H` if Task 1 added them.

- [ ] **Step 4: Implement `lib.rs`**

Remove the line `#[allow(dead_code)] // wired into set_collapsed in Task 3; remove this attribute there` above `mod strip;`.

In `BootInfo` after `cursor_poll: bool,` add:

```rust
    /// True where a hidden island is the visible strip of `strip.rs` (Windows).
    native_strip: bool,
```

and in `boot` after `cursor_poll: platform::CURSOR_POLL,` add `native_strip: island::NATIVE_STRIP,`.

Above `set_collapsed` add:

```rust
/// Places the window for `collapsed`. On Windows a collapsed island is the visible strip, whose
/// thread is (re)started for this monitor; a visible island has no strip thread.
fn place_island(app: &AppHandle, pref: &str, collapsed: bool) {
    #[cfg(windows)]
    if !collapsed {
        strip::leave();
    }
    island::apply_geometry(app, pref, collapsed);
    #[cfg(windows)]
    if collapsed {
        match island::strip_geometry(app, pref) {
            Some(geo) => strip::enter(app.clone(), geo),
            None => log::line("strip: no monitor to place it on"),
        }
    }
}
```

Replace `set_collapsed` (doc comment and body) with:

```rust
/// Hidden island → the strip zone (Windows: the visible strip, its thread running; elsewhere the
/// invisible wake strip) and the cursor poll parked; anything else → full panel, no strip thread
/// and 60 Hz polling. The observer publishes every change only while the island is visible (spec §9).
#[tauri::command]
fn set_collapsed(app: AppHandle, shared: State<Shared>, observe: State<observe::Handle>, collapsed: bool) {
    let pref = shared.settings.lock().unwrap().screen.clone();
    shared.gate.collapsed.store(collapsed, Ordering::Relaxed);
    place_island(&app, &pref, collapsed);
    // A resize invalidates the click-through flag: the strip zone passes every click, the panel
    // takes the mouse until the next poll tick decides.
    island::refresh_click_through(&app, &shared.gate);
    shared.gate.set_active(!collapsed);
    observe.send(Signal::Visible(!collapsed));
}
```

In `save_settings` replace `island::apply_geometry(&app, &settings.screen, collapsed);` with `place_island(&app, &settings.screen, collapsed);`. In `reposition` replace `island::apply_geometry(&app, &pref, collapsed);` with `place_island(&app, &pref, collapsed);` (a `screen-changed` from the strip's catcher lands here and restarts the strip on the new layout).

- [ ] **Step 5: The wake on the TypeScript side**

`bridge.ts`, in `BootInfo` after `cursorPoll: boolean;` add:

```ts
  /** Windows: a hidden island is the visible strip driven by src-tauri/src/strip.rs. */
  nativeStrip: boolean;
```

`island.ts`, after `followPageCursor() { … }` add:

```ts
  /**
   * strip.rs (Windows): the cursor touched the line, or its catcher window. Same as the old
   * wake strip's mouseenter: hidden → petit.
   */
  onStripTouch() {
    Sound.resume();
    if (State.mode === "hidden") this.fsm.mouseEntered();
  }
```

`main.ts`, after the `cursor` listener add:

```ts
  await onEvent<null>("strip-touch", () => island.onStripTouch());
```

- [ ] **Step 6: Verify**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|warning|error"` and `npx tsc --noEmit && npx vite build 2>&1 | tail -3`
Expected: lib **384 passed** (1 ignored), hook 32 passed, 0 warnings (no `dead_code` left in `strip.rs` — every item is now reachable from `set_collapsed`; `Strip::catcher` carries its own `cfg_attr`); `tsc` and `vite build` exit 0.

- [ ] **Step 7: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/island.rs windows/src-tauri/src/lib.rs windows/src/core/bridge.ts windows/src/main.ts windows/src/island/island.ts && git commit -m "feat(island): hide to the strip zone with its thread, reset click-through after parking (#28)"
```

---

### Task 4: Drawing the strip, its stretch, fullscreen and the quiet look

**Files:**
- Create: `windows/src/island/strip.ts`
- Modify: `windows/src/island/island.ts` (fields, `build`, `setMode`, `updateWindowCollapsed`, `syncDom`, new methods)
- Modify: `windows/src/main.ts` (`setPaused`, listeners, `enableNativeStrip`)
- Modify: `windows/src/core/bridge.ts` (`BridgeEvent`)
- Modify: `windows/src/style.css` (new rules after `#wake-strip`)

**Interfaces:**
- Consumes: events `strip-cursor` (`{x, y} | null`, logical px of the 600×150 window), `strip-fullscreen` (`{on: boolean}`) from Task 2; `BootInfo.nativeStrip` (Task 3).
- Produces: `export const STRIP_ZONE_W = 600` and `export function lineAt(cursor: { x: number; y: number } | null): LineBox` in `island/strip.ts`; `Island.enableNativeStrip()`, `Island.onStripCursor(p)`, `Island.onStripFullscreen(on)`.

There is no front-end test framework; the checks are `npx tsc --noEmit` and `npx vite build`. Do not add a test runner. The stretch formula is pinned by the Rust tests of `line_at` (Task 1); `lineAt` below is the same code line for line.

- [ ] **Step 1: The TS mirror of `line_at`**

`windows/src/island/strip.ts`:

```ts
// Geometry of the visible strip of a hidden island (Windows), shared with
// src-tauri/src/strip.rs: same constants, same formula as `line_at` (spec §3).
// Rust decides the touch from it; the page only draws it. No DOM here.

export const STRIP_ZONE_W = 600;
const STRIP_LINE_W = 120;
const STRIP_LINE_H = 4;
const STRETCH_REACH = 150;
const STRETCH_LEN = 200;
const STRETCH_THICK = 8;
const STRETCH_FOLLOW = 0.5;

/** Logical px of the strip window, origin top-left. */
export interface LineBox { x: number; y: number; w: number; h: number }

const BASE: LineBox = { x: (STRIP_ZONE_W - STRIP_LINE_W) / 2, y: 0, w: STRIP_LINE_W, h: STRIP_LINE_H };

function distance(px: number, py: number, r: LineBox): number {
  const dx = Math.max(r.x - px, 0, px - (r.x + r.w));
  const dy = Math.max(r.y - py, 0, py - (r.y + r.h));
  return Math.hypot(dx, dy);
}

/** The line for a cursor near it, or at rest for null. */
export function lineAt(cursor: { x: number; y: number } | null): LineBox {
  if (!cursor) return BASE;
  const t = Math.min(1, Math.max(0, 1 - distance(cursor.x, cursor.y, BASE) / STRETCH_REACH));
  const len = STRIP_LINE_W + t * STRETCH_LEN;
  const thick = STRIP_LINE_H + t * STRETCH_THICK;
  const centre = STRIP_ZONE_W / 2;
  const pulled = centre + t * STRETCH_FOLLOW * (cursor.x - centre);
  const c = Math.min(STRIP_ZONE_W - len / 2, Math.max(len / 2, pulled));
  return { x: c - len / 2, y: 0, w: len, h: thick };
}
```

- [ ] **Step 2: The line in the island**

`island.ts`: add `import { STRIP_ZONE_W, lineAt } from "./strip";` after `import { IslandStateMachine } from "./fsm";`.

Fields, after `private wakeStrip!: HTMLElement;`:

```ts
  /** Windows: the visible line of a hidden island (strip.rs drives it). */
  private stripLine!: HTMLElement;
  private nativeStrip = false;
  /** strip-fullscreen: an app covers the island's monitor, so the line is not shown. */
  private stripFullscreen = false;
```

In `build()`, after `this.wakeStrip = h("div", { id: "wake-strip" });` add `this.stripLine = h("div", { id: "strip-line", class: "rest" });` and replace `this.root.append(this.wakeStrip, this.islandEl);` with `this.root.append(this.wakeStrip, this.stripLine, this.islandEl);`.

In `setMode`, after `State.mode = mode;` add:

```ts
    // Leaving the strip: it comes back at rest the next time the island hides.
    if (prev === "hidden") this.onStripCursor(null);
```

Replace `updateWindowCollapsed()` with:

```ts
  private updateWindowCollapsed() {
    if (this.collapseTimer != null) {
      window.clearTimeout(this.collapseTimer);
      this.collapseTimer = null;
    }
    if (State.mode === "hidden") {
      // Let the island finish retracting, then drop the window to the strip zone: from there
      // the OS delivers no cursor events, so nothing polls at all.
      this.collapseTimer = window.setTimeout(() => {
        this.collapseTimer = null;
        if (State.mode !== "hidden") return;
        this.collapsed = true;
        this.syncStrip();
        void Bridge.setCollapsed(true);
      }, 420);
    } else if (this.collapsed) {
      // Grow the window back before the island animates open.
      this.collapsed = false;
      this.syncStrip();
      void Bridge.setCollapsed(false);
    }
  }
```

In `syncDom()`, after `this.header.sync();` add `this.syncStrip();`.

After `onStripTouch()` (Task 3) add:

```ts
  /** boot.nativeStrip: the hidden island shows the line. */
  enableNativeStrip() {
    this.nativeStrip = true;
    this.syncStrip();
  }

  /**
   * strip-cursor: the line follows a cursor near it; null puts it back with one 150 ms
   * transition. Only cursor events move it: no frame loop runs for the strip.
   */
  onStripCursor(p: { x: number; y: number } | null) {
    if (!this.nativeStrip) return;
    const box = lineAt(p);
    this.stripLine.classList.toggle("rest", p == null);
    this.stripLine.style.width = `${box.w}px`;
    this.stripLine.style.height = `${box.h}px`;
    this.stripLine.style.left = `calc(50% + ${box.x + box.w / 2 - STRIP_ZONE_W / 2}px)`;
  }

  /** strip-fullscreen: the line hides while an app covers the island's monitor. */
  onStripFullscreen(on: boolean) {
    this.stripFullscreen = on;
    this.syncStrip();
  }

  /** Shown only once the window is the strip zone; dashed and paler while paused (quiet). */
  private syncStrip() {
    const on = this.nativeStrip && State.mode === "hidden" && this.collapsed && !this.stripFullscreen;
    this.stripLine.classList.toggle("on", on);
    this.stripLine.classList.toggle("quiet", State.paused);
  }
```

- [ ] **Step 3: Events and pause in `main.ts`, event names in `bridge.ts`**

`main.ts`: after `if (boot && !boot.cursorPoll) island.followPageCursor();` add `if (boot?.nativeStrip) island.enableNativeStrip();`. After the `strip-touch` listener (Task 3) add:

```ts
  await onEvent<{ x: number; y: number } | null>("strip-cursor", (p) => island.onStripCursor(p));
  await onEvent<{ on: boolean }>("strip-fullscreen", ({ on }) => island.onStripFullscreen(on));
```

In `setPaused`, after `island.syncHold();` add `State.notify();` (a pause from the tray while hidden must redraw the line dashed, and the header's quiet button of Task 5 must follow).

`bridge.ts`, in `BridgeEvent` after `| { name: "cursor"; payload: { x: number; y: number } }` add:

```ts
  | { name: "strip-cursor"; payload: { x: number; y: number } | null }
  | { name: "strip-touch"; payload: null }
  | { name: "strip-fullscreen"; payload: { on: boolean } }
```

- [ ] **Step 4: CSS**

`style.css`, right after the `#wake-strip { … }` rule (the existing rule stays as it is):

```css
/* Windows: a hidden island is a thin line at the top centre (strip.rs moves it). The black
   outline is the island's own backing, so the line reads on light and dark screens alike. */
#strip-line {
  position: fixed;
  top: 0;
  left: 50%;
  width: 120px;
  height: 4px;
  transform: translateX(-50%);
  border-radius: 999px;
  background: #000;
  box-shadow: 0 0 0 2px #000;
  pointer-events: none;
  display: none;
}

#strip-line::after {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background: rgba(255, 255, 255, 0.35);
}

#strip-line.on {
  display: block;
}

/* Back to rest: one short transition, then no frames. */
#strip-line.rest {
  transition: width 0.15s ease-out, height 0.15s ease-out, left 0.15s ease-out;
}

/* Quiet mode (pause): paler and dashed. */
#strip-line.quiet::after {
  background: repeating-linear-gradient(90deg, rgba(255, 255, 255, 0.18) 0 6px, transparent 6px 10px);
}
```

- [ ] **Step 5: Verify**

Run: `cd /c/Users/HP/source/repos/mayak/windows && npx tsc --noEmit && npx vite build 2>&1 | tail -3`
Expected: both exit 0 (`npm ci` first if `node_modules` is missing). `cargo test --workspace` is unchanged (384 + 32): nothing Rust changed.

- [ ] **Step 6: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src && git commit -m "feat(island): draw the strip line, stretch it towards the cursor, hide it over fullscreen apps"
```

---

### Task 5: Collapse button, quiet button, petit-hover collapse, the tray check

**Files:**
- Modify: `windows/src/island/fsm.ts` (`forceCollapse`, `collapsedByButton`, clears, `schedulePetitHide`)
- Modify: `windows/src/views/icons.ts` (three icons), `windows/src/views/views.ts` (`ViewActions`, `buildHeader`)
- Modify: `windows/src/island/island.ts` (actions, `quietHandler`, `#petit-collapse`), `windows/src/main.ts` (`quietHandler`), `windows/src/style.css` (`#petit-collapse`)
- Modify: `windows/src-tauri/src/tray.rs` (Pause check item), `windows/src-tauri/src/lib.rs` (`set_paused`)

**Interfaces:**
- Consumes: `setPaused` in `main.ts` (with the `State.notify()` of Task 4).
- Produces: `IslandStateMachine.forceCollapse(): void`, `IslandStateMachine.collapsedByButton: boolean`; `ViewActions.forceCollapse(): void`, `ViewActions.toggleQuiet(): void`; `Island.quietHandler: (() => void) | null`; `ICONS.bell`, `ICONS.bellSlash`, `ICONS.chevronUp`; `pub struct tray::PauseItem`, `pub fn tray::show_paused(app: &AppHandle, paused: bool)`; `set_paused(app, paused)` (wire unchanged: `{ paused }`).

The FSM has no test runner; the state rules are checked by `tsc` here and by acceptance items 3 and 5 in Task 6. The tray check mark is verified live only. Note on the flag (spec §5, §6): in this FSM nothing re-opens a hidden island from `holdVisible`, and `forceCollapse` goes petit → hidden directly, so a button-collapsed island already ignores the hold; `collapsedByButton` makes that explicit in the one place the hold is read, and every way out of hidden clears it.

- [ ] **Step 1: FSM**

`fsm.ts`, after `holdVisible = false;` add:

```ts
  /**
   * The collapse button hid a compact island (spec §5): holdVisible does not hold it until the
   * next event (alert, reveal) or until the user opens the island.
   */
  collapsedByButton = false;
```

Add `this.collapsedByButton = false;` as the first line of `launch()`, of `click()` (after its `if` guard), of `reveal()` (after its `if` guard), of `forceHome()` and of `forcePetit()`, and in `mouseEntered()` inside `case "hidden":` before `this.cancelTimers();`.

After `forceHidden() { … }` add:

```ts
  /** Collapse button (spec §5): home → petit, petit → hidden; elsewhere nothing. */
  forceCollapse() {
    if (this.state === "home") {
      this.cancelTimers();
      this.transition("petit");
    } else if (this.state === "petit") {
      this.cancelTimers();
      this.collapsedByButton = true;
      this.transition("hidden");
    }
  }
```

In `schedulePetitHide`, replace `if (this.holdVisible) return;` with `if (this.holdVisible && !this.collapsedByButton) return;`.

- [ ] **Step 2: Icons**

`icons.ts`, before `} as const;` add:

```ts
  // bell.fill (quiet mode off)
  bell: "M12 2.8a1.5 1.5 0 0 0-1.5 1.5v.6A6 6 0 0 0 6 10.8v4.4L4.2 17.4v1.1h15.6v-1.1L18 15.2v-4.4a6 6 0 0 0-4.5-5.9v-.6A1.5 1.5 0 0 0 12 2.8zm-2.2 16.9a2.2 2.2 0 0 0 4.4 0h-4.4z",
  // bell.slash.fill (quiet mode on)
  bellSlash: "M12 2.8a1.5 1.5 0 0 0-1.5 1.5v.6A6 6 0 0 0 6 10.8v4.4L4.2 17.4v1.1h15.6v-1.1L18 15.2v-4.4a6 6 0 0 0-4.5-5.9v-.6A1.5 1.5 0 0 0 12 2.8zm-2.2 16.9a2.2 2.2 0 0 0 4.4 0h-4.4zM3.7 2.3 2.3 3.7l18 18 1.4-1.4-18-18z",
  // chevron.up (collapse)
  chevronUp: "M5.5 15 12 8.5 18.5 15",
```

- [ ] **Step 3: Header buttons**

`views.ts`, in `ViewActions` after `collapse(): void;` add:

```ts
  /** Collapse button: home → petit, petit → strip (fsm.forceCollapse). */
  forceCollapse(): void;
  /** Quiet button: the tray's Pause, without hiding the island. */
  toggleQuiet(): void;
```

In `buildHeader`, after the `soundBtn` line add:

```ts
  const quietBtn = h("button", { title: "Quiet mode", onclick: () => actions.toggleQuiet() }, svg(ICONS.bell, 14));
  const collapseBtn = h(
    "button",
    { title: "Collapse", onclick: () => actions.forceCollapse() },
    svg(ICONS.chevronUp, 12, { stroke: 2.4 }),
  );
```

replace `h("div", { class: "header-actions" }, gearBtn, soundBtn),` with `h("div", { class: "header-actions" }, gearBtn, soundBtn, quietBtn, collapseBtn),` and in `sync()` after the two `soundBtn` lines add:

```ts
      clear(quietBtn);
      quietBtn.append(svg(State.paused ? ICONS.bellSlash : ICONS.bell, 14));
      quietBtn.title = State.paused ? "Quiet mode is on" : "Quiet mode";
```

- [ ] **Step 4: Island actions and the petit-hover button**

`island.ts`: replace `import { h } from "../views/dom";` with `import { h, svg } from "../views/dom";` and add `import { ICONS } from "../views/icons";` after it.

Fields, after `private stripFullscreen = false;`:

```ts
  /** Compact island: hide to the strip, shown while the cursor is on the island. */
  private petitCollapse!: HTMLElement;
  /** main.ts hands the tray's setPaused here: the quiet button is that same pause (spec §5). */
  quietHandler: (() => void) | null = null;
```

In the `actions` object of `build()`, after `collapse: () => this.collapse(),` add:

```ts
      forceCollapse: () => {
        State.isPinned = false;
        this.fsm.pinned = false;
        this.fsm.forceCollapse();
      },
      toggleQuiet: () => this.quietHandler?.(),
```

After `this.countdown = h("div", { id: "countdown" });` add:

```ts
    this.petitCollapse = h(
      "button",
      {
        id: "petit-collapse",
        title: "Hide",
        onmousedown: (e: Event) => {
          // The compact island opens on mousedown: this press must not reach it.
          e.stopPropagation();
          Sound.resume();
          this.fsm.forceCollapse();
        },
      },
      svg(ICONS.chevronUp, 9, { stroke: 2.4 }),
    );
```

and add `this.petitCollapse,` after `this.countdown,` in the `h("div", { id: "island" }, …)` children.

In `onCursor`, after `this.wasInIsland = inIsland;` add `this.petitCollapse.classList.toggle("shown", inIsland);`.

In `syncDom`, after `const compact = State.mode === "compact";` add `this.petitCollapse.style.display = compact ? "grid" : "none";`.

`main.ts`, right after the `setPaused` function add:

```ts
  // The island's quiet button is the tray's Pause, except that it leaves the island open (spec §5).
  island.quietHandler = () => {
    setPaused(!State.paused);
    Sound.play("blip");
  };
```

`style.css`, after the `#counters` rule:

```css
/* Compact island: hide it to the strip. Shown while the cursor is on the island. */
#petit-collapse {
  position: absolute;
  top: 8px;
  left: 72px;
  width: 16px;
  height: 16px;
  border: 0;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.07);
  color: var(--dim-2);
  display: none;
  place-items: center;
  cursor: default;
  opacity: 0;
  transition: opacity 0.15s ease-out;
}

#petit-collapse.shown {
  opacity: 1;
}
```

- [ ] **Step 5: The tray shows the same state**

`tray.rs`: replace the imports with

```rust
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};
```

add above `pub fn build`:

```rust
/// The tray's Pause, kept so the island's quiet button and the tray always show one state (spec §5).
pub struct PauseItem(CheckMenuItem<Wry>);

/// Called with every pause change, whichever side made it (`set_paused`).
pub fn show_paused(app: &AppHandle, paused: bool) {
    let Some(item) = app.try_state::<PauseItem>() else { return };
    if let Err(err) = item.0.set_checked(paused) {
        crate::log::line(format!("tray pause check: {err}"));
    }
}
```

replace `let pause = MenuItem::with_id(app, "pause", "Pause", true, None::<&str>)?;` with

```rust
    let pause = CheckMenuItem::with_id(app, "pause", "Pause", true, false, None::<&str>)?;
```

and before `let mut builder = …` add `app.manage(PauseItem(pause.clone()));`.

`lib.rs`: replace `set_paused` with

```rust
/// Tray → Pause and the island's quiet button. Paused means paused: the pollers stop talking to
/// the network, not just the island stopping showing things. The tray item shows the same state.
#[tauri::command]
fn set_paused(app: AppHandle, paused: bool) {
    integrations::set_paused(paused);
    tray::show_paused(&app, paused);
}
```

- [ ] **Step 6: Verify**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|warning|error"` and `npx tsc --noEmit && npx vite build 2>&1 | tail -3`
Expected: lib 384 passed (1 ignored), hook 32 passed, 0 warnings; `tsc` and `vite build` exit 0.

- [ ] **Step 7: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src windows/src-tauri/src/tray.rs windows/src-tauri/src/lib.rs && git commit -m "feat(island): collapse and quiet buttons, hide the compact island to the strip"
```

---

### Task 6: Live acceptance, CPU and documentation

**Files:**
- Modify: `docs/superpowers/specs/2026-10-06-mayak-island-strip-design.md`
- Modify: `windows/README.md` (the «Using it» table)
- Modify: `.git/mayak-pr-notes.md` (local, not tracked)

**Interfaces:** consumes the branch after Tasks 1-5; produces the filled §9 acceptance results, the aligned spec and the PR text.

This task is run **by the controller with the user** (real mouse, fullscreen apps, the tray). Only Steps 1-2 and 6 are implementer work.

- [ ] **Step 1: Align the spec with what was built**

In the spec (Russian, keep its line endings): §4.1 — the hook's state is a thread-local of the strip thread (its callbacks run on it), not an atomic; a monitor change while hidden arrives as `WM_DISPLAYCHANGE` at the catcher → `screen-changed` → `reposition`, which restarts the strip; every strip start first sends `strip-fullscreen` with the current state. §4.3 — while fullscreen the hook stays but neither `strip-cursor` nor `strip-touch` reaches the island. §4.4 — after parking and in `set_collapsed` the flag is `parked_ignore(collapsed, NATIVE_STRIP)` (both sides write the same value, no race). §5 — the tray Pause is a check item kept in step by `set_paused`; `collapsedByButton` guards the only hold check. §6 — `strip_enter/strip_leave` are `strip::enter/leave` (Rust functions, not commands); `windows` features added: `Win32_Graphics_Gdi`, `Win32_System_LibraryLoader`, `Win32_UI_Accessibility`. §7 risks — a maximised window with an auto-hidden taskbar overhangs the monitor and also hides the strip; a scale change while hidden is picked up at the next wake. Commit:

```bash
cd /c/Users/HP/source/repos/mayak && git add docs/superpowers/specs/2026-10-06-mayak-island-strip-design.md && git commit -m "docs: align the island strip spec with the implementation"
```

- [ ] **Step 2: README rows**

In `windows/README.md`, «Using it» table, after the row `| Move the mouse to the very top-centre of the screen | Mochi peeks out |` add:

```markdown
| Bring the mouse near the thin line at the top centre | It stretches towards the pointer; touch it and Mochi peeks out |
| Click ⌃ in the island header, or on the small island | It folds away, down to the line |
| Click the bell in the island header | Quiet mode — the same as the tray's Pause, but the island stays open |
```

Commit: `git add windows/README.md && git commit -m "docs: the island strip, collapse and quiet buttons in the README"`.

- [ ] **Step 3: Build and start the dev app outside the Claude package**

Use the pattern of the earlier sub-projects (WMI `Win32_Process.Create`, so the app runs outside the session's virtualized `%LOCALAPPDATA%`):

```powershell
$cl = 'cmd.exe /d /c "set CARGO_TARGET_DIR=C:\tmp\mayak-target&& set WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222&& set PATH=C:\Users\HP\.cargo\bin;%PATH%&& cd /d C:\Users\HP\source\repos\mayak\windows&& npm run tauri dev > C:\tmp\mayak-spike\dev4.log 2>&1"'
Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = $cl }
```

Expected: `mayak.exe` from `C:\tmp\mayak-target\debug` runs; `dev4.log` ends with `Running ... mayak.exe`. If a release `mayak.exe` the user runs is up, ask the user to quit it from the tray first (single instance); never stop it yourself.

- [ ] **Step 4: Run spec §9 acceptance items 1-5 with the user**

Record each as PASS/FAIL with the evidence (what the user saw, `mayak.log` lines starting `strip:`) in `.git/mayak-pr-notes.md`:
1. Hidden island shows the line; approaching from below at the centre stretches it; a touch opens `petit` the first time, 10 attempts in a row.
2. Clicks at the top edge outside the line reach what is under the zone: browser tabs under the 600×150 zone click normally.
3. Collapse button: `home` → `petit` → strip, also while an agent waits for an answer; the next agent event opens the island.
4. A fullscreen video (or F11 in a browser) hides the line, and touching the top edge there does not open the island; leaving fullscreen brings the line back; an agent event in fullscreen opens the island.
5. Quiet button: the line turns pale and dashed when hidden, an event neither opens the island nor sounds, the tray's Pause shows the check; turning it off with the button brings notifications back and clears the check.
Also check: unplug or rearrange a monitor while hidden → the line moves to the new top centre.

- [ ] **Step 5: CPU (spec §9 item 6)**

With the island hidden and the mouse still: `powershell -File C:\tmp\mayak-accept\sample-cpu.ps1 -Seconds 60 -OutPath C:\tmp\mayak-accept\cpu-strip-idle.log` — expected `tree_avg_percent_of_machine` < 0.100. Then the same with the user moving the mouse continuously over the zone: `-OutPath C:\tmp\mayak-accept\cpu-strip-move.log` — record the number (a visible but bounded rise is expected). Record both RESULT lines in `.git/mayak-pr-notes.md`.

- [ ] **Step 6: Full verification and clean-up**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|warning"` and `npx tsc --noEmit && npx vite build 2>&1 | tail -3`.
Expected: lib 384 passed (1 ignored), hook 32 passed, 0 warnings, build OK. Stop only the `tauri dev` tree Step 3 started.

- [ ] **Step 7: Ask before publishing**

Ask the user, verbatim: «Запушить ветку `feature/island-strip` в `origin` и открыть PR в `main`? Проверки на GitHub отключены, проверка локальная.» Only on an explicit yes:

```bash
git push -u origin feature/island-strip
gh pr create --repo mahach666/mayak --base main --head feature/island-strip --title "Mayak: visible island strip, collapse and quiet buttons" --body-file "$(git rev-parse --git-dir)/mayak-pr4-body.md"
```

(`mayak-pr4-body.md` is written from the §9 results of Steps 4-5; nothing in braces may remain.)

---

## Self-Review

- **Spec coverage:** §1 goals → Tasks 3-5. §2 states, zone/line sizes, colours, quiet look, transitions → Task 3 (`collapsed_size`, zone window), Task 4 (CSS, `syncStrip`), Task 5 (hold unchanged except the button flag). §3 zone, distance, stretch, follow, touch, return transition, pure functions → Task 1 (`in_zone`, `stretch`, `line_at`, `touches`), Task 4 (`lineAt`, `.rest`). §4.1 thread, hook, sender, reinstall per hide → Task 2 (`run`, `mouse_proc`, `start`, `enter`), Task 3 (`place_island`). §4.2 catcher → Task 2 (`create_catcher`, `catcher_proc`), Task 1 (`geometry`). §4.3 fullscreen → Task 1 (`is_fullscreen`, `Translator`), Task 2 (`win_event`, `recheck_fullscreen`), Task 4 (`onStripFullscreen`). §4.4 #28 → Task 3. §5 buttons → Task 5. §6 files → all tasks (Cargo features in Task 2). §7 CPU and risks → Global Constraints, Task 2 design, Task 6 Steps 1 and 5. §8 other platforms → `NATIVE_STRIP`, `collapsed_size(false)`, `cfg(windows)` on `mod strip` and `place_island`. §9 tests and acceptance → Tasks 1-3 and Task 6.
- **Placeholder scan:** none. Every code step carries its code; the only conditional instructions (a windows-crate signature difference, `dead_code` on two constants) name the exact fix.
- **Type consistency:** `PxRect`, `Rect`, `StripGeometry`, `geometry`, `Msg`, `Filter::feed`, `Translator::on`, `Out` (Task 1) are used unchanged in Task 2; `strip::enter(AppHandle, StripGeometry)`/`strip::leave()` (Task 2) match `place_island` (Task 3); `island::strip_geometry` returns `Option<StripGeometry>` built by `strip::geometry`; `top_centre` (Task 1) is the placement of both `apply_geometry` and `geometry`. Event names and payloads match between `emit` (Task 2) and `main.ts` (Tasks 3-4): `strip-cursor` `{x,y}|null`, `strip-touch` `null`, `strip-fullscreen` `{on}`, `screen-changed`. `BootInfo.native_strip` ↔ `nativeStrip`. `ViewActions.forceCollapse/toggleQuiet` (Task 5) are implemented in `island.ts` the same task. `STRIP_ZONE_W` is 600 in both `island.rs` and `strip.ts`.
- **Review Focus:** 1 → Task 1 geometry tests (×3); 2 → Task 1 `a_message_the_channel_refused_is_sent_again_at_the_next_event`; 3 → Task 1 `nothing_reaches_the_island_while_an_app_is_fullscreen` and `a_cursor_on_the_line_wakes_the_island_once_until_it_leaves`; 4 → Task 3 `parked_the_window_passes_clicks_only_as_the_native_strip`; 5 → Task 2 `the_strip_thread_reports_its_catcher_and_ends_cleanly_every_time`.
- **Test counts:** baseline 362 + Task 1 (18 strip + 1 island) = 381; Task 2 +1 = 382; Task 3 +2 = 384; Tasks 4-5 add no Rust test. Hook stays 32.
