//! The tray icon: the widget has no taskbar button (skipTaskbar), so this is where it is shown / hidden, its settings
//! opened, its size reset and the app quit. A left click on the icon shows / hides the widget.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

fn toggle(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(true) {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn create(app: &AppHandle, russian: bool) -> tauri::Result<()> {
    let t = |en: &'static str, ru: &'static str| if russian { ru } else { en };
    let toggle_item = MenuItem::with_id(app, "toggle", t("Show / hide", "Показать / скрыть"), true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", t("Settings…", "Настройки…"), true, None::<&str>)?;
    let autosize = MenuItem::with_id(app, "autosize", t("Automatic size", "Авторазмер"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t("Quit", "Выход"), true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&toggle_item, &settings, &autosize, &PredefinedMenuItem::separator(app)?, &quit],
    )?;
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().ok_or(tauri::Error::InvalidIcon(std::io::Error::other("no icon")))?)
        .tooltip("cc-widget")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle(app),
            // the page opens its settings / goes back to the automatic size (widget.js)
            "settings" => {
                show(app);
                let _ = app.emit_to("main", "tray:settings", ());
            }
            "autosize" => {
                show(app);
                let _ = app.emit_to("main", "tray:autosize", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}
