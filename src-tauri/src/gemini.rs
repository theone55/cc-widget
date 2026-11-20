//! Gemini limits (Google Antigravity CLI `agy`). One signed-in account per machine: `agy --print /usage
//! --output-format json` prints a bucket per model group (Gemini models, Claude and GPT models …) with the remaining
//! fraction and the reset time. No `agy` binary → no row at all; a failed / not signed-in call → a greyed row with the
//! error. The probe runs at most once per 5 minutes in a background thread — the page never waits for it.

use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_EVERY_MS: u64 = 5 * 60_000;

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    /// short group label: "Gemini", "3p", or the group's own name
    pub group: String,
    /// the bucket's own name (shown when the window is unknown)
    pub name: String,
    /// the window length in minutes; None = unknown string
    pub mins: Option<u64>,
    /// 0..100
    pub used: f64,
    /// ISO 8601 as agy prints it ("2026-11-27T15:31:14Z"); the page parses it
    pub reset_time: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Gemini {
    pub buckets: Vec<Bucket>,
    /// the error text (not signed in, not eligible, timeout …); None = fine
    pub error: Option<String>,
    pub updated_ms: u64,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// "weekly" → 10080 … tolerant: daily / day, hourly, monthly, "5h", "5 hours", "30m"; unknown → None
pub fn window_mins(w: &str) -> Option<u64> {
    let w = w.trim().to_lowercase();
    match w.as_str() {
        "weekly" | "week" | "7d" => return Some(7 * 1440),
        "daily" | "day" | "1d" => return Some(1440),
        "hourly" | "hour" => return Some(60),
        "monthly" | "month" => return Some(30 * 1440),
        _ => {}
    }
    let digits: String = w.chars().take_while(|c| c.is_ascii_digit()).collect();
    let n: u64 = digits.parse().ok()?;
    let unit = w[digits.len()..].trim();
    match unit.chars().next()? {
        'm' if !unit.starts_with("mo") => Some(n),
        'h' => Some(n * 60),
        'd' => Some(n * 1440),
        'w' => Some(n * 7 * 1440),
        _ => None,
    }
}

/// "Gemini Models" → "Gemini", "Claude and GPT models" → "3p", anything else → its first words, trimmed
pub fn short_group(name: &str) -> String {
    let low = name.to_lowercase();
    if low.contains("gemini") {
        "Gemini".into()
    } else if low.contains("claude") || low.contains("gpt") {
        "3p".into()
    } else {
        name.split_whitespace().next().unwrap_or(name).chars().take(8).collect()
    }
}

/// The text `agy --print /usage --output-format json` printed → the data
pub fn parse_output(text: &str, now: u64) -> Gemini {
    let mut g = Gemini { updated_ms: now, ..Default::default() };
    let Ok(v) = serde_json::from_str::<Value>(text.trim().trim_start_matches('\u{feff}')) else {
        g.error = Some(text.trim().chars().take(120).collect::<String>()).filter(|s| !s.is_empty()).or(Some("no answer".into()));
        return g;
    };
    let status = v["status"].as_str().unwrap_or("");
    let groups = v["command"]["data"]["groups"].as_array();
    if !status.eq_ignore_ascii_case("SUCCESS") || groups.is_none() {
        let msg = v["error"].as_str().or(v["response"].as_str()).or(v["message"].as_str()).unwrap_or(if status.is_empty() { "unexpected answer" } else { status });
        g.error = Some(msg.chars().take(160).collect());
        return g;
    }
    for grp in groups.unwrap() {
        let group = short_group(grp["name"].as_str().unwrap_or("?"));
        for b in grp["buckets"].as_array().into_iter().flatten() {
            let Some(rem) = b["remaining_fraction"].as_f64() else { continue };
            g.buckets.push(Bucket {
                group: group.clone(),
                name: b["name"].as_str().unwrap_or("").to_string(),
                mins: b["window"].as_str().and_then(window_mins),
                used: ((1.0 - rem) * 100.0).clamp(0.0, 100.0),
                reset_time: b["reset_time"].as_str().map(String::from),
            });
        }
    }
    g
}

// ---- the agy binary ----

fn exe() -> Option<PathBuf> {
    #[cfg(windows)]
    let p = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("agy").join("bin").join("agy.exe"));
    #[cfg(not(windows))]
    let p = crate::paths::home().map(|h| h.join(".local/bin/agy"));
    p.filter(|p| p.is_file())
}

