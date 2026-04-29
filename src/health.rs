use std::{sync::Arc, time::Instant};

use futures::{stream, StreamExt};
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::{
    config::Config,
    rpc::rpc_call,
    settings::{historical_probe, DEFAULT_SPARSE_TOPIC, SETTINGS, ZERO_ADDRESS},
    state::RelayState,
    types::{EndpointChecks, HealthSnapshot},
    util::{now_iso, now_ms, rpc_error_message, to_hex},
};

pub async fn run_health_round(client: &Client, state: Arc<RwLock<RelayState>>, config: &Config) {
    {
        let mut state = state.write().await;
        if state.round_running {
            println!("[health] previous round still running; skipping overlap");
            return;
        }
        state.round_running = true;
        state.round += 1;
        state.last_round_started_at = Some(now_iso());
    }

    let urls = {
        let state = state.read().await;
        state.endpoints.keys().cloned().collect::<Vec<_>>()
    };

    let snapshots = stream::iter(urls)
        .map(|url| async move { check_endpoint(client, url, config).await })
        .buffer_unordered(SETTINGS.health_concurrency)
        .collect::<Vec<_>>()
        .await;

    let reference_latest_block = snapshots
        .iter()
        .filter_map(|snapshot| snapshot.latest_block)
        .max()
        .unwrap_or_default();

    let mut state = state.write().await;
    state.reference_latest_block = reference_latest_block;

    for snapshot in snapshots {
        if let Some(endpoint) = state.endpoints.get_mut(&snapshot.url) {
            apply_snapshot(
                endpoint,
                snapshot,
                reference_latest_block,
                config.min_block_range,
            );
        }
    }

    let healthy_count = state.healthy_endpoints().len();
    let total_count = state.endpoints.len();
    let round = state.round;

    state.round_running = false;
    state.last_round_finished_at = Some(now_iso());

    println!(
        "[health] round {round}: {healthy_count}/{total_count} healthy; reference block {reference_latest_block}"
    );
}

async fn check_endpoint(client: &Client, url: String, config: &Config) -> HealthSnapshot {
    let started = Instant::now();
    let checked_at_ms = now_ms();
    let mut reason = Vec::new();
    let mut checks = EndpointChecks::default();
    let mut latest_block = None;

    let chain = rpc_call(client, &url, "eth_chainId", json!([])).await;
    checks.chain_ok = chain
        .result
        .as_ref()
        .and_then(Value::as_str)
        .is_some_and(|chain_id| chain_id == config.chain_id_hex);
    if !checks.chain_ok {
        reason.push(format!(
            "chainId failed: {}",
            chain
                .error
                .as_ref()
                .map(rpc_error_message)
                .unwrap_or_default()
        ));
    }

    if checks.chain_ok {
        let block = rpc_call(client, &url, "eth_blockNumber", json!([])).await;
        latest_block = block
            .result
            .as_ref()
            .and_then(Value::as_str)
            .and_then(|block| u64::from_str_radix(block.trim_start_matches("0x"), 16).ok());
        if latest_block.is_none() {
            reason.push(format!(
                "blockNumber failed: {}",
                block
                    .error
                    .as_ref()
                    .map(rpc_error_message)
                    .unwrap_or_default()
            ));
        }

        checks.recent_range_ok = if let Some(latest) = latest_block {
            let from = latest.saturating_sub(config.min_block_range - 1);
            let result = rpc_call(
                client,
                &url,
                "eth_getLogs",
                json!([{
                    "fromBlock": to_hex(from),
                    "toBlock": to_hex(latest),
                    "topics": [sparse_topic(config.chain_id)]
                }]),
            )
            .await;
            result.result.as_ref().is_some_and(Value::is_array)
        } else {
            false
        };
        if !checks.recent_range_ok {
            reason.push(format!("recent eth_getLogs range failed"));
        }

        let (archive_ok, historical_range_ok) =
            check_archive(client, &url, config, latest_block).await;
        checks.archive_ok = archive_ok;
        checks.historical_range_ok = historical_range_ok;
        if !checks.archive_ok {
            reason.push("historical state failed".to_string());
        }
        if !checks.historical_range_ok {
            reason.push("historical eth_getLogs range failed".to_string());
        }
    }

    HealthSnapshot {
        url,
        reason,
        latest_block,
        latency_ms: started.elapsed().as_millis() as u64,
        checks,
        checked_at: now_iso(),
        checked_at_ms,
    }
}

