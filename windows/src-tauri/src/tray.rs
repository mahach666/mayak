// Notification-area icon: Open, Settings, Focus, Pause, Quit. While sessions wait for an answer,
// the icon carries their number and the tooltip names them, longest wait first.

use std::sync::{Mutex, PoisonError};

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::image::Image;
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::observe::model::{PhaseView, SessionsSnapshot};

/// The flame silhouette scripts/gen-icons.mjs draws for the tray, decoded at compile time.
const TRAY_ICON: Image<'static> = tauri::include_image!("icons/tray.png");

use crate::island::WINDOW_LABEL;

/// The tray's Pause, kept so the island's quiet button and the tray always show one state (spec §5).
pub struct PauseItem(CheckMenuItem<Wry>);

/// The tray's Focus, kept in step with the island's focus button the same way (night spec §A).
pub struct FocusItem(CheckMenuItem<Wry>);

/// The three plain items, kept so the island can set their texts in its language (batch spec §C).
/// Pause and Focus are reached through `PauseItem` and `FocusItem`.
pub struct TextItems {
    open: MenuItem<Wry>,
    settings: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

/// The five texts in the island's language; the island sends them after every page load.
/// `waiting`, `minutes` and `just_now` build the tooltip while sessions wait (`WaitTexts`); a page
/// that does not send them leaves the English ones.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayTexts {
    pub open: String,
    pub settings: String,
    pub focus: String,
    pub pause: String,
    pub quit: String,
    #[serde(default)]
    pub waiting: Option<String>,
    #[serde(default)]
    pub minutes: Option<String>,
    #[serde(default)]
    pub just_now: Option<String>,
}

/// Longest text a tray item takes, in characters; the island's own are far shorter.
pub const MAX_TEXT_CHARS: usize = 64;

impl TrayTexts {
    /// Each text cut to `MAX_TEXT_CHARS` characters (never in the middle of a character).
    pub fn capped(self) -> Self {
        let cap = |s: String| s.chars().take(MAX_TEXT_CHARS).collect::<String>();
        Self {
            open: cap(self.open),
            settings: cap(self.settings),
            focus: cap(self.focus),
            pause: cap(self.pause),
            quit: cap(self.quit),
            waiting: self.waiting.map(cap),
            minutes: self.minutes.map(cap),
            just_now: self.just_now.map(cap),
        }
    }
}

/// The tooltip's texts while sessions wait. Rust keeps no dictionary: until the island sends its
/// own (`set_texts`), these are the English ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitTexts {
    /// "{n} waiting": the tooltip's first line, after "Mayak · ".
    pub waiting: String,
    /// "{m} min": how long a session has waited, one or more whole minutes.
    pub minutes: String,
    /// "just now": a wait shorter than a minute.
    pub just_now: String,
}

impl Default for WaitTexts {
    fn default() -> Self {
        Self { waiting: "{n} waiting".into(), minutes: "{m} min".into(), just_now: "just now".into() }
    }
}

impl WaitTexts {
    /// The island's texts; one it did not send, or sent empty, stays English.
    fn from_island(texts: &TrayTexts) -> Self {
        let pick = |given: &Option<String>, fallback: String| {
            given.clone().filter(|s| !s.trim().is_empty()).unwrap_or(fallback)
        };
        let en = Self::default();
        Self {
            waiting: pick(&texts.waiting, en.waiting),
            minutes: pick(&texts.minutes, en.minutes),
            just_now: pick(&texts.just_now, en.just_now),
        }
    }
}

/// What the tray shows of the waiting sessions: the texts, and the tooltip and the number last set
/// on the tray (None until first set, or after a failed set), so the tray is only told of a change.
#[derive(Default)]
pub struct WaitState {
    texts: Mutex<WaitTexts>,
    tip: Mutex<Option<String>>,
    shown: Mutex<Option<usize>>,
}

/// Records `next` in `last` and says whether it differs from what was there: the tray is told only
/// then. Called under the lock; the tray call itself comes after the lock is released.
fn note_change<T: PartialEq>(last: &mut Option<T>, next: T) -> bool {
    if last.as_ref() == Some(&next) {
        return false;
    }
    *last = Some(next);
    true
}

