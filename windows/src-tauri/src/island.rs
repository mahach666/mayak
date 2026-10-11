// Island window: placement on the chosen display, the two window sizes
// (full panel / invisible wake strip), click-through and the cursor poll.
//
// There is no notch on a PC, so the island is a black shape drawn at the top
// centre of the main display inside a borderless, transparent, always-on-top
// window that never takes focus.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::platform::{self, cursor_physical, left_button_down};

/// Logical size of the full window — the largest island view, like the macOS panel. 560 tall
/// for the question card (answers spec §6); every other view is at most 320.
pub const PANEL_W: f64 = 720.0;
pub const PANEL_H: f64 = 560.0;
/// How far down the window a held button takes the mouse (a file drag must find the island):
/// the 320 px of the panel before the question card grew it. Below that, only the island does.
pub const DRAG_ZONE_H: f64 = 320.0;
/// Logical size of the invisible strip that wakes the island when it is hidden.
pub const STRIP_W: f64 = 240.0;
pub const STRIP_H: f64 = 6.0;
/// Logical size of the window while a Windows island is hidden: the zone in which the visible
/// strip reacts to the cursor (spec §2). The webview passes every click there; `strip.rs` decides.
pub const STRIP_ZONE_W: f64 = 600.0;
pub const STRIP_ZONE_H: f64 = 150.0;
/// Where a hidden island is the visible strip of `strip.rs` (spec §8: Windows only). Elsewhere it
/// stays the invisible 240×6 wake strip.
pub const NATIVE_STRIP: bool = cfg!(windows);

pub const WINDOW_LABEL: &str = "island";

/// Margin around the island that still counts as "on the island", in logical px.
/// Wider than the macOS 6 pt because a click must never be swallowed.
const HIT_MARGIN: f64 = 14.0;

/// Whether the window takes the mouse at (`x`, `y`), window-logical px: over the island shape
/// `r` (with `HIT_MARGIN`), or anywhere in the window's top `DRAG_ZONE_H` while the button is
/// `down`. Everywhere else the window is click-through, so the transparent part of the 560 px
/// panel never swallows a click.
pub fn takes_mouse(x: f64, y: f64, r: IslandRect, size: (f64, f64), down: bool) -> bool {
    let on_island = r.w > 0.0
        && x >= r.x - HIT_MARGIN
        && x <= r.x + r.w + HIT_MARGIN
        && y >= r.y - HIT_MARGIN
        && y <= r.y + r.h + HIT_MARGIN;
    // A file being dragged has to be able to find us. WS_EX_TRANSPARENT
    // — what click-through is on Windows — hides the window from
    // WindowFromPoint, so OLE never offers the drag to the webview and
    // the page sees no HTML5 drag event. macOS has no such problem: AppKit
    // delivers drags to registered destinations whatever ignoresMouseEvents
    // says. So while a button is held over the top of the panel, that part
    // takes the mouse, which also makes the drop zone as forgiving as the Mac's.
    let dragging = down && x >= 0.0 && x <= size.0 && y >= 0.0 && y <= size.1.min(DRAG_ZONE_H);
    on_island || dragging
}

/// Poll period while the cursor is near the island or the left button is held, in ms (60 Hz).
const FAST_POLL_MS: u64 = 16;
/// Poll period while the cursor is far from the island with no button held, in ms (20 Hz): the
/// page draws a far cursor at most at the fire's pace anyway (pace.ts cursorNeedsFrame).
const SLOW_POLL_MS: u64 = 50;
/// The fastest cursor the slow rate still sees before it reaches the island's hit margin, in
/// logical px per second: the whole width of a 1080p screen in under half a second.
const FAST_CURSOR_PX_PER_S: f64 = 4000.0;
/// How far beyond `HIT_MARGIN` the poll already runs fast: what `FAST_CURSOR_PX_PER_S` covers in
/// one slow tick, so a cursor seen outside it at a slow tick cannot be over a button by the next.
const NEAR_DISTANCE: f64 = FAST_CURSOR_PX_PER_S * SLOW_POLL_MS as f64 / 1000.0;
/// How often the poll checks the display layout.
const SCREEN_CHECK: Duration = Duration::from_millis(500);

