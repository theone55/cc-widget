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
        .icon(tauri::image::Image::from_bytes(tray_icon_png())?)
        .icon_as_template(cfg!(target_os = "macos"))
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

// The tray icons are drawn at the size they are shown at (scripts/tray-icons.cjs) — a scaled one was blurry.
// macOS: a template image (black + alpha) that the menu bar tints like its other icons — the violet app icon was nearly
// invisible on a dark bar (tester, 2026-10-23); 36 px high = 18 pt on a Retina screen.
#[cfg(target_os = "macos")]
fn tray_icon_png() -> &'static [u8] {
    include_bytes!("../icons/tray/template.png")
}

// Windows: a filled colored disc (the thin line art got lost among the other tray icons too), the one as big as a small
// icon at this DPI, so Windows doesn't scale it
#[cfg(windows)]
fn tray_icon_png() -> &'static [u8] {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};
    const ICONS: [(i32, &[u8]); 6] = [
        (16, include_bytes!("../icons/tray/color-16.png")),
        (20, include_bytes!("../icons/tray/color-20.png")),
        (24, include_bytes!("../icons/tray/color-24.png")),
        (32, include_bytes!("../icons/tray/color-32.png")),
        (40, include_bytes!("../icons/tray/color-40.png")),
        (48, include_bytes!("../icons/tray/color-48.png")),
    ];
    // the process is DPI aware (Tauri), so this is the small icon size at the system DPI: 16 at 100 %, 24 at 150 %…
    let want = unsafe { GetSystemMetrics(SM_CXSMICON) };
    ICONS.iter().find(|(size, _)| *size >= want).unwrap_or(&ICONS[ICONS.len() - 1]).1
}

// Linux: the same disc at the 24 px most panels show
#[cfg(target_os = "linux")]
fn tray_icon_png() -> &'static [u8] {
    include_bytes!("../icons/tray/color-24.png")
}
