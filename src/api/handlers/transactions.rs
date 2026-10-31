//! Transfers: signing, submitting, finding and proving them, what's waiting, and what fee gets
//! one into the next block.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};

use super::error::ApiError;
use super::unbanned_sender;
use crate::api::dto::transactions::{
    BlockFeesEntry, FeesResponse, PlacementResponse, ProofResponse, SignRequest,
    TransactionResponse,
};
use crate::api::AppState;
use crate::application::Located;
use crate::domain::accounting::transaction::Transaction;
use crate::domain::accounting::Destination;
use crate::domain::consensus::randao;
use crate::domain::crypto;
use crate::domain::crypto::address::Address;

/// A transaction by signature, and whether it's in a block yet.
pub async fn get_transaction(
    State(state): State<AppState>,
    Path(signature): Path<String>,
) -> Result<Json<TransactionResponse>, ApiError> {
    let located = state.queries.locate(&signature).ok_or_else(|| {
        ApiError::not_found("no transaction with that signature, in a block or waiting")
    })?;
    Ok(Json(transaction_response(located)))
}

/// Every transaction to or from an address: pending ones first, then confirmed, newest first.
pub async fn get_address_transactions(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<Vec<TransactionResponse>>, ApiError> {
    let history = state.queries.history(&Address::parse(&address)?);
    Ok(Json(history.into_iter().map(transaction_response).collect()))
}

fn transaction_response(located: Located) -> TransactionResponse {
    TransactionResponse {
        transaction: located.transaction,
        status: if located.block.is_some() { "confirmed" } else { "pending" }.to_string(),
        block: located.block.map(|b| PlacementResponse {
            height: b.height,
            hash: b.hash,
            position: b.position,
            timestamp: b.timestamp,
            confirmations: b.confirmations,
        }),
    }
}

/// A Merkle proof that the confirmed transfer with this signature is in its block.
pub async fn get_proof(
    State(state): State<AppState>,
    Path(signature): Path<String>,
) -> Result<Json<ProofResponse>, ApiError> {
    let proof = state
        .queries
        .prove(&signature)
        .ok_or_else(|| ApiError::not_found("no block holds a transaction with that signature"))?;
    Ok(Json(ProofResponse {
        transaction: proof.transaction,
        leaf: proof.leaf,
        proof: proof.steps,
        header: proof.header,
        confirmations: proof.confirmations,
    }))
}

/// Signs a transfer without submitting it. A convenience for local experiments.
pub async fn sign_transaction(
    State(state): State<AppState>,
    Json(req): Json<SignRequest>,
) -> Result<Json<Transaction>, ApiError> {
    let key = crypto::parse_secret_key(&req.secret_key)
        .ok_or_else(|| ApiError::bad_request("secret_key must be 32 bytes of hex"))?;
    let from = Address::of(&key);
    let nonce = req.nonce.unwrap_or_else(|| state.queries.next_nonce(&from));
    let to = Destination::parse(&req.to)?;
    let chain = state.queries.chain_id();
    if to != Destination::Stake {
        return Ok(Json(Transaction::signed(&key, &chain, to, req.amount, nonce, req.fee)));
    }
    let due = state.queries.account(&from).commitment_due;
    let commitment = req.commitment.or_else(|| due.map(|epoch| randao::commitment(&key, epoch)));
    Ok(Json(Transaction::stake(&key, &chain, req.amount, nonce, req.fee, commitment)))
}

pub async fn submit_transaction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(tx): Json<Transaction>,
) -> Result<(StatusCode, Json<Transaction>), ApiError> {
    // Gossip follows from the mempool accepting it; see `Network::gossip_accepted`.
    let from = unbanned_sender(&state, &headers)?;
    state.node.submit_from(tx.clone(), from.as_deref())?;
    Ok((StatusCode::ACCEPTED, Json(tx)))
}

pub async fn get_pending(State(state): State<AppState>) -> Json<Vec<Transaction>> {
    Json(state.queries.pending())
}

pub async fn get_fees(State(state): State<AppState>) -> Json<FeesResponse> {
    let report = state.queries.fees();
    Json(FeesResponse {
        fee: report.next_block.fee,
        // Without a limit there's always room, and no number to give for it.
        room: report.capacity.max().map(|_| report.next_block.room),
        capacity: report.capacity.max(),
        waiting: report.waiting,
        recent: report
            .recent
            .into_iter()
            .map(|b| BlockFeesEntry {
                height: b.height,
                transfers: b.transfers,
                full: b.full,
                lowest_fee: b.lowest_fee,
            })
            .collect(),
    })
}
