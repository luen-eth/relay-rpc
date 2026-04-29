use std::sync::Arc;

use bytes::Bytes;
use reqwest::{Client, StatusCode};
use tokio::sync::RwLock;

use crate::{
    request_analysis::analyze_body,
    rpc::{
        is_range_error, is_rate_limit_error, only_retryable_errors, raw_proxy_call,
        rpc_errors_from_body,
    },
    settings::SETTINGS,
    state::{endpoint_supports_range, note_range_rejected, note_range_success, RelayState},
    types::RawProxyResponse,
    util::now_ms,
};

pub async fn proxy_request(
    client: &Client,
    state: Arc<RwLock<RelayState>>,
    body: Bytes,
) -> Result<ProxiedResponse, ProxyError> {
    let analysis = analyze_body(&body);
    let ordered = {
        let mut state = state.write().await;
        let mut candidates = state
            .healthy_endpoints()
            .into_iter()
            .filter(|endpoint| endpoint_supports_range(endpoint, analysis.max_get_logs_range))
            .collect::<Vec<_>>();

        if candidates.is_empty() {
            candidates = state.healthy_endpoints();
        }

        if candidates.is_empty() {
            return Err(ProxyError::NoHealthyUpstream);
        }

        let index = state
            .round_robin
            .entry(analysis.route_key.clone())
            .and_modify(|value| *value += 1)
            .or_insert(1);
        let start = (*index - 1) % candidates.len();
        candidates.rotate_left(start);
        candidates
            .into_iter()
            .take(SETTINGS.retry_attempts)
            .map(|endpoint| endpoint.url)
            .collect::<Vec<_>>()
    };

    let mut last_failure = None;
    for url in ordered {
        let upstream = raw_proxy_call(client, &url, body.clone()).await;
        let errors = rpc_errors_from_body(&upstream.body);
        if upstream.ok && !only_retryable_errors(&upstream.body) {
            let mut state = state.write().await;
            if let Some(endpoint) = state.endpoints.get_mut(&url) {
                note_range_success(endpoint, analysis.max_get_logs_range);
            }
            return Ok(ProxiedResponse {
                upstream: url,
                response: upstream,
            });
        }

        apply_proxy_failure(
            &state,
            &url,
            &upstream,
            &errors,
            analysis.max_get_logs_range,
        )
        .await;
        last_failure = Some(upstream);
    }

    Err(ProxyError::UpstreamFailed(
        last_failure
            .and_then(|failure| failure.error)
            .unwrap_or_else(|| "all upstreams failed".to_string()),
    ))
}

async fn apply_proxy_failure(
    state: &Arc<RwLock<RelayState>>,
    url: &str,
    upstream: &RawProxyResponse,
    errors: &[serde_json::Value],
    requested_range: Option<u64>,
) {
    let mut state = state.write().await;
    let Some(endpoint) = state.endpoints.get_mut(url) else {
        return;
    };

    endpoint.proxy_failures += 1;
    endpoint.consecutive_failures += 1;

    if upstream.status == StatusCode::TOO_MANY_REQUESTS || errors.iter().any(is_rate_limit_error) {
        endpoint.cooldown_until_ms = now_ms() + SETTINGS.rate_limit_cooldown_ms;
        endpoint.reason = vec![format!(
            "rate limited until {}ms",
            endpoint.cooldown_until_ms
        )];
        return;
    }

    if errors.iter().any(is_range_error) {
        note_range_rejected(endpoint, requested_range);
        endpoint.reason = vec![format!("range rejected: {:?}", requested_range)];
        return;
    }

    endpoint.healthy = false;
    endpoint.reason = vec![upstream
        .error
        .clone()
        .unwrap_or_else(|| "upstream failure".to_string())];
}

pub struct ProxiedResponse {
    pub upstream: String,
    pub response: RawProxyResponse,
}

#[derive(Debug)]
pub enum ProxyError {
    NoHealthyUpstream,
    UpstreamFailed(String),
}

impl ProxyError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::NoHealthyUpstream => StatusCode::SERVICE_UNAVAILABLE,
            Self::UpstreamFailed(_) => StatusCode::BAD_GATEWAY,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::NoHealthyUpstream => "no_healthy_upstream",
            Self::UpstreamFailed(_) => "upstream_failed",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NoHealthyUpstream => "No healthy archive RPC is currently available".to_string(),
            Self::UpstreamFailed(message) => message.clone(),
        }
    }
}
