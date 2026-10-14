//! Limits and context come from Claude Code's statusline: Claude Code pipes a JSON with `rate_limits` and
//! `context_window` into the statusline command on every update. "Connect" puts this widget's own binary in front
//! of the account's statusline (`cc-widget --statusline <config dir>`): it saves the numbers for the widget, then runs
//! your previous statusline command with the same input and prints its output, so the terminal looks as before.
//! "Disconnect" puts the previous statusline back.

use crate::paths;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SAVED: &str = "cc-widget-statusline.json";
const BACKUP: &str = "settings.json.cc-widget-backup";

fn exe() -> Result<String, String> {
    let p = std::env::current_exe().map_err(|e| e.to_string())?;
    // forward slashes: Claude Code may run the command through bash on Windows too
    Ok(p.to_string_lossy().replace('\\', "/"))
}

fn our_command(dir: &Path) -> Result<String, String> {
    Ok(format!("\"{}\" --statusline \"{}\"", exe()?, dir.to_string_lossy().replace('\\', "/")))
}

fn is_ours(status_line: &Value) -> bool {
    status_line.get("command").and_then(|c| c.as_str()).is_some_and(|c| c.contains("--statusline") && c.contains("cc-widget"))
}

fn read_settings(dir: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(dir.join("settings.json")) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text).map_err(|e| format!("settings.json: {e}")),
        _ => Ok(json!({})),
    }
}

fn write_settings(dir: &Path, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = dir.join("settings.json.cc-widget-tmp");
    std::fs::write(&tmp, text + "\n").map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join("settings.json")).map_err(|e| e.to_string())
}

/// "connected" (ours), "other" (another statusline already writes the limits file), "off"
pub fn status(config_dir: &str) -> String {
    let Some(dir) = paths::resolve(config_dir) else { return "off".into() };
    let settings = read_settings(&dir).unwrap_or(json!({}));
    if settings.get("statusLine").is_some_and(is_ours) {
        return "connected".into();
    }
    if paths::limits_file(&dir).exists() {
        return "other".into();
    }
    "off".into()
}

pub fn connect(config_dir: &str) -> Result<(), String> {
    let dir = paths::resolve(config_dir).ok_or("no such folder")?;
    if !dir.is_dir() {
        return Err(format!("{} does not exist", dir.display()));
    }
    let mut settings = read_settings(&dir)?;
    let previous = settings.get("statusLine").cloned();
    if previous.as_ref().is_some_and(is_ours) {
        return Ok(());
    }
    // a backup of the settings as they were, and the previous statusline to run behind ours
    if dir.join("settings.json").exists() {
        std::fs::copy(dir.join("settings.json"), dir.join(BACKUP)).map_err(|e| e.to_string())?;
    }
    std::fs::write(dir.join(SAVED), serde_json::to_string_pretty(&json!({ "statusLine": previous })).unwrap())
        .map_err(|e| e.to_string())?;
    let refresh = previous.as_ref().and_then(|p| p.get("refreshInterval")).cloned().unwrap_or(json!(30));
    let mut line = json!({ "type": "command", "command": our_command(&dir)?, "refreshInterval": refresh });
    if let Some(padding) = previous.as_ref().and_then(|p| p.get("padding")) {
        line["padding"] = padding.clone();
    }
    settings
        .as_object_mut()
        .ok_or("settings.json is not a JSON object")?
        .insert("statusLine".into(), line);
    write_settings(&dir, &settings)
}

pub fn disconnect(config_dir: &str) -> Result<(), String> {
    let dir = paths::resolve(config_dir).ok_or("no such folder")?;
    let mut settings = read_settings(&dir)?;
    if !settings.get("statusLine").is_some_and(is_ours) {
        return Ok(());
    }
    let previous = std::fs::read_to_string(dir.join(SAVED))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("statusLine").cloned())
        .filter(|v| !v.is_null());
    let obj = settings.as_object_mut().ok_or("settings.json is not a JSON object")?;
    match previous {
        Some(p) => obj.insert("statusLine".into(), p),
        None => obj.remove("statusLine"),
    };
    write_settings(&dir, &settings)?;
    let _ = std::fs::remove_file(dir.join(SAVED));
    Ok(())
}

