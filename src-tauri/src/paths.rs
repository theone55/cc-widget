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

/// GPT (Codex CLI) account folders: ~/.codex, every ~/.codex-* and $CODEX_HOME — the ones with an `auth.json`.
pub fn detect_codex() -> Vec<String> {
    detect_codex_in(home().as_deref(), std::env::var("CODEX_HOME").ok())
}

pub fn detect_codex_in(home: Option<&std::path::Path>, env_dir: Option<String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    if let Some(home) = home {
        if let Ok(entries) = std::fs::read_dir(home) {
            let mut names: Vec<String> = entries
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n == ".codex" || n.starts_with(".codex-"))
                .filter(|n| home.join(n).join("auth.json").is_file())
                .collect();
            names.sort();
            out.extend(names);
        }
    }
    if let Some(dir) = env_dir.filter(|d| !d.trim().is_empty()) {
        let p = PathBuf::from(&dir);
        let known = out.iter().any(|d| match home {
            Some(h) => h.join(d) == p,
            None => false,
        });
        if !known && p.join("auth.json").is_file() {
            out.push(dir);
        }
    }
    out
}

/// The label of a codex folder: ".codex" → "gpt", ".codex-2" → "gpt-2", any other folder → "gpt"
pub fn codex_label(dir: &str) -> String {
    let name = std::path::Path::new(dir.trim_end_matches(['/', '\\'])).file_name().and_then(|n| n.to_str()).unwrap_or("");
    match name.strip_prefix(".codex-") {
        Some(x) if !x.is_empty() => format!("gpt-{}", x.to_lowercase()),
        _ => "gpt".into(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("cc-widget-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn finds_codex_folders_with_auth_only() {
        let h = temp_home("codex");
        for d in [".codex", ".codex-2", ".codex-empty", ".claude"] {
            std::fs::create_dir_all(h.join(d)).unwrap();
        }
        std::fs::write(h.join(".codex/auth.json"), "{}").unwrap();
        std::fs::write(h.join(".codex-2/auth.json"), "{}").unwrap();
        // $CODEX_HOME elsewhere
        let extra = h.join("elsewhere");
        std::fs::create_dir_all(&extra).unwrap();
        std::fs::write(extra.join("auth.json"), "{}").unwrap();
        let found = detect_codex_in(Some(&h), Some(extra.to_string_lossy().into_owned()));
        assert_eq!(found, vec![".codex".to_string(), ".codex-2".to_string(), extra.to_string_lossy().into_owned()]);
        // $CODEX_HOME pointing at an already found folder is not added twice
        let again = detect_codex_in(Some(&h), Some(h.join(".codex").to_string_lossy().into_owned()));
        assert_eq!(again, vec![".codex".to_string(), ".codex-2".to_string()]);
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn codex_labels() {
        assert_eq!(codex_label(".codex"), "gpt");
        assert_eq!(codex_label(".codex-2"), "gpt-2");
        assert_eq!(codex_label(".codex-Work"), "gpt-work");
        assert_eq!(codex_label("/somewhere/else"), "gpt");
    }
}