/// Plain tooltip without waiting sessions; also the start of the tooltip with them.
const TIP_TITLE: &str = "Mayak";
/// Windows keeps 128 UTF-16 units of a tray tooltip, the terminating zero included.
pub const MAX_TIP_UTF16: usize = 127;
/// Sessions the tooltip names; more end with a "…" line (the number on the icon counts them all).
pub const MAX_TIP_ROWS: usize = 4;

/// The sessions waiting for an answer (a permission or a question), as (project name, wait start),
/// longest wait first; ties keep the snapshot order. A wait counts as long as the session waits,
/// also once its card was dismissed or its window opened (marked seen): a dismissed card is the
/// case the number and the reminders are for. A silent Codex row never counts.
pub fn waiting_rows(s: &SessionsSnapshot) -> Vec<(String, i64)> {
    let mut rows: Vec<(String, i64)> = s
        .projects
        .iter()
        .flat_map(|p| {
            p.sessions
                .iter()
                .filter(|x| x.silent.is_none() && matches!(x.phase, PhaseView::Waiting { .. }))
                .map(move |x| (p.name.clone(), x.since))
        })
        .collect();
    rows.sort_by_key(|(_, since)| *since);
    rows
}

/// A project name on one tooltip line: control characters (a newline in a folder name) become spaces.
fn one_line(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let clean = clean.trim();
    if clean.is_empty() { "?".to_string() } else { clean.to_string() }
}

/// "Mayak" with no waiting session; else "Mayak · 2 waiting" and one "name · 12 min" line per
/// session, at most MAX_TIP_ROWS of them, cut to MAX_TIP_UTF16 units (never inside a character).
/// Minutes are whole minutes since the wait began, as of `now`.
pub fn tooltip(rows: &[(String, i64)], now: i64, texts: &WaitTexts) -> String {
    if rows.is_empty() {
        return TIP_TITLE.to_string();
    }
    let mut lines = vec![format!("{TIP_TITLE} · {}", texts.waiting.replace("{n}", &rows.len().to_string()))];
    for (name, since) in rows.iter().take(MAX_TIP_ROWS) {
        let minutes = now.saturating_sub(*since).max(0) / 60_000;
        let ago = if minutes < 1 { texts.just_now.clone() } else { texts.minutes.replace("{m}", &minutes.to_string()) };
        lines.push(format!("{} · {ago}", one_line(name)));
    }
    if rows.len() > MAX_TIP_ROWS {
        lines.push("…".to_string());
    }
    let full = lines.join("\n");
    let mut out = String::new();
    let mut units = 0;
    for c in full.chars() {
        units += c.len_utf16();
        if units > MAX_TIP_UTF16 {
            break;
        }
        out.push(c);
    }
    out
}

/// Amber of the waiting rows (style.css --amber), the badge's fill.
const BADGE_FILL: [u8; 3] = [0xf5, 0xa5, 0x24];
/// The dark of the app icon's square (gen-icons.mjs SQUARE): the digit and the ring around the badge.
const BADGE_INK: [u8; 3] = [0x12, 0x12, 0x16];

/// 3×5 glyphs, one row per byte, bit 2 = left column. Digits 0-9, then "+" for ten or more.
const GLYPHS: [[u8; 5]; 11] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
    [0b000, 0b010, 0b111, 0b010, 0b000],
];

