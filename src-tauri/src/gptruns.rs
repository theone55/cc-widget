//! Running GPT calls: gpt-run.js (the owner's tools) writes one file per call,
//! `<runs root>/<session id>/<stamp>-<pid>-<code|review>.jsonl` — a header line {run:1, kind, account, model, dir, task, pid,
//! startedAt}, then codex events, a {pid} after a fall-over to another account, and {end:1,...} as the last line.
//! A call runs while it has no {end} line, its process is alive and it started less than 2 h ago.
//! Only the head line and the last bytes are read. Everything in the files is untrusted text.

use serde::Serialize;
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const MAX_AGE_MS: u64 = 2 * 3600 * 1000;
const HEAD_BYTES: u64 = 8 * 1024;
const TAIL_BYTES: u64 = 16 * 1024;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GptRun {
    /// "code" or "review"
    pub kind: String,
    pub account: String,
    pub model: String,
    pub dir: String,
    pub task: String,
    pub pid: u32,
    pub started_at: u64,
}

fn root() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("AGORA_RUNS_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).or_else(|| crate::paths::home().map(|h| h.join("AppData").join("Local")))?
    } else {
        crate::paths::home()?.join(".local").join("share")
    };
    Some(base.join("agora").join("runs"))
}

/// `20261109-101500-123-4242-code.jsonl`: digits and dashes, then the kind.
fn run_file(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".jsonl") else { return false };
    let Some((head, kind)) = stem.rsplit_once('-') else { return false };
    (kind == "code" || kind == "review") && !head.is_empty() && head.chars().all(|c| c.is_ascii_digit() || c == '-')
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn text_of(v: &Value, key: &str, max: usize) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").chars().take(max).collect()
}

/// The start time: milliseconds, or an ISO string.
fn started_at(v: &Value) -> Option<u64> {
    let s = v.get("startedAt")?;
    if let Some(n) = s.as_u64() {
        return Some(n);
    }
    if let Some(f) = s.as_f64() {
        return Some(f as u64);
    }
    None
}

/// What the file says without its middle: the header, the latest pid, whether it has ended. `None` = not a call (yet).
fn inspect(path: &Path) -> Option<(GptRun, bool)> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut head = vec![0u8; len.min(HEAD_BYTES) as usize];
    f.read_exact(&mut head).ok()?;
    let first = head.split(|b| *b == b'\n').next()?;
    let h: Value = serde_json::from_slice(first).ok()?;
    if h.get("run").is_none() {
        return None;
    }
    let mut pid = h.get("pid").and_then(|p| p.as_u64())? as u32;
    let mut ended = false;
    if len > 0 {
        let from = len.saturating_sub(TAIL_BYTES);
        f.seek(SeekFrom::Start(from)).ok()?;
        let mut tail = vec![];
        f.read_to_end(&mut tail).ok()?;
        let text = String::from_utf8_lossy(&tail);
        // the first line of a tail read from the middle may be cut: it just fails to parse
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
            if v.get("end").is_some() {
                ended = true;
            } else if let Some(p) = v.get("pid").and_then(|p| p.as_u64()).filter(|_| v.get("run").is_none()) {
                pid = p as u32;
            }
        }
    }
    let run = GptRun {
        kind: if text_of(&h, "kind", 10) == "review" { "review" } else { "code" }.to_string(),
        account: text_of(&h, "account", 40),
        model: text_of(&h, "model", 60),
        dir: text_of(&h, "dir", 300),
        task: text_of(&h, "task", 200),
        pid,
        started_at: started_at(&h)?,
    };
    Some((run, ended))
}

fn scan(root: &Path, now: u64, alive: &dyn Fn(u32) -> bool) -> Vec<GptRun> {
    let mut out = vec![];
    let Ok(subs) = std::fs::read_dir(root) else { return out };
    for sub in subs.flatten() {
        let Ok(files) = std::fs::read_dir(sub.path()) else { continue };
        for file in files.flatten() {
            if !file.file_name().to_str().is_some_and(run_file) {
                continue;
            }
            // a call younger than 2 h has written to its file within 2 h: older files are skipped unread
            let fresh = file
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .is_some_and(|d| now.saturating_sub(d.as_millis() as u64) < MAX_AGE_MS);
            if !fresh {
                continue;
            }
            let Some((run, ended)) = inspect(&file.path()) else { continue };
            if !ended && now.saturating_sub(run.started_at) < MAX_AGE_MS && alive(run.pid) {
                out.push(run);
            }
        }
    }
    out.sort_by_key(|r| r.started_at);
    out
}

pub fn running() -> Vec<GptRun> {
    let Some(root) = root() else { return vec![] };
    if !root.is_dir() {
        return vec![];
    }
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    scan(&root, now_ms(), &|pid| sys.process(sysinfo::Pid::from_u32(pid)).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ccw-gptruns-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sess")).unwrap();
        d
    }

    fn put(d: &Path, name: &str, lines: &[String]) {
        std::fs::write(d.join("sess").join(name), lines.join("\n") + "\n").unwrap();
    }

    fn head(pid: u32, started: u64) -> String {
        format!(r#"{{"run":1,"kind":"review","account":"gpt-1","model":"gpt-5","dir":"E:\\work\\new\\agora","task":"<b>x</b>","pid":{pid},"startedAt":{started}}}"#)
    }

    #[test]
    fn file_names() {
        assert!(run_file("20261109-101500-123-4242-code.jsonl"));
        assert!(run_file("20261109-101500-123-4242-review.jsonl"));
        assert!(!run_file("notes-code.jsonl"));
        assert!(!run_file("20261109-4242-other.jsonl"));
        assert!(!run_file("20261109-4242-code.json"));
    }

    #[test]
    fn running_ended_dead_and_old() {
        let d = dir("a");
        let now = now_ms();
        put(&d, "20261109-1-1-code.jsonl", &[head(10, now - 5000), r#"{"type":"event"}"#.into()]);
        put(&d, "20261109-2-2-code.jsonl", &[head(10, now - 5000), r#"{"end":1,"ok":true,"ms":5}"#.into()]);
        put(&d, "20261109-3-3-code.jsonl", &[head(99, now - 5000)]);
        put(&d, "20261109-4-4-code.jsonl", &[head(10, now - 3 * 3600 * 1000)]);
        // a fall-over moved the call to another process
        put(&d, "20261109-5-5-review.jsonl", &[head(99, now - 1000), r#"{"attempt":2}"#.into(), r#"{"pid":10}"#.into()]);
        put(&d, "20261109-6-6-code.jsonl", &["garbage".into()]);
        let runs = scan(&d, now, &|pid| pid == 10);
        assert_eq!(runs.len(), 2);
        // oldest first; both heads are "review"
        assert_eq!(runs[0].started_at, now - 5000);
        assert_eq!(runs[1].started_at, now - 1000);
        assert_eq!(runs[1].kind, "review");
        assert_eq!(runs[1].pid, 10);
        assert_eq!(runs[1].task, "<b>x</b>");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_long_file_is_read_by_its_tail() {
        let d = dir("b");
        let now = now_ms();
        let mut lines = vec![head(10, now - 1000)];
        for _ in 0..2000 {
            lines.push(format!(r#"{{"type":"item","text":"{}"}}"#, "x".repeat(100)));
        }
        lines.push(r#"{"end":1,"ok":false}"#.into());
        put(&d, "20261109-7-7-code.jsonl", &lines);
        assert!(scan(&d, now, &|_| true).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
