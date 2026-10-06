//! The visible strip of a hidden island, Windows only (spec 2026-10-06-mayak-island-strip-design.md).
//! The pure part: where the zone, the line and the touch catcher are, how the line stretches,
//! when a cursor touches it, what counts as fullscreen, what the mouse hook lets through, and how
//! raw strip messages become island events. The Win32 part (thread, hooks, catcher) follows it.

use crate::island::{top_centre, CursorPayload, STRIP_ZONE_H, STRIP_ZONE_W, WINDOW_LABEL};

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
}
