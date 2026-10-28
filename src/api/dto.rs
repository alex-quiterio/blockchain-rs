//! Request and response bodies. Kept apart from the domain so the wire format can change alone.

use chrono::{DateTime, Utc};
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
    /// Optional tip for whoever adds the block.
    #[serde(default)]
    pub fee: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ForgeRequest {
    /// The validator's key, which signs the block. Only for local experiments.
    pub secret_key: String,
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

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub message: String,
}
