//! An optional other machine in the widget (see settings::Remote) — a home server where Claude runs unattended:
//! its load, shown like the System section, and its Claude runs, shown under it. Both over HTTP with the counter's
//! token (counter::get): `hostUrl` answers the load, `runsUrl` the runs (with `passUrl`, through a short-lived pass).
//!
//! The load's format (be-local-admin's /api/v1/monitor/host):
//!   {"threads": [12, 3, …], "cores": 12, "memUsed": bytes, "memTotal": bytes,
//!    "gpu": {"name": "…", "load": 7, "temp": 50, "memUsedMb": 1000, "memTotalMb": 12000} | null}
//! The runs' format (be-claude-bridge's /api/admin/usage): {"runs": [{"id", "status": RUNNING|DONE|ERROR|STOPPED,
//!   "startedAt", "finishedAt", "title", "prompt", "origin"}, …]} — newest first; and "chat": {"sessionId", "model",
//!   "contextTokens", "contextWindow", "contextPercent", "cacheUntil", "warn", "sessionSince", "sessionMessages"} | null —
//!   the bridge's chat conversation, which the next chat message continues until /new.

use crate::counter;
use crate::settings::Remote;
use crate::system::{Gpu, Stats};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Host {
    threads: Vec<f32>,
    #[serde(default)]
    cores: usize,
    mem_used: u64,
    mem_total: u64,
    gpu: Option<HostGpu>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostGpu {
    name: String,
    load: f32,
    temp: Option<f32>,
    mem_used_mb: u64,
    mem_total_mb: u64,
}

/// the remote load as the System section's numbers: threads paired into cores like the local sampler does
fn stats(h: Host) -> Stats {
    let threads: Vec<f32> = h.threads.iter().map(|t| t.clamp(0.0, 100.0)).collect();
    let per_core = if h.cores > 0 && threads.len() == h.cores * 2 { 2 } else { 1 };
    let cores = threads.chunks(per_core).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    let cpu = if threads.is_empty() { 0.0 } else { threads.iter().sum::<f32>() / threads.len() as f32 };
    Stats {
        cpu,
        threads,
        cores,
        per_core,
        mem_used: h.mem_used,
        mem_total: h.mem_total,
        cpu_temp: None,
        gpu: h.gpu.map(|g| Gpu {
            name: g.name,
            load: g.load,
            mem_used: g.mem_used_mb * 1024 * 1024,
            mem_total: g.mem_total_mb * 1024 * 1024,
            temp: g.temp,
        }),
    }
}

pub async fn host(remote: &Remote) -> Result<Stats, String> {
    let body = counter::get(&remote.host_url, "").await?;
    Ok(stats(serde_json::from_value(body).map_err(|e| format!("not a load: {e}"))?))
}

/// A run as the page shows it: working (RUNNING) or over; the times as the server wrote them (ISO), the page counts.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: i64,
    pub name: String,
    /// "busy" | "done" | "error"
    pub status: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub origin: Option<String>,
}

/// The runs worth a line: every running one, then the latest finished ones, at most `max` in all.
fn runs_of(body: &Value, max: usize) -> Vec<Run> {
    let text = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).map(str::to_string);
    let all: Vec<Run> = body
        .get("runs")
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|r| {
            // a title, else the prompt's first line, short
            let name = text(r, "title")
                .filter(|t| !t.trim().is_empty())
                .or_else(|| text(r, "prompt").map(|p| p.lines().next().unwrap_or("").chars().take(60).collect()))
                .unwrap_or_default();
            let status = match r.get("status").and_then(Value::as_str) {
                Some("RUNNING") => "busy",
                Some("ERROR") => "error",
                _ => "done",
            };
            Run {
                id: r.get("id").and_then(Value::as_i64).unwrap_or(0),
                name,
                status: status.into(),
                started_at: text(r, "startedAt"),
                finished_at: text(r, "finishedAt"),
                origin: text(r, "origin"),
            }
        })
        .collect();
    let (busy, over): (Vec<Run>, Vec<Run>) = all.into_iter().partition(|r| r.status == "busy");
    busy.into_iter().chain(over).take(max).collect()
}

/// A session the hub says waits for the owner: Claude Code's session id and what it asks (none for a redacted account).
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    pub session_id: String,
    pub reason: Option<String>,
}

/// the hub's sessions in WAITING (format: [{"sessionId", "status", "waitingReason", …}])
fn waiting_of(body: &Value) -> Vec<Waiting> {
    body.as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter(|s| s.get("status").and_then(Value::as_str) == Some("WAITING"))
        .filter_map(|s| {
            Some(Waiting {
                session_id: s.get("sessionId")?.as_str()?.to_string(),
                reason: s.get("waitingReason").and_then(Value::as_str).map(str::to_string),
            })
        })
        .collect()
}

