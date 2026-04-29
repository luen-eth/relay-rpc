mod chainlist;
mod config;
mod health;
mod request_analysis;
mod router;
mod rpc;
mod server;
mod settings;
mod state;
mod types;
mod util;

use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use reqwest::Client;
use tokio::{net::TcpListener, sync::RwLock, time};
use tracing::error;

use crate::{
    chainlist::refresh_chainlist,
    config::{ChainConfig, Config},
    health::run_health_round,
    server::{app, ChainRuntime},
    settings::SETTINGS,
    state::RelayState,
};

#[tokio::main]
async fn main() -> anyhow_free::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "relay_rpc=info,info".into()),
        )
        .init();

    let config = Config::load()?;
    let client = Client::builder()
        .user_agent(SETTINGS.user_agent)
        .timeout(Duration::from_millis(SETTINGS.proxy_timeout_ms))
        .build()?;

    let chains = initialize_chains(&client, &config).await?;
    for runtime in chains.values() {
        spawn_chainlist_refresh(
            client.clone(),
            runtime.state.clone(),
            runtime.config.clone(),
        );
        spawn_health_loop(
            client.clone(),
            runtime.state.clone(),
            runtime.config.clone(),
        );
    }
    let chain_ids = config
        .chain_ids()
        .into_iter()
        .map(|chain_id| chain_id.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let addr = SocketAddr::from(([0, 0, 0, 0], SETTINGS.port));
    let listener = TcpListener::bind(addr).await?;

    println!(
        "Relay RPC listening on http://127.0.0.1:{} (CHAIN_IDS={}, MIN_BLOCK_RANGE={}, HEALTH_INTERVAL_MS={}, MAX_HEALTH_AGE_MS={})",
        SETTINGS.port,
        chain_ids,
        config.min_block_range,
        config.health_interval_ms,
        config.max_health_age_ms
    );

    axum::serve(listener, app(client, Arc::new(chains))).await?;
    Ok(())
}

async fn initialize_chains(
    client: &Client,
    config: &Config,
) -> Result<HashMap<u64, ChainRuntime>, Box<dyn std::error::Error + Send + Sync>> {
    let mut chains = HashMap::new();

    for chain_config in &config.chains {
        let state = Arc::new(RwLock::new(RelayState::new()));
        refresh_chainlist(client, state.clone(), chain_config).await?;
        run_health_round(client, state.clone(), chain_config).await;

        chains.insert(
            chain_config.chain_id,
            ChainRuntime {
                config: chain_config.clone(),
                state,
            },
        );
    }

    Ok(chains)
}

fn spawn_health_loop(client: Client, state: Arc<RwLock<RelayState>>, config: ChainConfig) {
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_millis(config.health_interval_ms));
        loop {
            interval.tick().await;
            run_health_round(&client, state.clone(), &config).await;
        }
    });
}

fn spawn_chainlist_refresh(client: Client, state: Arc<RwLock<RelayState>>, config: ChainConfig) {
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_millis(SETTINGS.chainlist_refresh_ms));
        loop {
            interval.tick().await;
            if let Err(error) = refresh_chainlist(&client, state.clone(), &config).await {
                error!(%error, "chainlist refresh failed");
            }
        }
    });
}

mod anyhow_free {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
}