/// Blends `color` over the RGBA pixel at `i` with coverage `a` (0..1); the icon is straight alpha.
fn blend(px: &mut [u8], i: usize, color: [u8; 3], a: f32) {
    if a <= 0.0 {
        return;
    }
    let a = a.min(1.0);
    let under = f32::from(px[i + 3]) / 255.0;
    let out = a + under * (1.0 - a);
    for c in 0..3 {
        let mixed = (f32::from(color[c]) * a + f32::from(px[i + c]) * under * (1.0 - a)) / out;
        px[i + c] = mixed.round().clamp(0.0, 255.0) as u8;
    }
    px[i + 3] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// The tray icon's RGBA with the number of waiting sessions in a round amber badge at the bottom
/// right: 1 to 9 as a digit, ten or more as "+" (the tooltip has the number). 0 leaves it as it is.
/// The badge scales with the icon (drawn for the 32 px tray.png); a buffer that does not match
/// `width` × `height` is returned unchanged.
pub fn badge_rgba(base: &[u8], width: u32, height: u32, count: usize) -> Vec<u8> {
    let mut px = base.to_vec();
    let (w, h) = (width as usize, height as usize);
    if count == 0 || w == 0 || h == 0 || px.len() != w * h * 4 {
        return px;
    }
    let unit = w.min(h) as f32 / 32.0;
    let (cx, cy) = (w as f32 - 9.5 * unit, h as f32 - 9.5 * unit);
    let (ring, fill) = (9.5 * unit, 8.0 * unit);
    // Coverage by 4×4 samples per pixel: the dark ring, then the amber fill over it.
    const SS: usize = 4;
    for y in 0..h {
        for x in 0..w {
            let (mut in_ring, mut in_fill) = (0usize, 0usize);
            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = x as f32 + (sx as f32 + 0.5) / SS as f32 - cx;
                    let fy = y as f32 + (sy as f32 + 0.5) / SS as f32 - cy;
                    let d = (fx * fx + fy * fy).sqrt();
                    in_ring += usize::from(d <= ring);
                    in_fill += usize::from(d <= fill);
                }
            }
            let i = (y * w + x) * 4;
            blend(&mut px, i, BADGE_INK, in_ring as f32 / (SS * SS) as f32);
            blend(&mut px, i, BADGE_FILL, in_fill as f32 / (SS * SS) as f32);
        }
    }
    // The glyph: 3×5 cells of `cell` pixels, centred on the badge.
    let glyph = &GLYPHS[count.min(10)];
    let cell = ((2.0 * unit).round() as usize).max(1);
    let left = (cx - 1.5 * cell as f32).round() as isize;
    let top = (cy - 2.5 * cell as f32).round() as isize;
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..3 {
            if bits & (0b100 >> col) == 0 {
                continue;
            }
            for dy in 0..cell {
                for dx in 0..cell {
                    let x = left + (col * cell + dx) as isize;
                    let y = top + (row * cell + dy) as isize;
                    if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
                        continue;
                    }
                    blend(&mut px, (y as usize * w + x as usize) * 4, BADGE_INK, 1.0);
                }
            }
        }
    }
    px
}

/// Puts the waiting sessions of `snapshot` on the tray: the tooltip (its minutes are as of now) and
/// the number on the icon, each only when it changed. Called with every published snapshot and when
/// the pointer enters the icon, so there is no timer. Before the tray is built there is nothing to
/// set. Names never reach the log.
///
/// The observer thread calls this: the tray calls themselves are handed to the main thread and not
/// waited for (each tray call there would otherwise block this thread on the main one), and no lock
/// is held across a tray call, so the main thread (pointer on the icon, `tray_texts`) never waits
/// on a thread that waits on it. Every tray update runs on the main thread, in the order sent.
pub fn show_waiting(app: &AppHandle, snapshot: &SessionsSnapshot) {
    let Some(state) = app.try_state::<WaitState>() else { return };
    let rows = waiting_rows(snapshot);
    let texts = state.texts.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let tip = tooltip(&rows, crate::observe::now_ms(), &texts);
    let count = rows.len();
    let handle = app.clone();
    if let Err(err) = app.run_on_main_thread(move || apply_waiting(&handle, tip, count)) {
        crate::log::line(format!("tray: could not reach the main thread: {err}"));
    }
}

/// On the main thread: the tooltip and the icon, each set only when it changed. A failed set is
/// forgotten, so the next snapshot tries again.
fn apply_waiting(app: &AppHandle, tip: String, count: usize) {
    let (Some(state), Some(tray)) = (app.try_state::<WaitState>(), app.tray_by_id(crate::ident::APP_SLUG)) else {
        return;
    };
    let tip_changed = note_change(&mut state.tip.lock().unwrap_or_else(PoisonError::into_inner), tip.clone());
    if tip_changed && tray.set_tooltip(Some(tip)).is_err() {
        *state.tip.lock().unwrap_or_else(PoisonError::into_inner) = None;
        crate::log::line("tray: could not set the tooltip");
    }
    let count_changed = note_change(&mut state.shown.lock().unwrap_or_else(PoisonError::into_inner), count);
    if !count_changed {
        return;
    }
    let icon = Image::new_owned(
        badge_rgba(TRAY_ICON.rgba(), TRAY_ICON.width(), TRAY_ICON.height(), count),
        TRAY_ICON.width(),
        TRAY_ICON.height(),
    );
    if let Err(err) = tray.set_icon(Some(icon)) {
        *state.shown.lock().unwrap_or_else(PoisonError::into_inner) = None;
        crate::log::line(format!("tray icon: {err}"));
    }
}

