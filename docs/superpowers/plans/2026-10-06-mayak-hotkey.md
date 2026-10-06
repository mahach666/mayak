# Mayak: global hotkey for the island Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One key combination (`Alt+M` by default), pressed in any program, opens the island on the session list, or folds an open island into the strip. Settings → General can record a new combination or turn it off, and says when another program holds it.

**Architecture:** A new module `hotkey.rs` holds the part that does not depend on the OS. It has the pure combo parser and its normal form, the status type, and `Slot`, the registration state machine (register, release, pause, resume). `Slot` talks to the OS only through a small `HotkeyOs` trait, so it is unit-tested. `apply` and the two commands also live there. The Windows part is `hotkey/win.rs` (same layout as `focus/mod.rs` + `focus/win.rs`). It turns a combo into `MOD_*`/VK codes and runs one thread for the whole life of the app. That thread owns the `RegisterHotKey(NULL, …)` registration and sleeps in `GetMessageW`. It receives each new combo through a channel plus `WM_APP + 1`, and pause/resume as `WM_APP + 2/3`. It reports every status and every press to a sink. The sink calls `app.emit`/`app.emit_to`, which do not block. In the island, the TS handler of `hotkey` either calls `fsm.forceHidden()` or `alert("sessions")`. Settings records the combination from `KeyboardEvent.code` while the registration is paused.

**Tech Stack:** Rust (Tauri 2, `windows` crate 0.61.3: `Win32_UI_Input_KeyboardAndMouse` is already enabled, so there are no new features and no new crates), TypeScript (no new packages, no test runner).

**Spec:** `docs/superpowers/specs/2026-10-06-mayak-hotkey-design.md` (in Russian; text added to the spec is also in Russian). Style model: `docs/superpowers/plans/2026-10-06-mayak-island-strip.md` and its `strip.rs` (thread with its own message loop, a ready handshake, `Drop` that posts `WM_QUIT` and joins, and a lifecycle test that ends in `Disconnected`).

## Global Constraints

- Toggle (spec §1, §4): from `hidden` (strip), `compact` (`petit`) or `coucou` (greeting), `alert("sessions")`, which opens on the session list like the tray's «Open», with the usual open sound. From `expanded` (`home`), on any view, `fsm.forceHidden()` into the strip. The «collapsed by hand» flag is the collapse button's flag: `holdVisible` does not hold the island until the next event.
- Quiet mode (pause) does not affect the key. A pinned island waiting for an agent's answer still folds. The next event opens it again (spec §4).
- Default `"Alt+M"`; both Alt keys work (`MOD_ALT` covers left and right Alt). An empty string means off. Field `hotkey` (camelCase in JSON). Settings written by an older build read as `"Alt+M"` (`#[serde(default)]`) (spec §2).
- Format (spec §2): modifiers, then the key, joined by `+`. Modifiers are `Ctrl`, `Alt`, `Shift`, `Win`, in that order, each at most once. Keys: letters `A`–`Z`, digits `0`–`9`, `F1`–`F24`, `Space`. At least one modifier, except `F1`–`F24`. Case does not matter and spaces around `+` are allowed. `ctrl + shift+m` → `Ctrl+Shift+M`.
- The key is a virtual key code, not a character: `'M'` = 0x4D, `F1` = 0x70, `Space` = 0x20. So the combination works in every layout; on a Russian layout this key is «Ь» (spec §2).
- `parse(&str) -> Result<Combo, ComboError>`, `Combo::to_string()`, `Combo { ctrl, alt, shift, win: bool, key: Key }`. Errors: `Empty` (not an error: means off), `UnknownKey`, `UnknownModifier`, `DuplicateModifier`, `NoModifier`, `TwoKeys` (spec §2).
- `HotkeyStatus { combo: String, state: "off" | "active" | "taken" | "invalid" | "unsupported", error: Option<String> }`: returned by the command `hotkey_status` and sent as the event `hotkey-status` on every change. A press reaches the island as the event `hotkey` with no payload (spec §3.1).
- Windows (spec §3.2): one thread for the whole life of the app, `GetMessageW`. `RegisterHotKey(NULL, id, MOD_* | MOD_NOREPEAT, vk)` is called from that same thread. Change of combination: `PostThreadMessageW(WM_APP + 1)` with the new `Combo` (through a channel). Pause: `WM_APP + 2` releases, `WM_APP + 3` restores. `ERROR_HOTKEY_ALREADY_REGISTERED` (1409) → `taken`; any other error → `taken` with the Windows error text.
- `cfg(not(windows))`: `apply` returns `unsupported` («Global shortcut is not available on this platform yet»), and Settings shows that instead of the field (spec §3.3).
- Settings → General (spec §5): row «Show / hide island». Button showing the combination (`Alt+M`) or `Off`; a click starts recording: «Press keys…», with `hotkey_pause(true)` until it ends. The first valid press saves the normal form and calls `hotkey_pause(false)`. `Esc` cancels with no change. An invalid combination (a bare letter) shows a hint and keeps waiting. The «Off» button clears. Status line: `taken` — «Alt+M is taken by another app — choose another»; `invalid` — the error text; `unsupported` — the platform message; `active` and `off` — nothing.
- 0 % CPU at rest: the registration thread sleeps in `GetMessageW` and wakes only on a press or a request. No timers, no polling.
- No new crates; only `windows` crate features could be added (none are needed). Existing views are not restyled. The only new UI is the Settings → General row, built from the existing classes `row`, `hint` and the plain `button` style.
- Out of scope: several combinations for different actions; local keys inside the island; macOS and Linux implementations (spec §1).
- Build environment: always `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target`. **Never** touch `windows/target/release`. A dev Mayak runs under `tauri dev` from this working tree and rebuilds on every file change: do not stop it. When cargo prints `Blocking waiting for file lock`, it is waiting for that rebuild; let it wait. Front-end checks are `npx tsc --noEmit` and `npx vite build` (not `npm run build`: its `prebuild` runs cargo).
- Tests never register `Alt+M`, which the running dev Mayak holds. The thread tests use `Ctrl+Alt+Shift+F24` and `Ctrl+Alt+Shift+F23`, one combo per test, because cargo runs tests in parallel.
- Baseline before this plan (measured 2026-10-06 on `feature/hotkey` at `c1ab209`): `cargo test --workspace` = **390 lib passed (1 ignored) + 32 hook passed**, 0 failed, 0 warnings.
  `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|warning"`
- Keep each existing file's line endings (the working tree is CRLF, `core.autocrlf=true`; check with `file`). Edit in place, never rewrite whole files. New files may be written with LF; git normalizes them.
- Commit messages: conventional, one line, **no trailers** (no Co-Authored-By, no «Generated with»). Never mention an AI assistant anywhere: code, comments, docs, PR text. Never push without the user's explicit yes (Task 6 asks).
- Branch: `feature/hotkey` (spec at `c1ab209`).
- Only the live run can verify these (Task 6): a real `WM_HOTKEY` from the keyboard in another program, left and right Alt, the Russian layout, holding the keys, recording in the real Settings window, a combination another program really holds, and CPU.

## Review Focus

1. Recording in Settings: a combination saved while the old one is released must be registered when recording ends. `Esc`, leaving the window or reloading the page must bring the old one back exactly once and never register twice. Pinned by `a_shortcut_saved_while_recording_is_registered_when_recording_ends` and `cancelling_a_recording_brings_the_old_shortcut_back_once` in Task 2.
2. «Off», or a hand-edited value that does not parse, after a working combination must release the old one. Otherwise `Alt+M` keeps toggling after the user turned it off. Pinned by `turning_the_shortcut_off_or_breaking_it_releases_the_old_one` in Task 2.
3. A combination another program holds: `taken` with no text for 1409, any other refusal with its Windows text, and a retry once recording ends. Pinned by `a_taken_shortcut_is_reported_and_tried_again_after_a_recording` (pure) and `a_shortcut_held_by_another_thread_is_taken_until_it_lets_go` (real `RegisterHotKey`) in Task 2.
4. Upgrade: a settings.json from an older build has no `hotkey` key and loads `Alt+M`. A user's `""` (off) must stay off through save and load, never come back as `Alt+M`. Pinned by `an_older_settings_file_gets_alt_m_and_an_empty_one_stays_off` in Task 1.
5. Holding the keys must toggle once (spec acceptance 2): every registration, for every key, carries `MOD_NOREPEAT`. Pinned by `holding_the_keys_never_repeats_the_toggle` in Task 1.

