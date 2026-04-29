use std::{collections::HashMap, env, fs, path::Path};

#[derive(Clone, Debug)]
pub struct Config {
    pub chain_id: u64,
    pub chain_id_hex: String,
    pub min_block_range: u64,
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let dot_env = read_dot_env(".env");
        let chain_id = read_u64("CHAIN_ID", 56, &dot_env)?;
        let min_block_range = read_u64("MIN_BLOCK_RANGE", 10001, &dot_env)?;

        if min_block_range <= 10_000 {
            return Err("MIN_BLOCK_RANGE must be greater than 10000".into());
        }

        Ok(Self {
            chain_id,
            chain_id_hex: format!("0x{:x}", chain_id),
            min_block_range,
        })
    }
}

fn read_u64(
    key: &str,
    fallback: u64,
    dot_env: &HashMap<String, String>,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let raw = env::var(key)
        .ok()
        .or_else(|| dot_env.get(key).cloned())
        .unwrap_or_else(|| fallback.to_string());
    Ok(raw.parse::<u64>()?)
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
