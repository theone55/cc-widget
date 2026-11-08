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
    /// the question a bg session ended its turn with (state "blocked" + needs); `blocked` is true then too
    needs: Option<String>,
    /// the session that started this bg one, when its launcher left jobs/<id>/parent.json ({"sessionId": "..."})
    parent: Option<String>,
    /// how full the context window is, % — from the statusline's file
    context: Option<f64>,
    /// the session's task list, if Claude keeps one (the session-progress mod)
    progress: Option<Progress>,
    /// short model name from the transcript's last assistant message ("opus 5.5")
    model: Option<String>,
    /// the Agent-tool subagents running (or just finished) inside this session
    subagents: Vec<Subagent>,
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
    read_progress(&dir.join("session-progress").join(format!("{session_id}.json")))
}

fn read_progress(path: &std::path::Path) -> Option<Progress> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Progress>(&text).ok().filter(|p| !p.tasks.is_empty())
}

// ---- subagents (Agent tool) of a session ----

/// `agent-<id>.meta.json` next to a subagent's transcript (`projects/<key>/<session id>/subagents/`). Internal file.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentMeta {
    agent_type: Option<String>,
    description: Option<String>,
    /// "background" for a run_in_background agent
    request_shape: Option<String>,
    finished: Option<bool>,
    /// the alias the agent was started with ("sonnet"); the transcript knows the exact model
    model: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Subagent {
    id: String,
    agent_type: String,
    description: String,
    running: bool,
    background: bool,
    /// short model name ("sonnet 5.5")
    model: Option<String>,
    /// ms since the epoch: when it started (running) or finished (done)
    since: u64,
    /// its own task list: `session-progress/<session id>/agent-<id>.json` (the mod writes it for a subagent's calls)
    progress: Option<Progress>,
}

/// a finished subagent stays in the list this long
const SUB_DONE_MS: u64 = 3 * 60_000;
/// an unfinished one whose transcript hasn't grown for this long is taken for dead (crashed / killed parent)
const SUB_STALE_MS: u64 = 10 * 60_000;
const SUB_MAX: usize = 8;

/// Some(running) if the agent is worth listing now, None to hide it. `quiet_ms` = time since its transcript last grew.
fn sub_visible(finished: bool, quiet_ms: u64) -> Option<bool> {
    if finished {
        (quiet_ms <= SUB_DONE_MS).then_some(false)
    } else {
        (quiet_ms <= SUB_STALE_MS).then_some(true)
    }
}

/// "claude-opus-5-5" → "opus 5.5", "claude-haiku-4-5-20251001" → "haiku 4.5"; an alias ("sonnet") stays as it is.
fn short_model(id: &str) -> String {
    let Some(rest) = id.strip_prefix("claude-") else { return id.to_string() };
    let rest = rest.split('[').next().unwrap_or(rest);
    let mut parts = rest.split('-');
    let family = parts.next().unwrap_or_default();
    let ver: Vec<&str> = parts.filter(|p| !p.is_empty() && p.len() < 6 && p.chars().all(|c| c.is_ascii_digit())).collect();
    if ver.is_empty() { family.to_string() } else { format!("{family} {}", ver.join(".")) }
}

/// The model of the last assistant message near the end of a transcript (its last 64 KB), as a short name.
/// Cached by (path, modified time): the widget polls every 2 s and a transcript grows only while a turn runs.
fn last_model(path: &std::path::Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    type Cache = std::collections::HashMap<std::path::PathBuf, (u64, Option<String>)>;
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Cache>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let meta = std::fs::metadata(path).ok()?;
    let stamp = meta.modified().map_or(0, ms) ^ meta.len();
    if let Some((s, m)) = cache.lock().ok()?.get(path) {
        if *s == stamp {
            return m.clone();
        }
    }
    let mut file = std::fs::File::open(path).ok()?;
    let start = meta.len().saturating_sub(64 * 1024);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = vec![];
    file.take(64 * 1024).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let found = text.rmatch_indices("\"model\":\"").find_map(|(i, key)| {
        let v = &text[i + key.len()..];
        let id = &v[..v.find('"')?];
        (id.starts_with("claude-")).then(|| short_model(id))
    });
    if let Ok(mut c) = cache.lock() {
        if c.len() > 200 {
            c.clear();
        }
        c.insert(path.to_path_buf(), (stamp, found.clone()));
    }
    found
}

/// A session's model: the transcript `projects/<key>/<session id>.jsonl` (probed in every project folder, like the subagents').
fn session_model(dir: &std::path::Path, session_id: &str) -> Option<String> {
    if !safe_id(session_id) {
        return None;
    }
    std::fs::read_dir(dir.join("projects")).ok()?.flatten().find_map(|p| {
        let t = p.path().join(format!("{session_id}.jsonl"));
        t.is_file().then(|| last_model(&t)).flatten()
    })
}

fn ms(t: std::time::SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && !id.contains(['/', '\\']) && !id.contains("..")
}

