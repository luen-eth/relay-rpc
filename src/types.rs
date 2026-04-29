use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct ChainlistNetwork {
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub rpc: Vec<Value>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct EndpointRange {
    #[serde(rename = "minSupported")]
    pub min_supported: u64,
    #[serde(rename = "maxObserved")]
    pub max_observed: u64,
    #[serde(rename = "rejectedAbove")]
    pub rejected_above: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct EndpointChecks {
    #[serde(rename = "chainOk")]
    pub chain_ok: bool,
    #[serde(rename = "archiveOk")]
    pub archive_ok: bool,
    #[serde(rename = "recentRangeOk")]
    pub recent_range_ok: bool,
    #[serde(rename = "historicalRangeOk")]
    pub historical_range_ok: bool,
    #[serde(rename = "lagOk")]
    pub lag_ok: bool,
    #[serde(rename = "freshOk")]
    pub fresh_ok: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Endpoint {
    pub url: String,
    pub healthy: bool,
    pub reason: Vec<String>,
    #[serde(rename = "latestBlock")]
    pub latest_block: Option<u64>,
    #[serde(rename = "latencyMs")]
    pub latency_ms: Option<u64>,
    #[serde(rename = "cooldownUntilMs")]
    pub cooldown_until_ms: u64,
    pub range: EndpointRange,
    pub checks: EndpointChecks,
    #[serde(rename = "lastCheckedAt")]
    pub last_checked_at: Option<String>,
    #[serde(skip)]
    pub last_checked_at_ms: Option<u64>,
    #[serde(rename = "lastHealthyAt")]
    pub last_healthy_at: Option<String>,
    #[serde(rename = "consecutiveFailures")]
    pub consecutive_failures: u64,
    #[serde(rename = "proxyFailures")]
    pub proxy_failures: u64,
}

impl Endpoint {
    pub fn new(url: String, min_block_range: u64) -> Self {
        Self {
            url,
            healthy: false,
            reason: vec!["not checked yet".to_string()],
            latest_block: None,
            latency_ms: None,
            cooldown_until_ms: 0,
            range: EndpointRange {
                min_supported: min_block_range,
                max_observed: min_block_range,
                rejected_above: None,
            },
            checks: EndpointChecks::default(),
            last_checked_at: None,
            last_checked_at_ms: None,
            last_healthy_at: None,
            consecutive_failures: 0,
            proxy_failures: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HealthSnapshot {
    pub url: String,
    pub reason: Vec<String>,
    pub latest_block: Option<u64>,
    pub latency_ms: u64,
    pub checks: EndpointChecks,
    pub checked_at: String,
    pub checked_at_ms: u64,
}

#[derive(Debug)]
pub struct RpcResult {
    pub result: Option<Value>,
    pub error: Option<Value>,
}

#[derive(Debug)]
pub struct RawProxyResponse {
    pub ok: bool,
    pub status: reqwest::StatusCode,
    pub body: bytes::Bytes,
    pub content_type: String,
    pub error: Option<String>,
}
