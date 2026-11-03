use crate::model::{now_iso, timestamp};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

fn jwt(value: &str) -> Value {
    value
        .split('.')
        .nth(1)
        .and_then(|part| URL_SAFE_NO_PAD.decode(part).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}
pub fn account_key(email: &str, id: &str) -> String {
    let email = email.trim().to_lowercase();
    let id = id.trim().to_lowercase();
    let raw = if !email.is_empty() && !id.is_empty() {
        format!("{email}\0{id}")
    } else {
        format!("{email}{id}")
    };
    if raw.is_empty() {
        return String::new();
    }
    let mut hash = Sha256::new();
    hash.update(b"codex\0");
    hash.update(raw.as_bytes());
    hash.update(b"\0");
    format!("sha256:{:x}", hash.finalize())
}
fn plan(raw: &str) -> String {
    match raw.to_lowercase().as_str() {
        "pro" => "Pro 200".into(),
        "prolite" | "pro_lite" | "pro-lite" | "pro lite" => "Pro 100".into(),
        "promax" | "pro_max" | "pro-max" | "pro max" => "Pro 500".into(),
        "plus" => "Plus".into(),
        _ => raw.chars().take(40).collect(),
    }
}
fn first<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|key| value.get(*key).filter(|value| !value.is_null()))
}
fn reset_credits(payload: &Value) -> Option<Value> {
    let count = first(payload, &["available_count", "availableCount"])?.as_u64()?;
    let expirations: Vec<String> = payload["credits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|credit| credit["status"] == "available")
        .filter_map(|credit| first(credit, &["expires_at", "expiresAt"]))
        .filter_map(|value| {
            value
                .as_i64()
                .and_then(|seconds| seconds.checked_mul(1000))
                .map(timestamp)
                .or_else(|| {
                    value
                        .as_str()
                        .and_then(|date| chrono::DateTime::parse_from_rfc3339(date).ok())
                        .map(|date| timestamp(date.timestamp_millis()))
                })
        })
        .collect();
    Some(json!({"availableCount":count,"expirations":expirations}))
}
pub fn map_usage(payload: &Value, key: &str, fallback_plan: &str) -> Value {
    let raw_plan = first(payload, &["plan_type", "planType"])
        .and_then(Value::as_str)
        .unwrap_or(fallback_plan);
    let rate = first(
        payload,
        &["rate_limit", "rateLimit", "rateLimits", "rate_limits"],
    )
    .unwrap_or(&Value::Null);
    let mut windows = Vec::new();
    for keys in [
        ["primary_window", "primaryWindow", "primary"],
        ["secondary_window", "secondaryWindow", "secondary"],
    ] {
        let Some(raw) = first(rate, &keys) else {
            continue;
        };
        let minutes = first(raw, &["limit_window_seconds", "limitWindowSeconds"])
            .and_then(Value::as_f64)
            .map(|seconds| seconds / 60.0)
            .or_else(|| {
                first(raw, &["windowDurationMins", "window_duration_mins"]).and_then(Value::as_f64)
            });
        let kind = if minutes.unwrap_or(0.0) >= 7.0 * 1440.0 {
            "weekly"
        } else if minutes.unwrap_or(0.0) >= 1440.0 {
            "daily"
        } else {
            "session"
        };
        let reset = first(raw, &["reset_at", "resetAt", "resets_at", "resetsAt"]);
        let reset = reset.and_then(|value| {
            value
                .as_i64()
                .map(|seconds| {
                    timestamp(if seconds > 10_000_000_000 {
                        seconds
                    } else {
                        seconds * 1000
                    })
                })
                .or_else(|| value.as_str().map(str::to_string))
        });
        let Some(used) = first(raw, &["used_percent", "usedPercent"])
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value >= 0.0)
        else {
            continue;
        };
        let used = used.min(100.0);
        if let (Some(minutes), Some(reset)) = (minutes, reset) {
            windows.push(json!({"kind":kind,"limitId":"codex","usedPercent":used,"windowMinutes":minutes,"resetsAt":reset}));
        }
    }
    json!({"provider":"codex","accountKey":key,"accountLabel":plan(raw_plan),"planLabel":plan(raw_plan),"status":"ok","source":"oauth","updatedAt":now_iso(),"windows":windows})
}
pub async fn fetch(client: &reqwest::Client, codex_home: &Path) -> Result<Value, String> {
    let bytes =
        std::fs::read(codex_home.join("auth.json")).map_err(|_| "未找到本机 Codex 登录信息")?;
    let auth: Value = serde_json::from_slice(&bytes).map_err(|_| "本机 Codex 登录信息无法读取")?;
    let tokens = auth.get("tokens").unwrap_or(&auth);
    let access = first(tokens, &["access_token", "accessToken"])
        .and_then(Value::as_str)
        .ok_or("本机 Codex 登录已失效，请在 Codex 中重新登录")?;
    let claims = jwt(first(tokens, &["id_token", "idToken"])
        .and_then(Value::as_str)
        .unwrap_or(""));
    let nested = &claims["https://api.openai.com/auth"];
    let id = first(tokens, &["account_id", "accountId"])
        .or_else(|| nested.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let email = claims["email"]
        .as_str()
        .or_else(|| nested["email"].as_str())
        .unwrap_or_default();
    let raw_plan = nested["chatgpt_plan_type"]
        .as_str()
        .or_else(|| claims["chatgpt_plan_type"].as_str())
        .unwrap_or_default();
    let fedramp = nested["chatgpt_account_is_fedramp"]
        .as_bool()
        .unwrap_or(false)
        && nested["chatgpt_account_id"].as_str() == Some(id);
    let origin = if fedramp {
        "https://chatgpt.us/backend-api"
    } else {
        "https://chatgpt.com/backend-api"
    };
    let mut request = client
        .get(format!("{origin}/wham/usage"))
        .bearer_auth(access)
        .header("accept", "application/json")
        .header("openai-beta", "codex-1")
        .header("originator", "Codex Desktop")
        .timeout(std::time::Duration::from_secs(15));
    if !id.is_empty() {
        request = request.header("chatgpt-account-id", id);
    }
    if fedramp {
        request = request.header("x-openai-fedramp", "true");
    }
    let response = request.send().await.map_err(|_| "Codex 额度暂时无法连接")?;
    if !response.status().is_success() {
        return Err(format!(
            "Codex 额度读取失败（{}），可稍后刷新",
            response.status().as_u16()
        ));
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|_| "Codex 额度返回格式无法读取")?;
    let mut provider = map_usage(&payload, &account_key(email, id), raw_plan);
    let mut credits_request = client
        .get(format!("{origin}/wham/rate-limit-reset-credits"))
        .bearer_auth(access)
        .header("accept", "application/json")
        .header("openai-beta", "codex-1")
        .header("originator", "Codex Desktop")
        .timeout(std::time::Duration::from_secs(10));
    if !id.is_empty() {
        credits_request = credits_request.header("chatgpt-account-id", id);
    }
    if fedramp {
        credits_request = credits_request.header("x-openai-fedramp", "true");
    }
    if let Ok(response) = credits_request.send().await {
        if response.status().is_success() {
            if let Ok(payload) = response.json::<Value>().await {
                if let Some(credits) = reset_credits(&payload) {
                    provider["resetCredits"] = credits;
                }
            }
        }
    }
    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pro_window_maps_to_shared_hub_contract() {
        let provider = map_usage(
            &json!({"plan_type":"prolite","rate_limit":{"secondary_window":{"used_percent":35,"limit_window_seconds":604800,"reset_at":1794139200}}}),
            "key",
            "",
        );
        assert_eq!(provider["planLabel"], "Pro 100");
        assert_eq!(provider["windows"][0]["kind"], "weekly");
        assert_eq!(provider["windows"][0]["windowMinutes"], 10080.0);
    }
    #[test]
    fn identity_matches_upstream_composite_hash() {
        assert_eq!(
            account_key(" Test@Example.com ", " ACCOUNT "),
            account_key("test@example.com", "account")
        );
        assert_ne!(
            account_key("a@example.com", "account"),
            account_key("b@example.com", "account")
        );
        assert_eq!(account_key("", ""), "");
    }
    #[test]
    fn credit_expiry_accepts_both_official_timestamp_shapes_without_counting_expired_cards() {
        let mapped = reset_credits(&json!({"availableCount":2,"credits":[
            {"status":"available","expiresAt":1794139200},
            {"status":"available","expires_at":"2026-11-08T12:00:00Z"},
            {"status":"expired","expiresAt":1794139200}]}))
        .unwrap();
        assert_eq!(mapped["availableCount"], 2);
        assert_eq!(mapped["expirations"].as_array().unwrap().len(), 2);
        assert_eq!(mapped["expirations"][0], mapped["expirations"][1]);
    }
}
