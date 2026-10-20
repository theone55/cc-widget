//! The widget's settings: one JSON file in the app's config folder. Every field has a default, so an old file or a
//! hand-edited one with fields missing still loads.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct Account {
    /// shown on the widget ("work", "personal"…)
    pub label: String,
    /// the Claude Code config folder: ".claude", "~/.claude-work", or an absolute path
    pub dir: String,
    /// its color on the widget ("#60a5fa"); empty = one from the built-in palette by position
    pub color: String,
}

impl Default for Account {
    fn default() -> Self {
        Account { label: "claude".into(), dir: ".claude".into(), color: String::new() }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct Colors {
    pub ok: String,
    pub warn: String,
    pub full: String,
    pub accent: String,
}

impl Default for Colors {
    fn default() -> Self {
        Colors { ok: "#4ade80".into(), warn: "#fb923c".into(), full: "#f87171".into(), accent: "#a78bfa".into() }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct Thresholds {
    /// context %: from here "time to /compact"
    pub context_warn: u8,
    /// context %: from here "time to /clear"
    pub context_full: u8,
    pub limit_warn: u8,
    pub limit_full: u8,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds { context_warn: 70, context_full: 85, limit_warn: 70, limit_full: 90 }
    }
}

/// An optional number from your own server, shown in the header (e.g. a site's visitors today / 7 d / 30 d).
/// GET `url` (with a bearer token if one is saved) → JSON; `fields` are dot paths into it.
/// With `passUrl` the saved token is first swapped there for a short-lived pass (`{"pass": "…"}`), which is
/// then the bearer for `url`.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Counter {
    pub enabled: bool,
    pub label: String,
    pub url: String,
    pub pass_url: String,
    pub fields: Vec<String>,
    /// minutes between fetches
    pub every: u32,
}

/// An optional other machine (a home server running Claude unattended): its load like the System section, and its
/// Claude runs (remote.rs has the formats). Uses the counter's token; `passUrl` is only for the runs.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Remote {
    pub enabled: bool,
    /// the group's name in the widget ("server", "119")
    pub label: String,
    /// its color, like an account's ("" = the palette's)
    pub color: String,
    pub host_url: String,
    pub runs_url: String,
    /// the session hub (be-claude-bridge /api/admin/hub/sessions): which of this machine's sessions wait for a
    /// permission or an answer — the local session files don't say (through the pass URL)
    pub hub_url: String,
    pub pass_url: String,
}

/// Where the window is (physical pixels of the desktop).
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
}

/// The size you dragged the window to (logical pixels). `h`: the height you let go at — kept as it is (since 0.1.6;
/// a size saved before has none and opens at the full layout's height); `level` = how dense the layout was, 0..8.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Size {
    pub w: f64,
    #[serde(default)]
    pub h: Option<f64>,
    pub level: u8,
}

/// The optional "System" section: CPU (each thread), memory, temperatures and an NVIDIA card's load.
#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct SystemPanel {
    pub enabled: bool,
    /// a tile per CPU core instead of one CPU bar
    pub threads: bool,
    /// the graphics card (NVIDIA, through nvidia-smi)
    pub gpu: bool,
}

impl Default for SystemPanel {
    fn default() -> Self {
        SystemPanel { enabled: false, threads: true, gpu: true }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// "auto" (the system's), "dark", "light"
    pub theme: String,
    /// "auto" (the system's), "en", "ru"
    pub language: String,
    /// the background's opacity, 30..100
    pub opacity: u8,
    pub colors: Colors,
    pub thresholds: Thresholds,
    pub accounts: Vec<Account>,
    pub show_limits: bool,
    pub show_context: bool,
    /// background jobs (`claude --bg`) too
    pub show_background: bool,
    /// the details (folder, time, resets) open
    pub expanded: bool,
    pub counter: Counter,
    pub system: SystemPanel,
    pub remote: Remote,
    /// the window's place and size, saved here on every change (they used to live in the webview's localStorage,
    /// which is written to disk lazily: a shutdown lost them)
    pub pos: Option<Pos>,
    /// None = the automatic size
    pub size: Option<Size>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "auto".into(),
            language: "auto".into(),
            opacity: 94,
            colors: Colors::default(),
            thresholds: Thresholds::default(),
            accounts: crate::paths::detect_accounts()
                .into_iter()
                .map(|dir| {
                    let label = dir.trim_start_matches(".claude").trim_start_matches('-');
                    Account { label: if label.is_empty() { "claude".into() } else { label.into() }, dir, color: String::new() }
                })
                .collect(),
            show_limits: true,
            show_context: true,
            show_background: true,
            expanded: false,
            counter: Counter { every: 5, ..Counter::default() },
            system: SystemPanel::default(),
            remote: Remote::default(),
            pos: None,
            size: None,
        }
    }
}

pub fn file(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("settings.json")
}

pub fn load(config_dir: &std::path::Path) -> Settings {
    std::fs::read_to_string(file(config_dir))
        .ok()
        // a file saved by Notepad / PowerShell may start with a BOM
        .and_then(|t| serde_json::from_str(t.trim_start_matches('﻿')).ok())
        .unwrap_or_default()
}

pub fn save(config_dir: &std::path::Path, settings: &Settings) -> Result<(), String> {
    std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    // write a temp file and rename it: a crash mid-write never leaves half a settings file
    let tmp = config_dir.join("settings.json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, file(config_dir)).map_err(|e| e.to_string())
}
