//! Sessions of the other engines (Gemini / GPT tabs of Agora, rig runs). Claude Code writes its own session files, but
//! agy (Gemini) and Codex write none, so Agora keeps one file per such tab in `<local data>/agora/engine-sessions/`:
//! `<agora pid>-<tab id>.json`, rewritten on every status change and at least every 60 s while the tab is open (a
//! heartbeat), deleted when the tab or the window closes. Written atomically (`.tmp`, then rename).
//!
//! ```json
//! { "v": 1, "engine": "gemini" | "gpt", "account": "gemini", "model": "gemini-4-neon", "cwd": "E:\\work\\x",
//!   "title": "tab title", "status": "busy" | "idle" | "ask" | "error" | "limit", "pid": 1234, "tabId": "t3",
//!   "debug": false, "startedAt": 1795850000000, "updatedAt": 1795850060000, "doing": "last tool, may be empty" }
//! ```
//!
//! `pid` is the Agora process (a click focuses its window), `debug` marks a debug / rig window. A file with another `v`,
//! an unknown engine or no status is skipped; so is one whose pid is gone (the widget also deletes that leftover of a
//! crashed Agora) or whose `updatedAt` is older than 3 minutes. Everything in the files is untrusted text.

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// a file not rewritten for this long belongs to a frozen or crashed window (the heartbeat is 60 s)
const STALE_MS: u64 = 3 * 60_000;
/// a folder with more files than this is not an Agora's: the rest is ignored
const MAX_FILES: usize = 64;
const MAX_BYTES: u64 = 16 * 1024;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineSession {
    /// "gemini" or "gpt"
    pub engine: String,
    pub account: String,
    pub model: String,
    pub cwd: String,
    pub title: String,
    /// Agora's own status: busy / idle / ask / error / limit
    pub status: String,
    /// the widget's state: "working", "waiting" or "idle"
    pub state: String,
    /// the Agora process: what a click focuses
    pub pid: u32,
    pub tab_id: String,
    /// a debug / rig window
    pub debug: bool,
    pub started_at: u64,
    pub updated_at: u64,
    pub doing: String,
}

fn root() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("AGORA_ENGINE_SESSIONS_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).or_else(|| crate::paths::home().map(|h| h.join("AppData").join("Local")))?
    } else {
        crate::paths::home()?.join(".local").join("share")
    };
    Some(base.join("agora").join("engine-sessions"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn text_of(v: &Value, key: &str, max: usize) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").chars().take(max).collect()
}

fn ms_of(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64().or_else(|| x.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))).unwrap_or(0)
}

/// Agora's tab status → the widget's state: busy → working; ask / error / limit → waiting (the tab needs you);
/// idle → idle. Anything else is not a status of the contract.
pub fn state_of(status: &str) -> Option<&'static str> {
    match status {
        "busy" => Some("working"),
        "ask" | "error" | "limit" => Some("waiting"),
        "idle" => Some("idle"),
        _ => None,
    }
}

/// One file's text → a session; `None` = not ours (bad JSON, another `v`, unknown engine / status, no pid).
pub fn parse(text: &str) -> Option<EngineSession> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v.get("v").and_then(Value::as_u64) != Some(1) {
        return None;
    }
    let engine = match v.get("engine").and_then(Value::as_str)? {
        "gemini" => "gemini",
        "gpt" => "gpt",
        _ => return None,
    };
    let status = text_of(&v, "status", 20);
    let state = state_of(&status)?;
    let pid = u32::try_from(v.get("pid").and_then(Value::as_u64)?).ok().filter(|p| *p > 0)?;
    Some(EngineSession {
        engine: engine.into(),
        account: text_of(&v, "account", 40),
        model: text_of(&v, "model", 60),
        cwd: text_of(&v, "cwd", 300),
        title: text_of(&v, "title", 120),
        status,
        state: state.into(),
        pid,
        tab_id: text_of(&v, "tabId", 40),
        debug: v.get("debug").and_then(Value::as_bool).unwrap_or(false),
        started_at: ms_of(&v, "startedAt"),
        updated_at: ms_of(&v, "updatedAt"),
        doing: text_of(&v, "doing", 200),
    })
}

/// Not rewritten for 3 minutes (a missing `updatedAt` counts as the epoch: stale).
pub fn stale(s: &EngineSession, now: u64) -> bool {
    now.saturating_sub(s.updated_at) > STALE_MS
}

