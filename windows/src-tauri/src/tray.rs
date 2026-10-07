// Notification-area icon: Open, Settings, Focus, Pause, Quit.

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

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
#[derive(Debug, serde::Deserialize)]
pub struct TrayTexts {
    pub open: String,
    pub settings: String,
    pub focus: String,
    pub pause: String,
    pub quit: String,
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
        }
    }
}

/// Sets the five texts. Before the tray is built there is nothing to set. A failure is logged with
/// a fixed line: the texts themselves are UI texts and never reach the log.
pub fn set_texts(app: &AppHandle, texts: TrayTexts) {
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

    let mut builder = TrayIconBuilder::with_id(crate::ident::APP_SLUG)
        .tooltip("Mayak")
        .menu(&menu)
        .on_menu_event(|app: &AppHandle, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "settings" => crate::show_settings_window(app),
            // "open", "pause" and "focus" go to the island, which owns those states.
            id => {
                let _ = app.emit_to(WINDOW_LABEL, "tray", id.to_string());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(s: &str) -> TrayTexts {
        TrayTexts { open: s.into(), settings: s.into(), focus: s.into(), pause: s.into(), quit: s.into() }
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