/// Whether (`x`, `y`), window-logical px, is within `NEAR_DISTANCE` of the island's hit box `r`
/// (with `HIT_MARGIN`). Nothing drawn yet is near nothing: only a held button takes the mouse then.
fn near_island(x: f64, y: f64, r: IslandRect) -> bool {
    let reach = HIT_MARGIN + NEAR_DISTANCE;
    r.w > 0.0 && x >= r.x - reach && x <= r.x + r.w + reach && y >= r.y - reach && y <= r.y + r.h + reach
}

/// The poll's next period, in ms: fast while the cursor is near the island (click-through must
/// flip before it reaches a button) or the button is held (a file drag, the release the page
/// waits for); slow everywhere else.
fn poll_period_ms(x: f64, y: f64, r: IslandRect, down: bool) -> u64 {
    if down || near_island(x, y, r) { FAST_POLL_MS } else { SLOW_POLL_MS }
}

#[derive(Serialize, Clone)]
pub struct CursorPayload {
    pub x: f64,
    pub y: f64,
}

#[derive(Serialize, Clone)]
pub struct ScreenInfo {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    /// The notch of a MacBook screen in logical px, `[width, height]`; null elsewhere.
    pub notch: Option<(f64, f64)>,
}

/// The notch the island sits in (macOS only).
pub fn notch() -> Option<(f64, f64)> {
    #[cfg(target_os = "macos")]
    return platform::notch();
    #[cfg(not(target_os = "macos"))]
    None
}

/// The island shape in window-logical coordinates, pushed by the front end.
/// The poll thread owns the click-through decision so it lands in the same 16 ms
/// tick as the cursor read — an IPC round trip here loses clicks.
#[derive(Clone, Copy, Default, Debug)]
pub struct IslandRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Wakes / parks the cursor poll thread so a hidden island costs literally nothing.
pub struct PollGate {
    active: Mutex<bool>,
    cv: Condvar,
    pub collapsed: AtomicBool,
    pub rect: Mutex<IslandRect>,
    /// Mirrors the window flag so we only call into the OS when it changes.
    ignoring: AtomicBool,
}

impl PollGate {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(false),
            cv: Condvar::new(),
            collapsed: AtomicBool::new(true),
            rect: Mutex::new(IslandRect::default()),
            ignoring: AtomicBool::new(false),
        }
    }

    pub fn set_rect(&self, rect: IslandRect) {
        *self.rect.lock().unwrap() = rect;
    }

    /// Records the flag just written to the window, so the next poll tick only calls into the
    /// OS when the flag has to change.
    pub fn remember_ignore(&self, ignoring: bool) {
        self.ignoring.store(ignoring, Ordering::Relaxed);
    }

    pub fn set_active(&self, on: bool) {
        let mut guard = self.active.lock().unwrap();
        *guard = on;
        self.cv.notify_all();
    }

    fn wait_until_active(&self) {
        let mut guard = self.active.lock().unwrap();
        while !*guard {
            guard = self.cv.wait(guard).unwrap();
        }
    }

    fn is_active(&self) -> bool {
        *self.active.lock().unwrap()
    }

    /// Sleeps `d`, or less if the poll is parked meanwhile: a slow tick never delays the parked
    /// click-through write.
    fn nap(&self, d: Duration) {
        let guard = self.active.lock().unwrap();
        let _ = self.cv.wait_timeout_while(guard, d, |active| *active).unwrap();
    }
}

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(WINDOW_LABEL)
}

fn monitor_contains(m: &Monitor, x: f64, y: f64) -> bool {
    let p = m.position();
    let s = m.size();
    x >= p.x as f64
        && x < (p.x + s.width as i32) as f64
        && y >= p.y as f64
        && y < (p.y + s.height as i32) as f64
}

/// The display the island lives on: the primary one, or the one under the cursor.
fn target_monitor(app: &AppHandle, pref: &str) -> Option<Monitor> {
    let monitors = app.available_monitors().ok()?;
    if pref == "cursor" {
        if let Some((cx, cy)) = cursor_physical() {
            if let Some(m) = monitors.iter().find(|m| monitor_contains(m, cx, cy)) {
                return Some(m.clone());
            }
        }
    }
    app.primary_monitor()
        .ok()
        .flatten()
        .or_else(|| monitors.into_iter().next())
}

pub fn screen_info(app: &AppHandle, pref: &str) -> ScreenInfo {
    match target_monitor(app, pref) {
        Some(m) => {
            let scale = m.scale_factor();
            let p = m.position();
            let s = m.size();
            ScreenInfo {
                x: p.x as f64 / scale,
                y: p.y as f64 / scale,
                width: s.width as f64 / scale,
                height: s.height as f64 / scale,
                scale,
                notch: notch(),
            }
        }
        None => ScreenInfo { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0, scale: 1.0, notch: notch() },
    }
}

