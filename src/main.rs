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

use std::{net::SocketAddr, sync::Arc, time::Duration};

use reqwest::Client;
use tokio::{net::TcpListener, sync::RwLock, time};
use tracing::error;

use crate::{
    chainlist::refresh_chainlist, config::Config, health::run_health_round, server::app,
    settings::SETTINGS, state::RelayState,
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
    let state = Arc::new(RwLock::new(RelayState::new()));
    let client = Client::builder()
        .user_agent(SETTINGS.user_agent)
        .timeout(Duration::from_millis(SETTINGS.proxy_timeout_ms))
        .build()?;

    refresh_chainlist(&client, state.clone(), &config).await?;
    run_health_round(&client, state.clone(), &config).await;

    spawn_chainlist_refresh(client.clone(), state.clone(), config.clone());
    spawn_health_loop(client.clone(), state.clone(), config.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], SETTINGS.port));
    let listener = TcpListener::bind(addr).await?;

    println!(
        "Relay RPC listening on http://127.0.0.1:{} (CHAIN_ID={}, MIN_BLOCK_RANGE={})",
        SETTINGS.port, config.chain_id, config.min_block_range
    );

    axum::serve(listener, app(client, state, config)).await?;
    Ok(())
}

fn spawn_health_loop(client: Client, state: Arc<RwLock<RelayState>>, config: Config) {
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_millis(SETTINGS.health_interval_ms));
        loop {
            interval.tick().await;
            run_health_round(&client, state.clone(), &config).await;
        }
    });
}

fn spawn_chainlist_refresh(client: Client, state: Arc<RwLock<RelayState>>, config: Config) {
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
