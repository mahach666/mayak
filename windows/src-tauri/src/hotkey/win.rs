//! The Windows half of the global shortcut (spec §3.2): the Win32 codes of a `Combo`, and the
//! thread that owns its registration.

use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayoutList, MapVirtualKeyExW, ToUnicodeEx, HKL, HOT_KEY_MODIFIERS, MAPVK_VK_TO_VSC, MOD_ALT,
    MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VK_CONTROL, VK_F1, VK_MENU, VK_SHIFT, VK_SPACE,
};

use super::{altgr_like, Combo, Key};

/// `RegisterHotKey` arguments for `combo` (spec §2, §3.2): its modifiers, always with
/// MOD_NOREPEAT (holding the keys toggles once), and its virtual key. A virtual key names the
/// physical key, not the character, so `Alt+M` is the same key in every layout.
pub fn win32_codes(combo: &Combo) -> (HOT_KEY_MODIFIERS, u32) {
    let mut mods = MOD_NOREPEAT.0;
    for (on, flag) in [(combo.ctrl, MOD_CONTROL), (combo.alt, MOD_ALT), (combo.shift, MOD_SHIFT), (combo.win, MOD_WIN)] {
        if on {
            mods |= flag.0;
        }
    }
    let vk = match combo.key {
        // 'A'..='Z' and '0'..='9' are their own virtual keys.
        Key::Letter(c) => u32::from(c),
        Key::Digit(d) => u32::from(b'0' + d),
        Key::F(n) => u32::from(VK_F1.0) + u32::from(n) - 1,
        Key::Space => u32::from(VK_SPACE.0),
    };
    (HOT_KEY_MODIFIERS(mods), vk)
}

/// The character Ctrl+Alt(+Shift)+`vk` types on one of the installed keyboard layouts, if any.
/// Windows reads Ctrl+Alt as AltGr, so registering such a combination would stop the user typing
/// that character (AltGr+E is € on most European layouts, AltGr+A is ą in Polish).
fn ctrl_alt_types(vk: u32, shift: bool) -> Option<String> {
    let mut layouts = [HKL::default(); 32];
    let count = unsafe { GetKeyboardLayoutList(Some(&mut layouts[..])) };
    let count = usize::try_from(count).unwrap_or(0).min(layouts.len());

    let mut state = [0u8; 256];
    state[usize::from(VK_CONTROL.0)] = 0x80;
    state[usize::from(VK_MENU.0)] = 0x80;
    if shift {
        state[usize::from(VK_SHIFT.0)] = 0x80;
    }
    for &layout in &layouts[..count] {
        let scan = unsafe { MapVirtualKeyExW(vk, MAPVK_VK_TO_VSC, Some(layout)) };
        let mut buf = [0u16; 8];
        // Flag 0x4: leave the keyboard state alone, so a dead key met here does not change what
        // the user types next (Windows 10 1607 and later).
        let n = unsafe { ToUnicodeEx(vk, scan, &state, &mut buf, 0x4, Some(layout)) };
        let typed = match n {
            // A dead key (´ ^ ¨…) is still something the user types with it.
            n if n < 0 => String::from_utf16_lossy(&buf[..1]),
            0 => continue,
            n => String::from_utf16_lossy(&buf[..usize::try_from(n).unwrap_or(0).min(buf.len())]),
        };
        if typed.chars().any(|c| !c.is_control()) {
            return Some(typed);
        }
    }
    None
}

// ── The hotkey thread ─────────────────────────────────────────────────────────
//
// One thread for the life of the app, with its own message queue (spec §3.2). RegisterHotKey
// without a window puts WM_HOTKEY in the queue of the thread that registered, so this thread
// registers, releases and hears the presses. Between them it sleeps in GetMessageW: no timer,
// no poll. Its sink calls app.emit / app.emit_to, which do not block.

use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};
use std::thread::JoinHandle;

use ::windows::Win32::Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, LPARAM, WPARAM};
use ::windows::Win32::System::Threading::GetCurrentThreadId;
use ::windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
use ::windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_APP, WM_HOTKEY, WM_QUIT, WM_USER,
};

use super::{HotkeyOs, HotkeyStatus, Pair, Refusal, Wanted, Which};

