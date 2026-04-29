use std::{collections::HashSet, fs, path::Path, sync::Arc};

use reqwest::Client;
use serde_json::{Map, Value};
use tokio::sync::RwLock;

use crate::{
    config::ChainConfig,
    settings::SETTINGS,
    state::RelayState,
    types::{ChainlistNetwork, Endpoint},
    util::now_iso,
};

pub async fn refresh_chainlist(
    client: &Client,
    state: Arc<RwLock<RelayState>>,
    config: &ChainConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let networks = client
        .get(SETTINGS.chainlist_url)
        .timeout(SETTINGS.proxy_timeout())
        .send()
        .await?
        .error_for_status()?
        .json::<Vec<ChainlistNetwork>>()
        .await?;

    let Some(network) = networks
        .into_iter()
        .find(|network| network.chain_id == config.chain_id)
    else {
        return Err(format!("chain {} was not found in Chainlist", config.chain_id).into());
    };

    let chainlist_urls = extract_http_urls(network.rpc);
    let custom_urls =
        read_custom_rpc_urls(Path::new(SETTINGS.custom_rpc_list_path), config.chain_id)?;
    let chainlist_count = chainlist_urls.len();
    let custom_count = custom_urls.len();
    let urls = merge_urls(chainlist_urls, custom_urls);
    let url_set: HashSet<_> = urls.iter().cloned().collect();

    let mut state = state.write().await;
    for url in &urls {
        state
            .endpoints
            .entry(url.clone())
            .or_insert_with(|| Endpoint::new(url.clone(), config.min_block_range));
    }
    state.endpoints.retain(|url, _| url_set.contains(url));
    state.last_chainlist_refresh_at = Some(now_iso());

    println!(
        "[chainlist] chain {}: {} HTTP RPC endpoints loaded ({} Chainlist + {} custom)",
        config.chain_id,
        urls.len(),
        chainlist_count,
        custom_count
    );
    Ok(())
}

fn read_custom_rpc_urls(
    path: &Path,
    chain_id: u64,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    if !path.is_file() {
        println!(
            "[chainlist] {} ignored because it is not a regular file",
            path.display()
        );
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(path)?;
    let value = serde_json::from_str::<Value>(&content)?;
    let rpc_entries = custom_rpc_entries_for_chain(value, chain_id);

    Ok(extract_http_urls(rpc_entries))
}

fn custom_rpc_entries_for_chain(value: Value, chain_id: u64) -> Vec<Value> {
    match value {
        Value::Object(map) => custom_rpc_entries_from_object(map, chain_id),
        Value::Array(items) => {
            let contains_chain_objects = items.iter().any(|item| {
                item.as_object()
                    .is_some_and(|map| map.contains_key("chainId") || map.contains_key("rpc"))
            });

            if contains_chain_objects {
                items
                    .into_iter()
                    .filter_map(|item| match item {
                        Value::Object(map) => {
                            Some(custom_rpc_entries_from_chain_object(map, chain_id))
                        }
                        _ => None,
                    })
                    .flatten()
                    .collect()
            } else {
                items
            }
        }
        _ => Vec::new(),
    }
}

fn custom_rpc_entries_from_object(mut map: Map<String, Value>, chain_id: u64) -> Vec<Value> {
    if let Some(Value::Array(chains)) = map.remove("chains") {
        return chains
            .into_iter()
            .filter_map(|item| match item {
                Value::Object(chain_map) => {
                    Some(custom_rpc_entries_from_chain_object(chain_map, chain_id))
                }
                _ => None,
            })
            .flatten()
            .collect();
    }

    custom_rpc_entries_from_chain_object(map, chain_id)
}

fn custom_rpc_entries_from_chain_object(mut map: Map<String, Value>, chain_id: u64) -> Vec<Value> {
    if let Some(custom_chain_id) = map.get("chainId").and_then(Value::as_u64) {
        if custom_chain_id != chain_id {
            return Vec::new();
        }
    }

    match map.remove("rpc") {
        Some(Value::Array(rpc_entries)) => rpc_entries,
        _ => Vec::new(),
    }
}

fn merge_urls(primary: Vec<String>, secondary: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut urls = Vec::new();

    for url in primary.into_iter().chain(secondary) {
        if seen.insert(url.clone()) {
            urls.push(url);
        }
    }

    urls
}

fn extract_http_urls(rpc_entries: Vec<Value>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut urls = Vec::new();

    for entry in rpc_entries {
        let url = match entry {
            Value::String(url) => url,
            Value::Object(map) => map
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            _ => String::new(),
        };

        let url = url.trim_end_matches('/').to_string();
        if !url.starts_with("http") || url.contains("${") || url.contains('<') {
            continue;
        }
        if seen.insert(url.clone()) {
            urls.push(url);
        }
    }

    urls
}