// ---- `cc-widget --statusline <dir>`: runs on every statusline update, so it must be quick and never fail loudly ----

pub fn run(config_dir: &str) -> i32 {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let dir = PathBuf::from(config_dir);
    let data: Value = serde_json::from_str(&input).unwrap_or(json!({}));
    save_numbers(&dir, &data);

    let previous = std::fs::read_to_string(dir.join(SAVED))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("statusLine").and_then(|s| s.get("command")).and_then(|c| c.as_str()).map(String::from));
    let out = match previous {
        Some(command) => run_previous(&command, &input).unwrap_or_default(),
        None => default_line(&data),
    };
    let _ = std::io::stdout().write_all(out.as_bytes());
    0
}

fn save_numbers(dir: &Path, data: &Value) {
    if let Some(rl) = data.get("rate_limits").filter(|r| r.is_object()) {
        // every session of the account runs its statusline, an idle one with the numbers of its last answer (maybe
        // a window that has already reset): keep the newer of what's in the file and what this session says
        let old: Value = std::fs::read_to_string(paths::limits_file(dir))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(json!({}));
        let file = json!({
            "five_hour": newer(old.get("five_hour"), rl.get("five_hour")),
            "seven_day": newer(old.get("seven_day"), rl.get("seven_day")),
            "updatedAt": now_ms(),
        });
        let _ = std::fs::write(paths::limits_file(dir), file.to_string());
    }
    let session = data.get("session_id").and_then(|s| s.as_str()).unwrap_or("");
    if let (Some(path), Some(used)) = (paths::context_file(session), context_used(data)) {
        let file = json!({ "session_id": session, "used_pct": used, "timestamp": now_ms() / 1000 });
        let _ = std::fs::write(path, file.to_string());
    }
}

/// Of two snapshots of one limit window: the later window (resets_at), and within the same window the higher use
/// (usage only grows until the reset).
fn newer(a: Option<&Value>, b: Option<&Value>) -> Value {
    let reset = |v: &Value| v.get("resets_at").and_then(|r| r.as_f64()).unwrap_or(0.0);
    let used = |v: &Value| v.get("used_percentage").and_then(|r| r.as_f64()).unwrap_or(0.0);
    match (a.filter(|v| v.is_object()), b.filter(|v| v.is_object())) {
        (Some(a), Some(b)) => {
            if reset(b) > reset(a) || (reset(b) == reset(a) && used(b) >= used(a)) {
                b.clone()
            } else {
                a.clone()
            }
        }
        (Some(a), None) => a.clone(),
        (None, Some(b)) => b.clone(),
        (None, None) => Value::Null,
    }
}

