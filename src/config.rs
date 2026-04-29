use std::{collections::HashMap, env, fs, path::Path};

use crate::settings::SETTINGS;

#[derive(Clone, Debug)]
pub struct Config {
    pub chains: Vec<ChainConfig>,
    pub min_block_range: u64,
    pub health_interval_ms: u64,
    pub max_health_age_ms: u64,
    pub sources: ConfigSources,
}

#[derive(Clone, Debug)]
pub struct ConfigSources {
    pub chain_ids: &'static str,
    pub min_block_range: &'static str,
    pub health_interval_ms: &'static str,
}

#[derive(Clone, Debug)]
pub struct ChainConfig {
    pub chain_id: u64,
    pub chain_id_hex: String,
    pub min_block_range: u64,
    pub health_interval_ms: u64,
    pub max_health_age_ms: u64,
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let dot_env = read_dot_env(".env");
        let (chain_ids, chain_ids_source) = read_chain_ids(&dot_env)?;
        let (min_block_range, min_block_range_source) =
            read_u64("MIN_BLOCK_RANGE", 1000, &dot_env)?;
        let (health_interval_ms, health_interval_ms_source) =
            read_u64("HEALTH_INTERVAL_MS", SETTINGS.health_interval_ms, &dot_env)?;
        let max_health_age_ms = SETTINGS
            .max_health_age_ms
            .max(health_interval_ms.saturating_mul(3));

        if min_block_range < 1_000 {
            return Err(format!(
                "MIN_BLOCK_RANGE must be 1000 or greater; got {} from {}",
                min_block_range, min_block_range_source
            )
            .into());
        }
        if health_interval_ms == 0 {
            return Err(format!(
                "HEALTH_INTERVAL_MS must be greater than 0; got {} from {}",
                health_interval_ms, health_interval_ms_source
            )
            .into());
        }

        let chains = chain_ids
            .into_iter()
            .map(|chain_id| ChainConfig {
                chain_id,
                chain_id_hex: format!("0x{:x}", chain_id),
                min_block_range,
                health_interval_ms,
                max_health_age_ms,
            })
            .collect();

        Ok(Self {
            chains,
            min_block_range,
            health_interval_ms,
            max_health_age_ms,
            sources: ConfigSources {
                chain_ids: chain_ids_source,
                min_block_range: min_block_range_source,
                health_interval_ms: health_interval_ms_source,
            },
        })
    }

    pub fn chain_ids(&self) -> Vec<u64> {
        self.chains.iter().map(|chain| chain.chain_id).collect()
    }
}

fn read_chain_ids(
    dot_env: &HashMap<String, String>,
) -> Result<(Vec<u64>, &'static str), Box<dyn std::error::Error + Send + Sync>> {
    let (raw, source) = read_raw(&["CHAIN_IDS", "CHAIN_ID"], "56".to_string(), dot_env);

    let mut chain_ids = Vec::new();
    for item in raw.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let chain_id = item.parse::<u64>()?;
        if !chain_ids.contains(&chain_id) {
            chain_ids.push(chain_id);
        }
    }

    if chain_ids.is_empty() {
        return Err("CHAIN_IDS must contain at least one chain ID".into());
    }

    Ok((chain_ids, source))
}

fn read_u64(
    key: &str,
    fallback: u64,
    dot_env: &HashMap<String, String>,
) -> Result<(u64, &'static str), Box<dyn std::error::Error + Send + Sync>> {
    let (raw, source) = read_raw(&[key], fallback.to_string(), dot_env);
    Ok((raw.trim().parse::<u64>()?, source))
}

fn read_raw(
    keys: &[&str],
    fallback: String,
    dot_env: &HashMap<String, String>,
) -> (String, &'static str) {
    for key in keys {
        if let Ok(value) = env::var(key) {
            return (value, "environment");
        }
    }
    for key in keys {
        if let Some(value) = dot_env.get(*key) {
            return (value.clone(), ".env");
        }
    }
    (fallback, "default")
}

fn read_dot_env(path: impl AsRef<Path>) -> HashMap<String, String> {
    let Ok(content) = fs::read_to_string(path) else {
        return HashMap::new();
    };

    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            Some((
                key.trim().to_string(),
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            ))
        })
        .collect()
}