async fn check_archive(
    client: &Client,
    url: &str,
    config: &Config,
    latest_block: Option<u64>,
) -> (bool, bool) {
    if let Some(probe) = historical_probe(config.chain_id) {
        let state = rpc_call(
            client,
            url,
            "eth_call",
            json!([{
                "to": probe.contract,
                "data": probe.call_data
            }, to_hex(probe.state_block)]),
        )
        .await;
        let archive_ok = state
            .result
            .as_ref()
            .and_then(Value::as_str)
            .is_some_and(|result| result.to_lowercase().ends_with(probe.expected_suffix));

        let logs = rpc_call(
            client,
            url,
            "eth_getLogs",
            json!([{
                "fromBlock": to_hex(probe.logs_block),
                "toBlock": to_hex(probe.logs_block + config.min_block_range - 1),
                "address": probe.contract,
                "topics": [probe.sparse_topic]
            }]),
        )
        .await;
        return (
            archive_ok,
            logs.result.as_ref().is_some_and(Value::is_array),
        );
    }

    let Some(latest_block) = latest_block else {
        return (false, false);
    };
    let historical_block = latest_block
        .saturating_sub(config.min_block_range * 10)
        .max(1);
    let balance = rpc_call(
        client,
        url,
        "eth_getBalance",
        json!([ZERO_ADDRESS, to_hex(historical_block)]),
    )
    .await;
    let archive_ok = balance
        .result
        .as_ref()
        .and_then(Value::as_str)
        .is_some_and(|result| result.starts_with("0x"));

    let logs = rpc_call(
        client,
        url,
        "eth_getLogs",
        json!([{
            "fromBlock": to_hex(historical_block),
            "toBlock": to_hex(historical_block + config.min_block_range - 1),
            "topics": [DEFAULT_SPARSE_TOPIC]
        }]),
    )
    .await;

    (
        archive_ok,
        logs.result.as_ref().is_some_and(Value::is_array),
    )
}

fn apply_snapshot(
    endpoint: &mut crate::types::Endpoint,
    snapshot: HealthSnapshot,
    reference_latest_block: u64,
    min_block_range: u64,
) {
    endpoint.latest_block = snapshot.latest_block;
    endpoint.latency_ms = Some(snapshot.latency_ms);
    endpoint.last_checked_at = Some(snapshot.checked_at.clone());
    endpoint.last_checked_at_ms = Some(snapshot.checked_at_ms);
    endpoint.checks = snapshot.checks;

    endpoint.checks.lag_ok = endpoint
        .latest_block
        .is_some_and(|latest| latest + SETTINGS.max_block_lag >= reference_latest_block);
    endpoint.checks.fresh_ok =
        now_ms().saturating_sub(snapshot.checked_at_ms) <= SETTINGS.max_health_age_ms;

    endpoint.healthy = endpoint.checks.chain_ok
        && endpoint.checks.archive_ok
        && endpoint.checks.recent_range_ok
        && endpoint.checks.historical_range_ok
        && endpoint.checks.lag_ok
        && endpoint.checks.fresh_ok;

    if endpoint.healthy {
        endpoint.reason = vec!["ok".to_string()];
        endpoint.last_healthy_at = Some(now_iso());
        endpoint.consecutive_failures = 0;
        endpoint.range.min_supported = endpoint.range.min_supported.max(min_block_range);
        endpoint.range.max_observed = endpoint.range.max_observed.max(min_block_range);
    } else {
        let mut reason = snapshot.reason;
        if !endpoint.checks.lag_ok {
            reason.push(format!(
                "stale latest block: {:?} vs reference {}",
                endpoint.latest_block, reference_latest_block
            ));
        }
        endpoint.reason = reason;
        endpoint.consecutive_failures += 1;
    }
}

fn sparse_topic(chain_id: u64) -> &'static str {
    historical_probe(chain_id)
        .map(|probe| probe.sparse_topic)
        .unwrap_or(DEFAULT_SPARSE_TOPIC)
}