---

## File Structure

- Create `windows/src-tauri/src/hotkey.rs`: `Key`, `Combo`, `ComboError`, `parse`, `Display`, `DEFAULT_COMBO` (Task 1). `HotkeyState`, `HotkeyStatus`, `Wanted`, `Refusal`, `HotkeyOs`, `Slot` (Task 2). `apply`, `publish`, `hotkey_status`, `hotkey_pause`, `unsupported`/`failed` (Task 3).
- Create `windows/src-tauri/src/hotkey/win.rs`: `win32_codes` (Task 1); the thread, `Event`, `Registrar` and its `Drop` (Task 2).
- Modify `windows/src-tauri/src/settings.rs`: the `hotkey` field, `default_hotkey`, `Default`, one test (Task 1).
- Modify `windows/src-tauri/src/lib.rs`: `mod hotkey;` (Task 1); `save_settings`, `setup`, `generate_handler!` (Task 3).
- Modify `windows/src/core/bridge.ts`, `windows/src/core/state.ts`, `windows/src/island/island.ts`, `windows/src/main.ts` (Task 4).
- Modify `windows/src/settings/main.ts`, `windows/README.md` (Task 5).
- Modify `docs/superpowers/specs/2026-10-06-mayak-hotkey-design.md`, `.git/mayak-pr-notes.md` (local, untracked) (Task 6).
- Not modified: `windows/src-tauri/Cargo.toml`. `Win32_UI_Input_KeyboardAndMouse` (for `RegisterHotKey`, `MOD_*`, `VK_*`) is already in the `windows` feature list (line 40), and `Win32_UI_WindowsAndMessaging`, `Win32_System_Threading` and `Win32_Foundation` are present too.

**Task shape.** The suggested shape is kept, with two placements made explicit:
- `HotkeyStatus` and `Slot` (the pause/re-register state machine) go into Task 2 with the thread that drives them. They are pure and unit-tested there; only the OS calls stay in `win.rs`.
- Task 6 also aligns the spec with the decisions below, the way the strip plan did.

**Decisions where the spec is silent or loose** (Task 6 writes them into the spec):
1. *Files:* `hotkey.rs` + `hotkey/win.rs`, following `focus/`, rather than one file or `platform/windows.rs`. `Cargo.toml` is unchanged (see above).
2. *Parse:* modifiers are accepted in any order (`m + alt` → `Alt+M`). An unknown last part is `UnknownKey`; an unknown earlier part is `UnknownModifier`; no key at all (`Ctrl+Alt`, `Alt+`) is `UnknownKey`. `normalize` is no separate function: the normal form is `parse(x)?.to_string()` (the tests define `normalize` that way).
3. *Order of statuses:* every status except `unsupported` comes from the registration thread, one per request, in request order (`Slot`). A late answer to an old request can never overwrite a newer one. An empty string and a parse error both release the previous combination (spec §3.1 says «ничего не регистрирует»).
4. *Pause:* a combination saved while paused is registered, and reported, at `hotkey_pause(false)`. A resume without a pause does nothing. Settings resumes after saving, on `Esc`, on «Off», when the window loses focus or is hidden, and when its page loads. That way a reload or a close in the middle of recording cannot leave the combination released.
5. *Errors outside `RegisterHotKey`:* a failure to start the thread or to hand it a request becomes `taken` with the error text (spec: «any other error → taken with text»). On macOS/Linux, every value is `unsupported`, so Settings never shows a field that would do nothing.
6. *Events:* `hotkey` goes only to the island window (`emit_to`); `hotkey-status` goes to every window, because Settings shows it. Before the first status, `hotkey_status` answers `off`.
7. *Island:* unlike the tray's «Open», the key does not lift the pause (spec §4: pause does not affect the key). It opens `"sessions"` as the spec says, not `State.defaultView()`. Folding clears both pin flags (`State.isPinned`, `fsm.pinned`) and sets `fsm.collapsedByButton`.
8. *Recording:* the key is read from `KeyboardEvent.code` (the physical key, like a VK), so recording does not depend on the layout. Numpad digits are not `0`–`9` and get the «use a letter…» hint.

---

### Task 1: Combo parsing, normal form, Win32 codes, and the `hotkey` setting

**Files:**
- Create: `windows/src-tauri/src/hotkey.rs`
- Create: `windows/src-tauri/src/hotkey/win.rs`
- Modify: `windows/src-tauri/src/lib.rs` (module list: after `mod hooks;`, line 8)
- Modify: `windows/src-tauri/src/settings.rs` (struct after `alert_error`, line 34; `default_true` at line 41; `Default` at line 63; tests at the end)

**Interfaces:**
- Consumes: nothing new.
- Produces (later tasks rely on these exact names):
  `pub const hotkey::DEFAULT_COMBO: &str = "Alt+M"`
  `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum hotkey::Key { Letter(char), Digit(u8), F(u8), Space }`: `Letter` is upper case `'A'..='Z'`, `Digit` is `0..=9`, `F` is `1..=24`
  `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct hotkey::Combo { pub ctrl: bool, pub alt: bool, pub shift: bool, pub win: bool, pub key: Key }`, `impl Display` (the normal form, so `combo.to_string()`)
  `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum hotkey::ComboError { Empty, UnknownKey, UnknownModifier, DuplicateModifier, NoModifier, TwoKeys }`, `impl Display` (one sentence ending in `.`)
  `pub fn hotkey::parse(text: &str) -> Result<Combo, ComboError>`
  `pub fn hotkey::win::win32_codes(combo: &Combo) -> (HOT_KEY_MODIFIERS, u32)`: modifiers always include `MOD_NOREPEAT`; the second value is the VK
  `Settings.hotkey: String` (JSON `hotkey`, default `"Alt+M"`, `""` = off)

- [ ] **Step 1: The module skeleton with its tests**

`windows/src-tauri/src/hotkey.rs`:

```rust
//! The global shortcut that shows and hides the island (spec 2026-10-06-mayak-hotkey-design.md).
//! The shared part: what a shortcut is and how it is read and written, the registration state
//! machine, the status Settings shows. The Windows registration is in `win.rs`; macOS and Linux
//! answer «not supported» until they are ported (spec §3.3).

#![cfg_attr(not(windows), allow(dead_code))]

use std::fmt;

#[cfg(windows)]
mod win;

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
```

`windows/src-tauri/src/hotkey/win.rs`:

```rust
//! The Windows half of the global shortcut (spec §3.2): the Win32 codes of a `Combo`, and the
//! thread that owns its registration.

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
```

In `lib.rs`, after `mod hooks;` add:

```rust
#[allow(dead_code)] // wired into setup and save_settings in Task 3; remove this attribute there
mod hotkey;
```

- [ ] **Step 2: The settings test**

Append inside `mod tests` of `settings.rs` (after `hooks_installed_is_no_longer_stored`):

```rust
    #[test]
    fn an_older_settings_file_gets_alt_m_and_an_empty_one_stays_off() {
        // Exactly the keys a settings.json from before the shortcut has.
        let old = r#"{"soundEnabled":true,"soundVolume":0.12,"autoCloseInterval":15,"absenceInterval":180,"activeIntegrations":["integration_github"],"screen":"primary","autostart":false,"model":"claude-opus-5","alertWaiting":true,"alertFinished":true,"alertError":true}"#;
        let s: super::Settings = serde_json::from_str(old).expect("an older settings.json must still load");
        assert_eq!(s.hotkey, "Alt+M");
        assert_eq!(super::Settings::default().hotkey, "Alt+M");
        // Off is the empty string, and it stays off through a save and a load.
        let mut off = super::Settings::default();
        off.hotkey = String::new();
        let v = serde_json::to_value(&off).unwrap();
        assert_eq!(v["hotkey"], "");
        let back: super::Settings = serde_json::from_value(v).unwrap();
        assert_eq!(back.hotkey, "");
    }
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak hotkey:: 2>&1 | grep -E "^error" | sort -u | head -12`
Expected: FAIL to compile, with ``error[E0425]: cannot find function `parse` in this scope``, ``cannot find type `Combo` ``/`` `Key` ``/`` `ComboError` ``, ``cannot find value `DEFAULT_COMBO` ``, ``cannot find function `win32_codes` `` and ``no field `hotkey` on type `Settings` ``.

