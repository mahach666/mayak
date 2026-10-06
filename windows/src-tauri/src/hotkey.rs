//! The global shortcut that shows and hides the island (spec 2026-10-06-mayak-hotkey-design.md).
//! The shared part: what a shortcut is and how it is read and written, the registration state
//! machine, the status Settings shows. The Windows registration is in `win.rs`; macOS and Linux
//! answer «not supported» until they are ported (spec §3.3).

#![cfg_attr(not(windows), allow(dead_code))]

use std::fmt;

use serde::Serialize;

#[cfg(windows)]
mod win;

/// What a new settings.json, and one written before the shortcut existed, get (spec §2).
pub const DEFAULT_COMBO: &str = "Alt+M";

/// The key of a shortcut: a letter `A`–`Z` (upper case), a digit, `F1`–`F24` or Space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Letter(char),
    Digit(u8),
    F(u8),
    Space,
}

/// A shortcut: its modifiers and one key (spec §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: Key,
}

/// Why a text is not a shortcut (spec §2). `Empty` is not a mistake: it means off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComboError {
    Empty,
    UnknownKey,
    UnknownModifier,
    DuplicateModifier,
    NoModifier,
    TwoKeys,
}

impl fmt::Display for ComboError {
    /// The `invalid` text Settings shows under the shortcut (spec §5).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "No shortcut.",
            Self::UnknownKey => "Unknown key: use a letter, a digit, F1–F24 or Space.",
            Self::UnknownModifier => "Unknown modifier: use Ctrl, Alt, Shift or Win.",
            Self::DuplicateModifier => "A modifier appears twice.",
            Self::NoModifier => "Add Ctrl, Alt, Shift or Win: only F1–F24 work alone.",
            Self::TwoKeys => "Only one key besides the modifiers.",
        })
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Letter(c) => write!(f, "{c}"),
            Self::Digit(d) => write!(f, "{d}"),
            Self::F(n) => write!(f, "F{n}"),
            Self::Space => f.write_str("Space"),
        }
    }
}

impl fmt::Display for Combo {
    /// The normal form of spec §2: `Ctrl`, `Alt`, `Shift`, `Win`, then the key, joined by `+`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift"), (self.win, "Win")] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

/// Reads a shortcut (spec §2). Modifiers may come in any order and any case, with spaces around
/// `+`; the normal form is `parse(text)?.to_string()`.
pub fn parse(text: &str) -> Result<Combo, ComboError> {
    if text.trim().is_empty() {
        return Err(ComboError::Empty);
    }
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let last = parts.len() - 1;
    let mut combo = Combo { ctrl: false, alt: false, shift: false, win: false, key: Key::Space };
    let mut key = None;
    for (i, part) in parts.iter().enumerate() {
        if let Some(flag) = modifier(&mut combo, part) {
            if *flag {
                return Err(ComboError::DuplicateModifier);
            }
            *flag = true;
        } else if let Some(k) = key_of(part) {
            if key.is_some() {
                return Err(ComboError::TwoKeys);
            }
            key = Some(k);
        } else {
            // The last part is where the key goes; anything unknown before it was meant as a modifier.
            return Err(if i == last { ComboError::UnknownKey } else { ComboError::UnknownModifier });
        }
    }
    let Some(key) = key else { return Err(ComboError::UnknownKey) };
    let modified = combo.ctrl || combo.alt || combo.shift || combo.win;
    // A bare letter, digit or Space would take over ordinary typing.
    if !modified && !matches!(key, Key::F(_)) {
        return Err(ComboError::NoModifier);
    }
    Ok(Combo { key, ..combo })
}

/// The flag of `combo` that a modifier name sets, any case; `None` when `part` is not one.
fn modifier<'a>(combo: &'a mut Combo, part: &str) -> Option<&'a mut bool> {
    match part.to_ascii_lowercase().as_str() {
        "ctrl" => Some(&mut combo.ctrl),
        "alt" => Some(&mut combo.alt),
        "shift" => Some(&mut combo.shift),
        "win" => Some(&mut combo.win),
        _ => None,
    }
}

