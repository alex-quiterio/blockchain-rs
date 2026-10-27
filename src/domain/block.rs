use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// Value objects of the domain. They derive serde because their JSON form is also how
// blocks are hashed, stored and exchanged between nodes.

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
