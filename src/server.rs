use std::{collections::HashMap, sync::Arc};

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use reqwest::Client;
use serde::Serialize;
use serde_json::json;
use tokio::sync::RwLock;

use crate::{
    config::ChainConfig,
    router::proxy_request,
    settings::SETTINGS,
    state::{endpoint_usable, PublicEndpoint, RelayState},
    util::now_ms,
};

#[derive(Clone)]
pub struct ChainRuntime {
    pub config: ChainConfig,
    pub state: Arc<RwLock<RelayState>>,
}

#[derive(Clone)]
pub struct AppState {
    pub client: Client,
    pub chains: Arc<HashMap<u64, ChainRuntime>>,
}

pub fn app(client: Client, chains: Arc<HashMap<u64, ChainRuntime>>) -> Router {
    Router::new()
        .route("/", get(health).post(proxy_default))
        .route("/health", get(health))
        .route("/rpcs", get(rpcs))
        .route("/{chain_id}", get(chain_health).post(proxy_chain))
        .route("/{chain_id}/", get(chain_health).post(proxy_chain))
        .route("/{chain_id}/health", get(chain_health))
        .route("/{chain_id}/rpcs", get(chain_rpcs))
        .with_state(AppState { client, chains })
}

async fn proxy_default(State(app): State<AppState>, body: Bytes) -> Response {
    if app.chains.len() != 1 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "chain_id_required",
                "message": "Use /{chainId}/, for example /56/"
            })),
        )
            .into_response();
    }

    let runtime = app
        .chains
        .values()
        .next()
        .expect("len checked above")
        .clone();
    proxy_for_runtime(app.client, runtime, body).await
}

async fn proxy_chain(
    Path(chain_id): Path<u64>,
    State(app): State<AppState>,
    body: Bytes,
) -> Response {
    let Some(runtime) = app.chains.get(&chain_id).cloned() else {
        return chain_not_found(chain_id);
    };

    proxy_for_runtime(app.client, runtime, body).await
}

async fn proxy_for_runtime(client: Client, runtime: ChainRuntime, body: Bytes) -> Response {
    if body.len() > SETTINGS.max_body_bytes {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"error": "body_too_large"})),
        )
            .into_response();
    }

    match proxy_request(&client, runtime.state.clone(), body).await {
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
            headers.insert(
                HeaderName::from_static("x-proxy-chain-id"),
                HeaderValue::from_str(&runtime.config.chain_id.to_string())
                    .unwrap_or_else(|_| HeaderValue::from_static("0")),
            );
            let healthy_count = runtime
                .state
                .read()
                .await
                .healthy_endpoints()
                .len()
                .to_string();
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

async fn health(State(app): State<AppState>) -> Json<MultiHealthResponse> {
    Json(multi_health_response(&app).await)
}

async fn rpcs(State(app): State<AppState>) -> Json<MultiRpcsResponse> {
    let mut chains = Vec::new();
    for runtime in sorted_runtimes(&app) {
        let state = runtime.state.read().await;
        chains.push(RpcsResponse {
            chain_id: runtime.config.chain_id,
            health: health_response(&state, &runtime.config),
            endpoints: public_endpoints(&state),
        });
    }

    Json(MultiRpcsResponse { chains })
}

async fn chain_health(Path(chain_id): Path<u64>, State(app): State<AppState>) -> Response {
    let Some(runtime) = app.chains.get(&chain_id).cloned() else {
        return chain_not_found(chain_id);
    };

    let state = runtime.state.read().await;
    Json(health_response(&state, &runtime.config)).into_response()
}

async fn chain_rpcs(Path(chain_id): Path<u64>, State(app): State<AppState>) -> Response {
    let Some(runtime) = app.chains.get(&chain_id).cloned() else {
        return chain_not_found(chain_id);
    };

    let state = runtime.state.read().await;
    Json(RpcsResponse {
        chain_id,
        health: health_response(&state, &runtime.config),
        endpoints: public_endpoints(&state),
    })
    .into_response()
}

async fn multi_health_response(app: &AppState) -> MultiHealthResponse {
    let mut chains = Vec::new();
    for runtime in sorted_runtimes(app) {
        let state = runtime.state.read().await;
        chains.push(health_response(&state, &runtime.config));
    }

    let healthy_chain_count = chains.iter().filter(|chain| chain.ok).count();
    MultiHealthResponse {
        ok: healthy_chain_count > 0,
        chain_count: chains.len(),
        healthy_chain_count,
        config: PublicMultiConfig {
            chain_ids: sorted_chain_ids(app),
            min_block_range: app
                .chains
                .values()
                .next()
                .map(|runtime| runtime.config.min_block_range)
                .unwrap_or_default(),
            port: SETTINGS.port,
            health_interval_ms: SETTINGS.health_interval_ms,
            chainlist_refresh_ms: SETTINGS.chainlist_refresh_ms,
            max_block_lag: SETTINGS.max_block_lag,
            max_health_age_ms: SETTINGS.max_health_age_ms,
            rate_limit_cooldown_ms: SETTINGS.rate_limit_cooldown_ms,
        },
        chains,
    }
}

fn health_response(state: &RelayState, config: &ChainConfig) -> HealthResponse {
    let healthy = state.healthy_endpoints();
    HealthResponse {
        chain_id: config.chain_id,
        ok: !healthy.is_empty(),
        healthy_count: healthy.len(),
        total_count: state.endpoints.len(),
        reference_latest_block: state.reference_latest_block,
        round: state.round,
        round_running: state.round_running,
        last_round_started_at: state.last_round_started_at.clone(),
        last_round_finished_at: state.last_round_finished_at.clone(),
        last_chainlist_refresh_at: state.last_chainlist_refresh_at.clone(),
        config: PublicChainConfig {
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

fn sorted_runtimes(app: &AppState) -> Vec<ChainRuntime> {
    let mut runtimes = app.chains.values().cloned().collect::<Vec<_>>();
    runtimes.sort_by_key(|runtime| runtime.config.chain_id);
    runtimes
}

fn sorted_chain_ids(app: &AppState) -> Vec<u64> {
    let mut chain_ids = app.chains.keys().copied().collect::<Vec<_>>();
    chain_ids.sort_unstable();
    chain_ids
}

fn chain_not_found(chain_id: u64) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "error": "chain_not_configured",
            "message": format!("Chain {chain_id} is not configured")
        })),
    )
        .into_response()
}

#[derive(Serialize)]
pub struct PublicMultiConfig {
    #[serde(rename = "chainIds")]
    pub chain_ids: Vec<u64>,
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
pub struct PublicChainConfig {
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
pub struct MultiHealthResponse {
    pub ok: bool,
    #[serde(rename = "chainCount")]
    pub chain_count: usize,
    #[serde(rename = "healthyChainCount")]
    pub healthy_chain_count: usize,
    pub config: PublicMultiConfig,
    pub chains: Vec<HealthResponse>,
}

#[derive(Serialize)]
pub struct HealthResponse {
    #[serde(rename = "chainId")]
    pub chain_id: u64,
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
    pub config: PublicChainConfig,
    #[serde(rename = "healthyRpcs")]
    pub healthy_rpcs: Vec<PublicEndpoint>,
}

#[derive(Serialize)]
pub struct MultiRpcsResponse {
    pub chains: Vec<RpcsResponse>,
}

#[derive(Serialize)]
pub struct RpcsResponse {
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub health: HealthResponse,
    pub endpoints: Vec<PublicEndpoint>,
}
