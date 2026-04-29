use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct RuntimeSettings {
    pub port: u16,
    pub chainlist_url: &'static str,
    pub custom_rpc_list_path: &'static str,
    pub chainlist_refresh_ms: u64,
    pub health_interval_ms: u64,
    pub health_timeout_ms: u64,
    pub proxy_timeout_ms: u64,
    pub health_concurrency: usize,
    pub max_block_lag: u64,
    pub max_health_age_ms: u64,
    pub retry_attempts: usize,
    pub rate_limit_cooldown_ms: u64,
    pub max_body_bytes: usize,
    pub user_agent: &'static str,
}

pub const SETTINGS: RuntimeSettings = RuntimeSettings {
    port: 8546,
    chainlist_url: "https://chainlist.org/rpcs.json",
    custom_rpc_list_path: "customrpclist.json",
    chainlist_refresh_ms: 5 * 60 * 1000,
    health_interval_ms: 5000,
    health_timeout_ms: 2500,
    proxy_timeout_ms: 15000,
    health_concurrency: 24,
    max_block_lag: 15,
    max_health_age_ms: 15000,
    retry_attempts: 3,
    rate_limit_cooldown_ms: 30000,
    max_body_bytes: 2 * 1024 * 1024,
    user_agent: "relay-rpc",
};

impl RuntimeSettings {
    pub fn health_timeout(self) -> Duration {
        Duration::from_millis(self.health_timeout_ms)
    }

    pub fn proxy_timeout(self) -> Duration {
        Duration::from_millis(self.proxy_timeout_ms)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HistoricalProbe {
    pub state_block: u64,
    pub logs_block: u64,
    pub contract: &'static str,
    pub call_data: &'static str,
    pub expected_suffix: &'static str,
    pub sparse_topic: &'static str,
}

pub const DEFAULT_SPARSE_TOPIC: &str =
    "0xe68d2c359a771606c400cf8b87000cf5864010363d6a736e98f5047b7bbe18e9";
pub const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";

pub fn historical_probe(chain_id: u64) -> Option<HistoricalProbe> {
    match chain_id {
        56 => Some(HistoricalProbe {
            state_block: 82103946,
            logs_block: 82103946,
            contract: "0xd78d74565e80f34a1bc1e12c7006916cd491f30e",
            call_data: "0x8da5cb5b",
            expected_suffix: "ec6a5c8ad84e3e8c73a617ffccdab3bddba6d94e",
            sparse_topic: DEFAULT_SPARSE_TOPIC,
        }),
        _ => None,
    }
}
