use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use super::dto::{
    AddPeerResponse, BalanceResponse, ChainQuery, ConsensusResponse, ErrorResponse, ForgeRequest,
    MineRequest, NodeResponse, PeerRequest, PeerResponse, PlacementResponse, ProofResponse,
    ProposalRequest, ReplaceResponse, SignRequest, StakeEntry, StatusResponse, SyncResponse,
    TransactionResponse, ValidityResponse, WalletResponse,
};
use super::AppState;
use super::PEER_HEADER;
use crate::application::{Located, NodeError};
use crate::domain::address::Address;
use crate::domain::block::STAKE;
use crate::domain::block::{Block, BlockHeader, Transaction};
use crate::domain::chain::Extension;
use crate::domain::crypto;
use crate::domain::error::ChainError;
use crate::domain::randao;
use crate::domain::rules::Consensus;
use crate::domain::values::Coins;

/// Maps domain and application errors onto HTTP status codes and JSON error bodies.
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, message: message.into() }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self { status: StatusCode::NOT_FOUND, message: message.into() }
    }
}

impl From<ChainError> for ApiError {
    fn from(err: ChainError) -> Self {
        let status = match err {
            ChainError::InsufficientFunds { .. }
            | ChainError::InsufficientStake { .. }
            | ChainError::BadNonce { .. }
            | ChainError::MissingCommitment
            | ChainError::UnexpectedCommitment => StatusCode::UNPROCESSABLE_ENTITY,
            ChainError::BadSignature => StatusCode::UNAUTHORIZED,
            ChainError::NotYourTurn { .. } => StatusCode::FORBIDDEN,
            ChainError::NotProofOfWork
            | ChainError::NotProofOfStake
            | ChainError::StaleBlock
            | ChainError::ReorgTooDeep { .. } => StatusCode::CONFLICT,
            _ => StatusCode::BAD_REQUEST,
        };
        Self { status, message: err.to_string() }
    }
}

impl From<NodeError> for ApiError {
    fn from(err: NodeError) -> Self {
        match err {
            NodeError::Chain(e) => e.into(),
            NodeError::Storage(_) => {
                Self { status: StatusCode::INTERNAL_SERVER_ERROR, message: err.to_string() }
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorResponse { message: self.message })).into_response()
    }
}

/// Every block, or with `?from=N` the blocks from height N on, and with `&limit=M` at most M of
/// them.
pub async fn get_chain(
    State(state): State<AppState>,
    Query(query): Query<ChainQuery>,
) -> Json<Vec<Block>> {
    let from = query.from.unwrap_or(0);
    Json(state.node.blocks_between(from, query.limit.unwrap_or(usize::MAX)))
}

/// Every block's header, or with `?from=N` the headers from height N on. What a light client
/// follows instead of whole blocks.
pub async fn get_headers(
    State(state): State<AppState>,
    Query(query): Query<ChainQuery>,
) -> Json<Vec<BlockHeader>> {
    Json(state.node.headers_from(query.from.unwrap_or(0)))
}