/// `A`–`Z`, `0`–`9`, `F1`–`F24` or `Space`, any case. Anything else, the Russian «Ь» included,
/// is no key.
fn key_of(part: &str) -> Option<Key> {
    let upper = part.to_ascii_uppercase();
    if upper == "SPACE" {
        return Some(Key::Space);
    }
    match upper.as_bytes() {
        [c @ b'A'..=b'Z'] => Some(Key::Letter(char::from(*c))),
        [d @ b'0'..=b'9'] => Some(Key::Digit(d - b'0')),
        [b'F', digits @ ..] => {
            let digits = std::str::from_utf8(digits).ok()?;
            let n: u8 = digits.parse().ok()?;
            // Only the plain number: `F05` is not how anyone writes F5.
            ((1..=24).contains(&n) && digits == n.to_string()).then_some(Key::F(n))
        }
        _ => None,
    }
}

// ── Status and registration (spec §3) ─────────────────────────────────────────

/// `state` of a `HotkeyStatus`, lower case on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyState {
    Off,
    Active,
    Taken,
    Invalid,
    Unsupported,
}

/// What Settings shows under the shortcut (spec §3.1): `hotkey_status` and `hotkey-status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HotkeyStatus {
    pub combo: String,
    pub state: HotkeyState,
    pub error: Option<String>,
}

impl HotkeyStatus {
    pub fn off() -> Self {
        Self { combo: String::new(), state: HotkeyState::Off, error: None }
    }

    pub fn invalid(text: &str, error: ComboError) -> Self {
        Self { combo: text.to_string(), state: HotkeyState::Invalid, error: Some(error.to_string()) }
    }

    pub fn active(combo: &Combo) -> Self {
        Self { combo: combo.to_string(), state: HotkeyState::Active, error: None }
    }

    /// Held by another program (spec §3.2): no text for ERROR_HOTKEY_ALREADY_REGISTERED, the
    /// OS's own text for any other refusal.
    pub fn taken(combo: &Combo, refusal: Refusal) -> Self {
        Self {
            combo: combo.to_string(),
            state: HotkeyState::Taken,
            error: (!refusal.already_registered).then_some(refusal.message),
        }
    }
}

/// What `apply` asks of the registration: nothing, nothing because the text is wrong, or a shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanted {
    Off,
    Invalid { text: String, error: ComboError },
    Combo(Combo),
}

impl Wanted {
    /// The `hotkey` setting as a request: empty is off (spec §2).
    pub fn from_text(text: &str) -> Self {
        match parse(text) {
            Ok(combo) => Self::Combo(combo),
            Err(ComboError::Empty) => Self::Off,
            Err(error) => Self::Invalid { text: text.to_string(), error },
        }
    }
}

/// Why the OS refused a registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// ERROR_HOTKEY_ALREADY_REGISTERED: another program holds the shortcut.
    pub already_registered: bool,
    /// The OS's own text, shown when the refusal is anything else.
    pub message: String,
}

/// The platform's hold on one shortcut, used only from the thread that owns it.
pub trait HotkeyOs {
    fn register(&mut self, combo: &Combo) -> Result<(), Refusal>;
    /// Releases what `register` took; only called after it succeeded.
    fn unregister(&mut self);
}

/// The registration state machine of spec §3.2, apart from the OS: one shortcut, released while
/// Settings records a new one (spec §5). Every request is answered in order, so the status of an
/// older request never lands after a newer one.
#[derive(Debug, Default)]
pub struct Slot {
    combo: Option<Combo>,
    paused: bool,
    registered: bool,
}

impl Slot {
    /// A new request; the old shortcut is released first. `None`: a shortcut set while paused is
    /// registered, and reported, when the pause ends.
    pub fn set(&mut self, wanted: Wanted, os: &mut impl HotkeyOs) -> Option<HotkeyStatus> {
        self.release(os);
        match wanted {
            Wanted::Off => {
                self.combo = None;
                Some(HotkeyStatus::off())
            }
            Wanted::Invalid { text, error } => {
                self.combo = None;
                Some(HotkeyStatus::invalid(&text, error))
            }
            Wanted::Combo(combo) => {
                self.combo = Some(combo);
                if self.paused { None } else { Some(self.register(combo, os)) }
            }
        }
    }

    /// Recording starts: the keys must reach Settings instead of toggling the island.
    pub fn pause(&mut self, os: &mut impl HotkeyOs) {
        self.paused = true;
        self.release(os);
    }

    /// Recording ends (saved, Esc, Off, window left, page loaded): the shortcut is registered
    /// again, once. A resume without a pause does nothing.
    pub fn resume(&mut self, os: &mut impl HotkeyOs) -> Option<HotkeyStatus> {
        if !self.paused {
            return None;
        }
        self.paused = false;
        let combo = self.combo?;
        Some(self.register(combo, os))
    }

