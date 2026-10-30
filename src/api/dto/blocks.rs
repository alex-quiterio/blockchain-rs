//! Bodies for mining, forging, and who may add the next block.

use serde::{Deserialize, Serialize};

use crate::domain::accounting::Coins;
use crate::domain::consensus::Reveal;
use crate::domain::crypto::address::Address;

#[derive(Debug, Serialize, Deserialize)]
pub struct MineRequest {
    pub miner: String,
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
    pub reveal: Option<Reveal>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConsensusResponse {
    /// `proof_of_work` or `proof_of_stake`.
    pub kind: String,
    /// Proof of work only.
    pub difficulty: Option<usize>,
    pub reward: Coins,
    /// The most transfers a block may hold. `null` means no limit.
    pub max_block_transfers: Option<u64>,
    /// Proof of stake only: who must forge the next block. `null` means anyone may.
    pub next_validator: Option<String>,
    pub stakes: Vec<StakeEntry>,
    pub total_stake: Coins,
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
pub struct StakeEntry {
    pub address: Address,
    pub amount: Coins,
}