pub async fn waiting(remote: &Remote) -> Result<Vec<Waiting>, String> {
    Ok(waiting_of(&counter::get(&remote.hub_url, &remote.pass_url).await?))
}

/// The bridge's chat session (the same /api/admin/usage, field `chat`): how full it is and how old — the next chat
/// message continues it until /new. Absent on servers that don't send it.
#[derive(Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub context_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub context_percent: Option<u32>,
    pub cache_until: Option<String>,
    #[serde(default)]
    pub warn: bool,
    pub session_since: Option<String>,
    #[serde(default)]
    pub session_messages: u64,
}

/// The runs and the chat's session, as the page shows them.
#[derive(Serialize, Debug, PartialEq)]
pub struct Runs {
    pub runs: Vec<Run>,
    pub chat: Option<Chat>,
}

/// the chat's session when there is one (a new conversation after /new has none yet)
fn chat_of(body: &Value) -> Option<Chat> {
    let chat: Chat = serde_json::from_value(body.get("chat")?.clone()).ok()?;
    chat.session_id.as_ref()?;
    Some(chat)
}

pub async fn runs(remote: &Remote) -> Result<Runs, String> {
    let body = counter::get(&remote.runs_url, &remote.pass_url).await?;
    Ok(Runs { runs: runs_of(&body, 4), chat: chat_of(&body) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_host_becomes_the_system_numbers() {
        let h: Host = serde_json::from_value(json!({"threads": [10, 30, 50, 70], "cores": 2, "memUsed": 4, "memTotal": 8,
            "gpu": {"name": "RTX", "load": 7, "temp": 50, "memUsedMb": 1, "memTotalMb": 2}})).unwrap();
        let s = stats(h);
        assert_eq!(s.per_core, 2);
        assert_eq!(s.cores, vec![20.0, 60.0]);
        assert_eq!(s.cpu, 40.0);
        assert_eq!(s.gpu.unwrap().mem_total, 2 * 1024 * 1024);
        // no GPU, cores unknown: a column per thread
        let s = stats(serde_json::from_value(json!({"threads": [5, 6, 7], "memUsed": 1, "memTotal": 2, "gpu": null})).unwrap());
        assert_eq!((s.per_core, s.cores.len()), (1, 3));
    }

    #[test]
    fn the_hub_names_the_waiting_sessions() {
        let body = json!([
            {"sessionId": "a", "status": "WAITING", "waitingReason": "Claude needs your permission to use Bash"},
            {"sessionId": "b", "status": "BUSY"},
            {"sessionId": "c", "status": "WAITING", "waitingReason": null},
        ]);
        assert_eq!(waiting_of(&body), vec![
            Waiting { session_id: "a".into(), reason: Some("Claude needs your permission to use Bash".into()) },
            Waiting { session_id: "c".into(), reason: None },
        ]);
        assert!(waiting_of(&json!({})).is_empty());
    }

    #[test]
    fn the_chat_session_comes_with_the_runs() {
        let body = json!({"runs": [], "chat": {"sessionId": "4c4c", "model": "sonnet", "contextTokens": 312000,
            "contextWindow": 1000000, "contextPercent": 31, "contextAt": "2026-10-24T10:00:00Z",
            "cacheUntil": "2026-10-24T11:00:00Z", "warn": true, "sessionSince": "2026-10-19T17:34:00Z", "sessionMessages": 21}});
        let chat = chat_of(&body).unwrap();
        assert_eq!((chat.context_tokens, chat.context_percent, chat.warn, chat.session_messages), (Some(312000), Some(31), true, 21));
        // after /new: no session yet → no line; an older bridge without the field → none
        assert!(chat_of(&json!({"chat": {"sessionId": null, "warn": false, "sessionMessages": 0}})).is_none());
        assert!(chat_of(&json!({"runs": []})).is_none());
        assert!(chat_of(&json!({"chat": null})).is_none());
    }

    #[test]
    fn running_runs_first_then_the_latest() {
        let body = json!({"runs": [
            {"id": 3, "status": "DONE", "title": "", "prompt": "[order #88] smart manager\nmore", "startedAt": "a"},
            {"id": 2, "status": "RUNNING", "title": "Morning digest", "startedAt": "b"},
            {"id": 1, "status": "ERROR", "title": "Backup check"},
        ]});
        let runs = runs_of(&body, 2);
        assert_eq!(runs.iter().map(|r| (r.id, r.status.as_str())).collect::<Vec<_>>(), vec![(2, "busy"), (3, "done")]);
        assert_eq!(runs[1].name, "[order #88] smart manager");
        assert!(runs_of(&json!({}), 4).is_empty());
    }
}
