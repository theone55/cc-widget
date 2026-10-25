//! Claude Code writes one JSON per running session into `<config dir>/sessions/<pid>.json` and keeps its `status`
//! current: `busy` while it works, `idle` once the turn is over and it waits for you. This is Claude Code's own
//! internal file, not a documented API — if a Claude Code update changes it, the list goes empty until this is
//! adapted (nothing breaks).

use crate::{paths, settings::Account};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionFile {
    pid: u32,
    session_id: Option<String>,
    cwd: Option<String>,
    name: Option<String>,
    kind: Option<String>,
    status: Option<String>,
    status_updated_at: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// the account's label from the settings
    account: String,
    /// its index in the settings (the UI colors accounts by it)
    account_index: usize,
    pid: u32,
    session_id: String,
    name: String,
    cwd: String,
    /// "interactive" (a terminal) or "bg" (a background job)
    kind: String,
    status: String,
    /// ms since the epoch: when the status last changed
    since: u64,
    /// how full the context window is, % — from the statusline's file
    context: Option<f64>,
    /// the session's task list, if Claude keeps one (the session-progress mod)
    progress: Option<Progress>,
}

/// `<config dir>/session-progress/<session id>.json`, written by the session-progress mod each time Claude
/// updates its task list: every task with a rough percent done.
#[derive(Serialize, Deserialize)]
pub struct Progress {
    #[serde(default)]
    overall: u32,
    #[serde(default)]
    tasks: Vec<Task>,
}

#[derive(Serialize, Deserialize)]
pub struct Task {
    title: String,
    percent: u32,
    #[serde(default)]
    active: bool,
}

fn progress(dir: &std::path::Path, session_id: &str) -> Option<Progress> {
    if session_id.is_empty() {
        return None;
    }
    let text = std::fs::read_to_string(dir.join("session-progress").join(format!("{session_id}.json"))).ok()?;
    serde_json::from_str::<Progress>(&text).ok().filter(|p| !p.tasks.is_empty())
}

#[derive(Deserialize)]
struct CtxFile {
    used_pct: Option<f64>,
}

fn context_used(session_id: &str) -> Option<f64> {
    let text = std::fs::read_to_string(paths::context_file(session_id)?).ok()?;
    serde_json::from_str::<CtxFile>(&text).ok()?.used_pct
}

/// The sessions of every account whose process is still alive (a crashed session leaves its file behind),
/// the most recently changed first.
pub fn read(accounts: &[Account]) -> Vec<Session> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let mut out = vec![];
    for (index, account) in accounts.iter().enumerate() {
        let Some(dir) = paths::resolve(&account.dir) else { continue };
        let Ok(entries) = std::fs::read_dir(dir.join("sessions")) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            // a file being rewritten right now is just skipped until the next poll
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Ok(f) = serde_json::from_str::<SessionFile>(&text) else { continue };
            if sys.process(sysinfo::Pid::from_u32(f.pid)).is_none() {
                continue;
            }
            let session_id = f.session_id.unwrap_or_default();
            out.push(Session {
                account: account.label.clone(),
                account_index: index,
                pid: f.pid,
                context: context_used(&session_id),
                progress: progress(&dir, &session_id),
                session_id,
                name: f.name.unwrap_or_default(),
                cwd: f.cwd.unwrap_or_default(),
                kind: f.kind.unwrap_or_default(),
                status: f.status.unwrap_or_default(),
                since: f.status_updated_at.unwrap_or(0),
            });
        }
    }
    out.sort_by(|a, b| b.since.cmp(&a.since));
    out
}

// ---- limits: per ACCOUNT (every session of an account spends the same 5h / 7d budget) ----

#[derive(Serialize, Deserialize)]
pub struct Bucket {
    used_percentage: Option<f64>,
    /// epoch seconds
    resets_at: Option<u64>,
}

#[derive(Serialize, Deserialize)]
pub struct LimitsFile {
    five_hour: Option<Bucket>,
    seven_day: Option<Bucket>,
    #[serde(rename = "updatedAt")]
    updated_at: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    account: String,
    account_index: usize,
    #[serde(flatten)]
    file: LimitsFile,
}

pub fn limits(accounts: &[Account]) -> Vec<Limits> {
    accounts
        .iter()
        .enumerate()
        .filter_map(|(index, account)| {
            let dir = paths::resolve(&account.dir)?;
            let text = std::fs::read_to_string(paths::limits_file(&dir)).ok()?;
            let file = serde_json::from_str::<LimitsFile>(&text).ok()?;
            Some(Limits { account: account.label.clone(), account_index: index, file })
        })
        .collect()
}
