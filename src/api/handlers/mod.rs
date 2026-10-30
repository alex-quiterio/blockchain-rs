//! The HTTP handlers, one file per part of the API. Each turns a request into a call on the
//! node, its queries or its network, and the answer into a response body.

pub mod blocks;
pub mod chain;
mod error;
pub mod network;
pub mod transactions;
pub mod wallets;

use axum::http::HeaderMap;

use super::{AppState, PEER_HEADER};

/// Tells peers, except the one it came from, about a new tip without making the caller wait.
fn gossip_chain(state: &AppState, except: Option<String>) {
    let network = state.network.clone();
    tokio::spawn(async move { network.broadcast_tip(except.as_deref()).await });
}

/// The peer that sent this request, if it said. A peer this node doesn't know yet is added, which
/// introduces this node back, without making the caller wait.
fn sender(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let url = headers.get(PEER_HEADER)?.to_str().ok()?.trim_end_matches('/').to_string();
    let (network, peer) = (state.network.clone(), url.clone());
    tokio::spawn(async move { network.add_peer(&peer).await });
    Some(url)
}
