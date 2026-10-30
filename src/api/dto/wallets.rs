//! Bodies for keypairs and what an address holds.

use serde::{Deserialize, Serialize};

use crate::domain::values::Coins;

#[derive(Debug, Serialize, Deserialize)]
pub struct WalletResponse {
    pub address: String,
    /// Only for local experiments: a real wallet never sends its key over the wire.
    pub secret_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BalanceResponse {
    pub address: String,
    pub balance: Coins,
    pub staked: Coins,
    pub next_nonce: u64,
    /// The epoch this address's next commitment would be, and so the one after its active one.
    pub next_epoch: u64,
    /// Reveals left on its active commitment, if it has one.
    pub reveals_left: Option<u32>,
    /// If its next stake must carry a commitment, the epoch to make it for.
    pub commitment_due: Option<u64>,
}
