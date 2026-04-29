use std::collections::HashMap;

use serde::Serialize;

use crate::{settings::SETTINGS, types::Endpoint, util::now_ms};

#[derive(Debug)]
pub struct RelayState {
    pub endpoints: HashMap<String, Endpoint>,
    pub reference_latest_block: u64,
    pub round: u64,
    pub round_running: bool,
    pub last_round_started_at: Option<String>,
    pub last_round_finished_at: Option<String>,
    pub last_chainlist_refresh_at: Option<String>,
    pub round_robin: HashMap<String, usize>,
}

impl RelayState {
    pub fn new() -> Self {
        Self {
            endpoints: HashMap::new(),
            reference_latest_block: 0,
            round: 0,
            round_running: false,
            last_round_started_at: None,
            last_round_finished_at: None,
            last_chainlist_refresh_at: None,
            round_robin: HashMap::new(),
        }
    }

    pub fn healthy_endpoints(&self) -> Vec<Endpoint> {
        let now = now_ms();
        let mut endpoints: Vec<_> = self
            .endpoints
            .values()
            .filter(|endpoint| endpoint_usable(endpoint, self.reference_latest_block, now))
            .cloned()
            .collect();
        endpoints.sort_by_key(|endpoint| endpoint_score(endpoint, self.reference_latest_block));
        endpoints
    }
}

pub fn endpoint_usable(endpoint: &Endpoint, reference_latest_block: u64, now: u64) -> bool {
    if !endpoint.healthy || endpoint.cooldown_until_ms > now {
        return false;
    }
    let Some(latest_block) = endpoint.latest_block else {
        return false;
    };
    let Some(last_checked_at_ms) = endpoint.last_checked_at_ms else {
        return false;
    };
    latest_block + SETTINGS.max_block_lag >= reference_latest_block
        && now.saturating_sub(last_checked_at_ms) <= SETTINGS.max_health_age_ms
}

pub fn endpoint_score(endpoint: &Endpoint, reference_latest_block: u64) -> u64 {
    let lag = reference_latest_block.saturating_sub(endpoint.latest_block.unwrap_or_default());
    lag * 10_000 + endpoint.latency_ms.unwrap_or(999_999) + endpoint.proxy_failures * 500
}

pub fn endpoint_supports_range(endpoint: &Endpoint, requested_range: Option<u64>) -> bool {
    let Some(range) = requested_range else {
        return true;
    };
    if range <= endpoint.range.min_supported || range <= endpoint.range.max_observed {
        return true;
    }
    if let Some(rejected_above) = endpoint.range.rejected_above {
        return range < rejected_above;
    }
    true
}

pub fn note_range_success(endpoint: &mut Endpoint, requested_range: Option<u64>) {
    if let Some(range) = requested_range {
        endpoint.range.max_observed = endpoint.range.max_observed.max(range);
    }
}

pub fn note_range_rejected(endpoint: &mut Endpoint, requested_range: Option<u64>) {
    if let Some(range) = requested_range {
        endpoint.range.rejected_above = Some(
            endpoint
                .range
                .rejected_above
                .map_or(range, |existing| existing.min(range)),
        );
    }
}

#[derive(Serialize)]
pub struct PublicEndpoint {
    pub url: String,
    pub healthy: bool,
    pub usable: bool,
    pub reason: Vec<String>,
    #[serde(rename = "latestBlock")]
    pub latest_block: Option<u64>,
    #[serde(rename = "referenceLatestBlock")]
    pub reference_latest_block: u64,
    pub lag: Option<i64>,
    #[serde(rename = "latencyMs")]
    pub latency_ms: Option<u64>,
    #[serde(rename = "cooldownUntilMs")]
    pub cooldown_until_ms: u64,
    pub range: crate::types::EndpointRange,
    pub checks: crate::types::EndpointChecks,
    #[serde(rename = "lastCheckedAt")]
    pub last_checked_at: Option<String>,
    #[serde(rename = "lastHealthyAt")]
    pub last_healthy_at: Option<String>,
    #[serde(rename = "consecutiveFailures")]
    pub consecutive_failures: u64,
    #[serde(rename = "proxyFailures")]
    pub proxy_failures: u64,
}

impl PublicEndpoint {
    pub fn from_endpoint(endpoint: &Endpoint, reference_latest_block: u64, usable: bool) -> Self {
        Self {
            url: endpoint.url.clone(),
            healthy: endpoint.healthy,
            usable,
            reason: endpoint.reason.clone(),
            latest_block: endpoint.latest_block,
            reference_latest_block,
            lag: endpoint
                .latest_block
                .map(|latest| reference_latest_block as i64 - latest as i64),
            latency_ms: endpoint.latency_ms,
            cooldown_until_ms: endpoint.cooldown_until_ms,
            range: endpoint.range.clone(),
            checks: endpoint.checks.clone(),
            last_checked_at: endpoint.last_checked_at.clone(),
            last_healthy_at: endpoint.last_healthy_at.clone(),
            consecutive_failures: endpoint.consecutive_failures,
            proxy_failures: endpoint.proxy_failures,
        }
    }
}