/// The subagents of a session, newest first. The transcript folder's `<key>` comes from the cwd (and a worktree session
/// changes it), so every project folder is probed for `<session id>/subagents` — a stat each.
fn subagents(dir: &std::path::Path, session_id: &str, now: u64) -> Vec<Subagent> {
    if !safe_id(session_id) {
        return vec![];
    }
    let Ok(projects) = std::fs::read_dir(dir.join("projects")) else { return vec![] };
    let mut out = vec![];
    for project in projects.flatten() {
        let Ok(files) = std::fs::read_dir(project.path().join(session_id).join("subagents")) else { continue };
        for f in files.flatten() {
            let path = f.path();
            let Some(stem) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".meta.json")) else { continue };
            let Some(id) = stem.strip_prefix("agent-").filter(|i| safe_id(i)) else { continue };
            let Ok(transcript) = std::fs::metadata(path.with_file_name(format!("{stem}.jsonl"))) else { continue };
            let Ok(meta) = std::fs::read_to_string(&path) else { continue };
            let Ok(meta) = serde_json::from_str::<AgentMeta>(&meta) else { continue };
            let modified = transcript.modified().map_or(0, ms);
            let Some(running) = sub_visible(meta.finished == Some(true), now.saturating_sub(modified)) else { continue };
            let created = transcript.created().map_or(modified, ms);
            out.push(Subagent {
                id: id.to_string(),
                agent_type: meta.agent_type.unwrap_or_default(),
                description: meta.description.unwrap_or_default().chars().take(120).collect(),
                running,
                background: meta.request_shape.as_deref() == Some("background"),
                model: last_model(&path.with_file_name(format!("{stem}.jsonl"))).or(meta.model),
                since: if running { created } else { modified },
                progress: read_progress(&dir.join("session-progress").join(session_id).join(format!("agent-{id}.json"))),
            });
        }
    }
    out.sort_by(|a, b| b.running.cmp(&a.running).then(b.since.cmp(&a.since)));
    out.truncate(SUB_MAX);
    out
}

/// `<config dir>/jobs/<job id>/state.json` of a background job: while a permission prompt nobody sees blocks it,
/// it holds `"state":"working","tempo":"blocked"`. Internal file like the session's, read for bg sessions only.
/// The second kind of waiting: `"state":"blocked"` + `"needs":"<question>"` — the session ended its turn with a question.
#[derive(Deserialize, Default)]
struct JobState {
    state: Option<String>,
    tempo: Option<String>,
    needs: Option<String>,
}

fn parse_job_state(text: &str) -> Option<JobState> {
    serde_json::from_str(text).ok()
}

fn is_blocked(s: &JobState) -> bool {
    s.state.as_deref() == Some("working") && s.tempo.as_deref() == Some("blocked")
}

/// the question a `state:"blocked"` job waits to have answered (cut to 300 chars); none for any other state
fn question(s: &JobState) -> Option<String> {
    if s.state.as_deref() != Some("blocked") {
        return None;
    }
    let n = s.needs.as_deref()?.trim();
    (!n.is_empty()).then(|| n.chars().take(300).collect())
}