/// The context fill, %: tokens in / window size when both are there, else 100 − remaining_percentage.
fn context_used(data: &Value) -> Option<f64> {
    let cw = data.get("context_window")?;
    let total = cw.get("total_input_tokens").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let size = cw.get("context_window_size").and_then(|v| v.as_f64()).unwrap_or(0.0);
    if total > 0.0 && size > 0.0 {
        return Some((total / size * 100.0).clamp(0.0, 100.0).round());
    }
    let remaining = cw.get("remaining_percentage").and_then(|v| v.as_f64())?;
    Some((100.0 - remaining).clamp(0.0, 100.0).round())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn run_previous(command: &str, input: &str) -> Option<String> {
    let mut child = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    }
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .ok()?;
    child.stdin.take()?.write_all(input.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// When there was no statusline before: a short one of our own.
fn default_line(data: &Value) -> String {
    let mut parts = vec![];
    if let Some(m) = data.get("model").and_then(|m| m.get("display_name")).and_then(|m| m.as_str()) {
        parts.push(m.to_string());
    }
    if let Some(c) = context_used(data) {
        parts.push(format!("ctx {c}%"));
    }
    for (key, label) in [("five_hour", "5h"), ("seven_day", "7d")] {
        if let Some(p) = data.get("rate_limits").and_then(|r| r.get(key)).and_then(|b| b.get("used_percentage")).and_then(|v| v.as_f64()) {
            parts.push(format!("{label} {}%", p.round()));
        }
    }
    parts.join(" │ ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cc-widget-test-{name}-{}", now_ms()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn connect_keeps_the_previous_statusline_and_disconnect_restores_it() {
        let dir = temp_dir("connect");
        let before = json!({ "model": "opus", "statusLine": { "type": "command", "command": "node sl.js", "refreshInterval": 60 } });
        std::fs::write(dir.join("settings.json"), before.to_string()).unwrap();
        let d = dir.to_string_lossy().to_string();

        connect(&d).unwrap();
        let s = read_settings(&dir).unwrap();
        assert!(is_ours(&s["statusLine"]));
        assert_eq!(s["statusLine"]["refreshInterval"], 60);
        assert_eq!(s["model"], "opus");
        assert_eq!(status(&d), "connected");
        connect(&d).unwrap(); // twice is harmless: the saved previous one stays the user's
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(SAVED)).unwrap()).unwrap();
        assert_eq!(saved["statusLine"]["command"], "node sl.js");

        disconnect(&d).unwrap();
        assert_eq!(read_settings(&dir).unwrap(), before);
        assert!(!dir.join(SAVED).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn connect_without_a_statusline_and_disconnect_removes_ours() {
        let dir = temp_dir("bare");
        let d = dir.to_string_lossy().to_string();
        connect(&d).unwrap();
        assert!(is_ours(&read_settings(&dir).unwrap()["statusLine"]));
        disconnect(&d).unwrap();
        assert_eq!(read_settings(&dir).unwrap(), json!({}));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_numbers_are_saved_for_the_widget() {
        let dir = temp_dir("numbers");
        let data = json!({
            "session_id": "test-session-cc-widget",
            "context_window": { "total_input_tokens": 50000, "context_window_size": 200000 },
            "rate_limits": { "five_hour": { "used_percentage": 12.4, "resets_at": 1 }, "seven_day": { "used_percentage": 40 } }
        });
        save_numbers(&dir, &data);
        let limits: Value = serde_json::from_str(&std::fs::read_to_string(paths::limits_file(&dir)).unwrap()).unwrap();
        assert_eq!(limits["five_hour"]["used_percentage"], 12.4);
        let ctx_path = paths::context_file("test-session-cc-widget").unwrap();
        let ctx: Value = serde_json::from_str(&std::fs::read_to_string(&ctx_path).unwrap()).unwrap();
        assert_eq!(ctx["used_pct"], 25.0);
        assert_eq!(default_line(&data), "ctx 25% │ 5h 12% │ 7d 40%");
        let _ = std::fs::remove_file(ctx_path);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_idle_sessions_old_numbers_never_replace_newer_ones() {
        let dir = temp_dir("newer");
        let now_window = json!({ "rate_limits": { "five_hour": { "used_percentage": 8, "resets_at": 2000 } } });
        let old_window = json!({ "rate_limits": { "five_hour": { "used_percentage": 60, "resets_at": 1000 } } });
        save_numbers(&dir, &now_window);
        save_numbers(&dir, &old_window);
        let read = || serde_json::from_str::<Value>(&std::fs::read_to_string(paths::limits_file(&dir)).unwrap()).unwrap();
        assert_eq!(read()["five_hour"]["used_percentage"], 8);
        // the same window: the higher use wins whichever session writes last
        save_numbers(&dir, &json!({ "rate_limits": { "five_hour": { "used_percentage": 5, "resets_at": 2000 } } }));
        assert_eq!(read()["five_hour"]["used_percentage"], 8);
        save_numbers(&dir, &json!({ "rate_limits": { "five_hour": { "used_percentage": 9, "resets_at": 2000 } } }));
        assert_eq!(read()["five_hour"]["used_percentage"], 9);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn context_falls_back_to_remaining_percentage() {
        assert_eq!(context_used(&json!({ "context_window": { "remaining_percentage": 30 } })), Some(70.0));
        assert_eq!(context_used(&json!({})), None);
    }

    #[test]
    fn a_session_id_never_leaves_the_temp_folder() {
        assert!(paths::context_file("../../etc/passwd").is_none());
        assert!(paths::context_file("").is_none());
        assert!(paths::context_file("4c5ded12-4cfe-4814-b12f-a805e95a7694").is_some());
    }
}