/// The id of each of Mayak's shortcuts on this thread.
fn hotkey_id(which: Which) -> i32 {
    match which {
        Which::Island => 1,
        Which::Next => 2,
    }
}
/// A new `Wanted` is waiting in the channel (spec §3.2: WM_APP + 1).
const WM_SET: u32 = WM_APP + 1;
/// Settings started recording: release the shortcut (WM_APP + 2).
const WM_PAUSE: u32 = WM_APP + 2;
/// Recording ended: register it again (WM_APP + 3).
const WM_RESUME: u32 = WM_APP + 3;

/// What the thread hands to its sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A registration changed (spec §3.1).
    Status(Which, HotkeyStatus),
    /// WM_HOTKEY: a shortcut was pressed.
    Pressed(Which),
}

/// One registration of the calling thread: `RegisterHotKey(NULL, …)` binds it to that thread.
struct ThreadHotkey(Which);

impl HotkeyOs for ThreadHotkey {
    fn register(&mut self, combo: &Combo) -> Result<(), Refusal> {
        let (mods, vk) = win32_codes(combo);
        unsafe { RegisterHotKey(None, hotkey_id(self.0), mods, vk) }.map_err(|err| Refusal {
            already_registered: err.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult(),
            message: err.message(),
        })
    }

    fn unregister(&mut self) {
        if let Err(err) = unsafe { UnregisterHotKey(None, hotkey_id(self.0)) } {
            crate::log::line(format!("hotkey: UnregisterHotKey: {err}"));
        }
    }

    fn types_character(&self, combo: &Combo) -> Option<String> {
        if !altgr_like(combo) {
            return None;
        }
        ctrl_alt_types(win32_codes(combo).1, combo.shift)
    }
}

/// Hands the statuses a request produced to the sink, island first.
fn report(statuses: [Option<HotkeyStatus>; 2], sink: &mut impl FnMut(Event)) {
    for (which, status) in Which::ALL.into_iter().zip(statuses) {
        if let Some(status) = status {
            sink(Event::Status(which, status));
        }
    }
}

fn run(cmds: Receiver<[Wanted; 2]>, mut sink: impl FnMut(Event), ready: SyncSender<u32>) {
    let mut msg = MSG::default();
    // A thread has no message queue until its first USER call, and PostThreadMessageW fails
    // until it has one: make it before telling `start` the thread id.
    unsafe {
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
    }
    if ready.send(unsafe { GetCurrentThreadId() }).is_err() {
        return;
    }
    let mut pair = Pair::default();
    let mut os = [ThreadHotkey(Which::Island), ThreadHotkey(Which::Next)];
    // 0 on WM_QUIT, -1 on error: both end the thread.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        match msg.message {
            WM_HOTKEY => {
                if let Some(which) = Which::ALL.into_iter().find(|w| msg.wParam.0 == hotkey_id(*w) as usize) {
                    sink(Event::Pressed(which));
                }
            }
            // Every waiting request, in order: the last status is the last request's.
            WM_SET => {
                for wanted in cmds.try_iter() {
                    report(pair.set(wanted, &mut os), &mut sink);
                }
            }
            WM_PAUSE => pair.pause(&mut os),
            WM_RESUME => report(pair.resume(&mut os), &mut sink),
            _ => {}
        }
    }
    pair.release(&mut os);
}

/// The running hotkey thread. Dropping it releases the shortcut and ends the thread.
pub struct Registrar {
    thread_id: u32,
    cmds: Sender<[Wanted; 2]>,
    thread: Option<JoinHandle<()>>,
}

impl Registrar {
    /// Starts the thread; every status and every press goes to `sink`, on that thread. Returns
    /// once its queue exists, so the first `set` cannot be lost.
    pub fn start(sink: impl FnMut(Event) + Send + 'static) -> std::io::Result<Self> {
        let (cmds, rx) = channel::<[Wanted; 2]>();
        let (ready_tx, ready_rx) = sync_channel::<u32>(1);
        let thread = std::thread::Builder::new().name("hotkey".into()).spawn(move || run(rx, sink, ready_tx))?;
        match ready_rx.recv() {
            Ok(thread_id) => Ok(Self { thread_id, cmds, thread: Some(thread) }),
            Err(_) => {
                let _ = thread.join();
                Err(std::io::Error::other(super::THREAD_ENDED))
            }
        }
    }

