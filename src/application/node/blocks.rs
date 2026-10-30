//! Changing the chain: mining and forging a block from what's waiting, and taking another
//! node's blocks.

use std::sync::atomic::{AtomicBool, Ordering};

use ed25519_dalek::SigningKey;

use super::{NodeError, NodeService};
use crate::domain::address::Address;
use crate::domain::block::{Block, Transaction};
use crate::domain::chain::block::BlockError;
use crate::domain::chain::ChainError;
use crate::domain::chain::Extension;

impl NodeService {
    /// What the mempool offers for a new block, as many as one holds.
    fn candidates(&self) -> Vec<Transaction> {
        let capacity = self.chain.lock().unwrap().rules().block_capacity;
        self.mempool.lock().unwrap().candidates(capacity)
    }

    /// Appends a proof-of-stake block signed elsewhere and saves it.
    pub fn append_forged(&self, block: Block) -> Result<Block, NodeError> {
        self.commit(|chain| chain.append_forged(block).cloned())
    }

    /// Forges a proof-of-stake block signed with `key` and saves it.
    pub fn forge(&self, key: &SigningKey) -> Result<Block, NodeError> {
        let pending = self.candidates();
        self.commit(|chain| chain.forge(key, &pending).cloned())
    }

    /// Mines a block and saves it. The chain stays free while the proof of work is searched for,
    /// so the node keeps answering and taking peers' blocks. If one of those takes the tip first,
    /// the race is lost and mining stops with `StaleBlock`.
    pub fn mine(&self, miner: &Address) -> Result<Block, NodeError> {
        self.mine_unless(miner, &AtomicBool::new(false))
    }

    /// Like `mine`, but also stops with `StaleBlock` once `cancelled` is set, say because whoever
    /// asked for the block has stopped waiting for it.
    pub fn mine_unless(&self, miner: &Address, cancelled: &AtomicBool) -> Result<Block, NodeError> {
        let pending = self.candidates();
        let template = self.chain.lock().unwrap().mining_template(miner, &pending)?;
        let tip = template.prev_hash.clone();
        let keep_going = || {
            !cancelled.load(Ordering::Relaxed) && *self.chain.lock().unwrap().tip().hash() == tip
        };
        let block = template.mine_while(keep_going).ok_or(BlockError::StaleBlock)?;
        self.commit(|chain| chain.append_mined(block).cloned())
    }

    /// Offers another node's chain. Returns whether it was adopted and the length afterwards.
    pub fn adopt_if_heavier(&self, blocks: Vec<Block>) -> Result<(bool, usize), NodeError> {
        self.commit(|chain| {
            let adopted = chain.adopt_if_heavier(blocks)?;
            Ok::<_, ChainError>((adopted, chain.blocks().len()))
        })
    }

    /// Offers another node's blocks from some height on. Returns what came of it and the length
    /// afterwards.
    pub fn extend(&self, blocks: Vec<Block>) -> Result<(Extension, usize), NodeError> {
        self.commit(|chain| {
            let extension = chain.extend(blocks)?;
            Ok::<_, ChainError>((extension, chain.blocks().len()))
        })
    }
}
