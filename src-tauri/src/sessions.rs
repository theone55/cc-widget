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
    /// the short id of a background job (`<config dir>/jobs/<id>/`), what `claude attach` takes
    job_id: Option<String>,
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
    /// what `claude attach` takes for a bg session: the job id, else the session id
    attach_id: String,
    /// the account's resolved config dir (CLAUDE_CONFIG_DIR of the attach window)
    config_dir: String,
    status: String,
    /// ms since the epoch: when the status last changed
    since: u64,
    /// a bg session blocked by a permission prompt nobody sees (jobs/<id>/state.json)
    blocked: bool,
    /// the session that started this bg one, when its launcher left jobs/<id>/parent.json ({"sessionId": "..."})
    parent: Option<String>,
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

/// `<config dir>/jobs/<job id>/state.json` of a background job: while a permission prompt nobody sees blocks it,
/// it holds `"state":"working","tempo":"blocked"`. Internal file like the session's, read for bg sessions only.
#[derive(Deserialize, Default)]
struct JobState {
    state: Option<String>,
    tempo: Option<String>,
}

fn parse_job_state(text: &str) -> Option<JobState> {
    serde_json::from_str(text).ok()
}

fn is_blocked(s: &JobState) -> bool {
    s.state.as_deref() == Some("working") && s.tempo.as_deref() == Some("blocked")
}

/// true when the job waits on a permission prompt; a missing / unreadable file or odd id = false
fn job_blocked(dir: &std::path::Path, job_id: &str) -> bool {
    if job_id.is_empty() || job_id.contains(['/', '\\']) || job_id.contains("..") {
        return false;
    }
    std::fs::read_to_string(dir.join("jobs").join(job_id).join("state.json"))
        .ok()
        .and_then(|t| parse_job_state(&t))
        .is_some_and(|s| is_blocked(&s))
}

/// `<config dir>/jobs/<job id>/parent.json` — Claude Code doesn't record who started a bg job; a launcher that knows
/// (e.g. a script running `claude --bg` from a session) may leave `{"sessionId": "<the starting session>"}` there.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParentFile {
    session_id: Option<String>,
}

fn job_parent(dir: &std::path::Path, job_id: &str) -> Option<String> {
    if job_id.is_empty() || job_id.contains(['/', '\\']) || job_id.contains("..") {
        return None;
    }
    let text = std::fs::read_to_string(dir.join("jobs").join(job_id).join("parent.json")).ok()?;
    serde_json::from_str::<ParentFile>(&text).ok()?.session_id.filter(|s| !s.is_empty() && s.len() <= 80)
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
            let job_id = f.job_id.filter(|j| !j.is_empty());
            let blocked = f.kind.as_deref() == Some("bg") && job_id.as_deref().is_some_and(|j| job_blocked(&dir, j));
            let parent = if f.kind.as_deref() == Some("bg") { job_id.as_deref().and_then(|j| job_parent(&dir, j)) } else { None };
            let attach_id = job_id.unwrap_or_else(|| session_id.clone());
            out.push(Session {
                blocked,
                parent,
                attach_id,
                config_dir: dir.to_string_lossy().into_owned(),
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
    under_parents(out)
}

/// A background session whose parent is in the list goes right under it (several: newest first, as sorted);
/// one whose parent is gone keeps its place.
fn under_parents(sorted: Vec<Session>) -> Vec<Session> {
    let ids: std::collections::HashSet<String> = sorted.iter().map(|s| s.session_id.clone()).collect();
    let (children, top): (Vec<Session>, Vec<Session>) =
        sorted.into_iter().partition(|s| s.parent.as_ref().is_some_and(|p| ids.contains(p) && *p != s.session_id));
    let mut children: Vec<Option<Session>> = children.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(top.len() + children.len());
    for s in top {
        let id = s.session_id.clone();
        out.push(s);
        for c in children.iter_mut() {
            if c.as_ref().is_some_and(|c| c.parent.as_deref() == Some(id.as_str())) {
                out.push(c.take().unwrap());
            }
        }
    }
    out.extend(children.into_iter().flatten()); // a child of a child: kept, at the end
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_state_blocked_or_not() {
        assert!(is_blocked(&parse_job_state(r#"{"state":"working","tempo":"blocked"}"#).unwrap()));
        assert!(!is_blocked(&parse_job_state(r#"{"state":"working","tempo":"active"}"#).unwrap()));
        assert!(!is_blocked(&parse_job_state(r#"{"state":"done","tempo":"blocked"}"#).unwrap()));
        assert!(parse_job_state("not json").is_none());
    }

    #[test]
    fn job_state_from_disk_and_missing() {
        let dir = std::env::temp_dir().join(format!("cc-widget-jobs-{}", std::process::id()));
        let job = dir.join("jobs").join("abc12345");
        std::fs::create_dir_all(&job).unwrap();
        std::fs::write(job.join("state.json"), r#"{"state":"working","tempo":"blocked"}"#).unwrap();
        assert!(job_blocked(&dir, "abc12345"));
        assert!(!job_blocked(&dir, "nofile"));
        assert!(!job_blocked(&dir, "../abc12345"));
        std::fs::write(job.join("parent.json"), r#"{"sessionId":"p-1"}"#).unwrap();
        assert_eq!(job_parent(&dir, "abc12345").as_deref(), Some("p-1"));
        assert_eq!(job_parent(&dir, "nofile"), None);
        assert_eq!(job_parent(&dir, "../abc12345"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn s(id: &str, parent: Option<&str>) -> Session {
        Session {
            blocked: false, parent: parent.map(String::from), attach_id: String::new(), config_dir: String::new(),
            account: String::new(), account_index: 0, pid: 0, context: None, progress: None,
            session_id: id.into(), name: String::new(), cwd: String::new(), kind: String::new(), status: String::new(), since: 0,
        }
    }

    #[test]
    fn a_child_goes_under_its_parent() {
        let order = |v: Vec<Session>| under_parents(v).into_iter().map(|s| s.session_id).collect::<Vec<_>>();
        // sorted newest first: the children are newer than their parent
        assert_eq!(order(vec![s("c1", Some("p")), s("x", None), s("c2", Some("p")), s("p", None)]), ["x", "p", "c1", "c2"]);
        // a parent that is gone: the child keeps its place
        assert_eq!(order(vec![s("c", Some("gone")), s("x", None)]), ["c", "x"]);
    }
}
