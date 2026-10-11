//! macOS: the shortcuts through Carbon's `RegisterEventHotKey`, which needs no permission (the
//! Mac app's own shortcuts use it too). Carbon hotkeys belong to the main thread and its run loop,
//! so every request is handed there with `run_on_main_thread`; a press arrives on that thread too,
//! in `on_hotkey`. `Win` is ⌘ and `Alt` is ⌥.

use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use tauri::AppHandle;

use super::{Combo, HotkeyOs, HotkeyStatus, Key, Pair, Refusal, Wanted, Which};

/// What the main thread hands to the sink, as the Windows thread does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Status(Which, HotkeyStatus),
    Pressed(Which),
}

mod carbon {
    use std::ffi::c_void;

    pub type OSStatus = i32;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct EventHotKeyID {
        pub signature: u32,
        pub id: u32,
    }

    #[repr(C)]
    pub struct EventTypeSpec {
        pub event_class: u32,
        pub event_kind: u32,
    }

    pub type Handler = extern "C" fn(next: *mut c_void, event: *mut c_void, user: *mut c_void) -> OSStatus;

    /// 'keyb', kEventHotKeyPressed.
    pub const CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
    pub const HOTKEY_PRESSED: u32 = 5;
    /// kEventParamDirectObject '----', typeEventHotKeyID 'hkid'.
    pub const PARAM_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
    pub const TYPE_HOTKEY_ID: u32 = u32::from_be_bytes(*b"hkid");
    /// eventHotKeyExistsErr: another app holds the shortcut.
    pub const HOTKEY_EXISTS: OSStatus = -9878;

    pub const CMD: u32 = 1 << 8;
    pub const SHIFT: u32 = 1 << 9;
    pub const OPTION: u32 = 1 << 11;
    pub const CONTROL: u32 = 1 << 12;

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        pub fn GetEventDispatcherTarget() -> *mut c_void;
        pub fn InstallEventHandler(
            target: *mut c_void,
            handler: Handler,
            num_types: usize,
            types: *const EventTypeSpec,
            user: *mut c_void,
            out: *mut *mut c_void,
        ) -> OSStatus;
        pub fn RegisterEventHotKey(
            code: u32,
            modifiers: u32,
            id: EventHotKeyID,
            target: *mut c_void,
            options: u32,
            out: *mut *mut c_void,
        ) -> OSStatus;
        pub fn UnregisterEventHotKey(hotkey: *mut c_void) -> OSStatus;
        pub fn GetEventParameter(
            event: *mut c_void,
            name: u32,
            desired: u32,
            actual: *mut u32,
            size: usize,
            actual_size: *mut usize,
            data: *mut c_void,
        ) -> OSStatus;
    }
}

/// 'MYAK': the signature of Mayak's hotkey ids.
const SIGNATURE: u32 = u32::from_be_bytes(*b"MYAK");

/// The virtual key code (US layout position) of a key; F21–F24 have none on a Mac keyboard.
pub fn key_code(key: Key) -> Option<u32> {
    const LETTERS: [u32; 26] = [
        0, 11, 8, 2, 14, 3, 5, 4, 34, 38, 40, 37, 46, 45, 31, 35, 12, 15, 1, 17, 32, 9, 13, 7, 16, 6,
    ];
    const DIGITS: [u32; 10] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25];
    const F_KEYS: [u32; 20] = [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106, 64, 79, 80, 90];
    match key {
        Key::Letter(c) => LETTERS.get((c as u8).checked_sub(b'A')? as usize).copied(),
        Key::Digit(d) => DIGITS.get(d as usize).copied(),
        Key::F(n) => F_KEYS.get((n as usize).checked_sub(1)?).copied(),
        Key::Space => Some(49),
    }
}

/// Carbon's modifier mask: Ctrl → ⌃, Alt → ⌥, Shift → ⇧, Win → ⌘.
pub fn modifiers(combo: &Combo) -> u32 {
    [(combo.ctrl, carbon::CONTROL), (combo.alt, carbon::OPTION), (combo.shift, carbon::SHIFT), (combo.win, carbon::CMD)]
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(0, |mask, (_, bit)| mask | bit)
}

fn hotkey_id(which: Which) -> u32 {
    which.index() as u32 + 1
}

/// One registration; the ref lives on the main thread only.
struct CarbonHotkey {
    which: Which,
    hotkey: usize,
}

impl HotkeyOs for CarbonHotkey {
    fn register(&mut self, combo: &Combo) -> Result<(), Refusal> {
        let Some(code) = key_code(combo.key) else {
            return Err(Refusal { already_registered: false, message: format!("{} has no key on a Mac keyboard", combo.key) });
        };
        let id = carbon::EventHotKeyID { signature: SIGNATURE, id: hotkey_id(self.which) };
        let mut out: *mut c_void = std::ptr::null_mut();
        let status = unsafe {
            carbon::RegisterEventHotKey(code, modifiers(combo), id, carbon::GetEventDispatcherTarget(), 0, &mut out)
        };
        match status {
            0 => {
                self.hotkey = out as usize;
                Ok(())
            }
            carbon::HOTKEY_EXISTS => Err(Refusal { already_registered: true, message: String::new() }),
            other => Err(Refusal { already_registered: false, message: format!("RegisterEventHotKey failed ({other})") }),
        }
    }

    fn unregister(&mut self) {
        if self.hotkey != 0 {
            let status = unsafe { carbon::UnregisterEventHotKey(self.hotkey as *mut c_void) };
            if status != 0 {
                crate::log::line(format!("hotkey: UnregisterEventHotKey: {status}"));
            }
            self.hotkey = 0;
        }
    }
}

