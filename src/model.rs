use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Sender used for the block reward, which has no signature.
pub const COINBASE: &str = "coinbase";

/// A transfer of coins. Addresses are hex-encoded ed25519 public keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub from: String,
    pub to: String,
    pub amount: u64,
    /// How many transactions the sender has made before this one. Stops replays.
    pub nonce: u64,
    /// Hex-encoded ed25519 signature over `signing_payload()`. Empty for coinbase.
    pub signature: String,
}

impl Transaction {
    pub fn signing_payload(from: &str, to: &str, amount: u64, nonce: u64) -> Vec<u8> {
        format!("{from}|{to}|{amount}|{nonce}").into_bytes()
    }

    pub fn payload(&self) -> Vec<u8> {
        Self::signing_payload(&self.from, &self.to, self.amount, self.nonce)
    }

    pub fn is_coinbase(&self) -> bool {
        self.from == COINBASE
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub index: u64,
    pub timestamp: DateTime<Utc>,
    pub prev_hash: String,
    pub transactions: Vec<Transaction>,
    /// Proof-of-work counter.
    pub nonce: u64,
    pub hash: String,
}

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
pub struct ErrorResponse {
    pub message: String,
}
