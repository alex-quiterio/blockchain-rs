//! Adding blocks: mining under proof of work, forging under proof of stake, and the rules and
//! state that decide who may add the next one.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};

use super::error::ApiError;
use super::gossip_chain;
use crate::api::dto::blocks::{
    ConsensusResponse, ForgeRequest, MineRequest, ProposalRequest, StakeEntry, TurnEntry,
};
use crate::api::AppState;
use crate::application::off_runtime;
use crate::domain::accounting::Coins;
use crate::domain::chain::block::Block;
use crate::domain::consensus::rules::Consensus;
use crate::domain::consensus::turns::TURN_SECS;
use crate::domain::crypto;
use crate::domain::crypto::address::Address;

pub async fn mine(
    State(state): State<AppState>,
    Json(req): Json<MineRequest>,
) -> Result<(StatusCode, Json<Block>), ApiError> {
    let miner = Address::parse(&req.miner)?;
    let node = state.node.clone();
    // If the caller hangs up, this handler is dropped and the guard stops the search.
    let cancelled = Arc::new(AtomicBool::new(false));
    let _guard = CancelOnDrop(cancelled.clone());
    let block = off_runtime(move || node.mine_unless(&miner, &cancelled)).await?;
    gossip_chain(&state, None);
    Ok((StatusCode::CREATED, Json(block)))
}

pub async fn get_consensus(State(state): State<AppState>) -> Json<ConsensusResponse> {
    let consensus = state.queries.consensus();
    let pos = consensus.rules.is_proof_of_stake();
    let retarget = consensus.rules.retarget();
    let (kind, difficulty) = match consensus.rules.consensus {
        Consensus::ProofOfWork { difficulty, .. } => ("proof_of_work", Some(difficulty)),
        Consensus::ProofOfStake => ("proof_of_stake", None),
    };
    Json(ConsensusResponse {
        kind: kind.to_string(),
        difficulty,
        reward: consensus.rules.reward,
        max_block_transfers: consensus.rules.block_capacity.max(),
        next_validator: consensus.next_validator.map(|a| a.to_string()),
        turns: consensus
            .turns
            .into_iter()
            .map(|(validator, from)| TurnEntry { validator, from })
            .collect(),
        turn_secs: pos.then_some(TURN_SECS),
        // Stakes are coins people own, so together they're never more than all there are.
        total_stake: Coins::total(consensus.stakes.iter().map(|(_, n)| *n)).unwrap_or_default(),
        stakes: consensus
            .stakes
            .into_iter()
            .map(|(address, amount)| StakeEntry { address, amount })
            .collect(),
        beacon: pos.then_some(consensus.beacon),
        work: (!pos).then(|| consensus.next_work.to_string()),
        target_block_secs: retarget.map(|r| r.target_block_secs),
        retarget_interval: retarget.map(|r| r.interval),
        next_retarget: consensus.next_retarget,
    })
}

/// The unsigned block a validator should sign next. Signing it where the key lives and sending
/// it to `POST /blocks` forges without the key ever leaving the client.
pub async fn propose(
    State(state): State<AppState>,
    Json(req): Json<ProposalRequest>,
) -> Result<Json<Block>, ApiError> {
    Ok(Json(state.queries.propose(&Address::parse(&req.validator)?, req.reveal)?))
}

/// Accepts a proof-of-stake block signed by its validator.
pub async fn submit_block(
    State(state): State<AppState>,
    Json(block): Json<Block>,
) -> Result<(StatusCode, Json<Block>), ApiError> {
    let node = state.node.clone();
    let block = off_runtime(move || node.append_forged(block)).await?;
    gossip_chain(&state, None);
    Ok((StatusCode::CREATED, Json(block)))
}

/// Forges the next proof-of-stake block, signed by the validator whose key is sent. A shortcut
/// for scripts; the web UI signs in the browser with `/forge/proposal` and `/blocks` instead.
pub async fn forge(
    State(state): State<AppState>,
    Json(req): Json<ForgeRequest>,
) -> Result<(StatusCode, Json<Block>), ApiError> {
    let key = crypto::parse_secret_key(&req.secret_key)
        .ok_or_else(|| ApiError::bad_request("secret_key must be 32 bytes of hex"))?;
    let node = state.node.clone();
    let block = off_runtime(move || node.forge(&key)).await?;
    gossip_chain(&state, None);
    Ok((StatusCode::CREATED, Json(block)))
}

/// Sets its flag when dropped, which tells a search nobody is waiting for to stop.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
