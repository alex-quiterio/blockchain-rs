use tokio::signal;

use std::sync::Arc;

use blockchain::api::{router, AppState};
use blockchain::application::{Network, NodeService};
use blockchain::infrastructure::{log_event, ChainJsonRepository, Config, HttpPeerClient};

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
    let repository = Box::new(ChainJsonRepository::new(chain_file));
    let started = std::time::Instant::now();
    // Listening from the start, so the log says when a new chain is started.
    let node = NodeService::opening(repository, config.rules, config.reorg_limit)
        .listening(log_event)
        .open()
        .map_err(|e| format!("{chain_file}: {e}"))?
        .with_snapshots_every(config.snapshot_every)
        .with_max_waiting(config.max_waiting);
    // A new chain says so through the log; a restored one says how much was loaded.
    if !node.started() {
        let length = node.queries().tip().height + 1;
        match node.restored_from() {
            Some(height) => tracing::info!(
                "Loaded {length} blocks from {chain_file} in {:.0?}, replaying only those after the snapshot at {height}",
                started.elapsed()
            ),
            None => tracing::info!("Loaded {length} blocks from {chain_file} in {:.0?}", started.elapsed()),
        }
    }
    let client = Box::new(HttpPeerClient::new());
    let network = Network::new(Arc::new(node), client, Some(config.public_url))
        .with_max_peers(config.max_peers)
        .with_forget_after(config.forget_after);
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
