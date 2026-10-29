//! The HTTP interface: routes, handlers and their request and response bodies.

pub mod dto;
mod handlers;

use std::sync::Arc;

use axum::{
    http::{header, HeaderValue},
    routing::{get, post},
    Router,
};
use tower_http::{services::ServeDir, set_header::SetResponseHeader, trace::TraceLayer};

use chrono::{DateTime, Utc};

use crate::application::{ChainQueries, Network, NodeService};

/// The header a node names itself in when it talks to a peer: when it gossips, so the peer
/// doesn't relay the message straight back, and when it polls, so a peer that forgot it adds it
/// back.
pub const PEER_HEADER: &str = "x-peer";

#[derive(Clone)]
pub struct AppState {
    pub node: Arc<NodeService>,
    pub queries: ChainQueries,
    pub network: Arc<Network>,
    pub started_at: DateTime<Utc>,
}

impl AppState {
    pub fn new(network: Network) -> Self {
        let node = network.node().clone();
        let queries = node.queries();
        Self { node, queries, network: Arc::new(network), started_at: Utc::now() }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/chain", get(handlers::get_chain).post(handlers::replace_chain))
        .route("/chain/valid", get(handlers::get_validity))
        .route("/chain/work", get(handlers::get_work))
        .route("/status", get(handlers::get_status))
        .route("/wallets", post(handlers::create_wallet))
        .route("/balances/:address", get(handlers::get_balance))
        .route("/transactions", post(handlers::submit_transaction))
        .route("/transactions/sign", post(handlers::sign_transaction))
        .route("/transactions/pending", get(handlers::get_pending))
        .route("/transactions/:signature", get(handlers::get_transaction))
        .route("/transactions/:signature/proof", get(handlers::get_proof))
        .route("/blocks/:id", get(handlers::get_block))
        .route("/addresses/:address/transactions", get(handlers::get_address_transactions))
        .route("/headers", get(handlers::get_headers))
        .route("/mine", post(handlers::mine))
        .route("/forge", post(handlers::forge))
        .route("/forge/proposal", post(handlers::propose))
        .route("/blocks", post(handlers::submit_block))
        .route("/consensus", get(handlers::get_consensus))
        .route("/peers", get(handlers::get_peers).post(handlers::add_peer))
        .route("/sync", post(handlers::sync))
        .route("/node", get(handlers::get_node))
        .fallback_service(web_ui())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// The files in `public/`. Browsers must revalidate them on every load, so the page, its
/// styles and its script always come from the same version.
fn web_ui() -> SetResponseHeader<ServeDir, HeaderValue> {
    SetResponseHeader::overriding(
        ServeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/public")),
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    )
}
