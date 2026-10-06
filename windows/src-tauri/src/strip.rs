//! The visible strip of a hidden island, Windows only (spec 2026-10-06-mayak-island-strip-design.md).
//! The pure part: where the zone, the line and the touch catcher are, how the line stretches,
//! when a cursor touches it, what counts as fullscreen, what the mouse hook lets through, and how
//! raw strip messages become island events. The Win32 part (thread, hooks, catcher) follows it.

use crate::island::{top_centre, STRIP_ZONE_H, STRIP_ZONE_W};

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
