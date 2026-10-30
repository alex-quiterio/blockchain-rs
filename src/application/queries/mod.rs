//! The read side of a node: what the chain holds and what it adds up to, for anyone who asks.
//! Nothing here changes the chain; `NodeService` makes every change. One file per kind of
//! question: the chain, transactions, accounts, fees and consensus.

mod accounts;
mod chain;
mod consensus;
mod fees;
mod transactions;

use std::sync::{Arc, Mutex};

use crate::domain::chain::Blockchain;
use crate::domain::mempool::Mempool;

pub use accounts::Account;
pub use chain::{ChainStatus, TipStatus};
pub use consensus::ConsensusState;
pub use fees::{BlockFees, FeeReport, RECENT_BLOCKS};
pub use transactions::{Located, Placement, TransactionProof};

/// Answers questions about a node's chain. Cheap to clone: every clone reads the same chain.
#[derive(Clone)]
pub struct ChainQueries {
    chain: Arc<Mutex<Blockchain>>,
    mempool: Arc<Mutex<Mempool>>,
}

impl ChainQueries {
    /// Where both are read, the chain is locked before the mempool, as everywhere in a node.
    pub(super) fn new(chain: Arc<Mutex<Blockchain>>, mempool: Arc<Mutex<Mempool>>) -> Self {
        Self { chain, mempool }
    }
}