/// A block by height or hash.
pub async fn get_block(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Block>, ApiError> {
    state
        .node
        .block(&id)
        .map(Json)
        .ok_or_else(|| ApiError::not_found("no block with that height or hash"))
}

/// A transaction by signature, and whether it's in a block yet.
pub async fn get_transaction(
    State(state): State<AppState>,
    Path(signature): Path<String>,
) -> Result<Json<TransactionResponse>, ApiError> {
    let located = state.node.locate(&signature).ok_or_else(|| {
        ApiError::not_found("no transaction with that signature, in a block or waiting")
    })?;
    Ok(Json(transaction_response(located)))
}

/// Every transaction to or from an address: pending ones first, then confirmed, newest first.
pub async fn get_address_transactions(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<Vec<TransactionResponse>>, ApiError> {
    let history = state.node.history(&Address::parse(&address)?);
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
        .node
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

/// Height, tip, weight and mempool size, without replaying the chain.
/// A peer polling it names itself, and is added back if this node had forgotten it.
pub async fn get_status(State(state): State<AppState>, headers: HeaderMap) -> Json<StatusResponse> {
    sender(&state, &headers);
    let tip = state.node.tip();
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
    let status = state.node.status();
    Json(ValidityResponse {
        valid: status.valid,
        length: status.length,
        weight: status.weight.to_string(),
    })
}

pub async fn create_wallet() -> (StatusCode, Json<WalletResponse>) {
    let (address, secret_key) = crypto::generate_keypair();
    (StatusCode::CREATED, Json(WalletResponse { address, secret_key }))
}

pub async fn get_balance(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<BalanceResponse>, ApiError> {
    let account = state.node.account(&Address::parse(&address)?);
    Ok(Json(BalanceResponse {
        address,
        balance: account.balance,
        staked: account.staked,
        next_nonce: account.next_nonce,
        next_epoch: account.validator.next_epoch,
        reveals_left: account.validator.reveals_left,
        commitment_due: account.commitment_due,
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
    let nonce = req.nonce.unwrap_or_else(|| state.node.next_nonce(&from));
    if req.to != STAKE {
        return Ok(Json(Transaction::signed(&key, &req.to, req.amount, nonce, req.fee)));
    }
    let due = state.node.account(&from).commitment_due;
    let commitment = req.commitment.or_else(|| due.map(|epoch| randao::commitment(&key, epoch)));
    Ok(Json(Transaction::stake(&key, req.amount, nonce, req.fee, commitment)))
}

pub async fn submit_transaction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(tx): Json<Transaction>,
) -> Result<(StatusCode, Json<Transaction>), ApiError> {
    state.node.submit(tx.clone())?;
    let (network, gossiped, from) = (state.network.clone(), tx.clone(), sender(&state, &headers));
    tokio::spawn(async move { network.broadcast_transaction(&gossiped, from.as_deref()).await });
    Ok((StatusCode::ACCEPTED, Json(tx)))
}

pub async fn get_pending(State(state): State<AppState>) -> Json<Vec<Transaction>> {
    Json(state.node.pending())
}

pub async fn mine(
    State(state): State<AppState>,
    Json(req): Json<MineRequest>,
) -> Result<(StatusCode, Json<Block>), ApiError> {
    let miner = Address::parse(&req.miner)?;
    let node = state.node.clone();
    // If the caller hangs up, this handler is dropped and the guard stops the search.
    let cancelled = Arc::new(AtomicBool::new(false));
    let _guard = CancelOnDrop(cancelled.clone());
    let block = tokio::task::spawn_blocking(move || node.mine_unless(&miner, &cancelled))
        .await
        .expect("mining task panicked")?;
    gossip_chain(&state, None);
    Ok((StatusCode::CREATED, Json(block)))
}

pub async fn get_consensus(State(state): State<AppState>) -> Json<ConsensusResponse> {
    let consensus = state.node.consensus();
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
        next_validator: consensus.next_validator.map(|a| a.to_string()),
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

/// Proof of work: the work each block required, by height, as strings. Takes `?from=N&limit=M`
/// like `/chain`.
pub async fn get_work(
    State(state): State<AppState>,
    Query(query): Query<ChainQuery>,
) -> Json<Vec<String>> {
    let history = state.node.work_history();
    let from = query.from.unwrap_or(0).min(history.len());
    let shown = history[from..].iter().take(query.limit.unwrap_or(usize::MAX));
    Json(shown.map(u128::to_string).collect())
}

/// The unsigned block a validator should sign next. Signing it where the key lives and sending
/// it to `POST /blocks` forges without the key ever leaving the client.
pub async fn propose(
    State(state): State<AppState>,
    Json(req): Json<ProposalRequest>,
) -> Result<Json<Block>, ApiError> {
    Ok(Json(state.node.propose(&Address::parse(&req.validator)?, req.reveal)?))
}

/// Accepts a proof-of-stake block signed by its validator.
pub async fn submit_block(
    State(state): State<AppState>,
    Json(block): Json<Block>,
) -> Result<(StatusCode, Json<Block>), ApiError> {
    let block = state.node.append_forged(block)?;
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
    let block = state.node.forge(&key)?;
    gossip_chain(&state, None);
    Ok((StatusCode::CREATED, Json(block)))
}

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

/// Tells peers, except the one it came from, about a new tip without making the caller wait.
fn gossip_chain(state: &AppState, except: Option<String>) {
    let network = state.network.clone();
    tokio::spawn(async move { network.broadcast_tip(except.as_deref()).await });
}

/// Sets its flag when dropped, which tells a search nobody is waiting for to stop.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// The peer that sent this request, if it said. A peer this node doesn't know yet is added, which
/// introduces this node back, without making the caller wait.
fn sender(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let url = headers.get(PEER_HEADER)?.to_str().ok()?.trim_end_matches('/').to_string();
    let (network, peer) = (state.network.clone(), url.clone());
    tokio::spawn(async move { network.add_peer(&peer).await });
    Some(url)
}