    /// Both shortcuts, island first, each a shortcut or none (spec §3.2: through the channel,
    /// then WM_APP + 1).
    pub fn set(&self, wanted: [Wanted; 2]) -> std::io::Result<()> {
        self.cmds.send(wanted).map_err(|_| std::io::Error::other(super::THREAD_GONE))?;
        self.post(WM_SET)
    }

    /// Settings starts recording a shortcut (`true`) or stopped (`false`).
    pub fn pause(&self, on: bool) -> std::io::Result<()> {
        self.post(if on { WM_PAUSE } else { WM_RESUME })
    }

    fn post(&self, msg: u32) -> std::io::Result<()> {
        unsafe { PostThreadMessageW(self.thread_id, msg, WPARAM(0), LPARAM(0)) }
            .map_err(|err| std::io::Error::other(err.message()))
    }
}

impl Drop for Registrar {
    /// WM_QUIT, then wait: the thread releases the shortcut on its way out. The thread is never
    /// joined from itself (a sink may drop its own registrar), and a thread that could not be
    /// told to quit and is still alive is left running rather than hanging the caller.
    fn drop(&mut self) {
        let posted = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Err(err) = &posted {
            crate::log::line(format!("hotkey: cannot post WM_QUIT to the hotkey thread: {err}"));
        }
        let Some(handle) = self.thread.take() else { return };
        if handle.thread().id() == std::thread::current().id() {
            return;
        }
        if posted.is_err() && !handle.is_finished() {
            crate::log::line("hotkey: the hotkey thread is left running");
            return;
        }
        if handle.join().is_err() {
            crate::log::line("hotkey: the hotkey thread panicked");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::{parse, HotkeyState};
    use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
    use std::time::Duration;

    /// `(modifiers, virtual key)` of a shortcut, as plain numbers.
    fn codes(text: &str) -> (u32, u32) {
        let (mods, vk) = win32_codes(&parse(text).unwrap());
        (mods.0, vk)
    }

    #[test]
    fn alt_m_is_mod_alt_and_the_virtual_key_of_m() {
        // 0x4000 = MOD_NOREPEAT, 0x0001 = MOD_ALT; 0x4D is the M key whatever the layout types there
        assert_eq!(codes("Alt+M"), (0x4001, 0x4D));
    }

    #[test]
    fn every_modifier_and_every_kind_of_key_has_its_win32_code() {
        assert_eq!(codes("Ctrl+Alt+Shift+Win+F24"), (0x400F, 0x87));
        assert_eq!(codes("F1"), (0x4000, 0x70));
        assert_eq!(codes("Alt+F12"), (0x4001, 0x7B));
        assert_eq!(codes("Win+Space"), (0x4008, 0x20));
        assert_eq!(codes("Ctrl+Alt+9"), (0x4003, 0x39));
        assert_eq!(codes("Ctrl+0"), (0x4002, 0x30));
        assert_eq!(codes("Ctrl+Shift+A"), (0x4006, 0x41));
        assert_eq!(codes("Shift+Win+Z"), (0x400C, 0x5A));
        assert_eq!(codes("Shift+F5"), (0x4004, 0x74));
    }

    #[test]
    fn holding_the_keys_never_repeats_the_toggle() {
        let keys = ('A'..='Z')
            .map(String::from)
            .chain((0..=9).map(|d: u8| d.to_string()))
            .chain((1..=24).map(|n: u8| format!("F{n}")))
            .chain(["Space".to_string()]);
        for key in keys {
            for mods in ["Alt", "Ctrl+Shift", "Win"] {
                let text = format!("{mods}+{key}");
                assert_ne!(codes(&text).0 & 0x4000, 0, "{text}: no MOD_NOREPEAT");
            }
        }
        assert_ne!(codes("F7").0 & 0x4000, 0, "F7: no MOD_NOREPEAT");
    }

    /// A registrar whose events land in a channel.
    fn started() -> (Registrar, Receiver<Event>) {
        let (tx, rx) = channel();
        let registrar = Registrar::start(move |e| {
            let _ = tx.send(e);
        })
        .expect("the hotkey thread starts");
        (registrar, rx)
    }

    fn next(rx: &Receiver<Event>) -> Result<Event, RecvTimeoutError> {
        rx.recv_timeout(Duration::from_secs(10))
    }

    /// Only the island's shortcut; the other one off.
    fn island(wanted: Wanted) -> [Wanted; 2] {
        [wanted, Wanted::Off]
    }

    fn status(which: Which, status: HotkeyStatus) -> Result<Event, RecvTimeoutError> {
        Ok(Event::Status(which, status))
    }

    #[test]
    fn the_hotkey_thread_registers_releases_registers_again_and_ends_cleanly() {
        // Never Alt+M: the running dev Mayak holds it. F21–F24 are for these tests, one each.
        let combo = parse("Ctrl+Alt+Shift+F24").unwrap();
        let (registrar, rx) = started();
        registrar.set(island(Wanted::Combo(combo))).unwrap();
        assert_eq!(next(&rx), status(Which::Island, HotkeyStatus::active(&combo)));
        assert_eq!(next(&rx), status(Which::Next, HotkeyStatus::off()));
        registrar.set(island(Wanted::Off)).unwrap();
        assert_eq!(next(&rx), status(Which::Island, HotkeyStatus::off()));
        assert_eq!(next(&rx), status(Which::Next, HotkeyStatus::off()));
        // Registering again on the same thread succeeds only if Off really released it.
        registrar.set(island(Wanted::Combo(combo))).unwrap();
        assert_eq!(next(&rx), status(Which::Island, HotkeyStatus::active(&combo)));
        assert_eq!(next(&rx), status(Which::Next, HotkeyStatus::off()));
        // Recording: released, then registered again and reported.
        registrar.pause(true).unwrap();
        registrar.pause(false).unwrap();
        assert_eq!(next(&rx), status(Which::Island, HotkeyStatus::active(&combo)));
        drop(registrar);
        // The thread is gone: the sink, and its sender, were dropped.
        assert_eq!(next(&rx), Err(RecvTimeoutError::Disconnected));
    }

    #[test]
    fn a_shortcut_held_by_another_thread_is_taken_until_it_lets_go() {
        let combo = parse("Ctrl+Alt+Shift+F23").unwrap();
        let (first, rx1) = started();
        first.set(island(Wanted::Combo(combo))).unwrap();
        assert_eq!(next(&rx1), status(Which::Island, HotkeyStatus::active(&combo)));
        assert_eq!(next(&rx1), status(Which::Next, HotkeyStatus::off()));
        let (second, rx2) = started();
        second.set(island(Wanted::Combo(combo))).unwrap();
        // ERROR_HOTKEY_ALREADY_REGISTERED (1409): taken, without the Windows text (spec §3.2).
        assert_eq!(
            next(&rx2),
            status(
                Which::Island,
                HotkeyStatus { combo: "Ctrl+Alt+Shift+F23".into(), state: HotkeyState::Taken, error: None }
            )
        );
        assert_eq!(next(&rx2), status(Which::Next, HotkeyStatus::off()));
        drop(first);
        assert_eq!(next(&rx1), Err(RecvTimeoutError::Disconnected));
        second.set(island(Wanted::Combo(combo))).unwrap();
        assert_eq!(next(&rx2), status(Which::Island, HotkeyStatus::active(&combo)));
    }

    #[test]
    fn one_thread_holds_both_shortcuts_and_they_can_swap() {
        let a = parse("Ctrl+Alt+Shift+F22").unwrap();
        let b = parse("Ctrl+Alt+Shift+F21").unwrap();
        let (registrar, rx) = started();
        registrar.set([Wanted::Combo(a), Wanted::Combo(b)]).unwrap();
        assert_eq!(next(&rx), status(Which::Island, HotkeyStatus::active(&a)));
        assert_eq!(next(&rx), status(Which::Next, HotkeyStatus::active(&b)));
        // Each takes the other's keys in one request: both are free by the time either registers.
        registrar.set([Wanted::Combo(b), Wanted::Combo(a)]).unwrap();
        assert_eq!(next(&rx), status(Which::Island, HotkeyStatus::active(&b)));
        assert_eq!(next(&rx), status(Which::Next, HotkeyStatus::active(&a)));
        drop(registrar);
        assert_eq!(next(&rx), Err(RecvTimeoutError::Disconnected));
    }
}
