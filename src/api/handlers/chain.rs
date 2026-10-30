//! Reading the chain, and offering this node someone else's: blocks, headers, the tip, the work
//! each block needed and a full replay.

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};

use super::error::ApiError;
use super::{gossip_chain, sender};
use crate::api::dto::chain::{ChainQuery, ReplaceResponse, StatusResponse, ValidityResponse};
use crate::api::AppState;
use crate::domain::block::{Block, BlockHeader};
use crate::domain::chain::Extension;

/// Every block, or with `?from=N` the blocks from height N on, and with `&limit=M` at most M of
/// them.
pub async fn get_chain(
    State(state): State<AppState>,
    Query(query): Query<ChainQuery>,
) -> Json<Vec<Block>> {
    let from = query.from.unwrap_or(0);
    Json(state.queries.blocks_between(from, query.limit.unwrap_or(usize::MAX)))
}

/// Every block's header, or with `?from=N` the headers from height N on. What a light client
/// follows instead of whole blocks.
pub async fn get_headers(
    State(state): State<AppState>,
    Query(query): Query<ChainQuery>,
) -> Json<Vec<BlockHeader>> {
    Json(state.queries.headers_from(query.from.unwrap_or(0)))
}

/// A block by height or hash.
pub async fn get_block(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Block>, ApiError> {
    state
        .queries
        .block(&id)
        .map(Json)
        .ok_or_else(|| ApiError::not_found("no block with that height or hash"))
}

/// Height, tip, weight and mempool size, without replaying the chain.
/// A peer polling it names itself, and is added back if this node had forgotten it.
pub async fn get_status(State(state): State<AppState>, headers: HeaderMap) -> Json<StatusResponse> {
    sender(&state, &headers);
    let tip = state.queries.tip();
    Json(StatusResponse {
        height: tip.height,
        tip: tip.hash,
        weight: tip.weight.to_string(),
        pending: tip.pending,
    })
}

/// Offers this node another node's blocks, from genesis or from any height on. They're adopted
/// only if they make a valid, heavier chain. A peer gossiping a new block sends just that block.
pub async fn replace_chain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(blocks): Json<Vec<Block>>,
) -> Result<Json<ReplaceResponse>, ApiError> {
    let node = state.node.clone();
    let (extension, length) = tokio::task::spawn_blocking(move || node.extend(blocks))
        .await
        .expect("replace task panicked")?;
    let from = sender(&state, &headers);
    let replaced = extension == Extension::Adopted;
    if replaced {
        gossip_chain(&state, from);
    }
    let missing_parent = extension == Extension::MissingParent;
    Ok(Json(ReplaceResponse { replaced, length, missing_parent }))
}

pub async fn get_validity(State(state): State<AppState>) -> Json<ValidityResponse> {
    let status = state.queries.status();
    Json(ValidityResponse {
        valid: status.problem.is_none(),
        problem: status.problem.map(|e| e.to_string()),
        length: status.length,
        weight: status.weight.to_string(),
    })
}

/// Proof of work: the work each block required, by height, as strings. Takes `?from=N&limit=M`
/// like `/chain`.
pub async fn get_work(
    State(state): State<AppState>,
    Query(query): Query<ChainQuery>,
) -> Json<Vec<String>> {
    let history = state.queries.work_history();
    let from = query.from.unwrap_or(0).min(history.len());
    let shown = history[from..].iter().take(query.limit.unwrap_or(usize::MAX));
    Json(shown.map(u128::to_string).collect())
}