/// Logical size of the window while the island is hidden.
pub fn collapsed_size(native_strip: bool) -> (f64, f64) {
    if native_strip {
        return (STRIP_ZONE_W, STRIP_ZONE_H);
    }
    // In a notch the wake zone is the notch itself: the pointer on it wakes the island.
    notch().unwrap_or((STRIP_W, STRIP_H))
}

/// The click-through flag of a parked poll: the visible strip passes every click (its catcher
/// and its hook take the touches, spec §2); anything else takes the mouse until the next tick.
pub fn parked_ignore(collapsed: bool, native_strip: bool) -> bool {
    collapsed && native_strip
}

/// Top-centre placement of a `logical` size on a monitor at (`mon_x`, `mon_y`), `mon_w` physical
/// px wide: `(x, y, physical width, physical height)`. `apply_geometry` and the strip zone
/// (`strip::geometry`) both place through here, so the zone the hook tests is the window.
pub fn top_centre(mon_x: i32, mon_y: i32, mon_w: u32, logical: (f64, f64), scale: f64) -> (i32, i32, u32, u32) {
    let pw = (logical.0 * scale).round().max(1.0) as u32;
    let ph = (logical.1 * scale).round().max(1.0) as u32;
    (mon_x + (mon_w as i32 - pw as i32) / 2, mon_y, pw, ph)
}

/// Places and sizes the window. `collapsed` picks the wake strip instead of the panel.
pub fn apply_geometry(app: &AppHandle, pref: &str, collapsed: bool) {
    let Some(win) = window(app) else { return };
    let Some(m) = target_monitor(app, pref) else { return };

    let scale = m.scale_factor();
    let mp = *m.position();
    let ms = *m.size();

    let (lw, lh) = if collapsed { collapsed_size(NATIVE_STRIP) } else { (PANEL_W, PANEL_H) };
    let (x, y, pw, ph) = top_centre(mp.x, mp.y, ms.width, (lw, lh), scale);

    // GTK never sizes a non-resizable window below its natural size (200 px
    // here), so on Linux the 6 px wake strip would stay a 200 px block. tao
    // re-applies the config's `resizable: false` after the first configure, so
    // this is asked every time, just before the resize. Undecorated, the window
    // still offers the user nothing to resize it by. (Found by @YossiYad, #44.)
    #[cfg(target_os = "linux")]
    let _ = win.set_resizable(true);
    let _ = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_position(PhysicalPosition::new(x, y));
    // Moving across displays can rescale the window: re-assert the physical size.
    let _ = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_always_on_top(true);
    // AppKit keeps a window under the menu bar; the island belongs at the notch.
    #[cfg(target_os = "macos")]
    platform::pin_to_top(&win);
}

/// Zone and catcher of the strip on the monitor `apply_geometry` uses for `pref`.
#[cfg(windows)]
pub fn strip_geometry(app: &AppHandle, pref: &str) -> Option<crate::strip::StripGeometry> {
    let m = target_monitor(app, pref)?;
    let p = m.position();
    let s = m.size();
    let monitor = crate::strip::PxRect { left: p.x, top: p.y, right: p.x + s.width as i32, bottom: p.y + s.height as i32 };
    Some(crate::strip::geometry(monitor, m.scale_factor()))
}

/// Position, size and scale of the monitor the island lives on. Any change here
/// means the island has to be placed again.
fn current_screen_key(app: &AppHandle) -> Option<(i32, i32, u32, u32, u64)> {
    let pref = app
        .try_state::<crate::Shared>()
        .map(|s| s.settings.lock().unwrap().screen.clone())
        .unwrap_or_else(|| "primary".into());
    let m = target_monitor(app, &pref)?;
    let p = m.position();
    let size = m.size();
    Some((p.x, p.y, size.width, size.height, m.scale_factor().to_bits()))
}

