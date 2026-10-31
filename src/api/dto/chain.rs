//! Bodies for reading the chain and offering this node another's.

use serde::{Deserialize, Serialize};

use crate::domain::accounting::ChainId;
use crate::domain::chain::block::BlockHash;

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ChainQuery {
    /// Only return blocks from this height on.
    pub from: Option<usize>,
    /// Return at most this many.
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
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
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct StatusResponse {
    /// The chain every transfer must name, so it can't be replayed on another: the genesis hash.
    pub chain: ChainId,
    pub height: u64,
    pub tip: BlockHash,
    /// A string, since it can outgrow a JSON number.
    pub weight: String,
    /// Transactions waiting in the mempool.
    pub pending: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ValidityResponse {
    pub valid: bool,
    /// Which block first breaks a rule, and how. `null` while the chain is valid.
    pub problem: Option<String>,
    pub length: usize,
    /// Total work or stake behind the chain. A string, since it can outgrow a JSON number.
    pub weight: String,
}
