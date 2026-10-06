//! The Windows half of the global shortcut (spec §3.2): the Win32 codes of a `Combo`, and the
//! thread that owns its registration.

use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VK_F1, VK_SPACE,
};

use super::{Combo, Key};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::parse;

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
        assert_eq!(codes("Shift+A"), (0x4004, 0x41));
        assert_eq!(codes("Shift+Z"), (0x4004, 0x5A));
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
}
