//! Where things are: the home folder, the Claude Code config folders, the temp folder.

use std::path::PathBuf;

pub fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// A config folder from the settings: absolute, or relative to the home folder (".claude"), "~/" allowed.
pub fn resolve(dir: &str) -> Option<PathBuf> {
    let dir = dir.trim();
    if dir.is_empty() {
        return None;
    }
    let rest = dir.strip_prefix("~/").or_else(|| dir.strip_prefix("~\\")).unwrap_or(dir);
    let p = PathBuf::from(rest);
    if p.is_absolute() {
        Some(p)
    } else {
        home().map(|h| h.join(p))
    }
}

/// Claude Code config folders found on this machine: ~/.claude, every ~/.claude-* (more accounts via
/// CLAUDE_CONFIG_DIR) and $CLAUDE_CONFIG_DIR itself — the ones that have a `sessions` folder.
pub fn detect_accounts() -> Vec<String> {
    let mut out = vec![];
    if let Some(home) = home() {
        if let Ok(entries) = std::fs::read_dir(&home) {
            let mut names: Vec<String> = entries
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n == ".claude" || n.starts_with(".claude-"))
                .filter(|n| home.join(n).join("sessions").is_dir())
                .collect();
            names.sort();
            out.extend(names);
        }
    }
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").and_then(|d| d.into_string().ok()) {
        let known = out.iter().any(|d| resolve(d) == resolve(&dir));
        if !known && PathBuf::from(&dir).join("sessions").is_dir() {
            out.push(dir);
        }
    }
    if out.is_empty() {
        out.push(".claude".into());
    }
    out
}

/// The context file a statusline leaves per session (the same name the owner's statusline.js uses).
pub fn context_file(session_id: &str) -> Option<PathBuf> {
    // the id goes into a path: only the UUID-like ids Claude Code writes
    if session_id.is_empty()
        || session_id.len() > 128
        || !session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(std::env::temp_dir().join(format!("claude-ctx-{session_id}.json")))
}

/// The account's limits, written by the statusline into its config folder.
pub fn limits_file(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("widget-limits.json")
}