- [ ] **Step 4: Implement the pure part of `hotkey.rs`**

Insert between `mod win;` and `#[cfg(test)]` in `hotkey.rs`:

```rust
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
```

- [ ] **Step 5: Implement `win32_codes`**

Insert at the top of `hotkey/win.rs`, between the module doc and `#[cfg(test)]`:

```rust
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
```

- [ ] **Step 6: The `hotkey` setting**

In `settings.rs`, after the `alert_error` field (`pub alert_error: bool,`) add:

```rust
    /// Global shortcut that shows and hides the island (hotkey spec §2): `Alt+M` unless the
    /// user changed it, empty = off. A settings.json from before it existed loads with the default.
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
```

after `fn default_true() -> bool { … }` add:

```rust
fn default_hotkey() -> String {
    crate::hotkey::DEFAULT_COMBO.to_string()
}
```

and in `impl Default for Settings`, after `alert_error: true,` add `hotkey: default_hotkey(),`.

- [ ] **Step 7: Run the tests to see them pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak hotkey:: 2>&1 | grep -E "^test |test result: ok. [1-9]|warning"` and `cargo test -p mayak settings::tests 2>&1 | grep -E "test result: ok. [1-9]"`
Expected: 7 tests `ok` (4 `hotkey::tests`, 3 `hotkey::win::tests`), `test result: ok. 7 passed`; `settings::tests` `ok. 7 passed`; no warning. Then `cargo test --workspace 2>&1 | grep -E "^test result|warning"`: lib **398 passed** (1 ignored), hook 32 passed, 0 warnings.

- [ ] **Step 8: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/hotkey.rs windows/src-tauri/src/hotkey/win.rs windows/src-tauri/src/lib.rs windows/src-tauri/src/settings.rs && git commit -m "feat(hotkey): parse, normal form and Win32 codes of the global shortcut, and its setting"
```

---

### Task 2: The registration state machine and the Windows hotkey thread

**Files:**
- Modify: `windows/src-tauri/src/hotkey.rs` (status, request and `Slot` between `key_of` and `#[cfg(test)]`; six tests)
- Modify: `windows/src-tauri/src/hotkey/win.rs` (thread part between `win32_codes` and `#[cfg(test)]`; two tests)

