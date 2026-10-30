//! Questions about fees: what's sure to get a transfer into the next block, and what recent
//! blocks took.

use super::ChainQueries;
use crate::domain::block::Transaction;
use crate::domain::mempool::FeeEstimate;
use crate::domain::values::{BlockCapacity, Coins};

/// What fee to pay: what's sure to win a place in the next block now, and what recent blocks
/// took.
pub struct FeeReport {
    pub capacity: BlockCapacity,
    /// Transfers waiting for a block.
    pub waiting: usize,
    pub next_block: FeeEstimate,
    /// The last few blocks, newest first.
    pub recent: Vec<BlockFees>,
}

/// What a block took, in fees.
pub struct BlockFees {
    pub height: u64,
    pub transfers: usize,
    /// It held as many transfers as a block may, so fees decided what got in.
    pub full: bool,
    /// The lowest fee among its transfers, if it had any.
    pub lowest_fee: Option<Coins>,
}

/// How many recent blocks a fee report looks back over.
pub const RECENT_BLOCKS: usize = 10;

impl ChainQueries {
    /// What fee to pay to get into the next block, and what the last `RECENT_BLOCKS` took.
    pub fn fees(&self) -> FeeReport {
        let (capacity, recent) = {
            let chain = self.chain.lock().unwrap();
            let capacity = chain.rules().block_capacity;
            let recent = chain.blocks()[1..]
                .iter()
                .rev()
                .take(RECENT_BLOCKS)
                .map(|block| {
                    let transfers = &block.transactions()[1..];
                    BlockFees {
                        height: block.index(),
                        transfers: transfers.len(),
                        full: !capacity.holds(transfers.len() + 1),
                        lowest_fee: transfers.iter().map(Transaction::fee).min(),
                    }
                })
                .collect();
            (capacity, recent)
        };
        let mempool = self.mempool.lock().unwrap();
        FeeReport {
            capacity,
            waiting: mempool.transactions().len(),
            next_block: mempool.fee_estimate(capacity),
            recent,
        }
    }
}
