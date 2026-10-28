use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use super::dto::{
    AddPeerResponse, BalanceResponse, ConsensusResponse, ErrorResponse, ForgeRequest, MineRequest,
    NodeResponse, PeerRequest, PeerResponse, ReplaceResponse, SignRequest, StakeEntry,
    SyncResponse, ValidityResponse, WalletResponse,
};
use super::AppState;
use crate::application::NodeError;
use crate::domain::address::Address;
use crate::domain::block::{Block, Transaction};
use crate::domain::crypto;
use crate::domain::error::ChainError;
use crate::domain::rules::Consensus;

/// Maps domain and application errors onto HTTP status codes and JSON error bodies.
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, message: message.into() }
    }
}

impl From<ChainError> for ApiError {
    fn from(err: ChainError) -> Self {
        let status = match err {
            ChainError::InsufficientFunds { .. }
            | ChainError::InsufficientStake { .. }
            | ChainError::BadNonce { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            ChainError::BadSignature => StatusCode::UNAUTHORIZED,
            ChainError::NotYourTurn { .. } => StatusCode::FORBIDDEN,
            ChainError::NotProofOfWork
            | ChainError::NotProofOfStake
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

pub async fn get_chain(State(state): State<AppState>) -> Json<Vec<Block>> {
    Json(state.node.blocks())
}

/// Offers this node another node's chain. It's adopted only if it is valid and heavier.
pub async fn replace_chain(
    State(state): State<AppState>,
    Json(blocks): Json<Vec<Block>>,
) -> Result<Json<ReplaceResponse>, ApiError> {
    let node = state.node.clone();
    let (replaced, length) = tokio::task::spawn_blocking(move || node.adopt_if_heavier(blocks))
        .await
        .expect("replace task panicked")?;
    if replaced {
        gossip_chain(&state);
    }
    Ok(Json(ReplaceResponse { replaced, length }))
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
    }))
}

/// Signs a transfer without submitting it. A convenience for local experiments.
pub async fn sign_transaction(
    State(state): State<AppState>,
    Json(req): Json<SignRequest>,
) -> Result<Json<Transaction>, ApiError> {
    let key = crypto::parse_secret_key(&req.secret_key)
        .ok_or_else(|| ApiError::bad_request("secret_key must be 32 bytes of hex"))?;
    let nonce = req.nonce.unwrap_or_else(|| state.node.next_nonce(&Address::of(&key)));
    Ok(Json(Transaction::signed(&key, &req.to, req.amount, nonce, req.fee)))
}

pub async fn submit_transaction(
    State(state): State<AppState>,
    Json(tx): Json<Transaction>,
) -> Result<(StatusCode, Json<Transaction>), ApiError> {
    state.node.submit(tx.clone())?;
    let (network, gossiped) = (state.network.clone(), tx.clone());
    tokio::spawn(async move { network.broadcast_transaction(&gossiped).await });
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
    let block = tokio::task::spawn_blocking(move || node.mine(&miner))
        .await
        .expect("mining task panicked")?;
    gossip_chain(&state);
    Ok((StatusCode::CREATED, Json(block)))
}

pub async fn get_consensus(State(state): State<AppState>) -> Json<ConsensusResponse> {
    let consensus = state.node.consensus();
    let (kind, difficulty) = match consensus.rules.consensus {
        Consensus::ProofOfWork { difficulty } => ("proof_of_work", Some(difficulty)),
        Consensus::ProofOfStake => ("proof_of_stake", None),
    };
    Json(ConsensusResponse {
        kind: kind.to_string(),
        difficulty,
        reward: consensus.rules.reward,
        next_validator: consensus.next_validator.map(|a| a.to_string()),
        total_stake: consensus.stakes.iter().map(|(_, n)| n).sum(),
        stakes: consensus
            .stakes
            .into_iter()
            .map(|(address, amount)| StakeEntry { address, amount })
            .collect(),
    })
}

/// Forges the next proof-of-stake block, signed by the validator whose key is sent.
pub async fn forge(
    State(state): State<AppState>,
    Json(req): Json<ForgeRequest>,
) -> Result<(StatusCode, Json<Block>), ApiError> {
    let key = crypto::parse_secret_key(&req.secret_key)
        .ok_or_else(|| ApiError::bad_request("secret_key must be 32 bytes of hex"))?;
    let block = state.node.forge(&key)?;
    gossip_chain(&state);
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
        gossip_chain(&state);
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

/// Tells peers about a new tip without making the caller wait for them.
fn gossip_chain(state: &AppState) {
    let network = state.network.clone();
    tokio::spawn(async move { network.broadcast_chain().await });
}
