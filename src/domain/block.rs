use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::address::Address;
use super::crypto;
use super::rules::ChainRules;

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

    /// The reward for mining block `height`. Its nonce is the height, so every one is unique.
    pub fn coinbase(miner: &Address, reward: u64, height: u64) -> Self {
        Self {
            from: COINBASE.to_string(),
            to: miner.to_string(),
            amount: reward,
            nonce: height,
            signature: String::new(),
        }
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

impl Block {
    /// The first block. Its timestamp is fixed, so every node with the same rules has the same one.
    pub fn genesis(rules: &ChainRules) -> Self {
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is a valid time");
        Self::mine(0, epoch, "0".repeat(64), Vec::new(), rules)
    }

    /// The proof of work: tries nonces from zero until the hash meets the difficulty.
    pub fn mine(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: String,
        transactions: Vec<Transaction>,
        rules: &ChainRules,
    ) -> Self {
        let mut block =
            Self { index, timestamp, prev_hash, transactions, nonce: 0, hash: String::new() };
        loop {
            block.hash = block.computed_hash();
            if rules.is_met_by(&block.hash) {
                return block;
            }
            block.nonce += 1;
        }
    }

    /// The hash of every field except `hash` itself.
    pub fn computed_hash(&self) -> String {
        let txs = serde_json::to_string(&self.transactions).expect("transactions serialize");
        let ts = self.timestamp.to_rfc3339_opts(SecondsFormat::Nanos, true);
        let (index, prev, nonce) = (self.index, &self.prev_hash, self.nonce);
        crypto::sha256_hex(format!("{index}|{ts}|{prev}|{txs}|{nonce}").as_bytes())
    }
}
