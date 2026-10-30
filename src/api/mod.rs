//! The HTTP interface: the routes (`router`), the handlers behind them and their request and
//! response bodies (`dto`), and the state every handler shares.

pub mod dto;
mod handlers;
mod router;

use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::application::{ChainQueries, Network, NodeService};

pub use router::router;

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
        let network = Arc::new(network);
        network.gossip_accepted();
        Self { node, queries, network, started_at: Utc::now() }
    }
}
