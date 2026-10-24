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
mod remote;
mod system;
mod tray;

use settings::Settings;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

struct AppState {
    config_dir: PathBuf,
    settings: Mutex<Settings>,
    system: system::Shared,
}

fn current(state: &tauri::State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn settings_get(state: tauri::State<AppState>) -> Settings {
    current(&state)
}

#[tauri::command]
fn settings_set(state: tauri::State<AppState>, mut settings: Settings) -> Result<(), String> {
    let mut cur = state.settings.lock().unwrap();
    // the window's place is saved by its own commands; the page's copy of it may be older
    settings.pos = cur.pos;
    settings.size = cur.size;
    settings::save(&state.config_dir, &settings)?;
    *cur = settings;
    Ok(())
}

/// The window was moved: remember where (saved right away, so a shutdown can't lose it).
#[tauri::command]
fn pos_set(state: tauri::State<AppState>, x: i32, y: i32) -> Result<(), String> {
    let mut cur = state.settings.lock().unwrap();
    cur.pos = Some(settings::Pos { x, y });
    settings::save(&state.config_dir, &cur)
}

/// The window was resized by you (or back to automatic: None).
#[tauri::command]
fn size_set(state: tauri::State<AppState>, size: Option<settings::Size>) -> Result<(), String> {
    let mut cur = state.settings.lock().unwrap();
    cur.size = size;
    settings::save(&state.config_dir, &cur)
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

/// The "System" section's numbers (only while it is on: the page asks every 2 s).
#[tauri::command]
async fn system_stats(state: tauri::State<'_, AppState>) -> Result<system::Stats, String> {
    let panel = current(&state).system;
    if !panel.enabled {
        return Err("off".into());
    }
    Ok(state.system.lock().unwrap().sample(panel.gpu))
}

/// The other machine's load (the page asks every 4 s while it is on).
#[tauri::command]
async fn remote_host(state: tauri::State<'_, AppState>) -> Result<system::Stats, String> {
    let remote = current(&state).remote;
    if !remote.enabled || remote.host_url.trim().is_empty() {
        return Err("off".into());
    }
    remote::host(&remote).await
}

/// The other machine's Claude runs (the running ones, then the latest) and its chat session.
#[tauri::command]
async fn remote_runs(state: tauri::State<'_, AppState>) -> Result<remote::Runs, String> {
    let remote = current(&state).remote;
    if !remote.enabled || remote.runs_url.trim().is_empty() {
        return Err("off".into());
    }
    remote::runs(&remote).await
}

/// Which of this machine's sessions the session hub says wait for the owner (a permission, a question).
#[tauri::command]
async fn remote_waiting(state: tauri::State<'_, AppState>) -> Result<Vec<remote::Waiting>, String> {
    let remote = current(&state).remote;
    if !remote.enabled || remote.hub_url.trim().is_empty() {
        return Err("off".into());
    }
    remote::waiting(&remote).await
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
            app.manage(AppState { config_dir, settings: Mutex::new(settings), system: Mutex::new(system::Sampler::new()) });
            tray::create(app.handle(), russian)?;
            // autostart keeps the path of the exe that turned it on: one switched on from a dev build (a console
            // window) or an older install location would start that one — the installed app writes its own path back
            if !cfg!(debug_assertions) {
                use tauri_plugin_autostart::ManagerExt;
                let autostart = app.autolaunch();
                if autostart.is_enabled().unwrap_or(false) {
                    let _ = autostart.enable();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings_get,
            settings_set,
            pos_set,
            size_set,
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
            system_stats,
            remote_host,
            remote_runs,
            remote_waiting,
        ])
        .run(tauri::generate_context!())
        .expect("error while running cc-widget");
}