/// `<pid>-<tab id>.json`, the characters of a file name Agora writes.
fn session_file(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".json") else { return false };
    let Some((pid, tab)) = stem.split_once('-') else { return false };
    !pid.is_empty() && pid.chars().all(|c| c.is_ascii_digit()) && !tab.is_empty() && tab.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The live sessions of a folder: parsed, not stale, pid alive; the files of dead pids are deleted (a crashed Agora's
/// leftovers). Oldest tab first, so rows don't reshuffle with every heartbeat.
fn scan(dir: &Path, now: u64, alive: &dyn Fn(u32) -> bool) -> Vec<EngineSession> {
    let mut out = vec![];
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten().take(MAX_FILES) {
        let name = entry.file_name();
        if !name.to_str().is_some_and(session_file) {
            continue;
        }
        let path = entry.path();
        // a file being renamed into place right now is just skipped until the next poll
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.len() > MAX_BYTES {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Some(s) = parse(&text) else { continue };
        if !alive(s.pid) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        if stale(&s, now) {
            continue;
        }
        out.push(s);
    }
    out.sort_by(|a, b| a.started_at.cmp(&b.started_at).then_with(|| (a.pid, &a.tab_id).cmp(&(b.pid, &b.tab_id))));
    out
}

pub fn read() -> Vec<EngineSession> {
    match root() {
        Some(dir) => scan(&dir, now_ms(), &crate::sessions::pid_alive),
        None => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(status: &str, pid: u32, updated: u64) -> String {
        format!(
            r#"{{"v":1,"engine":"gemini","account":"gemini","model":"gemini-4-neon","cwd":"E:\\work\\x","title":"t","status":"{status}","pid":{pid},"tabId":"t3","debug":false,"startedAt":1000,"updatedAt":{updated},"doing":"edit"}}"#
        )
    }

    #[test]
    fn the_status_map() {
        assert_eq!(state_of("busy"), Some("working"));
        for s in ["ask", "error", "limit"] {
            assert_eq!(state_of(s), Some("waiting"), "{s}");
        }
        assert_eq!(state_of("idle"), Some("idle"));
        assert_eq!(state_of("shell"), None);
        assert_eq!(state_of(""), None);
    }

    #[test]
    fn parsing_the_contract() {
        let s = parse(&file("busy", 1234, 5000)).unwrap();
        assert_eq!((s.engine.as_str(), s.state.as_str(), s.status.as_str()), ("gemini", "working", "busy"));
        assert_eq!((s.pid, s.tab_id.as_str(), s.debug, s.started_at, s.updated_at), (1234, "t3", false, 1000, 5000));
        assert_eq!((s.account.as_str(), s.model.as_str(), s.cwd.as_str(), s.doing.as_str()), ("gemini", "gemini-4-neon", r"E:\work\x", "edit"));
        let g = parse(r#"{"v":1,"engine":"gpt","status":"limit","pid":7,"debug":true}"#).unwrap();
        assert_eq!((g.engine.as_str(), g.state.as_str(), g.debug, g.updated_at), ("gpt", "waiting", true, 0));
    }

    #[test]
    fn what_is_not_ours_is_skipped() {
        assert!(parse("not json").is_none());
        assert!(parse(&file("busy", 1, 1).replace(r#""v":1"#, r#""v":2"#)).is_none());
        assert!(parse(&file("busy", 1, 1).replace(r#""v":1,"#, "")).is_none());
        assert!(parse(&file("busy", 1, 1).replace("gemini\",\"account", "claude\",\"account")).is_none());
        assert!(parse(&file("sleeping", 1, 1)).is_none());
        assert!(parse(&file("busy", 0, 1)).is_none());
        assert!(parse(r#"{"v":1,"engine":"gpt","status":"busy"}"#).is_none()); // no pid
        assert!(parse(r#"{"v":1,"engine":"gpt","status":"busy","pid":-4}"#).is_none());
    }

    #[test]
    fn long_text_is_cut() {
        let long = "я".repeat(500);
        let s = parse(&format!(r#"{{"v":1,"engine":"gpt","status":"idle","pid":3,"title":"{long}","doing":"{long}","cwd":"{long}"}}"#)).unwrap();
        assert_eq!((s.title.chars().count(), s.doing.chars().count(), s.cwd.chars().count()), (120, 200, 300));
    }

    #[test]
    fn staleness_is_three_minutes() {
        let s = parse(&file("busy", 1, 1_000_000)).unwrap();
        assert!(!stale(&s, 1_000_000 + STALE_MS));
        assert!(stale(&s, 1_000_000 + STALE_MS + 1));
        assert!(!stale(&s, 500_000)); // a clock step back is not stale
        assert!(stale(&parse(r#"{"v":1,"engine":"gpt","status":"idle","pid":3}"#).unwrap(), 10 * STALE_MS)); // no updatedAt
    }

    #[test]
    fn file_names() {
        assert!(session_file("1234-t3.json") && session_file("1234-t3-a_b.json"));
        assert!(!session_file("1234-t3.json.tmp") && !session_file("t3.json") && !session_file("x-t3.json") && !session_file("1234-.json"));
        assert!(!session_file("1234-../x.json") && !session_file("1234-a b.json"));
    }

    #[test]
    fn a_folder_keeps_live_fresh_files_and_drops_dead_pids() {
        let dir = std::env::temp_dir().join(format!("cc-widget-engines-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let w = |n: &str, t: &str| std::fs::write(dir.join(n), t).unwrap();
        let now = 10_000_000;
        w("100-t1.json", &file("busy", 100, now - 1_000)); // live and fresh
        w("100-t2.json", &file("ask", 100, now - STALE_MS - 1)); // live but stale: hidden, kept
        w("200-t1.json", &file("busy", 200, now)); // dead pid: hidden and deleted
        w("100-t3.json", "{ half written"); // skipped
        w("100-t4.json.tmp", &file("busy", 100, now)); // an unfinished write
        w("notes.txt", "x");
        let alive = |pid: u32| pid == 100;
        let got = scan(&dir, now, &alive);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].pid, got[0].state.as_str()), (100, "working"));
        assert!(dir.join("100-t2.json").exists() && dir.join("100-t3.json").exists());
        assert!(!dir.join("200-t1.json").exists());
        assert!(scan(&dir.join("missing"), now, &alive).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