/// The tray again from the observer's latest snapshot (the pointer entered the icon, new texts).
fn show_waiting_now(app: &AppHandle) {
    if let Some(observe) = app.try_state::<crate::observe::Handle>() {
        show_waiting(app, &observe.snapshot());
    }
}

/// Sets the five texts. Before the tray is built there is nothing to set. A failure is logged with
/// a fixed line: the texts themselves are UI texts and never reach the log.
pub fn set_texts(app: &AppHandle, texts: TrayTexts) {
    if let Some(state) = app.try_state::<WaitState>() {
        *state.texts.lock().unwrap_or_else(PoisonError::into_inner) = WaitTexts::from_island(&texts);
        show_waiting_now(app);
    }
    let (Some(items), Some(pause), Some(focus)) =
        (app.try_state::<TextItems>(), app.try_state::<PauseItem>(), app.try_state::<FocusItem>())
    else {
        return;
    };
    let results = [
        items.open.set_text(&texts.open),
        items.settings.set_text(&texts.settings),
        focus.0.set_text(&texts.focus),
        pause.0.set_text(&texts.pause),
        items.quit.set_text(&texts.quit),
    ];
    if results.iter().any(Result::is_err) {
        crate::log::line("tray: could not set the item texts");
    }
}

/// Called with every pause change, whichever side made it (`set_paused`).
pub fn show_paused(app: &AppHandle, paused: bool) {
    let Some(item) = app.try_state::<PauseItem>() else { return };
    if let Err(err) = item.0.set_checked(paused) {
        crate::log::line(format!("tray pause check: {err}"));
    }
}