**Interfaces:**
- Consumes: Task 1 (`Combo`, `ComboError`, `parse`, `win32_codes`); `crate::log::line`.
- Produces:
  `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] #[serde(rename_all = "lowercase")] pub enum hotkey::HotkeyState { Off, Active, Taken, Invalid, Unsupported }`
  `#[derive(Debug, Clone, PartialEq, Eq, Serialize)] pub struct hotkey::HotkeyStatus { pub combo: String, pub state: HotkeyState, pub error: Option<String> }` with `pub fn off() -> Self`, `pub fn invalid(text: &str, error: ComboError) -> Self`, `pub fn active(combo: &Combo) -> Self`, `pub fn taken(combo: &Combo, refusal: Refusal) -> Self`
  `#[derive(Debug, Clone, PartialEq, Eq)] pub enum hotkey::Wanted { Off, Invalid { text: String, error: ComboError }, Combo(Combo) }` with `pub fn from_text(text: &str) -> Self`
  `#[derive(Debug, Clone, PartialEq, Eq)] pub struct hotkey::Refusal { pub already_registered: bool, pub message: String }`
  `pub trait hotkey::HotkeyOs { fn register(&mut self, combo: &Combo) -> Result<(), Refusal>; fn unregister(&mut self); }`
  `#[derive(Debug, Default)] pub struct hotkey::Slot` with `pub fn set(&mut self, wanted: Wanted, os: &mut impl HotkeyOs) -> Option<HotkeyStatus>`, `pub fn pause(&mut self, os: &mut impl HotkeyOs)`, `pub fn resume(&mut self, os: &mut impl HotkeyOs) -> Option<HotkeyStatus>`, `pub fn release(&mut self, os: &mut impl HotkeyOs)`
  `#[derive(Debug, Clone, PartialEq, Eq)] pub enum hotkey::win::Event { Status(HotkeyStatus), Pressed }`
  `pub struct hotkey::win::Registrar` with `pub fn start(sink: impl FnMut(Event) + Send + 'static) -> std::io::Result<Registrar>` (returns once the thread's queue exists), `pub fn set(&self, wanted: Wanted) -> std::io::Result<()>`, `pub fn pause(&self, on: bool) -> std::io::Result<()>`, and `Drop` (posts `WM_QUIT`, joins; the thread releases the combination on its way out)

The live run (Task 6) is the only check of a `WM_HOTKEY` from a real keyboard. The tests below cover the rest on a real thread with real `RegisterHotKey`: register, release, register again, pause and resume, the 1409 conflict, a retry after the holder lets go, and a clean end.

- [ ] **Step 1: The failing `Slot` tests**

Append inside `mod tests` of `hotkey.rs`:

```rust
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
```

- [ ] **Step 2: The failing thread tests**

In `mod tests` of `hotkey/win.rs`, replace `use crate::hotkey::parse;` with:

```rust
    use crate::hotkey::{parse, HotkeyState};
    use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
    use std::time::Duration;
```

and append inside that module:

```rust
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

    #[test]
    fn the_hotkey_thread_registers_releases_registers_again_and_ends_cleanly() {
        // Never Alt+M: the running dev Mayak holds it. F23/F24 are for the other test and this one.
        let combo = parse("Ctrl+Alt+Shift+F24").unwrap();
        let (registrar, rx) = started();
        registrar.set(Wanted::Combo(combo)).unwrap();
        assert_eq!(next(&rx), Ok(Event::Status(HotkeyStatus::active(&combo))));
        registrar.set(Wanted::Off).unwrap();
        assert_eq!(next(&rx), Ok(Event::Status(HotkeyStatus::off())));
        // Registering again on the same thread succeeds only if Off really released it.
        registrar.set(Wanted::Combo(combo)).unwrap();
        assert_eq!(next(&rx), Ok(Event::Status(HotkeyStatus::active(&combo))));
        // Recording: released, then registered again and reported.
        registrar.pause(true).unwrap();
        registrar.pause(false).unwrap();
        assert_eq!(next(&rx), Ok(Event::Status(HotkeyStatus::active(&combo))));
        drop(registrar);
        // The thread is gone: the sink, and its sender, were dropped.
        assert_eq!(next(&rx), Err(RecvTimeoutError::Disconnected));
    }

    #[test]
    fn a_shortcut_held_by_another_thread_is_taken_until_it_lets_go() {
        let combo = parse("Ctrl+Alt+Shift+F23").unwrap();
        let (first, rx1) = started();
        first.set(Wanted::Combo(combo)).unwrap();
        assert_eq!(next(&rx1), Ok(Event::Status(HotkeyStatus::active(&combo))));
        let (second, rx2) = started();
        second.set(Wanted::Combo(combo)).unwrap();
        // ERROR_HOTKEY_ALREADY_REGISTERED (1409): taken, without the Windows text (spec §3.2).
        assert_eq!(
            next(&rx2),
            Ok(Event::Status(HotkeyStatus { combo: "Ctrl+Alt+Shift+F23".into(), state: HotkeyState::Taken, error: None }))
        );
        drop(first);
        assert_eq!(next(&rx1), Err(RecvTimeoutError::Disconnected));
        second.set(Wanted::Combo(combo)).unwrap();
        assert_eq!(next(&rx2), Ok(Event::Status(HotkeyStatus::active(&combo))));
    }
```

- [ ] **Step 3: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak hotkey:: 2>&1 | grep -E "^error" | sort -u | head -12`
Expected: compile errors, among them ``cannot find type `Slot` ``, ``cannot find trait `HotkeyOs` ``, ``cannot find type `Refusal` ``, ``failed to resolve: use of undeclared type `Wanted` ``, ``cannot find type `Registrar` `` and ``cannot find type `Event` ``.

- [ ] **Step 4: Implement the status and `Slot` in `hotkey.rs`**

Change `use std::fmt;` to:

```rust
use std::fmt;

use serde::Serialize;
```

Insert after `fn key_of` (before `#[cfg(test)]`):

```rust
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
```

- [ ] **Step 5: Implement the thread in `hotkey/win.rs`**

Insert after `fn win32_codes` (before `#[cfg(test)]`). If rustc reports a signature difference in windows 0.61.3, fix it the way `strip.rs` does: `Option<HWND>` parameters take `None`, and `GetMessageW` returns a `BOOL` read through `.0`.

```rust
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

use super::{HotkeyOs, HotkeyStatus, Refusal, Slot, Wanted};

/// The id of Mayak's one shortcut on this thread.
const HOTKEY_ID: i32 = 1;
/// A new `Wanted` is waiting in the channel (spec §3.2: WM_APP + 1).
const WM_SET: u32 = WM_APP + 1;
/// Settings started recording: release the shortcut (WM_APP + 2).
const WM_PAUSE: u32 = WM_APP + 2;
/// Recording ended: register it again (WM_APP + 3).
const WM_RESUME: u32 = WM_APP + 3;

/// What the thread hands to its sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The registration changed (spec §3.1).
    Status(HotkeyStatus),
    /// WM_HOTKEY: the shortcut was pressed.
    Pressed,
}

/// The registration of the calling thread: `RegisterHotKey(NULL, …)` binds it to that thread.
struct ThreadHotkey;

impl HotkeyOs for ThreadHotkey {
    fn register(&mut self, combo: &Combo) -> Result<(), Refusal> {
        let (mods, vk) = win32_codes(combo);
        unsafe { RegisterHotKey(None, HOTKEY_ID, mods, vk) }.map_err(|err| Refusal {
            already_registered: err.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult(),
            message: err.message(),
        })
    }

    fn unregister(&mut self) {
        if let Err(err) = unsafe { UnregisterHotKey(None, HOTKEY_ID) } {
            crate::log::line(format!("hotkey: UnregisterHotKey: {err}"));
        }
    }
}

fn run(cmds: Receiver<Wanted>, mut sink: impl FnMut(Event), ready: SyncSender<u32>) {
    let mut msg = MSG::default();
    // A thread has no message queue until its first USER call, and PostThreadMessageW fails
    // until it has one: make it before telling `start` the thread id.
    unsafe {
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
    }
    if ready.send(unsafe { GetCurrentThreadId() }).is_err() {
        return;
    }
    let mut slot = Slot::default();
    let mut os = ThreadHotkey;
    // 0 on WM_QUIT, -1 on error: both end the thread.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        match msg.message {
            WM_HOTKEY if msg.wParam.0 == HOTKEY_ID as usize => sink(Event::Pressed),
            // Every waiting request, in order: the last status is the last request's.
            WM_SET => {
                for wanted in cmds.try_iter() {
                    if let Some(status) = slot.set(wanted, &mut os) {
                        sink(Event::Status(status));
                    }
                }
            }
            WM_PAUSE => slot.pause(&mut os),
            WM_RESUME => {
                if let Some(status) = slot.resume(&mut os) {
                    sink(Event::Status(status));
                }
            }
            _ => {}
        }
    }
    slot.release(&mut os);
}

/// The running hotkey thread. Dropping it releases the shortcut and ends the thread.
pub struct Registrar {
    thread_id: u32,
    cmds: Sender<Wanted>,
    thread: Option<JoinHandle<()>>,
}

impl Registrar {
    /// Starts the thread; every status and every press goes to `sink`, on that thread. Returns
    /// once its queue exists, so the first `set` cannot be lost.
    pub fn start(sink: impl FnMut(Event) + Send + 'static) -> std::io::Result<Self> {
        let (cmds, rx) = channel::<Wanted>();
        let (ready_tx, ready_rx) = sync_channel::<u32>(1);
        let thread = std::thread::Builder::new().name("hotkey".into()).spawn(move || run(rx, sink, ready_tx))?;
        match ready_rx.recv() {
            Ok(thread_id) => Ok(Self { thread_id, cmds, thread: Some(thread) }),
            Err(_) => {
                let _ = thread.join();
                Err(std::io::Error::other("the hotkey thread ended before it was ready"))
            }
        }
    }

    /// A new shortcut, or none (spec §3.2: through the channel, then WM_APP + 1).
    pub fn set(&self, wanted: Wanted) -> std::io::Result<()> {
        self.cmds.send(wanted).map_err(|_| std::io::Error::other("the hotkey thread is gone"))?;
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
    /// WM_QUIT, then wait: the thread releases the shortcut on its way out. A thread that could
    /// not be told to quit and is still alive is left running rather than hanging the caller.
    fn drop(&mut self) {
        let posted = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Err(err) = &posted {
            crate::log::line(format!("hotkey: cannot post WM_QUIT to the hotkey thread: {err}"));
        }
        let Some(handle) = self.thread.take() else { return };
        if posted.is_err() && !handle.is_finished() {
            crate::log::line("hotkey: the hotkey thread is left running");
            return;
        }
        if handle.join().is_err() {
            crate::log::line("hotkey: the hotkey thread panicked");
        }
    }
}
```

- [ ] **Step 6: Run the tests and the suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak hotkey:: 2>&1 | grep -E "^test |test result: ok. [1-9]|warning|error"`
Expected: 15 tests `ok` (10 `hotkey::tests`, 5 `hotkey::win::tests`), `test result: ok. 15 passed`, the two thread tests in well under a second. If `a_shortcut_held_by_another_thread_is_taken_until_it_lets_go` reports `error: Some(…)` instead of `None`, the refusal was not 1409. Report the text and stop; do not loosen the assertion. Then `cargo test --workspace 2>&1 | grep -E "^test result|warning"`: lib **406 passed** (1 ignored), hook 32 passed, 0 warnings (the module is still under the Task 1 `#[allow(dead_code)]`).

- [ ] **Step 7: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/hotkey.rs windows/src-tauri/src/hotkey/win.rs && git commit -m "feat(hotkey): registration thread with pause, conflict detection and clean shutdown"
```

---

### Task 3: Wiring — `apply`, the commands, the events, the unsupported path

**Files:**
- Modify: `windows/src-tauri/src/hotkey.rs` (imports; `unsupported`/`failed` in `impl HotkeyStatus`; statics, `apply`, `publish`, `on_event` and the commands after `impl Slot`; one test)
- Modify: `windows/src-tauri/src/lib.rs` (the Task 1 attribute; `save_settings` lines 85-105; `setup` after `create_settings_window(&handle);` line 657; `generate_handler!` after `set_paused,` line 621)

**Interfaces:**
- Consumes: Task 2 (`HotkeyStatus`, `Wanted::from_text`, `Refusal`, `win::Registrar::{start, set, pause}`, `win::Event`); `island::WINDOW_LABEL`; `Settings.hotkey` (Task 1).
- Produces:
  `pub fn hotkey::HotkeyStatus::unsupported(text: &str) -> HotkeyStatus`, `pub fn hotkey::HotkeyStatus::failed(text: &str, why: String) -> HotkeyStatus` (`taken` with `why`)
  `pub fn hotkey::apply(app: &AppHandle, text: &str)`: called at start and from `save_settings` when `hotkey` changed
  `#[tauri::command] pub fn hotkey::hotkey_status() -> HotkeyStatus` (wire `hotkey_status`, no arguments; `off` before the first status)
  `#[tauri::command] pub fn hotkey::hotkey_pause(on: bool)` (wire `hotkey_pause { on }`)
  Event `hotkey` → island window only, payload `null`. Event `hotkey-status` → every window, payload `{ combo: string, state: "off" | "active" | "taken" | "invalid" | "unsupported", error: string | null }`.

After this task the dev Mayak, rebuilt, holds `Alt+M` and emits `hotkey` on every press. Nothing listens yet; Task 4 adds the listener. `mayak.log` gets one `hotkey: Active Alt+M` line.

- [ ] **Step 1: The failing wire-format test**

Append inside `mod tests` of `hotkey.rs`:

```rust
    #[test]
    fn the_status_reaches_the_page_with_a_lowercase_state_and_a_null_error() {
        use serde_json::json;
        assert_eq!(serde_json::to_value(HotkeyStatus::off()).unwrap(), json!({ "combo": "", "state": "off", "error": null }));
        let taken = HotkeyStatus::taken(
            &parse("Alt+M").unwrap(),
            Refusal { already_registered: false, message: "Access is denied.".into() },
        );
        assert_eq!(
            serde_json::to_value(taken).unwrap(),
            json!({ "combo": "Alt+M", "state": "taken", "error": "Access is denied." })
        );
        assert_eq!(
            serde_json::to_value(HotkeyStatus::unsupported("Alt+M")).unwrap(),
            json!({ "combo": "Alt+M", "state": "unsupported", "error": "Global shortcut is not available on this platform yet" })
        );
        assert_eq!(
            serde_json::to_value(HotkeyStatus::failed("Alt+M", "the hotkey thread is gone".into())).unwrap(),
            json!({ "combo": "Alt+M", "state": "taken", "error": "the hotkey thread is gone" })
        );
    }
```

- [ ] **Step 2: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak hotkey::tests::the_status 2>&1 | grep -E "^error" | sort -u`
Expected: ``error[E0599]: no function or associated item named `unsupported` found for struct `HotkeyStatus` `` (and the same for `failed`).

- [ ] **Step 3: Implement the wiring in `hotkey.rs`**

Change the imports to:

```rust
use std::fmt;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
```

Inside `impl HotkeyStatus`, after `fn taken`, add:

```rust
    /// macOS and Linux, until they are ported (spec §3.3).
    #[cfg_attr(windows, allow(dead_code))]
    pub fn unsupported(text: &str) -> Self {
        Self {
            combo: text.to_string(),
            state: HotkeyState::Unsupported,
            error: Some("Global shortcut is not available on this platform yet".into()),
        }
    }

    /// The thread could not be started or reached: shown as taken, with why (spec §3.2).
    pub fn failed(text: &str, why: String) -> Self {
        Self { combo: text.to_string(), state: HotkeyState::Taken, error: Some(why) }
    }
```

After `impl Slot { … }` (before `#[cfg(test)]`) add:

```rust
// ── The app's shortcut (spec §3.1) ────────────────────────────────────────────

/// The last status published: what `hotkey_status` answers. `None` before the first one.
static STATUS: Mutex<Option<HotkeyStatus>> = Mutex::new(None);
/// The hotkey thread, started by the first `apply` and kept for the life of the app.
#[cfg(windows)]
static REGISTRAR: Mutex<Option<win::Registrar>> = Mutex::new(None);

/// `text` is the shortcut from now on: at start, and from `save_settings` when it changed.
#[cfg(windows)]
pub fn apply(app: &AppHandle, text: &str) {
    let mut registrar = REGISTRAR.lock().unwrap();
    if registrar.is_none() {
        let events = app.clone();
        match win::Registrar::start(move |event| on_event(&events, event)) {
            Ok(started) => *registrar = Some(started),
            Err(err) => {
                crate::log::line(format!("hotkey: cannot start the hotkey thread: {err}"));
                publish(app, HotkeyStatus::failed(text, err.to_string()));
                return;
            }
        }
    }
    let Some(running) = registrar.as_ref() else { return };
    if let Err(err) = running.set(Wanted::from_text(text)) {
        crate::log::line(format!("hotkey: cannot hand the shortcut to its thread: {err}"));
        publish(app, HotkeyStatus::failed(text, err.to_string()));
    }
}

/// macOS and Linux: nothing is registered, whatever the text (spec §3.3).
#[cfg(not(windows))]
pub fn apply(app: &AppHandle, text: &str) {
    publish(app, HotkeyStatus::unsupported(text));
}

/// On the hotkey thread: a status for every window, a press for the island.
#[cfg(windows)]
fn on_event(app: &AppHandle, event: win::Event) {
    match event {
        win::Event::Status(status) => publish(app, status),
        win::Event::Pressed => {
            if let Err(err) = app.emit_to(crate::island::WINDOW_LABEL, "hotkey", ()) {
                crate::log::line(format!("hotkey emit failed: {err}"));
            }
        }
    }
}

/// Keeps the status `hotkey_status` answers and tells every window, when it changed.
fn publish(app: &AppHandle, status: HotkeyStatus) {
    {
        let mut current = STATUS.lock().unwrap();
        if current.as_ref() == Some(&status) {
            return;
        }
        *current = Some(status.clone());
    }
    crate::log::line(format!("hotkey: {:?} {}", status.state, status.combo));
    if let Err(err) = app.emit("hotkey-status", status) {
        crate::log::line(format!("hotkey-status emit failed: {err}"));
    }
}

/// Settings asks for the status when it opens; later changes arrive as `hotkey-status`.
#[tauri::command]
pub fn hotkey_status() -> HotkeyStatus {
    STATUS.lock().unwrap().clone().unwrap_or_else(HotkeyStatus::off)
}

/// Settings records a shortcut (`on`), or stopped: the current one is released meanwhile, so the
/// keys reach the page instead of toggling the island (spec §5).
#[tauri::command]
pub fn hotkey_pause(on: bool) {
    #[cfg(windows)]
    if let Some(running) = REGISTRAR.lock().unwrap().as_ref() {
        if let Err(err) = running.pause(on) {
            crate::log::line(format!("hotkey: pause {on}: {err}"));
        }
    }
    #[cfg(not(windows))]
    let _ = on;
}
```

- [ ] **Step 4: Implement `lib.rs`**

Remove the line `#[allow(dead_code)] // wired into setup and save_settings in Task 3; remove this attribute there` above `mod hotkey;`.

In `save_settings` replace

```rust
    let (screen_changed, autostart_changed) = {
        let mut current = shared.settings.lock().unwrap();
        let screen_changed = current.screen != settings.screen;
        let autostart_changed = current.autostart != settings.autostart;
        *current = settings.clone();
        (screen_changed, autostart_changed)
    };
```

with

```rust
    let (screen_changed, autostart_changed, hotkey_changed) = {
        let mut current = shared.settings.lock().unwrap();
        let screen_changed = current.screen != settings.screen;
        let autostart_changed = current.autostart != settings.autostart;
        let hotkey_changed = current.hotkey != settings.hotkey;
        *current = settings.clone();
        (screen_changed, autostart_changed, hotkey_changed)
    };
```

and after the `if screen_changed { … }` block add:

```rust
    // Only a real change reaches the hotkey thread: the island saves the sound and the
    // auto-close interval through here too (hotkey spec §3.1).
    if hotkey_changed {
        hotkey::apply(&app, &settings.hotkey);
    }
```

In `setup`, after `create_settings_window(&handle);` add:

```rust
            // The global shortcut, registered from the start (hotkey spec §3.1). A press before
            // the island's page listens is simply not seen.
            hotkey::apply(&handle, &loaded.hotkey);
```

In `tauri::generate_handler![…]`, after `set_paused,` add:

```rust
            hotkey::hotkey_status,
            hotkey::hotkey_pause,
```

- [ ] **Step 5: Verify**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|warning|error"`
Expected: lib **407 passed** (1 ignored), hook 32 passed, 0 warnings. No `dead_code` warnings remain: every item is now reachable from `apply` or a command, and `unsupported` carries its own `cfg_attr`. Then, once the dev Mayak has rebuilt (it restarts on its own): `grep "hotkey:" "$LOCALAPPDATA/Mayak/mayak.log" | tail -3` shows `hotkey: Active Alt+M`. A `Taken` line means another program holds `Alt+M` on this machine: record it for Task 6 and go on.

- [ ] **Step 6: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/src/hotkey.rs windows/src-tauri/src/lib.rs && git commit -m "feat(hotkey): register the shortcut at start and on save, status and pause commands"
```

---

### Task 4: The island toggles on `hotkey`; bridge types

**Files:**
- Modify: `windows/src/core/bridge.ts` (`Bridge` after `setPaused`, line 115; a type after `DroppedFile`, line 133; `BridgeEvent`, lines 189-198)
- Modify: `windows/src/core/state.ts` (`Settings` after `alertError`, line 111; `DEFAULT_SETTINGS` after `alertError: true,`, line 127)
- Modify: `windows/src/island/island.ts` (new method after `onStripTouch()`, line 763)
- Modify: `windows/src/main.ts` (listener after the `strip-fullscreen` one, line 32)

**Interfaces:**
- Consumes: events `hotkey` (payload `null`) and `hotkey-status`, commands `hotkey_status` and `hotkey_pause { on }` (Task 3); `IslandStateMachine.state`, `.pinned`, `.collapsedByButton`, `.forceHidden()`; `Island.alert(view)`.
- Produces: `export interface HotkeyStatus { combo: string; state: "off" | "active" | "taken" | "invalid" | "unsupported"; error: string | null }`; `Bridge.hotkeyStatus(): Promise<HotkeyStatus | null>`; `Bridge.hotkeyPause(on: boolean): Promise<void | null>`; `Settings.hotkey: string`, `DEFAULT_SETTINGS.hotkey = "Alt+M"`; `Island.onHotkey(): void`.

There is no front-end test framework, and none is added; the checks are `npx tsc --noEmit` and `npx vite build`. The toggle rules are verified live in Task 6, items 1–2 and the pinned/quiet checks.

- [ ] **Step 1: Bridge and settings types**

`bridge.ts`, in `Bridge` after the `setPaused` entry add:

```ts

  // ── Global shortcut (src-tauri/src/hotkey.rs) ─────────────────────────────
  /** Current state of the shortcut; every later change arrives as `hotkey-status`. */
  hotkeyStatus: () => call<HotkeyStatus>("hotkey_status"),
  /** Settings is recording a shortcut: release the current one meanwhile, so the keys reach the page. */
  hotkeyPause: (on: boolean) => call<void>("hotkey_pause", { on }),
```

after the `DroppedFile` interface add:

```ts

/** What Settings shows under the global shortcut (hotkey spec §3.1). */
export interface HotkeyStatus {
  combo: string;
  state: "off" | "active" | "taken" | "invalid" | "unsupported";
  error: string | null;
}
```

and in `BridgeEvent`, after `| { name: "strip-fullscreen"; payload: { on: boolean } }` add:

```ts
  | { name: "hotkey"; payload: null }
  | { name: "hotkey-status"; payload: HotkeyStatus }
```

`state.ts`, in `Settings` after `alertError: boolean;` add:

```ts
  /** Global shortcut that shows and hides the island, e.g. "Alt+M"; "" = off. Rust defaults it for older files. */
  hotkey: string;
```

and in `DEFAULT_SETTINGS` after `alertError: true,` add `hotkey: "Alt+M",`.

- [ ] **Step 2: The toggle**

`island.ts`, after the `onStripTouch() { … }` method add:

```ts
  /**
   * The global shortcut (src-tauri/src/hotkey.rs, spec §4). An open island folds into the strip,
   * pinned or not, and like the collapse button `holdVisible` does not hold it; hidden, compact
   * or greeting opens on the session list with the usual sound. Quiet mode neither stops it
   * nor is lifted: it is an explicit action.
   */
  onHotkey() {
    Sound.resume();
    if (this.fsm.state === "home") {
      State.isPinned = false;
      this.fsm.pinned = false;
      this.fsm.collapsedByButton = true;
      this.fsm.forceHidden();
    } else {
      this.alert("sessions");
    }
  }
```

`main.ts`, after `await onEvent<{ on: boolean }>("strip-fullscreen", ({ on }) => island.onStripFullscreen(on));` add:

```ts
  await onEvent<null>("hotkey", () => island.onHotkey());
```

- [ ] **Step 3: Verify**

Run: `cd /c/Users/HP/source/repos/mayak/windows && npx tsc --noEmit && npx vite build 2>&1 | tail -3`
Expected: both exit 0 (`npm ci` first if `node_modules` is missing). `cargo test --workspace` is unchanged (407 + 32): no Rust changed. Quick live check with the dev Mayak, which reloads the page on its own: ask the user to press `Alt+M` once in another program. The island opens on the session list; a second press folds it into the strip.

- [ ] **Step 4: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src/core/bridge.ts windows/src/core/state.ts windows/src/island/island.ts windows/src/main.ts && git commit -m "feat(island): the global shortcut opens the session list or folds the island"
```

---

### Task 5: Settings → General «Show / hide island»; README row

**Files:**
- Modify: `windows/src/settings/main.ts` (the `../core/bridge` import, lines 7-10; new helpers above `// ── General section`, line 584; `generalSection` rows, line 636-639)
- Modify: `windows/README.md` («Using it» table, after line 45)

**Interfaces:**
- Consumes: `Bridge.hotkeyStatus`, `Bridge.hotkeyPause`, `HotkeyStatus`, `Settings.hotkey` (Task 4); event `hotkey-status` (Task 3); the existing `settings`, `save()`, `h`, `onEvent`.
- Produces: `function shortcutOf(e: KeyboardEvent): { combo: string } | { hint: string } | null`, `function hotkeyStatusText(s: HotkeyStatus): string`, `function hotkeyRows(): HTMLElement[]` (all module-private).

Everything new uses the existing classes: `row` for the line, the plain `button` style for the field and «Off», `hint` for the status line. No CSS changes. The existing rows stay exactly as they are. Recording is verified live (Task 6 item 3).

- [ ] **Step 1: The row**

Replace the import

```ts
import {
  Bridge, onEvent, type Health, type HookPreview, type HookStatus, type HookTarget,
  type SessionsSnapshot, type SilentReason,
} from "../core/bridge";
```

with

```ts
import {
  Bridge, onEvent, type Health, type HookPreview, type HookStatus, type HookTarget,
  type HotkeyStatus, type SessionsSnapshot, type SilentReason,
} from "../core/bridge";
```

Above `// ── General section ───…` add:

```ts
// ── Global shortcut (hotkey spec §5) ──────────────────────────────────────────

/**
 * A keydown while recording → the shortcut in Rust's normal form (src-tauri/src/hotkey.rs: Ctrl,
 * Alt, Shift, Win, then the key), a hint for a press that cannot be one, or null for a lone
 * modifier (keep waiting). Read from `code`, the physical key, like Rust's virtual key: the
 * same shortcut in every keyboard layout.
 */
function shortcutOf(e: KeyboardEvent): { combo: string } | { hint: string } | null {
  if (/^(Control|Alt|Shift|Meta|OS)(Left|Right)$/.test(e.code)) return null;
  const letter = /^Key([A-Z])$/.exec(e.code);
  const digit = /^Digit([0-9])$/.exec(e.code);
  const fkey = /^F([1-9]|1[0-9]|2[0-4])$/.exec(e.code);
  const key = letter?.[1] ?? digit?.[1] ?? (fkey ? `F${fkey[1]}` : e.code === "Space" ? "Space" : null);
  if (key == null) return { hint: "Use a letter, a digit, F1–F24 or Space." };
  const mods = [
    e.ctrlKey ? "Ctrl" : null,
    e.altKey ? "Alt" : null,
    e.shiftKey ? "Shift" : null,
    e.metaKey ? "Win" : null,
  ].filter((m): m is string => m != null);
  // A bare letter, digit or Space would take over ordinary typing; F1–F24 may stand alone.
  if (mods.length === 0 && !fkey) return { hint: "Add Ctrl, Alt, Shift or Win: only F1–F24 work alone." };
  return { combo: [...mods, key].join("+") };
}

/** The line under the shortcut for a status from hotkey.rs: empty for active and off. */
function hotkeyStatusText(s: HotkeyStatus): string {
  switch (s.state) {
    case "taken":
      return `${s.combo} is taken by another app — choose another${s.error ? ` (${s.error})` : ""}`;
    case "invalid":
    case "unsupported":
      return s.error ?? "";
    default:
      return "";
  }
}

/**
 * «Show / hide island»: the shortcut as a button that records a new one, Off, and the status
 * line. While recording the shortcut is released (hotkey_pause), or pressing it would toggle
 * the island instead of reaching this page.
 */
function hotkeyRows(): HTMLElement[] {
  const field = h("button", {});
  const off = h("button", { text: "Off" });
  const line = h("div", { class: "hint" });
  let recording = false;
  let status: HotkeyStatus | null = null;

  function draw() {
    field.textContent = recording ? "Press keys…" : settings.hotkey || "Off";
    // macOS and Linux: the platform's message instead of the field (spec §3.3).
    const unsupported = status?.state === "unsupported";
    field.style.display = unsupported ? "none" : "";
    off.style.display = unsupported ? "none" : "";
    if (!recording) line.textContent = status ? hotkeyStatusText(status) : "";
    line.style.display = line.textContent ? "" : "none";
  }

  function hint(text: string) {
    line.textContent = text;
    line.style.display = "";
  }

  /** Ends a recording. `chosen` is saved before the pause ends, so the new shortcut is registered, not the old one. */
  async function finish(chosen: string | null) {
    if (!recording) return;
    recording = false;
    window.removeEventListener("keydown", onKey, true);
    const next = chosen ?? settings.hotkey;
    const changed = next !== settings.hotkey;
    settings.hotkey = next;
    draw();
    try {
      if (changed) await save();
    } finally {
      await Bridge.hotkeyPause(false);
    }
  }

  function onKey(e: KeyboardEvent) {
    // Nothing reaches the page while recording: no reload on F5, no click on Enter.
    e.preventDefault();
    e.stopPropagation();
    if (e.repeat) return;
    if (e.code === "Escape") {
      void finish(null);
      return;
    }
    const got = shortcutOf(e);
    if (got == null) return;
    if ("hint" in got) hint(got.hint);
    else void finish(got.combo);
  }

  field.addEventListener("click", () => {
    if (recording) return;
    recording = true;
    // A focused button would be clicked again by the Space or Enter of the recorded shortcut.
    field.blur();
    line.textContent = "";
    draw();
    void Bridge.hotkeyPause(true);
    window.addEventListener("keydown", onKey, true);
  });

  off.addEventListener("click", () => {
    if (recording) {
      void finish("");
      return;
    }
    if (settings.hotkey === "") return;
    settings.hotkey = "";
    draw();
    void save();
  });

  // Leaving the window (Alt+Tab, or closing it: it only hides) ends a recording unchanged, so
  // the shortcut never stays released.
  window.addEventListener("blur", () => void finish(null));
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) void finish(null);
  });

  let heard = false;
  void onEvent<HotkeyStatus>("hotkey-status", (s) => {
    heard = true;
    status = s;
    draw();
  });
  void Bridge.hotkeyStatus().then((s) => {
    // An event that arrived meanwhile is newer than this answer.
    if (heard) return;
    status = s;
    draw();
  });
  // A page reloaded in the middle of a recording left the shortcut released: give it back.
  // A resume without a pause does nothing (hotkey.rs Slot::resume).
  void Bridge.hotkeyPause(false);
  draw();

  return [
    h("div", { class: "row" }, h("label", { text: "Show / hide island" }), field, off),
    line,
  ];
}

```

In `generalSection`, after the row

```ts
    h("div", { class: "row" },
      h("label", { text: "Launch at startup" }),
      toggle(settings.autostart, (v) => { settings.autostart = v; void save(); }),
    ),
```

add `...hotkeyRows(),` as the next child of the section.

- [ ] **Step 2: README row**

In `windows/README.md`, «Using it» table, after the row `| Click the bell in the island header | Quiet mode — the same as the tray's Pause, but the island stays open |` add:

```markdown
| `Alt+M` in any app (change or turn it off in Settings → General) | The island opens on the session list; press it again and it folds away to the line |
```

- [ ] **Step 3: Verify**

Run: `cd /c/Users/HP/source/repos/mayak/windows && npx tsc --noEmit && npx vite build 2>&1 | tail -3`
Expected: both exit 0. `cargo test --workspace` is unchanged (407 + 32).

- [ ] **Step 4: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src/settings/main.ts windows/README.md && git commit -m "feat(settings): record, clear and show the status of the global shortcut"
```

---

### Task 6: Spec alignment, live acceptance, CPU, and the question before publishing

**Files:**
- Modify: `docs/superpowers/specs/2026-10-06-mayak-hotkey-design.md` (LF line endings; keep them)
- Modify: `.git/mayak-pr-notes.md` (local, untracked)
- Create: `.git/mayak-pr7-body.md` (local, untracked)

**Interfaces:** consumes the branch after Tasks 1-5; produces the aligned spec, the acceptance results and the PR text.

The controller runs this task **with the user** (real keyboard, real programs, the Settings window). Only Steps 1 and 5 are implementer work. The dev Mayak under `tauri dev` is already running the working tree; it rebuilt after each task. Do not stop it and do not start a second one.

- [ ] **Step 1: Write the decisions into the spec**

Append at the end of the spec (Russian, LF line endings):

```markdown

## 8. Уточнения при реализации

- **Файлы.** Общая часть — `windows/src-tauri/src/hotkey.rs`, Windows — `windows/src-tauri/src/hotkey/win.rs` (как `focus/mod.rs` и `focus/win.rs`). `Cargo.toml` не меняется: feature `Win32_UI_Input_KeyboardAndMouse` уже подключена.
- **Разбор.** Модификаторы принимаются в любом порядке и приводятся к порядку `Ctrl`, `Alt`, `Shift`, `Win`. Неизвестная последняя часть — `UnknownKey`, неизвестная часть перед ней — `UnknownModifier`; сочетание без клавиши (`Ctrl+Alt`, `Alt+`) — `UnknownKey`. Отдельной функции `normalize` нет: нормальная запись — `parse(x)?.to_string()`.
- **Порядок статусов.** Все статусы, кроме `unsupported`, выдаёт поток регистрации (`Slot`), по одному на каждый запрос и в порядке запросов, поэтому ответ на старый запрос не перекрывает новый. Пустая строка и ошибка разбора снимают прежнее сочетание.
- **Пауза.** Сочетание, сохранённое во время записи, регистрируется при `hotkey_pause(false)`; снятие паузы без паузы ничего не делает. Settings снимает паузу после сохранения, по `Esc`, по «Off», при потере окном фокуса, при его скрытии и при загрузке страницы, так что сочетание не остаётся снятым.
- **Прочие ошибки.** Сбой запуска потока или передачи ему запроса — `taken` с текстом ошибки. На macOS и Linux `apply` отдаёт `unsupported` для любой строки.
- **События.** `hotkey` идёт только в окно острова, `hotkey-status` — во все окна. До первого статуса `hotkey_status` отвечает `off`.
- **Запись в Settings.** Клавиша берётся из `KeyboardEvent.code` (физическая клавиша, как VK), поэтому запись не зависит от раскладки; цифры цифрового блока не принимаются.
- **Остров.** Клавиша не снимает паузу (пункт трея «Open» снимает) и открывает именно список сессий, а не `defaultView()`. При сворачивании снимаются оба флага закрепления (`State.isPinned`, `fsm.pinned`) и ставится `collapsedByButton`.
```

Commit:

```bash
cd /c/Users/HP/source/repos/mayak && git add docs/superpowers/specs/2026-10-06-mayak-hotkey-design.md && git commit -m "docs: align the hotkey spec with the implementation"
```

- [ ] **Step 2: Check the running dev Mayak**

Run: `grep -E "hotkey|Mayak .* started" "$LOCALAPPDATA/Mayak/mayak.log" | tail -5`
Expected: the last `--- Mayak … started ---` is followed by `hotkey: Active Alt+M`. If the line is `hotkey: Taken Alt+M`, another program holds `Alt+M`. Ask the user which one, and run item 4 below with it.

- [ ] **Step 3: Spec §7 acceptance items 1–5 with the user**

Record each as PASS/FAIL, with the evidence (what the user saw, and the `hotkey:` lines of `mayak.log`), in `.git/mayak-pr-notes.md` under `## Global hotkey acceptance (2026-10-06)`:
1. `Alt+M` from another program (a browser, a terminal): the strip opens on the session list, and a second press folds it into the strip. Repeat with left Alt and with right Alt, on the English and on the Russian layout. Also from `petit` and during the launch greeting: both open on the session list.
2. Holding `Alt+M` for two seconds: one toggle, no flashing.
3. Settings → General: click the field, which shows «Press keys…». Press `Ctrl+Alt+K`: the field shows it, `Alt+M` no longer works, `Ctrl+Alt+K` does. Click again and press `Esc`: no change, and `Ctrl+Alt+K` still works. Click again and press `K` alone: the hint shows and recording goes on; then `Alt+M` saves it. Click again, then Alt+Tab away: the recording ends with no change and the shortcut still works. «Off»: no key works and the field shows `Off`. Set `Alt+M` back at the end.
4. A combination another program holds (for example one set as a shortcut in that program, or a second registration of it by any tool the user has): the status line reads «… is taken by another app — choose another».
5. Also check: with quiet mode on (tray Pause), the key still opens and folds the island, and quiet mode stays on. With an agent's waiting card open (pinned), the key folds it, and the next agent event opens the island again.

- [ ] **Step 4: CPU (spec §7 item 5)**

With the island hidden and nothing pressed: `powershell -File C:\tmp\mayak-accept\sample-cpu.ps1 -Seconds 60 -OutPath C:\tmp\mayak-accept\cpu-hotkey-idle.log`. Expected: `tree_avg_percent_of_machine` < 0.100, as before this branch (`cpu-idle.log`: 0.010). Record the RESULT line in `.git/mayak-pr-notes.md`.

- [ ] **Step 5: Full verification**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|warning"` and `npx tsc --noEmit && npx vite build 2>&1 | tail -3`.
Expected: lib 407 passed (1 ignored), hook 32 passed, 0 warnings, build OK. `git status --short` is clean (the `.git/` notes are not tracked).

- [ ] **Step 6: Ask before publishing**

Write `.git/mayak-pr7-body.md` from the results of Steps 3-4: What (the toggle, Settings row, Windows thread, unsupported elsewhere), Checks (local) with the test counts and every acceptance line, and no trailer. Ask the user, verbatim: «Запушить ветку `feature/hotkey` в `origin` и открыть PR в `main`? Проверки на GitHub отключены, проверка локальная.» Only on an explicit yes:

```bash
cd /c/Users/HP/source/repos/mayak && git push -u origin feature/hotkey && gh pr create --repo mahach666/mayak --base main --head feature/hotkey --title "Mayak: global hotkey to show and hide the island" --body-file "$(git rev-parse --git-dir)/mayak-pr7-body.md"
```

The merge stays with the user: do not merge.

---

## Self-Review

- **Spec coverage:**
  - §1 toggle, default, both Alt, change/off/taken in Settings, multiplatform split → Tasks 1-5.
  - §1 rules (0 % CPU, no crates, no restyle) → Global Constraints; Task 2 (`GetMessageW` loop); Task 5 (existing classes only); Task 6 Step 4.
  - §2 format, case, spaces, normal form, VK, pure functions and every error → Task 1 (`parse`, `Display`, `key_of`, `modifier`, `win32_codes`, error table test); `#[serde(default)]` → Task 1 Step 6.
  - §3.1 `HotkeyStatus`, `hotkey_status`, `hotkey-status`, `apply` at start and on change, invalid without registering, the `hotkey` event → Task 2 (type, `Slot`), Task 3 (`apply`, `publish`, commands, `save_settings`, `setup`).
  - §3.2 one thread, `RegisterHotKey(NULL, …, MOD_NOREPEAT)`, `WM_APP + 1/2/3`, the channel, 1409 → taken, other errors with text, `WM_HOTKEY` → event → Task 2 (`run`, `ThreadHotkey`, `Registrar`), Task 3 (`on_event`).
  - §3.3 unsupported → Task 3 (`cfg(not(windows))` `apply`, `HotkeyStatus::unsupported`), Task 5 (field hidden).
  - §4 island table, pause, pinned → Task 4 (`onHotkey`).
  - §5 Settings row, recording, pause, Esc, hint, Off, status texts → Task 5.
  - §6 files → all tasks (`Cargo.toml` needs no change, stated in File Structure).
  - §7 tests → Tasks 1-3; acceptance → Task 6.
- **Placeholder scan:** none. Every code step carries its code. The only conditional instructions name their exact action: a windows-crate signature difference (fix the way `strip.rs` does), a 1409 assertion that fails (report it, do not loosen it), and `Taken` in the log (record it and use it in item 4).
- **Type consistency:**
  - `Combo`, `Key`, `ComboError`, `parse` (Task 1) are used unchanged in Tasks 2-3.
  - `win32_codes(&Combo) -> (HOT_KEY_MODIFIERS, u32)` (Task 1) is called by `ThreadHotkey::register` (Task 2).
  - `HotkeyStatus::{off, invalid, active, taken}`, `Wanted::{Off, Invalid { text, error }, Combo}`, `Wanted::from_text`, `Refusal { already_registered, message }`, `HotkeyOs::{register, unregister}` and `Slot::{set, pause, resume, release}` (Task 2) match their uses in `win.rs` (Task 2) and `apply` (Task 3).
  - `Registrar::{start, set, pause}` and `Event::{Status, Pressed}` (Task 2) match `apply`, `on_event` and `hotkey_pause` (Task 3).
  - The wire matches between Rust and TS: `hotkey_status` → `HotkeyStatus { combo, state, error }`, with `state` in lower case and `error` `null` (pinned by the Task 3 test); `hotkey_pause { on }` ↔ `Bridge.hotkeyPause(on)`; `hotkey` (payload `null`) ↔ `onEvent<null>("hotkey")`; `hotkey-status` ↔ `onEvent<HotkeyStatus>("hotkey-status")`.
  - `Settings.hotkey` (Rust, JSON `hotkey`) ↔ `Settings.hotkey` (TS), with `"Alt+M"` defaults on both sides.
  - `Island.onHotkey` (Task 4) uses `fsm.state`, `fsm.pinned`, `fsm.collapsedByButton` and `fsm.forceHidden()`, all public in `fsm.ts`.
- **Review Focus:**
  - 1 → Task 2 `a_shortcut_saved_while_recording_is_registered_when_recording_ends` and `cancelling_a_recording_brings_the_old_shortcut_back_once`.
  - 2 → Task 2 `turning_the_shortcut_off_or_breaking_it_releases_the_old_one`.
  - 3 → Task 2 `a_taken_shortcut_is_reported_and_tried_again_after_a_recording` and `a_shortcut_held_by_another_thread_is_taken_until_it_lets_go`.
  - 4 → Task 1 `an_older_settings_file_gets_alt_m_and_an_empty_one_stays_off`.
  - 5 → Task 1 `holding_the_keys_never_repeats_the_toggle`.
- **Test counts:** baseline 390.
  - Task 1 adds 8 (4 `hotkey::tests`, 3 `hotkey::win::tests`, 1 `settings::tests`) = 398.
  - Task 2 adds 8 (6 `Slot`, 2 thread) = 406.
  - Task 3 adds 1 = 407.
  - Tasks 4-6 add no Rust test. Hook stays at 32.
