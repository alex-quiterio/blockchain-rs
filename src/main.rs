use tokio::signal;

use std::sync::Arc;

use blockchain::api::{router, AppState};
use blockchain::application::{Network, NodeService};
use blockchain::infrastructure::{log_event, Config, HttpPeerClient, JsonFileRepository};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=debug".into()),
        )
        .init();

    let config = Config::from_env()?;
    let chain_file = &config.chain_file;
    let repository = Box::new(JsonFileRepository::new(chain_file));
    let started = std::time::Instant::now();
    let node = NodeService::open(repository, config.rules, config.reorg_limit)
        .map_err(|e| format!("{chain_file}: {e}"))?
        .with_snapshots_every(config.snapshot_every);
    let length = node.queries().tip().height + 1;
    match node.restored_from() {
        Some(height) => tracing::info!(
            "Loaded {length} blocks from {chain_file} in {:.0?}, replaying only those after the snapshot at {height}",
            started.elapsed()
        ),
        None => tracing::info!("Loaded {length} blocks from {chain_file} in {:.0?}", started.elapsed()),
    }
    node.subscribe(log_event);
    let client = Box::new(HttpPeerClient::new());
    let network = Network::new(Arc::new(node), client, Some(config.public_url));
    let state = AppState::new(network);
    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!("Node listening on {} ({})", config.bind_addr, config.rules);

    tokio::spawn(state.network.clone().keep_in_sync(config.peers, config.sync_every));

    axum::serve(listener, router(state)).with_graceful_shutdown(shutdown_signal()).await?;

    tracing::info!("Node stopped");
    Ok(())
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