fn probe(exe: PathBuf) -> Gemini {
    let fail = |e: &str| Gemini { error: Some(e.to_string()), updated_ms: now_ms(), ..Default::default() };
    let mut cmd = Command::new(exe);
    cmd.args(["--print", "/usage", "--output-format", "json"]).current_dir(std::env::temp_dir());
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Ok(child) = cmd.spawn() else { return fail("no-agy") };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(PROBE_TIMEOUT) {
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if out.status.success() {
                return parse_output(&stdout, now_ms());
            }
            // non-zero exit: an error text in the JSON when there is one, else stderr
            let g = parse_output(&stdout, now_ms());
            if g.error.is_some() && !stdout.trim().is_empty() {
                return g;
            }
            let err = String::from_utf8_lossy(&out.stderr);
            fail(err.trim().chars().take(160).collect::<String>().as_str()).with_default_error(out.status.code())
        }
        Ok(Err(_)) => fail("no-agy"),
        Err(_) => fail("timeout"),
    }
}

impl Gemini {
    fn with_default_error(mut self, code: Option<i32>) -> Self {
        if self.error.as_deref().is_none_or(str::is_empty) {
            self.error = Some(format!("agy exited with {}", code.map_or("a signal".into(), |c| c.to_string())));
        }
        self
    }
}

// ---- the cache: the page asks every 2 s, the probe runs at most every 5 minutes ----

struct Entry {
    data: Gemini,
    probed_ms: u64,
    busy: bool,
}

fn cache() -> &'static Mutex<Entry> {
    static CACHE: OnceLock<Mutex<Entry>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Entry { data: Gemini::default(), probed_ms: 0, busy: false }))
}

/// The Gemini row: empty when `agy` is not installed, otherwise one entry (data from the cache, a probe in the background)
pub fn limits() -> Vec<Gemini> {
    let Some(exe) = exe() else { return vec![] };
    let now = now_ms();
    let mut e = cache().lock().unwrap();
    if !e.busy && now.saturating_sub(e.probed_ms) >= PROBE_EVERY_MS {
        e.busy = true;
        std::thread::spawn(move || {
            let data = probe(exe);
            *cache().lock().unwrap() = Entry { data, probed_ms: now_ms(), busy: false };
        });
    }
    // before the first answer: an empty row with no error ("…")
    vec![e.data.clone()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = r#"{"conversation_id":"","status":"SUCCESS","response":"...","command":{"name":"usage","data":{"description":"...","groups":[{"name":"Gemini Models","description":"Models within this group: Gemini 3.8 Flash","buckets":[{"id":"gemini-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":0.9891999959945679,"reset_time":"2026-11-27T15:31:14Z"}]},{"name":"Claude and GPT models","description":"Models within this group: Claude Sonnet 4.6, Claude Opus 4.6, GPT-OSS 120B","buckets":[{"id":"3p-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":1,"reset_time":"2026-11-27T15:35:24Z"}]}]}}}"#;

    #[test]
    fn parses_the_real_output() {
        let g = parse_output(REAL, 7);
        assert!(g.error.is_none());
        assert_eq!(g.updated_ms, 7);
        assert_eq!(g.buckets.len(), 2);
        assert_eq!(g.buckets[0].group, "Gemini");
        assert_eq!(g.buckets[0].mins, Some(10080));
        assert!((g.buckets[0].used - 1.08).abs() < 0.01);
        assert_eq!(g.buckets[0].reset_time.as_deref(), Some("2026-11-27T15:31:14Z"));
        assert_eq!(g.buckets[1].group, "3p");
        assert_eq!(g.buckets[1].used, 0.0);
    }

    #[test]
    fn error_status_and_garbage() {
        let g = parse_output(r#"{"status":"ERROR","response":"Please sign in with agy auth login"}"#, 1);
        assert_eq!(g.error.as_deref(), Some("Please sign in with agy auth login"));
        assert!(g.buckets.is_empty());
        let g = parse_output("not json at all", 1);
        assert_eq!(g.error.as_deref(), Some("not json at all"));
        assert_eq!(parse_output("", 1).error.as_deref(), Some("no answer"));
        // SUCCESS without groups is still not usable
        assert!(parse_output(r#"{"status":"SUCCESS"}"#, 1).error.is_some());
    }

    #[test]
    fn windows_tolerant() {
        assert_eq!(window_mins("weekly"), Some(10080));
        assert_eq!(window_mins("Daily"), Some(1440));
        assert_eq!(window_mins("5h"), Some(300));
        assert_eq!(window_mins("5 hours"), Some(300));
        assert_eq!(window_mins("30m"), Some(30));
        assert_eq!(window_mins("fortnightly"), None);
    }
}
