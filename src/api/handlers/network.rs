//! This node among its peers: who they are, pulling from them now, and which build it runs.

use axum::{extract::State, http::StatusCode, Json};

use super::error::ApiError;
use super::gossip_chain;
use crate::api::dto::network::{
    AddPeerResponse, NodeResponse, PeerRequest, PeerResponse, SyncResponse,
};
use crate::api::AppState;

pub async fn get_peers(State(state): State<AppState>) -> Json<Vec<PeerResponse>> {
    Json(
        state
            .network
            .peers()
            .into_iter()
            .map(|(url, s)| PeerResponse {
                url,
                height: s.height,
                last_seen: s.last_seen,
                error: s.error,
            })
            .collect(),
    )
}

/// Adds a peer. The node introduces itself back, so the link goes both ways.
pub async fn add_peer(
    State(state): State<AppState>,
    Json(req): Json<PeerRequest>,
) -> Result<(StatusCode, Json<AddPeerResponse>), ApiError> {
    let added = state.network.add_peer(&req.url).await.map_err(ApiError::bad_request)?;
    let status = if added { StatusCode::CREATED } else { StatusCode::OK };
    Ok((status, Json(AddPeerResponse { url: req.url, added })))
}

/// Pulls from every peer now instead of waiting for the next scheduled sync.
pub async fn sync(State(state): State<AppState>) -> Json<SyncResponse> {
    let report = state.network.sync().await;
    if report.adopted_from.is_some() {
        gossip_chain(&state, report.adopted_from.clone());
    }
    Json(SyncResponse {
        adopted_from: report.adopted_from,
        reached: report.reached,
        length: report.length,
    })
}

pub async fn get_node(State(state): State<AppState>) -> Json<NodeResponse> {
    Json(NodeResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit: env!("GIT_COMMIT").to_string(),
        started_at: state.started_at,
    })
}
