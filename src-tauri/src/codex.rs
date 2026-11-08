//! GPT accounts (OpenAI Codex CLI): who is signed in and the rate-limit windows. An account is a CODEX_HOME folder
//! (~/.codex, ~/.codex-work …), like a Claude account is a CLAUDE_CONFIG_DIR folder.
//!
//! Where the numbers come from, in this order:
//!   1. `<CODEX_HOME>/agora-limits.json`, when it is younger than 10 minutes (written by the owner's other tooling);
//!   2. a short-lived `codex app-server` (stdio JSON-RPC: initialize → account/read → account/rateLimits/read),
//!      at most once per 5 minutes per account, in a background thread — the page never waits for it.
//! The windows are drawn as they come, labelled by their length: a free plan has one 30-day window, a paid one is
//! expected to have 5 h + 7 d.

use crate::{paths, settings::Account};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_EVERY_MS: u64 = 5 * 60_000;
const SNAPSHOT_MAX_AGE_MS: u64 = 10 * 60_000;
const DAY_MS: u64 = 24 * 3600 * 1000;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Window {
    /// the window's length in minutes (300 = 5 h, 10080 = 7 d, 43200 = 30 d)
    pub mins: Option<u64>,
    /// 0..100
    pub used: Option<f64>,
    /// epoch seconds
    pub resets_at: Option<f64>,
}

/// What we know about one account. The same shape as the `agora-limits.json` snapshot (its other fields are ignored).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Gpt {
    pub email: Option<String>,
    /// "free" | "plus" | "pro" | …
    pub plan: Option<String>,
    pub windows: Vec<Window>,
    /// codex says a limit is hit or spend control stopped it
    pub reached: bool,
    /// "login" (sign in again: `codex login`) | "no-codex" | "timeout" | other text; None = fine
    pub error: Option<String>,
    pub updated_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GptLimits {
    account: String,
    account_index: usize,
    #[serde(flatten)]
    data: Gpt,
    /// calls in the last 24 h from `agora-calls.jsonl`, when the account has that journal
    calls_day: Option<u32>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// account/read + account/rateLimits/read answers (their `result`s) → the account's data
pub fn parse_answers(account: &Value, limits: &Value, now: u64) -> Gpt {
    let mut g = Gpt { updated_ms: now, ..Default::default() };
    let acc = &account["account"];
    if acc.is_null() {
        g.error = Some("login".into());
        return g;
    }
    g.email = acc["email"].as_str().map(String::from);
    let rl = &limits["rateLimits"];
    g.plan = acc["planType"].as_str().or(rl["planType"].as_str()).map(String::from);
    for w in [&rl["primary"], &rl["secondary"]] {
        if w.is_object() {
            g.windows.push(Window {
                mins: w["windowDurationMins"].as_u64(),
                used: w["usedPercent"].as_f64(),
                resets_at: w["resetsAt"].as_f64(),
            });
        }
    }
    g.reached = !rl["rateLimitReachedType"].is_null() || rl["spendControlReached"].as_bool().unwrap_or(false);
    g
}

/// A JSON-RPC error message → the short word the page knows: an expired / missing sign-in is "login"
pub fn rpc_error(m: &Value) -> String {
    let e = &m["error"];
    let msg = e["message"].as_str().unwrap_or("error");
    let low = msg.to_lowercase();
    if e["code"].as_i64() == Some(401) || low.contains("token_expired") || low.contains("401") || low.contains("unauthorized") {
        return "login".into();
    }
    msg.chars().take(120).collect()
}

/// The snapshot file's text → the data, only when it is younger than 10 minutes and holds no error
pub fn parse_snapshot(text: &str, now: u64) -> Option<Gpt> {
    let g: Gpt = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let fresh = g.updated_ms > 0 && now.saturating_sub(g.updated_ms) < SNAPSHOT_MAX_AGE_MS;
    (g.error.is_none() && fresh).then_some(g)
}

/// Lines of `agora-calls.jsonl` ({"ts": epoch ms, …}) from the last 24 hours
pub fn calls_last_day(text: &str, now: u64) -> u32 {
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["ts"].as_u64().is_some_and(|t| t <= now + 60_000 && now.saturating_sub(t) < DAY_MS))
        .count() as u32
}

// ---- the codex binary ----