/// (blocked by a permission prompt, waits for an answer: the question) of a job; a missing / unreadable file or odd id = nothing
fn job_wait(dir: &std::path::Path, job_id: &str) -> (bool, Option<String>) {
    if job_id.is_empty() || job_id.contains(['/', '\\']) || job_id.contains("..") {
        return (false, None);
    }
    std::fs::read_to_string(dir.join("jobs").join(job_id).join("state.json"))
        .ok()
        .and_then(|t| parse_job_state(&t))
        .map_or((false, None), |s| (is_blocked(&s), question(&s)))
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
        // a GPT account has no sessions here (yet)
        if account.engine == "codex" {
            continue;
        }
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
            let (permission, needs) = if f.kind.as_deref() == Some("bg") {
                job_id.as_deref().map_or((false, None), |j| job_wait(&dir, j))
            } else {
                (false, None)
            };
            let blocked = permission || needs.is_some();
            let parent = if f.kind.as_deref() == Some("bg") { job_id.as_deref().and_then(|j| job_parent(&dir, j)) } else { None };
            let attach_id = job_id.unwrap_or_else(|| session_id.clone());
            out.push(Session {
                blocked,
                needs,
                parent,
                attach_id,
                config_dir: dir.to_string_lossy().into_owned(),
                account: account.label.clone(),
                account_index: index,
                pid: f.pid,
                context: context_used(&session_id),
                progress: progress(&dir, &session_id),
                subagents: subagents(&dir, &session_id, ms(std::time::SystemTime::now())),
                model: session_model(&dir, &session_id),
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
        .filter(|(_, account)| account.engine != "codex")
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
    fn job_state_question() {
        let q = |t: &str| question(&parse_job_state(t).unwrap());
        assert_eq!(q(r#"{"state":"blocked","needs":"which DB?"}"#).as_deref(), Some("which DB?"));
        assert_eq!(q(r#"{"state":"blocked","needs":"  "}"#), None);
        assert_eq!(q(r#"{"state":"blocked"}"#), None);
        assert_eq!(q(r#"{"state":"working","needs":"x"}"#), None);
        assert_eq!(q(&format!(r#"{{"state":"blocked","needs":"{}"}}"#, "я".repeat(500))).unwrap().chars().count(), 300);
        // a question is not a permission prompt
        assert!(!is_blocked(&parse_job_state(r#"{"state":"blocked","needs":"x"}"#).unwrap()));
    }

    #[test]
    fn job_state_from_disk_and_missing() {
        let dir = std::env::temp_dir().join(format!("cc-widget-jobs-{}", std::process::id()));
        let job = dir.join("jobs").join("abc12345");
        std::fs::create_dir_all(&job).unwrap();
        std::fs::write(job.join("state.json"), r#"{"state":"working","tempo":"blocked"}"#).unwrap();
        assert_eq!(job_wait(&dir, "abc12345"), (true, None));
        assert_eq!(job_wait(&dir, "nofile"), (false, None));
        assert_eq!(job_wait(&dir, "../abc12345"), (false, None));
        std::fs::write(job.join("state.json"), r#"{"state":"blocked","needs":"which DB?"}"#).unwrap();
        assert_eq!(job_wait(&dir, "abc12345"), (false, Some("which DB?".to_string())));
        std::fs::write(job.join("parent.json"), r#"{"sessionId":"p-1"}"#).unwrap();
        assert_eq!(job_parent(&dir, "abc12345").as_deref(), Some("p-1"));
        assert_eq!(job_parent(&dir, "nofile"), None);
        assert_eq!(job_parent(&dir, "../abc12345"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_names_and_transcript_tail() {
        assert_eq!(short_model("claude-opus-5-5"), "opus 5.5");
        assert_eq!(short_model("claude-haiku-4-5-20251001"), "haiku 4.5");
        assert_eq!(short_model("claude-sonnet-5-5[1m]"), "sonnet 5.5");
        assert_eq!(short_model("claude-fable"), "fable");
        assert_eq!(short_model("sonnet"), "sonnet");
        let dir = std::env::temp_dir().join(format!("cc-widget-model-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("projects").join("k")).unwrap();
        let t = dir.join("projects").join("k").join("sess-m.jsonl");
        std::fs::write(&t, "{\"message\":{\"model\":\"claude-opus-5-5\"}}\n{\"message\":{\"model\":\"<synthetic>\"}}\n{\"x\":1}\n").unwrap();
        assert_eq!(session_model(&dir, "sess-m").as_deref(), Some("opus 5.5"));
        assert_eq!(session_model(&dir, "nope"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subagent_visibility() {
        assert_eq!(sub_visible(false, 5_000), Some(true));
        assert_eq!(sub_visible(false, SUB_STALE_MS + 1), None);
        assert_eq!(sub_visible(true, 60_000), Some(false));
        assert_eq!(sub_visible(true, SUB_DONE_MS + 1), None);
    }

    #[test]
    fn subagents_from_disk() {
        let dir = std::env::temp_dir().join(format!("cc-widget-subs-{}", std::process::id()));
        let subs = dir.join("projects").join("E--work-x").join("sess-1").join("subagents");
        std::fs::create_dir_all(&subs).unwrap();
        let w = |n: &str, t: &str| std::fs::write(subs.join(n), t).unwrap();
        w("agent-aaa.jsonl", "{}");
        w("agent-aaa.meta.json", r#"{"agentType":"worker","description":"Port tests","requestShape":"background","finished":false}"#);
        w("agent-bbb.jsonl", "{}");
        w("agent-bbb.meta.json", r#"{"agentType":"scout","description":"Find callers","finished":true}"#);
        w("agent-ccc.meta.json", r#"{"agentType":"orphan"}"#); // no transcript: skipped
        std::fs::write(subs.join("notes.txt"), "x").unwrap();
        let pdir = dir.join("session-progress").join("sess-1");
        std::fs::create_dir_all(&pdir).unwrap();
        std::fs::write(pdir.join("agent-aaa.json"), r#"{"overall":47,"tasks":[{"title":"t","percent":47,"active":true}]}"#).unwrap();
        let now = ms(std::time::SystemTime::now());
        let got = subagents(&dir, "sess-1", now);
        assert_eq!(got.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["aaa", "bbb"]); // running first
        assert!(got[0].running && got[0].background && got[0].progress.as_ref().unwrap().overall == 47);
        assert!(!got[1].running && !got[1].background && got[1].progress.is_none());
        assert_eq!(got[0].description, "Port tests");
        assert!(subagents(&dir, "other", now).is_empty());
        assert!(subagents(&dir, "../sess-1", now).is_empty());
        // long after: everything is stale
        assert!(subagents(&dir, "sess-1", now + SUB_STALE_MS + 60_000).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn s(id: &str, parent: Option<&str>) -> Session {
        Session {
            blocked: false, needs: None, parent: parent.map(String::from), attach_id: String::new(), config_dir: String::new(),
            account: String::new(), account_index: 0, pid: 0, context: None, progress: None, subagents: vec![], model: None,
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
