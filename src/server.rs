use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use reqwest::Client;
use serde::Serialize;
use serde_json::json;
use tokio::sync::RwLock;

use crate::{
    config::Config,
    router::proxy_request,
    settings::SETTINGS,
    state::{endpoint_usable, PublicEndpoint, RelayState},
    util::now_ms,
};

#[derive(Clone)]
pub struct AppState {
    pub client: Client,
    pub state: Arc<RwLock<RelayState>>,
    pub config: Config,
}

pub fn app(client: Client, state: Arc<RwLock<RelayState>>, config: Config) -> Router {
    Router::new()
        .route("/", get(health))
        .route("/health", get(health))
        .route("/rpcs", get(rpcs))
        .route("/", post(proxy))
        .with_state(AppState {
            client,
            state,
            config,
        })
}

async fn proxy(State(app): State<AppState>, body: Bytes) -> Response {
    if body.len() > SETTINGS.max_body_bytes {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"error": "body_too_large"})),
        )
            .into_response();
    }

    match proxy_request(&app.client, app.state.clone(), body).await {
        Ok(proxied) => {
            let mut response = (proxied.response.status, proxied.response.body).into_response();
            let headers = response.headers_mut();
            headers.insert(
                HeaderName::from_static("content-type"),
                HeaderValue::from_str(&proxied.response.content_type)
                    .unwrap_or_else(|_| HeaderValue::from_static("application/json")),
            );
            headers.insert(
                HeaderName::from_static("x-upstream-rpc"),
                HeaderValue::from_str(&proxied.upstream)
                    .unwrap_or_else(|_| HeaderValue::from_static("unknown")),
            );
            let healthy_count = app.state.read().await.healthy_endpoints().len().to_string();
            headers.insert(
                HeaderName::from_static("x-proxy-healthy-count"),
                HeaderValue::from_str(&healthy_count)
                    .unwrap_or_else(|_| HeaderValue::from_static("0")),
            );
            response
        }
        Err(error) => (
            error.status(),
            Json(json!({
                "error": error.code(),
                "message": error.message()
            })),
        )
            .into_response(),
    }
}

async fn health(State(app): State<AppState>) -> Json<HealthResponse> {
    let state = app.state.read().await;
    Json(health_response(&state, &app.config, false))
}

async fn rpcs(State(app): State<AppState>) -> Json<RpcsResponse> {
    let state = app.state.read().await;
    let health = health_response(&state, &app.config, true);
    Json(RpcsResponse {
        health,
        endpoints: public_endpoints(&state),
    })
}

fn health_response(state: &RelayState, config: &Config, _include_all: bool) -> HealthResponse {
    let healthy = state.healthy_endpoints();
    HealthResponse {
        ok: !healthy.is_empty(),
        healthy_count: healthy.len(),
        total_count: state.endpoints.len(),
        reference_latest_block: state.reference_latest_block,
        round: state.round,
        round_running: state.round_running,
        last_round_started_at: state.last_round_started_at.clone(),
        last_round_finished_at: state.last_round_finished_at.clone(),
        last_chainlist_refresh_at: state.last_chainlist_refresh_at.clone(),
        config: PublicConfig {
            chain_id: config.chain_id,
            min_block_range: config.min_block_range,
            port: SETTINGS.port,
            health_interval_ms: SETTINGS.health_interval_ms,
            chainlist_refresh_ms: SETTINGS.chainlist_refresh_ms,
            max_block_lag: SETTINGS.max_block_lag,
            max_health_age_ms: SETTINGS.max_health_age_ms,
            rate_limit_cooldown_ms: SETTINGS.rate_limit_cooldown_ms,
        },
        healthy_rpcs: public_endpoints_from_slice(state, &healthy),
    }
}

fn public_endpoints(state: &RelayState) -> Vec<PublicEndpoint> {
    let healthy = state.healthy_endpoints();
    public_endpoints_for(state, &healthy)
}

fn public_endpoints_for(
    state: &RelayState,
    healthy: &[crate::types::Endpoint],
) -> Vec<PublicEndpoint> {
    let now = now_ms();
    state
        .endpoints
        .values()
        .map(|endpoint| {
            let usable = healthy.iter().any(|healthy| healthy.url == endpoint.url)
                && endpoint_usable(endpoint, state.reference_latest_block, now);
            PublicEndpoint::from_endpoint(endpoint, state.reference_latest_block, usable)
        })
        .collect()
}

fn public_endpoints_from_slice(
    state: &RelayState,
    endpoints: &[crate::types::Endpoint],
) -> Vec<PublicEndpoint> {
    let now = now_ms();
    endpoints
        .iter()
        .map(|endpoint| {
            let usable = endpoint_usable(endpoint, state.reference_latest_block, now);
            PublicEndpoint::from_endpoint(endpoint, state.reference_latest_block, usable)
        })
        .collect()
}

#[derive(Serialize)]
pub struct PublicConfig {
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    #[serde(rename = "minBlockRange")]
    pub min_block_range: u64,
    pub port: u16,
    #[serde(rename = "healthIntervalMs")]
    pub health_interval_ms: u64,
    #[serde(rename = "chainlistRefreshMs")]
    pub chainlist_refresh_ms: u64,
    #[serde(rename = "maxBlockLag")]
    pub max_block_lag: u64,
    #[serde(rename = "maxHealthAgeMs")]
    pub max_health_age_ms: u64,
    #[serde(rename = "rateLimitCooldownMs")]
    pub rate_limit_cooldown_ms: u64,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub ok: bool,
    #[serde(rename = "healthyCount")]
    pub healthy_count: usize,
    #[serde(rename = "totalCount")]
    pub total_count: usize,
    #[serde(rename = "referenceLatestBlock")]
    pub reference_latest_block: u64,
    pub round: u64,
    #[serde(rename = "roundRunning")]
    pub round_running: bool,
    #[serde(rename = "lastRoundStartedAt")]
    pub last_round_started_at: Option<String>,
    #[serde(rename = "lastRoundFinishedAt")]
    pub last_round_finished_at: Option<String>,
    #[serde(rename = "lastChainlistRefreshAt")]
    pub last_chainlist_refresh_at: Option<String>,
    pub config: PublicConfig,
    #[serde(rename = "healthyRpcs")]
    pub healthy_rpcs: Vec<PublicEndpoint>,
}

#[derive(Serialize)]
pub struct RpcsResponse {
    pub health: HealthResponse,
    pub endpoints: Vec<PublicEndpoint>,
}