#[cfg(windows)]
fn command() -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // the npm package's native binary (the codex.cmd shim goes through node and flashes a console)
    let native = std::env::var_os("APPDATA").map(PathBuf::from).and_then(|appdata| {
        let root = appdata.join("npm").join("node_modules").join("@openai").join("codex");
        let exe = root.join(r"node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe");
        exe.is_file().then_some((exe, root))
    });
    let mut c = match native {
        Some((exe, root)) => {
            let mut c = Command::new(exe);
            // what bin/codex.js sets before it starts the binary
            c.env("CODEX_MANAGED_PACKAGE_ROOT", root).env("CODEX_MANAGED_BY_NPM", "1");
            c
        }
        None => {
            let mut c = Command::new("cmd");
            c.args(["/c", "codex"]);
            c
        }
    };
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

#[cfg(not(windows))]
fn command() -> Command {
    // a GUI app on macOS / Linux often has a short PATH: look where npm / brew put it first
    let home = paths::home();
    let mut candidates: Vec<PathBuf> = vec![];
    if let Some(h) = &home {
        candidates.push(h.join(".npm-global/bin/codex"));
        candidates.push(h.join(".local/bin/codex"));
    }
    candidates.push("/opt/homebrew/bin/codex".into());
    candidates.push("/usr/local/bin/codex".into());
    let exe = candidates.into_iter().find(|p| p.is_file()).unwrap_or_else(|| "codex".into());
    Command::new(exe)
}

/// One short-lived `codex app-server`: initialize → initialized → account/read → account/rateLimits/read
fn probe(codex_home: &Path) -> Gpt {
    let fail = |e: &str| Gpt { error: Some(e.to_string()), updated_ms: now_ms(), ..Default::default() };
    let mut cmd = command();
    cmd.arg("app-server").env("CODEX_HOME", codex_home).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let Ok(mut child) = cmd.spawn() else { return fail("no-codex") };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        return fail("no-codex");
    };
    let (tx, rx) = mpsc::channel::<Value>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if tx.send(v).is_err() {
                    break;
                }
            }
        }
    });
    let mut send = |v: Value| writeln!(stdin, "{v}").and_then(|_| stdin.flush());
    let _ = send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"cc-widget","version":env!("CARGO_PKG_VERSION")}}}));
    let deadline = std::time::Instant::now() + PROBE_TIMEOUT;
    let (mut account, mut limits) = (None, None);
    let out = loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let m = match rx.recv_timeout(left) {
            Ok(m) => m,
            Err(mpsc::RecvTimeoutError::Timeout) => break fail("timeout"),
            // the process ended without answering: there is no codex behind the command
            Err(mpsc::RecvTimeoutError::Disconnected) => break fail("no-codex"),
        };
        if !m["error"].is_null() && m["id"].as_i64().is_some_and(|i| i >= 2) {
            break fail(&rpc_error(&m));
        }
        match m["id"].as_i64() {
            Some(1) => {
                let _ = send(json!({"jsonrpc":"2.0","method":"initialized"}));
                let _ = send(json!({"jsonrpc":"2.0","id":2,"method":"account/read","params":{}}));
                let _ = send(json!({"jsonrpc":"2.0","id":3,"method":"account/rateLimits/read","params":null}));
            }
            Some(2) => account = Some(m.get("result").cloned().unwrap_or(Value::Null)),
            Some(3) => limits = Some(m.get("result").cloned().unwrap_or(Value::Null)),
            _ => {}
        }
        if let (Some(a), Some(l)) = (&account, &limits) {
            break parse_answers(a, l, now_ms());
        }
    };
    drop(send);
    let _ = child.kill();
    let _ = child.wait();
    out
}

// ---- the cache: the page asks every 2 s, the probe runs at most every 5 minutes per account, off the page's thread ----

struct Entry {
    data: Gpt,
    probed_ms: u64,
    busy: bool,
}

fn cache() -> &'static Mutex<HashMap<PathBuf, Entry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Entry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn data_for(home: &Path) -> Gpt {
    let now = now_ms();
    if let Some(g) = std::fs::read_to_string(home.join("agora-limits.json")).ok().and_then(|t| parse_snapshot(&t, now)) {
        return g;
    }
    let mut map = cache().lock().unwrap();
    let e = map.entry(home.to_path_buf()).or_insert(Entry { data: Gpt::default(), probed_ms: 0, busy: false });
    if !e.busy && now.saturating_sub(e.probed_ms) >= PROBE_EVERY_MS {
        e.busy = true;
        let home = home.to_path_buf();
        std::thread::spawn(move || {
            let data = probe(&home);
            if let Some(e) = cache().lock().unwrap().get_mut(&home) {
                *e = Entry { data, probed_ms: now_ms(), busy: false };
            }
        });
    }
    e.data.clone()
}

