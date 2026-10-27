use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::chain::ChainError;
use crate::crypto;
use crate::model::{
    BalanceResponse, Block, ErrorResponse, MineRequest, SignRequest, Transaction,
    ValidityResponse, WalletResponse,
};
use crate::server::AppState;

/// Maps chain errors onto HTTP status codes and JSON error bodies.
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

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorResponse { message: self.message })).into_response()
    }
}

pub async fn get_chain(State(state): State<AppState>) -> Json<Vec<Block>> {
    Json(state.chain.lock().unwrap().blocks().to_vec())
}

pub async fn get_validity(State(state): State<AppState>) -> Json<ValidityResponse> {
    let chain = state.chain.lock().unwrap();
    Json(ValidityResponse { valid: chain.is_valid(), length: chain.blocks().len() })
}

pub async fn create_wallet() -> (StatusCode, Json<WalletResponse>) {
    let (address, secret_key) = crypto::generate_keypair();
    (StatusCode::CREATED, Json(WalletResponse { address, secret_key }))
}

pub async fn get_balance(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<BalanceResponse>, ApiError> {
    if !crypto::is_valid_address(&address) {
        return Err(ChainError::InvalidAddress(address).into());
    }
    let chain = state.chain.lock().unwrap();
    Ok(Json(BalanceResponse {
        balance: chain.balance(&address),
        next_nonce: chain.next_nonce(&address),
        address,
    }))
}

/// Signs a transfer without submitting it. A convenience for local experiments.
pub async fn sign_transaction(
    State(state): State<AppState>,
    Json(req): Json<SignRequest>,
) -> Result<Json<Transaction>, ApiError> {
    let key = crypto::parse_secret_key(&req.secret_key)
        .ok_or_else(|| ApiError::bad_request("secret_key must be 32 bytes of hex"))?;
    let from = crypto::address_of(&key);
    let nonce = match req.nonce {
        Some(n) => n,
        None => state.chain.lock().unwrap().next_nonce(&from),
    };
    let payload = Transaction::signing_payload(&from, &req.to, req.amount, nonce);
    Ok(Json(Transaction {
        signature: crypto::sign(&key, &payload),
        from,
        to: req.to,
        amount: req.amount,
        nonce,
    }))
}

pub async fn submit_transaction(
    State(state): State<AppState>,
    Json(tx): Json<Transaction>,
) -> Result<(StatusCode, Json<Transaction>), ApiError> {
    state.chain.lock().unwrap().submit(tx.clone())?;
    Ok((StatusCode::ACCEPTED, Json(tx)))
}

pub async fn get_pending(State(state): State<AppState>) -> Json<Vec<Transaction>> {
    Json(state.chain.lock().unwrap().pending().to_vec())
}

pub async fn mine(
    State(state): State<AppState>,
    Json(req): Json<MineRequest>,
) -> Result<(StatusCode, Json<Block>), ApiError> {
    let chain = state.chain.clone();
    let block = tokio::task::spawn_blocking(move || {
        chain.lock().unwrap().mine(&req.miner).cloned()
    })
    .await
    .expect("mining task panicked")?;
    Ok((StatusCode::CREATED, Json(block)))
}