/// Emits `cursor` (window-logical coordinates) while the island is visible: at ~60 Hz near the
/// island or with the button held, ~20 Hz elsewhere (`poll_period_ms`). Parked on a condvar the
/// rest of the time.
pub fn spawn_cursor_poll(app: AppHandle, gate: Arc<PollGate>) {
    std::thread::spawn(move || {
        // Remembered across wakes so a display change while hidden is noticed the
        // moment the island comes back.
        let mut last_screen: Option<(i32, i32, u32, u32, u64)> = None;
        // Without a cursor to read (Linux) the loop only watches the display
        // layout, and twice a second is plenty for that: waking at 60 Hz just to
        // find no cursor costs CPU for nothing.
        let base_period = if platform::CURSOR_POLL { FAST_POLL_MS } else { SCREEN_CHECK.as_millis() as u64 };
        loop {
            gate.wait_until_active();
            // The first tick after a wake always writes the flag: a parked write of this thread
            // may land after set_collapsed(false) has written its own and left the mirror wrong
            // (#28). `last` starts at MIN, so that tick is never skipped as "did not move".
            let mut force = true;
            let mut last = (f64::MIN, f64::MIN);
            // Per wake: a press left over from before the island hid is no release now.
            let mut was_down = false;
            let mut period = base_period;
            let mut screen_checked = Instant::now();
            // The window and its HWND, asked of tauri once per wake (`hwnd()` is a main-thread
            // round trip too); every tick then reads the geometry from the HWND itself.
            let mut probe: Option<(WebviewWindow, platform::WindowProbe)> = None;
            while gate.is_active() {
                gate.nap(Duration::from_millis(period));

                // Monitors get plugged in, unplugged, rearranged and rescaled, and
                // an island pinned to coordinates that no longer exist is an island
                // nobody can reach. Checked about twice a second — the cursor poll
                // is already running, so this costs one monitor query.
                if screen_checked.elapsed() >= SCREEN_CHECK {
                    screen_checked = Instant::now();
                    let now = current_screen_key(&app);
                    if now.is_some() && now != last_screen {
                        let first = last_screen.is_none();
                        last_screen = now;
                        if !first {
                            crate::log::line("display layout changed — repositioning".to_string());
                            let _ = app.emit_to(WINDOW_LABEL, "screen-changed", ());
                        }
                    }
                }

                if !platform::CURSOR_POLL {
                    continue;
                }

                if probe.is_none() {
                    probe = window(&app).and_then(|w| platform::WindowProbe::new(&w).map(|p| (w, p)));
                }
                let Some((win, hwnd)) = &probe else { continue };
                let Some((origin, inner, scale)) = hwnd.geometry() else {
                    probe = None;
                    continue;
                };
                let Some((cx, cy)) = cursor_physical() else { continue };
                let x = (cx - origin.0 as f64) / scale;
                let y = (cy - origin.1 as f64) / scale;
                let size = (inner.0 as f64 / scale, inner.1 as f64 / scale);
                // HTML5 has no event for a file drag that ends somewhere else: tell the page
                // when the button goes up. Before the "did not move" shortcut, since a release
                // needs no movement.
                let down = left_button_down();
                if was_down && !down {
                    let _ = app.emit_to(WINDOW_LABEL, "pointer-released", ());
                }
                was_down = down;

                let r = *gate.rect.lock().unwrap();
                period = poll_period_ms(x, y, r, down);

                if (x - last.0).abs() < 1.0 && (y - last.1).abs() < 1.0 {
                    continue;
                }
                last = (x, y);

                // Click-through: the window only takes the mouse over the island
                // shape. A small entry margin means the flag is already off by the
                // time a moving cursor reaches a button.
                let accept = takes_mouse(x, y, r, size, down);
                if force || gate.ignoring.load(Ordering::Relaxed) == accept {
                    force = false;
                    gate.ignoring.store(!accept, Ordering::Relaxed);
                    let _ = win.set_ignore_cursor_events(!accept);
                }

                let _ = win.emit("cursor", CursorPayload { x, y });
            }
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
        }
    });
}

