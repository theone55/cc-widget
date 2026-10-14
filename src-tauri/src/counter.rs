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

pub async fn fetch(counter: &Counter) -> Result<Vec<Value>, String> {
    if counter.url.trim().is_empty() {
        return Err("no-url".into());
    }
    let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().map_err(|e| e.to_string())?;
    let mut bearer = token();
    if !counter.pass_url.trim().is_empty() {
        let token = bearer.clone().ok_or("no-token")?;
        let pass: Value = client
            .get(counter.pass_url.trim())
            .bearer_auth(token)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        bearer = Some(pass.get("pass").and_then(|p| p.as_str()).ok_or("no pass in the answer")?.to_string());
    }
    let mut request = client.get(counter.url.trim());
    if let Some(b) = bearer {
        request = request.bearer_auth(b);
    }
    let body: Value = request
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
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
