use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use super::dto::{
    BalanceResponse, ErrorResponse, MineRequest, ReplaceResponse, SignRequest, ValidityResponse,
    WalletResponse,
};
use super::AppState;
use crate::application::NodeError;
use crate::domain::address::Address;
use crate::domain::block::{Block, Transaction};
use crate::domain::chain::ChainError;
use crate::domain::crypto;

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
            ChainError::InsufficientFunds { .. } | ChainError::BadNonce { .. } => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            ChainError::BadSignature => StatusCode::UNAUTHORIZED,
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

/// Offers this node another node's chain. It's adopted only if it is valid and longer.
pub async fn replace_chain(
    State(state): State<AppState>,
    Json(blocks): Json<Vec<Block>>,
) -> Result<Json<ReplaceResponse>, ApiError> {
    let node = state.node.clone();
    let (replaced, length) = tokio::task::spawn_blocking(move || node.adopt_if_longer(blocks))
        .await
        .expect("replace task panicked")?;
    Ok(Json(ReplaceResponse { replaced, length }))
}

pub async fn get_validity(State(state): State<AppState>) -> Json<ValidityResponse> {
    let (valid, length) = state.node.validity();
    Json(ValidityResponse { valid, length })
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
    Ok(Json(BalanceResponse { address, balance: account.balance, next_nonce: account.next_nonce }))
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
    let payload = Transaction::signing_payload(from.as_str(), &req.to, req.amount, nonce);
    Ok(Json(Transaction {
        signature: crypto::sign(&key, &payload),
        from: from.to_string(),
        to: req.to,
        amount: req.amount,
        nonce,
    }))
}

pub async fn submit_transaction(
    State(state): State<AppState>,
    Json(tx): Json<Transaction>,
) -> Result<(StatusCode, Json<Transaction>), ApiError> {
    state.node.submit(tx.clone())?;
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
    Ok((StatusCode::CREATED, Json(block)))
}
