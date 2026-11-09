//! Running GPT calls: gpt-run.js (the owner's tools) writes one file per call,
//! `<runs root>/<session id>/<stamp>-<pid>-<code|review>.jsonl` — a header line {run:1, kind, account, model, dir, task, pid,
//! startedAt}, then codex events, a {pid} after a fall-over to another account, and {end:1,...} as the last line.
//! A call runs while it has no {end} line, the node process of the file name (`<pid>`) is alive and started before the
//! call (not a reused pid), and the call started less than 2 h ago. The pids inside the file are codex's and not used.
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
    /// the folder of the run file = the id of the session that started the call
    pub session_id: String,
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

/// The pid in the file name: gpt-run.js's own node process, alive for the whole call (fall-overs included).
fn name_pid(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(".jsonl")?;
    let (head, _kind) = stem.rsplit_once('-')?;
    head.rsplit_once('-')?.1.parse().ok()
}

/// The start time in milliseconds.
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

/// What the file says without its middle: the header, whether it has ended. `None` = not a call (yet).
/// The header is under 8 KB: gpt-run.js cuts the task to 200 characters.
fn inspect(path: &Path, pid: u32) -> Option<(GptRun, bool)> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut head = vec![0u8; len.min(HEAD_BYTES) as usize];
    f.read_exact(&mut head).ok()?;
    let first = head.split(|b| *b == b'\n').next()?;
    let h: Value = serde_json::from_slice(first).ok()?;
    if h.get("run").is_none() {
        return None;
    }
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
        session_id: String::new(),
    };
    Some((run, ended))
}

/// `alive(pid, startedAt)`: is that process still the one that started the call (called only for unfinished calls).
fn scan(root: &Path, now: u64, alive: &dyn Fn(u32, u64) -> bool) -> Vec<GptRun> {
    let mut out = vec![];
    let Ok(subs) = std::fs::read_dir(root) else { return out };
    for sub in subs.flatten() {
        let Ok(files) = std::fs::read_dir(sub.path()) else { continue };
        for file in files.flatten() {
            let name = file.file_name();
            let Some(pid) = name.to_str().filter(|n| run_file(n)).and_then(name_pid) else { continue };
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
            let Some((mut run, ended)) = inspect(&file.path(), pid) else { continue };
            run.session_id = sub.file_name().to_string_lossy().chars().take(100).collect();
            if !ended && now.saturating_sub(run.started_at) < MAX_AGE_MS && alive(run.pid, run.started_at) {
                out.push(run);
            }
        }
    }
    out.sort_by_key(|r| r.started_at);
    out
}

/// A process with that pid that started shortly before the call: node starts, then writes the header within seconds.
/// A pid reused after a crash belongs to a process that started later and does not count.
fn same_process(proc_start_ms: u64, started_at: u64) -> bool {
    proc_start_ms <= started_at + 5_000 && proc_start_ms + 600_000 >= started_at
}

pub fn running() -> Vec<GptRun> {
    let Some(root) = root() else { return vec![] };
    if !root.is_dir() {
        return vec![];
    }
    // the process list is read only when some file holds an unfinished call, and only for those pids
    let sys = std::cell::RefCell::new(sysinfo::System::new());
    scan(&root, now_ms(), &|pid, started| {
        let pid = sysinfo::Pid::from_u32(pid);
        let mut sys = sys.borrow_mut();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        sys.process(pid).is_some_and(|p| same_process(p.start_time() * 1000, started))
    })
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
        assert_eq!(name_pid("20261109-101500-123-4242-code.jsonl"), Some(4242));
    }

    #[test]
    fn a_reused_pid_is_not_the_call() {
        let t = 1_000_000_000;
        assert!(same_process(t - 2_000, t));
        assert!(!same_process(t + 60_000, t));
        assert!(!same_process(t - 3_600_000, t));
    }

    #[test]
    fn running_ended_dead_and_old() {
        let d = dir("a");
        let now = now_ms();
        // the pid that counts is the node process in the name (10); the header's pid is codex's
        put(&d, "20261109-1-10-code.jsonl", &[head(77, now - 5000), r#"{"type":"event"}"#.into()]);
        put(&d, "20261109-2-10-code.jsonl", &[head(77, now - 5000), r#"{"end":1,"ok":true,"ms":5}"#.into()]);
        put(&d, "20261109-3-99-code.jsonl", &[head(77, now - 5000)]);
        put(&d, "20261109-4-10-code.jsonl", &[head(77, now - 3 * 3600 * 1000)]);
        // a fall-over moved codex to another process; node is the same
        put(&d, "20261109-5-10-review.jsonl", &[head(99, now - 1000), r#"{"attempt":2}"#.into(), r#"{"pid":98}"#.into()]);
        put(&d, "20261109-6-10-code.jsonl", &["garbage".into()]);
        let runs = scan(&d, now, &|pid, _| pid == 10);
        assert_eq!(runs.len(), 2);
        // oldest first; both heads are "review"
        assert_eq!(runs[0].started_at, now - 5000);
        assert_eq!(runs[1].started_at, now - 1000);
        assert_eq!(runs[1].kind, "review");
        assert_eq!(runs[1].pid, 10);
        assert_eq!(runs[1].session_id, "sess");
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
        assert!(scan(&d, now, &|_, _| true).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
