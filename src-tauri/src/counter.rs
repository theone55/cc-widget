//! The optional counter in the header: a few numbers from your own server (see settings::Counter).
//! Its token lives in the OS keychain (Windows Credential Manager, macOS Keychain, Secret Service on Linux) and never
//! goes back to the page.

use crate::settings::Counter;
use serde_json::Value;
use std::time::Duration;

const SERVICE: &str = "cc-widget";
const USER: &str = "counter-token";

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, USER).map_err(|e| e.to_string())
}

pub fn token() -> Option<String> {
    entry().ok()?.get_password().ok()
}

/// Saves the token (an empty string forgets it).
pub fn set_token(token: &str) -> Result<(), String> {
    let entry = entry()?;
    let token = token.trim();
    if token.is_empty() {
        let _ = entry.delete_credential();
        Ok(())
    } else {
        entry.set_password(token).map_err(|e| e.to_string())
    }
}

/// A dot path into the JSON ("visitors.today", "items.0.count").
fn pick<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').filter(|p| !p.is_empty()).try_fold(value, |v, key| match key.parse::<usize>() {
        Ok(i) if v.is_array() => v.get(i),
        _ => v.get(key),
    })
}

/// The short-lived pass from `pass_url`, kept for a while instead of asking for a new one on every fetch:
/// (the pass url + token it was made for, the pass, until when to use it).
static PASS: std::sync::Mutex<Option<(String, String, std::time::Instant)>> = std::sync::Mutex::new(None);
/// How long a pass is reused: under the hour the server gives it, so it never runs out mid-use.
const PASS_REUSE: Duration = Duration::from_secs(50 * 60);

async fn get_json(client: &reqwest::Client, url: &str, bearer: Option<&str>) -> Result<(u16, Option<Value>), String> {
    let mut request = client.get(url);
    if let Some(b) = bearer {
        request = request.bearer_auth(b);
    }
    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        return Ok((status, None));
    }
    Ok((status, Some(response.json().await.map_err(|e| e.to_string())?)))
}

/// The pass for this token: the kept one while it's fresh, else a new one from `pass_url`.
async fn pass(client: &reqwest::Client, pass_url: &str, token: &str, fresh: bool) -> Result<String, String> {
    let key = format!("{pass_url}
{token}");
    if !fresh {
        if let Some((k, pass, until)) = PASS.lock().unwrap().as_ref() {
            if *k == key && std::time::Instant::now() < *until {
                return Ok(pass.clone());
            }
        }
    }
    let (status, body) = get_json(client, pass_url, Some(token)).await?;
    let body = body.ok_or(format!("pass: HTTP {status}"))?;
    let pass = body.get("pass").and_then(|p| p.as_str()).ok_or("no pass in the answer")?.to_string();
    *PASS.lock().unwrap() = Some((key, pass.clone(), std::time::Instant::now() + PASS_REUSE));
    Ok(pass)
}

pub async fn fetch(counter: &Counter) -> Result<Vec<Value>, String> {
    let url = counter.url.trim();
    if url.is_empty() {
        return Err("no-url".into());
    }
    let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().map_err(|e| e.to_string())?;
    let token = token();
    let pass_url = counter.pass_url.trim();
    let body = if pass_url.is_empty() {
        let (status, body) = get_json(&client, url, token.as_deref()).await?;
        body.ok_or(format!("HTTP {status}"))?
    } else {
        let token = token.ok_or("no-token")?;
        let (status, body) = get_json(&client, url, Some(&pass(&client, pass_url, &token, false).await?)).await?;
        match body {
            Some(b) => b,
            // the kept pass was refused (revoked, the server restarted): one more try with a new one
            None if status == 401 || status == 403 => {
                let (status, body) = get_json(&client, url, Some(&pass(&client, pass_url, &token, true).await?)).await?;
                body.ok_or(format!("HTTP {status}"))?
            }
            None => return Err(format!("HTTP {status}")),
        }
    };
    // a field is "path" or "name=path": the name is only for the page
    Ok(counter
        .fields
        .iter()
        .map(|f| pick(&body, f.rsplit_once('=').map_or(f.as_str(), |(_, p)| p).trim()).cloned().unwrap_or(Value::Null))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn picks_dot_paths() {
        let v = json!({ "visitors": { "today": 48, "days7": 398 }, "list": [{ "n": 1 }] });
        assert_eq!(pick(&v, "visitors.today"), Some(&json!(48)));
        assert_eq!(pick(&v, "list.0.n"), Some(&json!(1)));
        assert_eq!(pick(&v, "nope.x"), None);
    }
}