/// macOS: wakes a hidden island when the pointer enters the notch. The page of an inactive app
/// is not told about the mouse, so, as `strip.rs` does on Windows, Rust watches the cursor and
/// emits `strip-touch`. The collapsed window is the notch itself, so "in the notch" is "in the
/// window". While the island is open the cursor poll does the work and this thread only naps.
#[cfg(target_os = "macos")]
pub fn spawn_notch_watch(app: AppHandle, gate: Arc<PollGate>) {
    /// Physical px under the window that still count, for a pointer coming up from below.
    const MARGIN: i32 = 6;
    std::thread::spawn(move || {
        let mut inside = false;
        let mut probe: Option<platform::WindowProbe> = None;
        loop {
            if !gate.collapsed.load(Ordering::Relaxed) {
                inside = false;
                probe = None;
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            let Some((cx, cy)) = cursor_physical() else {
                std::thread::sleep(Duration::from_millis(250));
                continue;
            };
            if probe.is_none() {
                probe = window(&app).and_then(|w| platform::WindowProbe::new(&w));
            }
            let zone = probe.as_ref().and_then(|p| p.geometry());
            let (now_inside, near) = match zone {
                Some(((x, y), (w, h), _)) => {
                    let (x, y, w, h) = (x as f64, y as f64, w as f64, h as f64);
                    let inside = cx >= x && cx < x + w && cy >= y && cy < y + h + MARGIN as f64;
                    (inside, cy < y + 4.0 * h)
                }
                None => (false, false),
            };
            if now_inside && !inside {
                let _ = app.emit_to(WINDOW_LABEL, "strip-touch", ());
            }
            inside = now_inside;
            // Near the top of the screen a pointer may be about to enter: look often there.
            std::thread::sleep(Duration::from_millis(if near { 30 } else { 120 }));
        }
    });
}

/// Re-applies click-through after the window or the island changed shape.
///
/// With the cursor poll (Windows) a visible window takes the mouse again and the next tick
/// decides from the cursor; the hidden one, the strip zone, passes every click. Without it
/// (Linux) the input region is set to the island itself, or to the whole wake strip while
/// collapsed.
pub fn refresh_click_through(app: &AppHandle, gate: &PollGate) {
    if platform::CURSOR_POLL {
        let ignore = parked_ignore(gate.collapsed.load(Ordering::Relaxed), NATIVE_STRIP);
        set_ignore_cursor(app, ignore);
        gate.remember_ignore(ignore);
        return;
    }
    let Some(win) = window(app) else { return };
    let region = if gate.collapsed.load(Ordering::Relaxed) {
        // The wake strip itself, never "the whole window": if the window ever
        // fails to shrink to the strip, the rest of it must not swallow clicks
        // meant for whatever sits under the top of the screen.
        Some((0.0, 0.0, STRIP_W, STRIP_H))
    } else {
        let r = *gate.rect.lock().unwrap();
        if r.w <= 0.0 {
            // Nothing drawn yet: nothing takes the mouse.
            Some((0.0, 0.0, 0.0, 0.0))
        } else {
            let x0 = (r.x - HIT_MARGIN).max(0.0);
            let y0 = (r.y - HIT_MARGIN).max(0.0);
            let x1 = r.x + r.w + HIT_MARGIN;
            let y1 = r.y + r.h + HIT_MARGIN;
            Some((x0, y0, x1 - x0, y1 - y0))
        }
    };
    platform::set_input_region(&win, region);
}

pub fn set_ignore_cursor(app: &AppHandle, ignore: bool) {
    if let Some(win) = window(app) {
        let _ = win.set_ignore_cursor_events(ignore);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_centre_places_the_panel_as_before() {
        // Only the height grew (320 -> 560, answers spec §6): same x, same width, same top.
        assert_eq!(top_centre(0, 0, 1920, (PANEL_W, PANEL_H), 1.0), (600, 0, 720, 560));
        assert_eq!(top_centre(-1920, 0, 1920, (PANEL_W, PANEL_H), 1.5), (-1500, 0, 1080, 840));
    }

    #[test]
    fn the_window_takes_the_mouse_only_over_the_island_and_the_old_drag_zone() {
        let panel = (PANEL_W, PANEL_H);
        // A 640×160 card, centred in the 720 px window.
        let card = IslandRect { x: 40.0, y: 0.0, w: 640.0, h: 160.0 };
        assert!(takes_mouse(360.0, 80.0, card, panel, false), "on the card");
        assert!(takes_mouse(40.0 - HIT_MARGIN, 160.0 + HIT_MARGIN, card, panel, false), "the margin's corner");
        assert!(!takes_mouse(40.0 - HIT_MARGIN - 1.0, 80.0, card, panel, false), "left of the margin");
        assert!(!takes_mouse(360.0, 160.0 + HIT_MARGIN + 1.0, card, panel, false), "below the margin");
        // The transparent part the taller panel added passes clicks, button up or held.
        assert!(!takes_mouse(360.0, 400.0, card, panel, false));
        assert!(!takes_mouse(360.0, 400.0, card, panel, true));
        assert!(!takes_mouse(360.0, DRAG_ZONE_H + 1.0, card, panel, true));
        // A held button over the old 720×320 panel still takes it (a file drag finds the island).
        assert!(takes_mouse(10.0, 300.0, card, panel, true));
        assert!(takes_mouse(0.0, DRAG_ZONE_H, card, panel, true));
        assert!(!takes_mouse(10.0, 300.0, card, panel, false));
        assert!(!takes_mouse(-1.0, 100.0, card, panel, true), "outside the window");
        // A 560 px question card takes the mouse down to its bottom.
        let tall = IslandRect { x: 40.0, y: 0.0, w: 640.0, h: 560.0 };
        assert!(takes_mouse(360.0, 550.0, tall, panel, false));
        // Nothing drawn yet: only a held button over the drag zone.
        let none = IslandRect::default();
        assert!(!takes_mouse(360.0, 10.0, none, panel, false));
        assert!(takes_mouse(360.0, 10.0, none, panel, true));
    }

    #[test]
    fn the_collapsed_window_is_the_zone_only_where_the_native_strip_runs() {
        assert_eq!(collapsed_size(true), (600.0, 150.0));
        assert_eq!(collapsed_size(false), (240.0, 6.0));
        assert_eq!(NATIVE_STRIP, cfg!(windows));
    }

    #[test]
    fn the_poll_runs_fast_near_the_island_or_with_the_button_held() {
        let card = IslandRect { x: 40.0, y: 0.0, w: 640.0, h: 160.0 };
        let edge = 160.0 + HIT_MARGIN + NEAR_DISTANCE;
        assert_eq!(poll_period_ms(360.0, 80.0, card, false), FAST_POLL_MS, "on the card");
        assert_eq!(poll_period_ms(360.0, edge, card, false), FAST_POLL_MS, "at the near edge");
        assert_eq!(poll_period_ms(360.0, edge + 1.0, card, false), SLOW_POLL_MS, "beyond it");
        assert_eq!(poll_period_ms(40.0 - HIT_MARGIN - NEAR_DISTANCE - 1.0, 80.0, card, false), SLOW_POLL_MS);
        assert_eq!(poll_period_ms(-3000.0, 900.0, card, false), SLOW_POLL_MS, "another display");
        assert_eq!(poll_period_ms(-3000.0, 900.0, card, true), FAST_POLL_MS, "button held anywhere");
        // Nothing drawn: only the button decides.
        let none = IslandRect::default();
        assert_eq!(poll_period_ms(360.0, 10.0, none, false), SLOW_POLL_MS);
        assert_eq!(poll_period_ms(360.0, 10.0, none, true), FAST_POLL_MS);
    }

    #[test]
    fn a_fast_cursor_seen_at_a_slow_tick_is_seen_at_a_fast_one_before_it_takes_the_mouse() {
        // One slow tick at the fastest cursor never crosses the whole near band.
        let step = FAST_CURSOR_PX_PER_S * SLOW_POLL_MS as f64 / 1000.0;
        assert!(step <= NEAR_DISTANCE);
        assert!(FAST_POLL_MS < SLOW_POLL_MS);
        let card = IslandRect { x: 40.0, y: 0.0, w: 640.0, h: 160.0 };
        let far_y = 160.0 + HIT_MARGIN + NEAR_DISTANCE + 0.5;
        assert_eq!(poll_period_ms(360.0, far_y, card, false), SLOW_POLL_MS);
        // Where it is at the next tick: still passing clicks, and polled fast from there on.
        assert!(!takes_mouse(360.0, far_y - step, card, (PANEL_W, PANEL_H), false));
        assert_eq!(poll_period_ms(360.0, far_y - step, card, false), FAST_POLL_MS);
    }

    #[test]
    fn a_parked_gate_cuts_the_nap_short() {
        let gate = PollGate::new();
        let t = Instant::now();
        gate.nap(Duration::from_secs(5));
        assert!(t.elapsed() < Duration::from_secs(1), "parked: no nap at all");
        gate.set_active(true);
        let t = Instant::now();
        gate.nap(Duration::from_millis(30));
        assert!(t.elapsed() >= Duration::from_millis(30), "active: the whole nap");
    }

    #[test]
    fn parked_the_window_passes_clicks_only_as_the_native_strip() {
        // upstream #28: the parked flag follows the state, never the poll's last tick
        assert!(parked_ignore(true, true));
        assert!(!parked_ignore(false, true));
        assert!(!parked_ignore(true, false));
        assert!(!parked_ignore(false, false));
    }
}
