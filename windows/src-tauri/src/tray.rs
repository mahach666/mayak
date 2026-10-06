// Notification-area icon: Open, Settings, Pause, Quit.

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::island::WINDOW_LABEL;

/// The tray's Pause, kept so the island's quiet button and the tray always show one state (spec §5).
pub struct PauseItem(CheckMenuItem<Wry>);

/// Called with every pause change, whichever side made it (`set_paused`).
pub fn show_paused(app: &AppHandle, paused: bool) {
    let Some(item) = app.try_state::<PauseItem>() else { return };
    if let Err(err) = item.0.set_checked(paused) {
        crate::log::line(format!("tray pause check: {err}"));
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Mayak", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pause", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(app, &[&open, &sep1, &settings, &pause, &sep2, &quit])?;

    app.manage(PauseItem(pause.clone()));

    let mut builder = TrayIconBuilder::with_id(crate::ident::APP_SLUG)
        .tooltip("Mayak")
        .menu(&menu)
        .on_menu_event(|app: &AppHandle, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "settings" => crate::show_settings_window(app),
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
