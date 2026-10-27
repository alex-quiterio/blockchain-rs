use tokio::signal;

use blockchain::api::{router, AppState};
use blockchain::application::NodeService;
use blockchain::domain::chain::{DEFAULT_DIFFICULTY, DEFAULT_REWARD};
use blockchain::infrastructure::JsonFileRepository;

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

    let difficulty = env_or("DIFFICULTY", DEFAULT_DIFFICULTY);
    let reward = env_or("REWARD", DEFAULT_REWARD);
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());
    let chain_file = std::env::var("CHAIN_FILE").unwrap_or_else(|_| DEFAULT_CHAIN_FILE.to_string());

    let repository = Box::new(JsonFileRepository::new(&chain_file));
    let node = NodeService::open(repository, difficulty, reward)
        .map_err(|e| format!("{chain_file}: {e}"))?;
    tracing::info!("Loaded {} blocks from {chain_file}", node.validity().1);
    let state = AppState::new(node);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Node listening on {addr} (difficulty {difficulty}, reward {reward})");

    axum::serve(listener, router(state)).with_graceful_shutdown(shutdown_signal()).await?;

    tracing::info!("Node stopped");
    Ok(())
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
