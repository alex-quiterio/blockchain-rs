//! Request and response bodies. Kept apart from the domain so the wire format can change alone.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct WalletResponse {
    pub address: String,
    /// Only for local experiments: a real wallet never sends its key over the wire.
    pub secret_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SignRequest {
    pub secret_key: String,
    pub to: String,
    pub amount: u64,
    /// Defaults to the sender's next nonce.
    pub nonce: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MineRequest {
    pub miner: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BalanceResponse {
    pub address: String,
    pub balance: u64,
    pub next_nonce: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidityResponse {
    pub valid: bool,
    pub length: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReplaceResponse {
    /// Whether the offered chain was adopted.
    pub replaced: bool,
    /// This node's chain length afterwards.
    pub length: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub message: String,
}