/// Called with every focus start and end, whichever side made it (`set_focus_check`).
pub fn show_focus(app: &AppHandle, on: bool) {
    let Some(item) = app.try_state::<FocusItem>() else { return };
    if let Err(err) = item.0.set_checked(on) {
        crate::log::line(format!("tray focus check: {err}"));
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Mayak", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let focus = CheckMenuItem::with_id(app, "focus", "Focus", true, false, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pause", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(app, &[&open, &sep1, &settings, &focus, &pause, &sep2, &quit])?;

    app.manage(PauseItem(pause.clone()));
    app.manage(FocusItem(focus.clone()));
    app.manage(TextItems { open: open.clone(), settings: settings.clone(), quit: quit.clone() });
    app.manage(WaitState::default());

    // Its own icon, not the default window icon: on Windows that is the ICO entry at the system icon
    // size, which above 100 % scale is the dark-square app icon shrunk into the tray.
    TrayIconBuilder::with_id(crate::ident::APP_SLUG)
        .icon(TRAY_ICON)
        .tooltip("Mayak")
        .menu(&menu)
        .on_menu_event(|app: &AppHandle, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "settings" => crate::show_settings_window(app),
            // "open", "pause" and "focus" go to the island, which owns those states.
            id => {
                let _ = app.emit_to(WINDOW_LABEL, "tray", id.to_string());
            }
        })
        // The tooltip's minutes are brought up to date as the pointer comes onto the icon.
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Enter { .. } = event {
                show_waiting_now(tray.app_handle());
            }
        })
        .build(app)?;
    // Sessions that already wait: the observer started before the tray.
    show_waiting_now(app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::observe::model::{AttentionKind, HostKind, ProjectView, SessionView, Source, Store, WaitView};
    use crate::observe::silent::SilentReason;

    fn waiting(wait: WaitView) -> PhaseView {
        PhaseView::Waiting { wait, label: String::new() }
    }

    fn row(key: &str, since: i64, phase: PhaseView, attention: Option<AttentionKind>) -> SessionView {
        SessionView {
            key: key.into(),
            agent: Source::Claude,
            host: HostKind::VsCode,
            title: None,
            phase,
            since,
            activity: None,
            detail: None,
            outcome: None,
            attention,
            acked: attention.is_none(),
            silent: None,
            context: None,
            model: None,
            compacted: None,
            turn: None,
            ao: None,
            agent_pid: None,
            host_session_id: None,
            logical_id: None,
            cwd: None,
            last_reply: None,
            chat: None,
        }
    }

    fn project(name: &str, sessions: Vec<SessionView>) -> ProjectView {
        ProjectView { key: name.into(), name: name.into(), root: format!(r"C:\work\{name}"), sessions }
    }

    const MIN: i64 = 60_000;

    #[test]
    fn waiting_rows_are_the_sessions_that_wait_longest_first() {
        let mut s = Store::new().snapshot(0);
        let mut silent = row("s", 0, waiting(WaitView::Permission), None);
        silent.silent = Some(SilentReason::HooksNotReaching);
        s.projects.push(project(
            "mayak",
            vec![
                row("a", 5 * MIN, waiting(WaitView::Input), Some(AttentionKind::Input)),
                row("b", MIN, PhaseView::Idle, Some(AttentionKind::Finished)),
                row("c", 2 * MIN, PhaseView::Working, None),
                silent,
            ],
        ));
        s.projects.push(project(
            "api",
            vec![
                row("d", 3 * MIN, waiting(WaitView::Permission), Some(AttentionKind::Permission)),
                row("e", 0, PhaseView::Idle, Some(AttentionKind::Error)),
            ],
        ));
        // Its card dismissed (marked seen), the session still waits and still counts.
        s.projects.push(project("web", vec![row("f", 4 * MIN, waiting(WaitView::Other), None)]));
        assert_eq!(
            waiting_rows(&s),
            vec![("api".to_string(), 3 * MIN), ("web".to_string(), 4 * MIN), ("mayak".to_string(), 5 * MIN)]
        );
        assert!(waiting_rows(&Store::new().snapshot(0)).is_empty());
    }

    #[test]
    fn the_tray_is_told_only_of_a_change() {
        let mut last: Option<usize> = None;
        assert!(note_change(&mut last, 0), "the first value is always set");
        assert!(!note_change(&mut last, 0));
        assert!(note_change(&mut last, 2));
        assert_eq!(last, Some(2));
        // A failed set forgets the value, so the same value is set again next time.
        last = None;
        assert!(note_change(&mut last, 2));
        let mut tip: Option<String> = Some("Mayak".into());
        assert!(!note_change(&mut tip, "Mayak".to_string()));
        assert!(note_change(&mut tip, "Mayak · 1 waiting\nmayak · just now".to_string()));
    }

    #[test]
    fn the_tooltip_names_each_wait_with_its_minutes() {
        let en = WaitTexts::default();
        assert_eq!(tooltip(&[], 0, &en), "Mayak");
        let rows = vec![("mayak".to_string(), 0), ("api".to_string(), 8 * MIN), ("web".to_string(), 12 * MIN - 1)];
        assert_eq!(
            tooltip(&rows, 12 * MIN, &en),
            "Mayak · 3 waiting\nmayak · 12 min\napi · 4 min\nweb · just now"
        );
        let ru = WaitTexts { waiting: "ждут ответа: {n}".into(), minutes: "{m} мин".into(), just_now: "только что".into() };
        assert_eq!(tooltip(&rows[..1], 3 * MIN + 59_999, &ru), "Mayak · ждут ответа: 1\nmayak · 3 мин");
        // A clock behind the wait start reads as just now, never as negative minutes.
        assert_eq!(tooltip(&rows[1..2], 0, &en), "Mayak · 1 waiting\napi · just now");
    }

    #[test]
    fn the_tooltip_names_four_waits_then_an_ellipsis_and_fits_windows() {
        let en = WaitTexts::default();
        let rows: Vec<(String, i64)> = (0..6).map(|i| (format!("p{i}"), 0)).collect();
        let tip = tooltip(&rows, 2 * MIN, &en);
        assert_eq!(tip.lines().count(), 1 + MAX_TIP_ROWS + 1);
        assert!(tip.starts_with("Mayak · 6 waiting\np0 · 2 min"));
        assert!(tip.ends_with("\n…"));
        let long: Vec<(String, i64)> = (0..4).map(|i| (format!("{i}{}", "設".repeat(60)), 0)).collect();
        let tip = tooltip(&long, 0, &en);
        assert!(tip.encode_utf16().count() <= MAX_TIP_UTF16);
        assert!(tip.starts_with("Mayak · 4 waiting\n0設設"));
    }

    #[test]
    fn a_name_with_a_line_break_stays_on_its_line() {
        let rows = vec![("my\nproj\t".to_string(), 0), ("\r\n".to_string(), 0)];
        assert_eq!(tooltip(&rows, 0, &WaitTexts::default()), "Mayak · 2 waiting\nmy proj · just now\n? · just now");
    }

    fn pixel(px: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * w + x) * 4;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    }

    #[test]
    fn no_waiting_session_leaves_the_icon_as_it_is() {
        let base = TRAY_ICON.rgba().to_vec();
        assert_eq!(badge_rgba(&base, 32, 32, 0), base);
        // A buffer that is not width × height × 4 is never drawn into.
        assert_eq!(badge_rgba(&base[..100], 32, 32, 3), base[..100].to_vec());
    }

    #[test]
    fn the_badge_is_amber_bottom_right_with_a_dark_digit() {
        let base = vec![0u8; 32 * 32 * 4];
        let one = badge_rgba(&base, 32, 32, 1);
        // Top left untouched; the badge's edge (no digit there) is the amber fill, opaque.
        assert_eq!(pixel(&one, 32, 2, 2), [0, 0, 0, 0]);
        assert_eq!(pixel(&one, 32, 22, 29), [0xf5, 0xa5, 0x24, 255]);
        // The ring outside the fill is the dark ink.
        assert_eq!(pixel(&one, 32, 22, 31)[..3], BADGE_INK);
        // "1" starts with its middle column: cells of 2 px from (left, top) = (20, 18).
        assert_eq!(pixel(&one, 32, 22, 19), [0x12, 0x12, 0x16, 255]);
        assert_eq!(pixel(&one, 32, 20, 19), [0xf5, 0xa5, 0x24, 255]);
        // Different numbers draw differently; ten and more share the "+".
        let nine = badge_rgba(&base, 32, 32, 9);
        assert_ne!(one, nine);
        assert_ne!(nine, badge_rgba(&base, 32, 32, 10));
        assert_eq!(badge_rgba(&base, 32, 32, 10), badge_rgba(&base, 32, 32, 42));
    }

    #[test]
    fn the_badge_goes_over_the_real_tray_icon() {
        let (w, h) = (TRAY_ICON.width(), TRAY_ICON.height());
        let out = badge_rgba(TRAY_ICON.rgba(), w, h, 2);
        assert_eq!(out.len(), TRAY_ICON.rgba().len());
        assert_ne!(out, TRAY_ICON.rgba());
        assert_eq!(pixel(&out, w as usize, 1, 1), pixel(TRAY_ICON.rgba(), w as usize, 1, 1));
    }

    #[test]
    fn the_wait_texts_come_from_the_island_or_stay_english() {
        let t: TrayTexts = serde_json::from_str(
            r#"{"open":"o","settings":"s","focus":"f","pause":"p","quit":"q","waiting":"ждут: {n}","minutes":"{m} мин","justNow":" "}"#,
        )
        .unwrap();
        let w = WaitTexts::from_island(&t);
        assert_eq!((w.waiting.as_str(), w.minutes.as_str()), ("ждут: {n}", "{m} мин"));
        assert_eq!(w.just_now, "just now", "an empty text stays English");
        let old: TrayTexts =
            serde_json::from_str(r#"{"open":"o","settings":"s","focus":"f","pause":"p","quit":"q"}"#).unwrap();
        assert_eq!(WaitTexts::from_island(&old), WaitTexts::default());
    }

    fn texts(s: &str) -> TrayTexts {
        TrayTexts {
            open: s.into(),
            settings: s.into(),
            focus: s.into(),
            pause: s.into(),
            quit: s.into(),
            waiting: Some(s.into()),
            minutes: None,
            just_now: None,
        }
    }

    #[test]
    fn short_texts_are_kept_as_they_are() {
        let t = texts("Mayak öffnen").capped();
        assert_eq!(t.open, "Mayak öffnen");
        assert_eq!(t.quit, "Mayak öffnen");
    }

    #[test]
    fn long_texts_are_cut_to_64_characters_not_bytes() {
        let t = texts(&"設".repeat(100)).capped();
        for s in [&t.open, &t.settings, &t.focus, &t.pause, &t.quit] {
            assert_eq!(s.chars().count(), MAX_TEXT_CHARS);
        }
    }

    #[test]
    fn the_texts_come_from_camel_case_json() {
        let t: TrayTexts = serde_json::from_str(
            r#"{"open":"Открыть Mayak","settings":"Настройки…","focus":"Фокус","pause":"Пауза","quit":"Выйти"}"#,
        )
        .unwrap();
        assert_eq!((t.open.as_str(), t.quit.as_str()), ("Открыть Mayak", "Выйти"));
    }
}
