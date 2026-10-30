//! Keypairs and what an address holds.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use super::error::ApiError;
use crate::api::dto::wallets::{BalanceResponse, WalletResponse};
use crate::api::AppState;
use crate::domain::address::Address;
use crate::domain::crypto;

pub async fn create_wallet() -> (StatusCode, Json<WalletResponse>) {
    let (address, secret_key) = crypto::generate_keypair();
    (StatusCode::CREATED, Json(WalletResponse { address, secret_key }))
}

pub async fn get_balance(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<BalanceResponse>, ApiError> {
    let account = state.queries.account(&Address::parse(&address)?);
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
