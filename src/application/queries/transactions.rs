//! Questions about transfers: where one is, an address's history, a proof that one is in its
//! block, and what's waiting.

use chrono::{DateTime, Utc};

use super::ChainQueries;
use crate::domain::address::Address;
use crate::domain::block::{Block, BlockHeader, Transaction};
use crate::domain::chain::Blockchain;
use crate::domain::merkle;
use crate::domain::values::BlockHash;

/// Evidence that a transaction is in a block: its leaf, the sibling hashes up to the block's
/// Merkle root, and the block's header, whose hash covers the root.
pub struct TransactionProof {
    pub transaction: Transaction,
    pub leaf: String,
    pub steps: Vec<merkle::Step>,
    pub header: BlockHeader,
    /// Blocks on top of it, counting its own.
    pub confirmations: u64,
}

/// Where a transaction is: in a block, or still waiting in the mempool.
pub struct Located {
    pub transaction: Transaction,
    /// The block holding it and its place there. `None` while it's pending.
    pub block: Option<Placement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub height: u64,
    pub hash: BlockHash,
    pub position: usize,
    pub timestamp: DateTime<Utc>,
    /// Blocks on top of it, counting its own.
    pub confirmations: u64,
}

impl ChainQueries {
    /// Finds the transfer signed `signature`, confirmed or pending.
    pub fn locate(&self, signature: &str) -> Option<Located> {
        let chain = self.chain.lock().unwrap();
        if let Some((block, position)) = find_transaction(chain.blocks(), signature) {
            return Some(Located {
                transaction: block.transactions()[position].clone(),
                block: Some(placement(&chain, block, position)),
            });
        }
        let mempool = self.mempool.lock().unwrap();
        let pending = mempool.find(signature)?;
        Some(Located { transaction: pending.clone(), block: None })
    }

    /// Every transaction to or from `address`, block rewards included: pending ones first, then
    /// confirmed ones, newest first.
    pub fn history(&self, address: &Address) -> Vec<Located> {
        let chain = self.chain.lock().unwrap();
        let mempool = self.mempool.lock().unwrap();
        let pending =
            mempool.involving(address).map(|tx| Located { transaction: tx.clone(), block: None });
        let confirmed = history_of(chain.blocks(), address).map(|(block, position)| Located {
            transaction: block.transactions()[position].clone(),
            block: Some(placement(&chain, block, position)),
        });
        pending.chain(confirmed).collect()
    }

    /// Proves that the confirmed transfer signed `signature` is in its block, or `None` if no
    /// block holds it.
    pub fn prove(&self, signature: &str) -> Option<TransactionProof> {
        let chain = self.chain.lock().unwrap();
        let (block, position) = find_transaction(chain.blocks(), signature)?;
        let transaction = block.transactions()[position].clone();
        Some(TransactionProof {
            leaf: merkle::leaf(&transaction),
            steps: merkle::proof(block.transactions(), position)?,
            header: block.header(),
            confirmations: chain.tip().index() - block.index() + 1,
            transaction,
        })
    }

    pub fn pending(&self) -> Vec<Transaction> {
        self.mempool.lock().unwrap().transactions().to_vec()
    }
}

/// Confirmed transactions to or from `address`, block rewards included, newest first: each with
/// its block and its place there.
fn history_of<'a>(
    blocks: &'a [Block],
    address: &'a Address,
) -> impl Iterator<Item = (&'a Block, usize)> {
    blocks.iter().rev().flat_map(move |block| {
        let positions = block.transactions().iter().enumerate();
        positions.filter(|(_, tx)| tx.involves(address)).map(move |(position, _)| (block, position))
    })
}

/// The confirmed block holding the transfer signed `signature`, and its place in the block.
fn find_transaction<'a>(blocks: &'a [Block], signature: &str) -> Option<(&'a Block, usize)> {
    if signature.is_empty() {
        return None;
    }
    blocks.iter().rev().find_map(|block| {
        let signed = |tx: &Transaction| tx.signature().is_some_and(|s| s.as_str() == signature);
        Some((block, block.transactions().iter().position(signed)?))
    })
}

fn placement(chain: &Blockchain, block: &Block, position: usize) -> Placement {
    Placement {
        height: block.index(),
        hash: block.hash().clone(),
        position,
        timestamp: block.timestamp(),
        confirmations: chain.tip().index() - block.index() + 1,
    }
}