    /// Lets go of the shortcut if it is held.
    pub fn release(&mut self, os: &mut impl HotkeyOs) {
        if self.registered {
            os.unregister();
            self.registered = false;
        }
    }

    fn register(&mut self, combo: Combo, os: &mut impl HotkeyOs) -> HotkeyStatus {
        match os.register(&combo) {
            Ok(()) => {
                self.registered = true;
                HotkeyStatus::active(&combo)
            }
            Err(refusal) => HotkeyStatus::taken(&combo, refusal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The normal form of spec §2: `to_string(parse(x))`.
    fn normalize(text: &str) -> Result<String, ComboError> {
        parse(text).map(|c| c.to_string())
    }

    fn combo(ctrl: bool, alt: bool, shift: bool, win: bool, key: Key) -> Combo {
        Combo { ctrl, alt, shift, win, key }
    }

    #[test]
    fn ordinary_shortcuts_parse_into_modifiers_and_one_key() {
        assert_eq!(parse("Alt+M"), Ok(combo(false, true, false, false, Key::Letter('M'))));
        assert_eq!(parse("ctrl + shift+m"), Ok(combo(true, false, true, false, Key::Letter('M'))));
        assert_eq!(parse("F5"), Ok(combo(false, false, false, false, Key::F(5))));
        assert_eq!(parse("Win+Space"), Ok(combo(false, false, false, true, Key::Space)));
        assert_eq!(parse("Ctrl+Alt+9"), Ok(combo(true, true, false, false, Key::Digit(9))));
    }

    #[test]
    fn every_malformed_shortcut_names_its_error() {
        use ComboError::*;
        for (text, error) in [
            ("", Empty),
            ("   ", Empty),
            ("Alt+Esc", UnknownKey),
            // a character, not a key: the Russian layout's «Ь» is written `Alt+M`
            ("Alt+Ь", UnknownKey),
            ("Alt+", UnknownKey),
            ("Ctrl+Alt", UnknownKey),
            ("F25", UnknownKey),
            ("F0", UnknownKey),
            ("Alt+F05", UnknownKey),
            ("Cmd+M", UnknownModifier),
            ("+M", UnknownModifier),
            ("Alt+alt+M", DuplicateModifier),
            ("M", NoModifier),
            ("5", NoModifier),
            ("Space", NoModifier),
            ("Alt+M+K", TwoKeys),
        ] {
            assert_eq!(parse(text), Err(error), "{text:?}");
        }
    }

    #[test]
    fn the_normal_form_orders_the_modifiers_and_reads_back_unchanged() {
        for (text, normal) in [
            ("ctrl + shift+m", "Ctrl+Shift+M"),
            ("shift+ALT+ctrl+win+f12", "Ctrl+Alt+Shift+Win+F12"),
            ("m + alt", "Alt+M"),
            (" win + space ", "Win+Space"),
            ("f24", "F24"),
            ("Ctrl+Alt+9", "Ctrl+Alt+9"),
        ] {
            assert_eq!(normalize(text).as_deref(), Ok(normal), "{text:?}");
            // round trip: the normal form is a fixed point of parse → to_string
            assert_eq!(normalize(normal).as_deref(), Ok(normal), "{normal:?}");
        }
    }

    #[test]
    fn the_default_is_alt_m_and_every_error_reads_as_a_sentence() {
        assert_eq!(normalize(DEFAULT_COMBO).as_deref(), Ok("Alt+M"));
        use ComboError::*;
        for error in [Empty, UnknownKey, UnknownModifier, DuplicateModifier, NoModifier, TwoKeys] {
            let text = error.to_string();
            assert!(text.len() > 4 && text.ends_with('.'), "{error:?}: {text:?}");
        }
    }

    /// The OS side of a slot, in memory: what it holds, and what the next `register` answers.
    #[derive(Default)]
    struct FakeOs {
        held: Option<Combo>,
        refuse: Option<Refusal>,
    }

    impl HotkeyOs for FakeOs {
        fn register(&mut self, combo: &Combo) -> Result<(), Refusal> {
            assert!(self.held.is_none(), "registered {combo} over {:?}", self.held);
            if let Some(refusal) = self.refuse.clone() {
                return Err(refusal);
            }
            self.held = Some(*combo);
            Ok(())
        }

        fn unregister(&mut self) {
            assert!(self.held.take().is_some(), "released a shortcut that was not held");
        }
    }

    fn wanted(text: &str) -> Wanted {
        Wanted::from_text(text)
    }

    fn active(text: &str) -> Option<HotkeyStatus> {
        Some(HotkeyStatus::active(&parse(text).unwrap()))
    }

    fn held(text: &str) -> Option<Combo> {
        Some(parse(text).unwrap())
    }

    #[test]
    fn a_new_shortcut_replaces_the_old_one() {
        let (mut slot, mut os) = (Slot::default(), FakeOs::default());
        assert_eq!(slot.set(wanted("Alt+M"), &mut os), active("Alt+M"));
        assert_eq!(os.held, held("Alt+M"));
        assert_eq!(slot.set(wanted("ctrl+alt+k"), &mut os), active("Ctrl+Alt+K"));
        assert_eq!(os.held, held("Ctrl+Alt+K"));
    }

    #[test]
    fn turning_the_shortcut_off_or_breaking_it_releases_the_old_one() {
        let (mut slot, mut os) = (Slot::default(), FakeOs::default());
        slot.set(wanted("Alt+M"), &mut os);
        assert_eq!(slot.set(wanted(""), &mut os), Some(HotkeyStatus::off()));
        assert_eq!(os.held, None);
        slot.set(wanted("Alt+M"), &mut os);
        assert_eq!(
            slot.set(wanted("Alt+Esc"), &mut os),
            Some(HotkeyStatus {
                combo: "Alt+Esc".into(),
                state: HotkeyState::Invalid,
                error: Some(ComboError::UnknownKey.to_string()),
            })
        );
        assert_eq!(os.held, None);
    }

    #[test]
    fn a_shortcut_saved_while_recording_is_registered_when_recording_ends() {
        let (mut slot, mut os) = (Slot::default(), FakeOs::default());
        slot.set(wanted("Alt+M"), &mut os);
        slot.pause(&mut os);
        assert_eq!(os.held, None, "recording: the keys must reach Settings");
        assert_eq!(slot.set(wanted("Ctrl+Alt+K"), &mut os), None);
        assert_eq!(os.held, None, "still recording");
        assert_eq!(slot.resume(&mut os), active("Ctrl+Alt+K"));
        assert_eq!(os.held, held("Ctrl+Alt+K"));
    }

    #[test]
    fn cancelling_a_recording_brings_the_old_shortcut_back_once() {
        let (mut slot, mut os) = (Slot::default(), FakeOs::default());
        // Settings resumes when its page loads, with nothing paused: nothing happens.
        assert_eq!(slot.resume(&mut os), None);
        slot.set(wanted("Alt+M"), &mut os);
        assert_eq!(slot.resume(&mut os), None);
        slot.pause(&mut os);
        slot.pause(&mut os);
        assert_eq!(slot.resume(&mut os), active("Alt+M"));
        // Esc, then the window losing focus: the second resume must not register again.
        assert_eq!(slot.resume(&mut os), None);
        assert_eq!(os.held, held("Alt+M"));
    }

    #[test]
    fn a_taken_shortcut_is_reported_and_tried_again_after_a_recording() {
        let (mut slot, mut os) = (Slot::default(), FakeOs::default());
        os.refuse = Some(Refusal { already_registered: true, message: "Hot key is already registered.".into() });
        assert_eq!(
            slot.set(wanted("Alt+M"), &mut os),
            Some(HotkeyStatus { combo: "Alt+M".into(), state: HotkeyState::Taken, error: None })
        );
        os.refuse = Some(Refusal { already_registered: false, message: "Access is denied.".into() });
        assert_eq!(
            slot.set(wanted("Ctrl+Alt+K"), &mut os),
            Some(HotkeyStatus { combo: "Ctrl+Alt+K".into(), state: HotkeyState::Taken, error: Some("Access is denied.".into()) })
        );
        assert_eq!(os.held, None);
        // The other program let go: the end of a recording tries again.
        os.refuse = None;
        slot.pause(&mut os);
        assert_eq!(slot.resume(&mut os), active("Ctrl+Alt+K"));
        assert_eq!(os.held, held("Ctrl+Alt+K"));
    }

    #[test]
    fn off_while_recording_is_reported_at_once_and_nothing_comes_back() {
        let (mut slot, mut os) = (Slot::default(), FakeOs::default());
        slot.set(wanted("Alt+M"), &mut os);
        slot.pause(&mut os);
        assert_eq!(slot.set(wanted(""), &mut os), Some(HotkeyStatus::off()));
        assert_eq!(slot.resume(&mut os), None);
        assert_eq!(os.held, None);
    }
}
