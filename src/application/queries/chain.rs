//! Questions about the chain itself: its blocks and headers, where its tip stands, whether it
//! replays, and the work each block needed.

use super::ChainQueries;
use crate::domain::accounting::ChainId;
use crate::domain::chain::block::BlockHash;
use crate::domain::chain::block::{owned, Block, BlockHeader};
use crate::domain::chain::{ChainError, HeaderVerdict};
use crate::domain::consensus::weight::ChainWeight;

pub struct TipStatus {
    /// The chain every transfer must name.
    pub chain: ChainId,
    pub height: u64,
    pub hash: BlockHash,
    pub weight: ChainWeight,
    pub pending: usize,
}

pub struct ChainStatus {
    /// Which block first breaks a rule, and how, or `None` if the whole chain replays.
    pub problem: Option<ChainError>,
    pub length: usize,
    pub weight: ChainWeight,
}

impl ChainQueries {
    pub fn blocks(&self) -> Vec<Block> {
        owned(self.chain.lock().unwrap().blocks())
    }

    /// The blocks from `height` on, for clients catching up.
    pub fn blocks_from(&self, height: usize) -> Vec<Block> {
        let chain = self.chain.lock().unwrap();
        owned(chain.blocks().get(height..).unwrap_or_default())
    }

    /// Up to `limit` blocks from `height` on, without copying the rest.
    pub fn blocks_between(&self, height: usize, limit: usize) -> Vec<Block> {
        let chain = self.chain.lock().unwrap();
        let blocks = chain.blocks().get(height..).unwrap_or_default();
        owned(&blocks[..limit.min(blocks.len())])
    }

    /// Up to `limit` headers from `height` on: everything a light client needs to follow the
    /// chain, and what a peer checks before fetching our blocks.
    pub fn headers_between(&self, height: usize, limit: usize) -> Vec<BlockHeader> {
        let chain = self.chain.lock().unwrap();
        let blocks = chain.blocks().get(height..).unwrap_or_default();
        blocks.iter().take(limit).map(|block| block.header()).collect()
    }

    /// Whether a peer's `headers` make a chain worth fetching, and from where. Weighed on a
    /// copy, which shares the blocks, so the chain isn't held while they're checked.
    pub fn weigh_headers(&self, headers: &[BlockHeader]) -> Result<HeaderVerdict, ChainError> {
        let chain = self.chain.lock().unwrap().clone();
        chain.weigh_headers(headers)
    }

    /// The block at a height, given as digits, or with a hash.
    pub fn block(&self, id: &str) -> Option<Block> {
        let chain = self.chain.lock().unwrap();
        let blocks = chain.blocks();
        match id.parse::<usize>() {
            Ok(height) => blocks.get(height),
            Err(_) => blocks.iter().rev().find(|block| block.hash().as_str() == id),
        }
        .map(|block| Block::clone(block))
    }

    /// Where the chain stands, without replaying it: cheap enough to poll every second.
    pub fn tip(&self) -> TipStatus {
        let chain = self.chain.lock().unwrap();
        TipStatus {
            chain: chain.ledger().chain().clone(),
            height: chain.blocks().len() as u64 - 1,
            hash: chain.tip().hash().clone(),
            weight: chain.weight(),
            pending: self.mempool.lock().unwrap().transactions().len(),
        }
    }

    /// The chain every transfer must name: its genesis hash.
    pub fn chain_id(&self) -> ChainId {
        self.chain.lock().unwrap().ledger().chain().clone()
    }

    /// Whether the chain replays cleanly, how long it is and what it weighs in a fork. The replay
    /// runs on a copy, which shares the blocks, so the chain isn't held while it's checked.
    pub fn status(&self) -> ChainStatus {
        let chain = self.chain.lock().unwrap().clone();
        ChainStatus {
            problem: chain.validate().err(),
            length: chain.blocks().len(),
            weight: chain.weight(),
        }
    }

    /// Proof of work: the work each block required, by height.
    pub fn work_history(&self) -> Vec<u128> {
        self.chain.lock().unwrap().work_history()
    }
}
