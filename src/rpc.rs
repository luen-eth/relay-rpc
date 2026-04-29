use bytes::Bytes;
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};

use crate::{
    settings::SETTINGS,
    types::{RawProxyResponse, RpcResult},
};

pub async fn rpc_call(client: &Client, url: &str, method: &str, params: Value) -> RpcResult {
    let response = client
        .post(url)
        .timeout(SETTINGS.health_timeout())
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await;

    let Ok(response) = response else {
        return RpcResult {
            result: None,
            error: Some(json!({"message": response.err().unwrap().to_string()})),
        };
    };

    let status = response.status();
    let value = response
        .json::<Value>()
        .await
        .unwrap_or_else(|error| json!({"error": {"message": error.to_string()}}));

    if !status.is_success() {
        return RpcResult {
            result: None,
            error: Some(value.get("error").cloned().unwrap_or(value)),
        };
    }

    RpcResult {
        result: value.get("result").cloned(),
        error: value.get("error").cloned(),
    }
}

pub async fn raw_proxy_call(client: &Client, url: &str, body: Bytes) -> RawProxyResponse {
    match client
        .post(url)
        .timeout(SETTINGS.proxy_timeout())
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("application/json")
                .to_string();
            let body = response.bytes().await.unwrap_or_default();
            RawProxyResponse {
                ok: status.is_success(),
                status,
                body,
                content_type,
                error: if status.is_success() {
                    None
                } else {
                    Some(format!("HTTP {}", status.as_u16()))
                },
            }
        }
        Err(error) => RawProxyResponse {
            ok: false,
            status: StatusCode::BAD_GATEWAY,
            body: Bytes::new(),
            content_type: "application/json".to_string(),
            error: Some(error.to_string()),
        },
    }
}

pub fn rpc_errors_from_body(body: &[u8]) -> Vec<Value> {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return Vec::new();
    };
    if let Value::Array(items) = value {
        return items
            .into_iter()
            .filter_map(|item| item.get("error").cloned())
            .collect();
    }
    value.get("error").cloned().into_iter().collect()
}

pub fn only_retryable_errors(body: &[u8]) -> bool {
    let errors = rpc_errors_from_body(body);
    !errors.is_empty() && errors.iter().all(is_retryable_error)
}

pub fn is_retryable_error(error: &Value) -> bool {
    is_rate_limit_error(error)
        || is_range_error(error)
        || normalized_error(error).contains("missing trie")
        || normalized_error(error).contains("header not found")
        || normalized_error(error).contains("historical state")
        || normalized_error(error).contains("not supported")
        || normalized_error(error).contains("timeout")
        || normalized_error(error).contains("temporar")
}

pub fn is_rate_limit_error(error: &Value) -> bool {
    let message = normalized_error(error);
    message.contains("429")
        || message.contains("too many")
        || message.contains("rate limit")
        || message.contains("cu limit")
}

pub fn is_range_error(error: &Value) -> bool {
    let message = normalized_error(error);
    message.contains("range")
        || message.contains("block range")
        || message.contains("more than")
        || message.contains("exceed maximum")
}

pub fn normalized_error(error: &Value) -> String {
    let code = error.get("code").map(Value::to_string).unwrap_or_default();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!("{code} {message}").to_lowercase()
}
