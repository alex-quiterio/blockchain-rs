//! Request and response bodies. Kept apart from the domain so the wire format can change alone.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::block::{BlockHeader, Transaction};
use crate::domain::merkle::Step;

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
    /// Optional tip for whoever adds the block.
    #[serde(default)]
    pub fee: u64,
    /// For a first stake. Worked out from `secret_key` when left out.
    pub commitment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ForgeRequest {
    /// The validator's key, which signs the block. Only for local experiments.
    pub secret_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProposalRequest {
    /// The address that will sign the block.
    pub validator: String,
    /// The next link of the validator's reveal chain. Leave it out while anyone may forge.
    pub reveal: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StakeEntry {
    pub address: String,
    pub amount: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConsensusResponse {
    /// `proof_of_work` or `proof_of_stake`.
    pub kind: String,
    /// Proof of work only.
    pub difficulty: Option<usize>,
    pub reward: u64,
    /// Proof of stake only: who must forge the next block. `null` means anyone may.
    pub next_validator: Option<String>,
    pub stakes: Vec<StakeEntry>,
    pub total_stake: u64,
    /// Proof of stake only: the randomness the next validator is drawn from.
    pub beacon: Option<String>,
    /// Proof of work only: the hashes the next block should take to find, on average. A string,
    /// since it can outgrow a JSON number.
    pub work: Option<String>,
    /// Proof of work with retargeting: the pace the difficulty aims for.
    pub target_block_secs: Option<u64>,
    /// Proof of work with retargeting: blocks between adjustments.
    pub retarget_interval: Option<u64>,
    /// Proof of work with retargeting: the next height whose work is adjusted.
    pub next_retarget: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MineRequest {
    pub miner: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BalanceResponse {
    pub address: String,
    pub balance: u64,
    pub staked: u64,
    pub next_nonce: u64,
    /// The epoch this address's next commitment would be, and so the one after its active one.
    pub next_epoch: u64,
    /// Reveals left on its active commitment, if it has one.
    pub reveals_left: Option<u32>,
    /// If its next stake must carry a commitment, the epoch to make it for.
    pub commitment_due: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidityResponse {
    pub valid: bool,
    pub length: usize,
    /// Total work or stake behind the chain. A string, since it can outgrow a JSON number.
    pub weight: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReplaceResponse {
    /// Whether the offered blocks were adopted.
    pub replaced: bool,
    /// This node's chain length afterwards.
    pub length: usize,
    /// The offered blocks build on one this node doesn't have: send more from further back.
    #[serde(default)]
    pub missing_parent: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PeerRequest {
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PeerResponse {
    pub url: String,
    /// The peer's chain length when last reached.
    pub height: Option<usize>,
    pub last_seen: Option<DateTime<Utc>>,
    /// Why the last attempt to reach it failed, if it did.
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AddPeerResponse {
    pub url: String,
    pub added: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncResponse {
    pub adopted_from: Option<String>,
    pub reached: usize,
    pub length: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeResponse {
    pub version: String,
    /// The git commit the node was built from. `-dirty` means uncommitted changes.
    pub commit: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ChainQuery {
    /// Only return blocks from this height on.
    pub from: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub height: u64,
    pub tip: String,
    /// A string, since it can outgrow a JSON number.
    pub weight: String,
    /// Transactions waiting in the mempool.
    pub pending: usize,
}

/// Evidence that a confirmed transaction is in its block. To check it: hash `transaction` into
/// `leaf`, combine it with each step's hash on the step's side up to `header.merkle_root`, and
/// check that `header.hash` is the hash of the header and belongs to a chain you trust.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProofResponse {
    pub transaction: Transaction,
    pub leaf: String,
    pub proof: Vec<Step>,
    pub header: BlockHeader,
    /// Blocks on top of it, counting its own.
    pub confirmations: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub message: String,
}
