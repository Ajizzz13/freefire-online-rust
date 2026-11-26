use serde_json::Value;
use std::thread;
use std::time::Duration;

const BASE: &str = "https://freefirejornal.com";
const REFERER: &str = "https://freefirejornal.com/descubra-quem-esta-online-no-free-fire-e-com-quem-esta-jogando-via-id/";
const UA: &str = "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Mobile Safari/537.36";

#[derive(Debug, Clone)]
pub struct OnlineResult {
    pub player_id: String,
    pub profile: Value,
    pub online: Value,
}

#[derive(Debug)]
pub enum OnlineError {
    InvalidId,
    Http(u16, String),
    Parse(String),
    Timeout(String),
    Upstream(String),
}

impl std::fmt::Display for OnlineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OnlineError::InvalidId => write!(f, "invalid id (8-20 digits)"),
            OnlineError::Http(c, m) => write!(f, "http {c}: {m}"),
            OnlineError::Parse(m) => write!(f, "parse error: {m}"),
            OnlineError::Timeout(m) => write!(f, "timeout: {m}"),
            OnlineError::Upstream(m) => write!(f, "upstream: {m}"),
        }
    }
}

fn post_json(path: &str, body: &Value) -> Result<Value, OnlineError> {
    let url = format!("{BASE}{path}");
    let resp = ureq::post(&url)
        .set("User-Agent", UA)
        .set("Referer", REFERER)
        .set("X-FFJ-FF-Tool", "online")
        .set("X-FFJ-Language", "pt-BR")
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| OnlineError::Upstream(e.to_string()))?;
    let status = resp.status();
    let v: Value = resp
        .into_json()
        .map_err(|e| OnlineError::Parse(e.to_string()))?;
    if status == 429 {
        return Err(OnlineError::Http(status, v.get("message").and_then(|x| x.as_str()).unwrap_or("rate limited").to_string()));
    }
    if !(200..300).contains(&status) {
        return Err(OnlineError::Http(status, v.to_string()));
    }
    Ok(v)
}

fn get_json(path: &str, query: &[(&str, &str)]) -> Result<Value, OnlineError> {
    let url = format!("{BASE}{path}");
    let mut req = ureq::get(&url)
        .set("User-Agent", UA)
        .set("Referer", REFERER)
        .set("X-FFJ-FF-Tool", "online")
        .set("X-FFJ-Language", "pt-BR");
    for (k, v) in query {
        req = req.query(k, v);
    }
    let resp = req.call().map_err(|e| OnlineError::Upstream(e.to_string()))?;
    let status = resp.status();
    let v: Value = resp
        .into_json()
        .map_err(|e| OnlineError::Parse(e.to_string()))?;
    if !(200..300).contains(&status) {
        return Err(OnlineError::Http(status, v.to_string()));
    }
    Ok(v)
}

/// Full flow (blocking). Run inside spawn_blocking when called from async code.
pub fn check_online_blocking(player_id: &str) -> Result<OnlineResult, OnlineError> {
    if !player_id.chars().all(|c| c.is_ascii_digit()) || !(8..=20).contains(&player_id.len()) {
        return Err(OnlineError::InvalidId);
    }

    // 1. bootstrap
    let boot = post_json("/api/freefire/online/bootstrap", &serde_json::json!({"id": player_id}))?;
    let token = boot.get("token").and_then(|x| x.as_str()).ok_or_else(|| {
        OnlineError::Upstream(boot.get("message").and_then(|x| x.as_str()).unwrap_or("no token").to_string())
    })?;
    let delay = (boot.get("delay").and_then(|x| x.as_i64()).unwrap_or(5)).clamp(5, 15) as u64;
    thread::sleep(Duration::from_secs(delay));

    // 2. prepare (with 425 retry)
    let mut poll_token: Option<String> = None;
    let mut profile: Option<Value> = None;
    loop {
        let body = serde_json::json!({"id": player_id, "token": token});
        match post_json("/api/freefire/online/prepare", &body) {
            Ok(prepared) => {
                let state = prepared.get("state").and_then(|x| x.as_str()).unwrap_or("");
                if state == "profile_ready" {
                    poll_token = prepared.get("pollToken").and_then(|x| x.as_str()).map(String::from);
                    profile = prepared.get("profile").cloned();
                    break;
                }
                if state == "searching_profile" {
                    poll_token = prepared.get("pollToken").and_then(|x| x.as_str()).map(String::from);
                    // poll status
                    if let Some(pt) = poll_token.clone() {
                        for _ in 0..120 {
                            thread::sleep(Duration::from_millis(1250));
                            let path = format!("/api/freefire/online/status/{}", player_id);
                            let poll = get_json(&path, &[("token", &pt)])?;
                            let st = poll.get("state").and_then(|x| x.as_str()).unwrap_or("");
                            if st == "profile_ready" {
                                profile = poll.get("profile").cloned().or(profile);
                                break;
                            }
                            if st != "searching_profile" {
                                return Err(OnlineError::Upstream(poll.get("message").and_then(|x| x.as_str()).unwrap_or("unexpected state").to_string()));
                            }
                        }
                    }
                    break;
                }
                return Err(OnlineError::Upstream(prepared.get("message").and_then(|x| x.as_str()).unwrap_or("prepare failed").to_string()));
            }
            Err(OnlineError::Http(425, msg)) => {
                let ra = serde_json::from_str::<Value>(&msg)
                    .ok()
                    .and_then(|v| v.get("retryAfter").and_then(|x| x.as_i64()))
                    .unwrap_or(2);
                thread::sleep(Duration::from_secs(ra.max(1) as u64));
                continue;
            }
            Err(e) => return Err(e),
        }
    }

    let pt = poll_token.ok_or_else(|| OnlineError::Upstream("no poll token".to_string()))?;

    // 3. check
    thread::sleep(Duration::from_millis(450));
    let checked = post_json(
        &format!("/api/freefire/online/check/{}", player_id),
        &serde_json::json!({"token": pt}),
    )?;
    if checked.get("state").and_then(|x| x.as_str()) != Some("completed") {
        return Err(OnlineError::Upstream(checked.get("message").and_then(|x| x.as_str()).unwrap_or("not completed").to_string()));
    }

    Ok(OnlineResult {
        player_id: player_id.to_string(),
        profile: checked.get("profile").cloned().or(profile).unwrap_or(Value::Null),
        online: checked.get("online").cloned().unwrap_or(Value::Null),
    })
}

/// Async wrapper for use from tokio.
pub async fn check_online(player_id: &str) -> Result<OnlineResult, OnlineError> {
    let id = player_id.to_string();
    tokio::task::spawn_blocking(move || check_online_blocking(&id))
        .await
        .map_err(|e| OnlineError::Upstream(e.to_string()))?
}
PY
echo "written"