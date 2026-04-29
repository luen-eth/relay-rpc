use std::sync::Arc;

use bytes::Bytes;
use reqwest::{Client, StatusCode};
use tokio::sync::RwLock;

use crate::{
    config::ChainConfig,
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
    config: &ChainConfig,
    body: Bytes,
) -> Result<ProxiedResponse, ProxyError> {
    let analysis = analyze_body(&body);
    let (ordered, selected_pool) = {
        let mut state = state.write().await;
        let prefer_archive =
            analysis.prefers_archive_pool(state.reference_latest_block, config.min_block_range);
        let mut selected_pool = if prefer_archive {
            RoutePool::Archive
        } else {
            RoutePool::Rpc
        };

        let mut candidates = endpoints_for_pool(&state, selected_pool, config.max_health_age_ms)
            .into_iter()
            .filter(|endpoint| endpoint_supports_range(endpoint, analysis.max_get_logs_range))
            .collect::<Vec<_>>();

        if candidates.is_empty() {
            candidates = endpoints_for_pool(&state, selected_pool, config.max_health_age_ms);
        }

        if candidates.is_empty() && selected_pool == RoutePool::Rpc {
            candidates = endpoints_for_pool(&state, RoutePool::Archive, config.max_health_age_ms)
                .into_iter()
                .filter(|endpoint| endpoint_supports_range(endpoint, analysis.max_get_logs_range))
                .collect::<Vec<_>>();
            if !candidates.is_empty() {
                selected_pool = RoutePool::Archive;
            }
        }

        if candidates.is_empty() {
            return Err(ProxyError::NoHealthyUpstream);
        }

        let route_key = format!("{}:{}", selected_pool.as_str(), analysis.route_key);
        let index = state
            .round_robin
            .entry(route_key)
            .and_modify(|value| *value += 1)
            .or_insert(1);
        let start = (*index - 1) % candidates.len();
        candidates.rotate_left(start);
        (
            candidates
                .into_iter()
                .take(SETTINGS.retry_attempts)
                .map(|endpoint| endpoint.url)
                .collect::<Vec<_>>(),
            selected_pool,
        )
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
                pool: selected_pool,
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

fn endpoints_for_pool(
    state: &RelayState,
    pool: RoutePool,
    max_health_age_ms: u64,
) -> Vec<crate::types::Endpoint> {
    match pool {
        RoutePool::Rpc => state.rpc_endpoints(max_health_age_ms),
        RoutePool::Archive => state.healthy_endpoints(max_health_age_ms),
    }
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

    endpoint.cooldown_until_ms = now_ms() + SETTINGS.rate_limit_cooldown_ms;
    endpoint.healthy = false;
    endpoint.reason = vec![upstream
        .error
        .clone()
        .unwrap_or_else(|| "upstream failure".to_string())];
}

pub struct ProxiedResponse {
    pub upstream: String,
    pub pool: RoutePool,
    pub response: RawProxyResponse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoutePool {
    Rpc,
    Archive,
}

impl RoutePool {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rpc => "rpc",
            Self::Archive => "archive",
        }
    }
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