/// The registrations and their state machine, touched on the main thread only.
struct State {
    pair: Pair,
    os: [CarbonHotkey; 2],
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static SINK: OnceLock<Box<dyn Fn(Event) + Send + Sync>> = OnceLock::new();

fn report(statuses: [Option<HotkeyStatus>; 2]) {
    let Some(sink) = SINK.get() else { return };
    for (which, status) in Which::ALL.into_iter().zip(statuses) {
        if let Some(status) = status {
            sink(Event::Status(which, status));
        }
    }
}

extern "C" fn on_hotkey(_next: *mut c_void, event: *mut c_void, _user: *mut c_void) -> carbon::OSStatus {
    let mut id = carbon::EventHotKeyID::default();
    let status = unsafe {
        carbon::GetEventParameter(
            event,
            carbon::PARAM_DIRECT_OBJECT,
            carbon::TYPE_HOTKEY_ID,
            std::ptr::null_mut(),
            std::mem::size_of::<carbon::EventHotKeyID>(),
            std::ptr::null_mut(),
            (&mut id as *mut carbon::EventHotKeyID).cast(),
        )
    };
    if status == 0 && id.signature == SIGNATURE {
        if let (Some(which), Some(sink)) = (Which::ALL.into_iter().find(|w| hotkey_id(*w) == id.id), SINK.get()) {
            sink(Event::Pressed(which));
        }
    }
    0
}

/// The shortcuts' hold on the main thread. Unlike Windows there is no thread of its own: the
/// value only carries the handle requests go through.
pub struct Registrar {
    app: AppHandle,
}

impl Registrar {
    /// Installs the press handler once; every status and press goes to `sink`, on the main thread.
    pub fn start(app: AppHandle, sink: impl Fn(Event) + Send + Sync + 'static) -> std::io::Result<Self> {
        if SINK.set(Box::new(sink)).is_err() {
            return Err(std::io::Error::other("the hotkey handler is already installed"));
        }
        let install = || -> i32 {
            let spec = carbon::EventTypeSpec { event_class: carbon::CLASS_KEYBOARD, event_kind: carbon::HOTKEY_PRESSED };
            let mut handler: *mut c_void = std::ptr::null_mut();
            let status = unsafe {
                carbon::InstallEventHandler(carbon::GetEventDispatcherTarget(), on_hotkey, 1, &spec, std::ptr::null_mut(), &mut handler)
            };
            *STATE.lock().unwrap() = Some(State {
                pair: Pair::default(),
                os: [CarbonHotkey { which: Which::Island, hotkey: 0 }, CarbonHotkey { which: Which::Next, hotkey: 0 }],
            });
            status
        };
        // Called from tauri's `setup`, already on the main thread: waiting there for a task queued
        // to the same thread would never end.
        let result = if objc2::MainThreadMarker::new().is_some() {
            Ok(install())
        } else {
            let (tx, rx) = std::sync::mpsc::sync_channel::<i32>(1);
            app.run_on_main_thread(move || {
                let _ = tx.send(install());
            })
            .map_err(|err| std::io::Error::other(err.to_string()))?;
            rx.recv()
        };
        match result {
            Ok(0) => Ok(Self { app }),
            Ok(status) => Err(std::io::Error::other(format!("InstallEventHandler failed ({status})"))),
            Err(_) => Err(std::io::Error::other(super::THREAD_ENDED)),
        }
    }

    /// Both shortcuts, island first, each a shortcut or none.
    pub fn set(&self, wanted: [Wanted; 2]) -> std::io::Result<()> {
        self.on_main(move |state| report(state.pair.set(wanted, &mut state.os)))
    }

    /// Settings starts recording a shortcut (`true`) or stopped (`false`).
    pub fn pause(&self, on: bool) -> std::io::Result<()> {
        self.on_main(move |state| {
            if on {
                state.pair.pause(&mut state.os);
            } else {
                report(state.pair.resume(&mut state.os));
            }
        })
    }

    fn on_main(&self, f: impl FnOnce(&mut State) + Send + 'static) -> std::io::Result<()> {
        self.app
            .run_on_main_thread(move || {
                if let Some(state) = STATE.lock().unwrap().as_mut() {
                    f(state);
                }
            })
            .map_err(|_| std::io::Error::other(super::THREAD_GONE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::parse;

    #[test]
    fn keys_have_their_mac_codes() {
        assert_eq!(key_code(Key::Letter('A')), Some(0));
        assert_eq!(key_code(Key::Letter('M')), Some(46));
        assert_eq!(key_code(Key::Letter('Z')), Some(6));
        assert_eq!(key_code(Key::Digit(0)), Some(29));
        assert_eq!(key_code(Key::Digit(9)), Some(25));
        assert_eq!(key_code(Key::F(1)), Some(122));
        assert_eq!(key_code(Key::F(20)), Some(90));
        assert_eq!(key_code(Key::F(21)), None);
        assert_eq!(key_code(Key::Space), Some(49));
    }

    #[test]
    fn win_is_command_and_alt_is_option() {
        assert_eq!(modifiers(&parse("Alt+M").unwrap()), carbon::OPTION);
        assert_eq!(modifiers(&parse("Win+Shift+Space").unwrap()), carbon::CMD | carbon::SHIFT);
        assert_eq!(modifiers(&parse("Ctrl+Alt+K").unwrap()), carbon::CONTROL | carbon::OPTION);
    }
}
