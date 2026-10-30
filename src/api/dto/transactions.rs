//! Bodies for signing, submitting, finding and proving transfers, and for fees.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::accounting::transaction::Transaction;
use crate::domain::accounting::Coins;
use crate::domain::chain::block::BlockHeader;
use crate::domain::chain::block::{BlockHash, MerkleNode, Step};
use crate::domain::consensus::Commitment;

#[derive(Debug, Serialize, Deserialize)]
pub struct SignRequest {
    pub secret_key: String,
    pub to: String,
    pub amount: Coins,
    /// Defaults to the sender's next nonce.
    pub nonce: Option<u64>,
    /// Optional tip for whoever adds the block.
    #[serde(default)]
    pub fee: Coins,
    /// For a first stake. Worked out from `secret_key` when left out.
    pub commitment: Option<Commitment>,
}

/// A transaction and where it is.
#[derive(Debug, Serialize, Deserialize)]
pub struct TransactionResponse {
    pub transaction: Transaction,
    /// `confirmed` or `pending`.
    pub status: String,
    /// The block holding it. `null` while it's pending.
    pub block: Option<PlacementResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlacementResponse {
    pub height: u64,
    pub hash: BlockHash,
    /// Its place among the block's transactions. 0 is the block reward.
    pub position: usize,
    pub timestamp: DateTime<Utc>,
    /// Blocks on top of it, counting its own.
    pub confirmations: u64,
}

/// Evidence that a confirmed transaction is in its block. To check it: hash `transaction` into
/// `leaf`, combine it with each step's hash on the step's side up to `header.merkle_root`, and
/// check that `header.hash` is the hash of the header and belongs to a chain you trust.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProofResponse {
    pub transaction: Transaction,
    pub leaf: MerkleNode,
    pub proof: Vec<Step>,
    pub header: BlockHeader,
    /// Blocks on top of it, counting its own.
    pub confirmations: u64,
}

/// What fee to pay: `fee` is sure to win a place in the next block, if nothing else arrives
/// first; `recent` is what the last few blocks took, newest first.
#[derive(Debug, Serialize, Deserialize)]
pub struct FeesResponse {
    pub fee: Coins,
    /// Places left in the next block. While there are any, no fee is needed.
    pub room: Option<usize>,
    /// The most transfers a block may hold. `null` means no limit.
    pub capacity: Option<u64>,
    pub waiting: usize,
    pub recent: Vec<BlockFeesEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BlockFeesEntry {
    pub height: u64,
    pub transfers: usize,
    pub full: bool,
    pub lowest_fee: Option<Coins>,
}