/// The limits of every GPT (codex) account of the settings. `account_index` is the account's place in the settings
/// list (the page colours accounts by it).
pub fn limits(accounts: &[Account]) -> Vec<GptLimits> {
    accounts
        .iter()
        .enumerate()
        .filter(|(_, a)| a.engine == "codex")
        .filter_map(|(index, account)| {
            let home = paths::resolve(&account.dir)?;
            let calls_day = std::fs::read_to_string(home.join("agora-calls.jsonl")).ok().map(|t| calls_last_day(&t, now_ms()));
            Some(GptLimits { account: account.label.clone(), account_index: index, data: data_for(&home), calls_day })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn free_plan_one_window() {
        let a = v(r#"{"account":{"type":"chatgpt","email":"a@b.c","planType":"free"}}"#);
        let l = v(r#"{"rateLimits":{"primary":{"usedPercent":12.5,"windowDurationMins":43200,"resetsAt":1800000000},"secondary":null,"rateLimitReachedType":null,"spendControlReached":false}}"#);
        let g = parse_answers(&a, &l, 5);
        assert_eq!(g.email.as_deref(), Some("a@b.c"));
        assert_eq!(g.plan.as_deref(), Some("free"));
        assert_eq!(g.windows, vec![Window { mins: Some(43200), used: Some(12.5), resets_at: Some(1_800_000_000.0) }]);
        assert!(!g.reached && g.error.is_none());
        assert_eq!(g.updated_ms, 5);
    }

    #[test]
    fn paid_plan_two_windows_and_reached() {
        let a = v(r#"{"account":{"type":"chatgpt","email":"a@b.c","planType":"plus"}}"#);
        let l = v(r#"{"rateLimits":{"primary":{"usedPercent":100,"windowDurationMins":300,"resetsAt":1},"secondary":{"usedPercent":40,"windowDurationMins":10080,"resetsAt":2},"rateLimitReachedType":"primary"}}"#);
        let g = parse_answers(&a, &l, 0);
        assert_eq!(g.windows.len(), 2);
        assert_eq!(g.windows[1].mins, Some(10080));
        assert!(g.reached);
    }

    #[test]
    fn no_account_means_login() {
        let g = parse_answers(&v(r#"{"account":null}"#), &v("{}"), 0);
        assert_eq!(g.error.as_deref(), Some("login"));
    }

    #[test]
    fn rpc_errors() {
        assert_eq!(rpc_error(&v(r#"{"error":{"code":401,"message":"x"}}"#)), "login");
        assert_eq!(rpc_error(&v(r#"{"error":{"code":-32000,"message":"token_expired"}}"#)), "login");
        assert_eq!(rpc_error(&v(r#"{"error":{"code":-32000,"message":"boom"}}"#)), "boom");
    }

    #[test]
    fn snapshot_must_be_young_and_clean() {
        let t = r#"{"name":"gpt","engine":"codex","email":"a@b.c","plan":"plus","windows":[{"mins":300,"used":3,"resetsAt":9}],"reached":false,"error":null,"updatedMs":1000000}"#;
        let g = parse_snapshot(t, 1_000_000 + 9 * 60_000).unwrap();
        assert_eq!(g.plan.as_deref(), Some("plus"));
        assert_eq!(g.windows[0].mins, Some(300));
        assert!(parse_snapshot(t, 1_000_000 + 11 * 60_000).is_none());
        assert!(parse_snapshot(&t.replace("null,\"updatedMs", "\"login\",\"updatedMs"), 1_000_000).is_none());
        assert!(parse_snapshot("nope", 1).is_none());
    }

    #[test]
    fn calls_in_the_last_day() {
        let now = 10 * DAY_MS;
        let text = format!(
            "{{\"ts\":{},\"what\":\"a\"}}\nbroken\n{{\"ts\":{}}}\n{{\"ts\":{},\"ok\":true}}\n",
            now - 1000,
            now - DAY_MS - 1,
            now - DAY_MS + 5000
        );
        assert_eq!(calls_last_day(&text, now), 2);
    }
}
