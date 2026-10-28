use tokio::signal;

use std::sync::Arc;
use std::time::Duration;

use blockchain::api::{router, AppState};
use blockchain::application::{Network, NodeService};
use blockchain::domain::rules::{ChainRules, DEFAULT_MAX_REORG_DEPTH};
use blockchain::infrastructure::{HttpPeerClient, JsonFileRepository};

const DEFAULT_ADDR: &str = "127.0.0.1:3000";
const DEFAULT_CHAIN_FILE: &str = "chain.json";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=debug".into()),
        )
        .init();

    let reward = env_or("REWARD", ChainRules::default().reward);
    let rules = match std::env::var("CONSENSUS").as_deref() {
        Ok("pos") => ChainRules::proof_of_stake(reward),
        Ok("pow") | Err(_) => ChainRules::proof_of_work(env_or("DIFFICULTY", 4), reward),
        Ok(other) => return Err(format!("CONSENSUS must be pow or pos, not {other}").into()),
    }
    .with_max_reorg_depth(env_or("MAX_REORG_DEPTH", DEFAULT_MAX_REORG_DEPTH));
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());
    let chain_file = std::env::var("CHAIN_FILE").unwrap_or_else(|_| DEFAULT_CHAIN_FILE.to_string());

    let repository = Box::new(JsonFileRepository::new(&chain_file));
    let node = NodeService::open(repository, rules).map_err(|e| format!("{chain_file}: {e}"))?;
    tracing::info!("Loaded {} blocks from {chain_file}", node.status().length);
    let public_url = std::env::var("PUBLIC_URL").unwrap_or_else(|_| format!("http://{addr}"));
    let network = Network::new(Arc::new(node), Box::new(HttpPeerClient::new()), Some(public_url));
    let state = AppState::new(network);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Node listening on {addr} ({rules})");

    let peers = std::env::var("PEERS").unwrap_or_default();
    let sync_every = Duration::from_secs(env_or("SYNC_SECS", 5).max(1));
    tokio::spawn(keep_in_sync(state.network.clone(), peers, sync_every));

    axum::serve(listener, router(state)).with_graceful_shutdown(shutdown_signal()).await?;

    tracing::info!("Node stopped");
    Ok(())
}

/// Joins the configured peers, then pulls from every peer on a fixed interval.
async fn keep_in_sync(network: Arc<Network>, peers: String, every: Duration) {
    for url in peers.split(',').map(str::trim).filter(|url| !url.is_empty()) {
        match network.add_peer(url).await {
            Ok(_) => tracing::info!("Peer {url}"),
            Err(e) => tracing::warn!("Skipping peer: {e}"),
        }
    }
    let mut ticker = tokio::time::interval(every);
    loop {
        ticker.tick().await;
        let report = network.sync().await;
        if let Some(peer) = report.adopted_from {
            tracing::info!("Adopted the heavier chain from {peer} ({} blocks)", report.length);
            network.broadcast_chain().await;
        }
    }
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to listen for ctrl-c");
    };
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
