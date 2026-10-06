//! The global shortcut that shows and hides the island (spec 2026-10-06-mayak-hotkey-design.md).
//! The shared part: what a shortcut is and how it is read and written, the registration state
//! machine, the status Settings shows. The Windows registration is in `win.rs`; macOS and Linux
//! answer «not supported» until they are ported (spec §3.3).

#![cfg_attr(not(windows), allow(dead_code))]

use std::fmt;

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
}
