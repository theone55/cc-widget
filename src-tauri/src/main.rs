// cc-widget — a tiny always-on-top window for Claude Code: which sessions wait for you, how full their context is,
// and how much of the 5-hour / 7-day limits each account has used.
//
// The Rust side reads local files and keeps the settings; the page in ui/ only draws. The commands below are
// everything the page can ask for.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod counter;
mod focus;
mod paths;
mod sessions;
mod settings;
mod statusline;
mod tray;

use settings::Settings;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

struct AppState {
    config_dir: PathBuf,
    settings: Mutex<Settings>,
}

fn current(state: &tauri::State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn settings_get(state: tauri::State<AppState>) -> Settings {
    current(&state)
}

#[tauri::command]
fn settings_set(state: tauri::State<AppState>, settings: Settings) -> Result<(), String> {
    settings::save(&state.config_dir, &settings)?;
    *state.settings.lock().unwrap() = settings;
    Ok(())
}

/// Claude Code config folders found on this machine (for "Find accounts" in the settings).
#[tauri::command]
fn detect_accounts() -> Vec<String> {
    paths::detect_accounts()
}

#[tauri::command]
async fn sessions(state: tauri::State<'_, AppState>) -> Result<Vec<sessions::Session>, String> {
    let accounts = current(&state).accounts;
    // a few small files and the process list: quick, but off the UI thread anyway
    tauri::async_runtime::spawn_blocking(move || sessions::read(&accounts)).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn limits(state: tauri::State<AppState>) -> Vec<sessions::Limits> {
    sessions::limits(&current(&state).accounts)
}

#[tauri::command]
fn statusline_status(dir: String) -> String {
    statusline::status(&dir)
}

#[tauri::command]
fn statusline_connect(dir: String) -> Result<(), String> {
    statusline::connect(&dir)
}

#[tauri::command]
fn statusline_disconnect(dir: String) -> Result<(), String> {
    statusline::disconnect(&dir)
}

#[tauri::command]
async fn counter_fetch(state: tauri::State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let counter = current(&state).counter;
    if !counter.enabled {
        return Err("off".into());
    }
    counter::fetch(&counter).await
}

#[tauri::command]
fn counter_set_token(token: String) -> Result<(), String> {
    counter::set_token(&token)
}

/// Brings up the window the session runs in (its terminal or IDE).
#[tauri::command]
async fn focus_session(pid: u32, cwd: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || focus::focus(pid, &cwd)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn counter_has_token() -> bool {
    counter::token().is_some()
}

fn main() {
    // command-line modes, no window:
    //   cc-widget --statusline <config dir>   Claude Code's statusline (see statusline.rs)
    //   cc-widget --token-stdin               save the counter's token piped in (never on a command line)
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--statusline") {
        std::process::exit(statusline::run(args.get(i + 1).map(String::as_str).unwrap_or(".")));
    }
    if args.iter().any(|a| a == "--token-stdin") {
        let mut token = String::new();
        let saved = std::io::Read::read_to_string(&mut std::io::stdin(), &mut token)
            .map_err(|e| e.to_string())
            .and_then(|_| counter::set_token(&token));
        if let Err(e) = &saved {
            eprintln!("{e}");
        }
        std::process::exit(if saved.is_ok() { 0 } else { 1 });
    }

    tauri::Builder::default()
        // a second start (autostart + a click on the shortcut) just shows the running one
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let settings = settings::load(&config_dir);
            let russian = settings.language == "ru"
                || (settings.language == "auto" && std::env::var("LANG").is_ok_and(|l| l.starts_with("ru")));
            app.manage(AppState { config_dir, settings: Mutex::new(settings) });
            tray::create(app.handle(), russian)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings_get,
            settings_set,
            detect_accounts,
            sessions,
            limits,
            statusline_status,
            statusline_connect,
            statusline_disconnect,
            counter_fetch,
            counter_set_token,
            counter_has_token,
            focus_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running cc-widget");
}
