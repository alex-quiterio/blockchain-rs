//! Questions about who adds the next block: the rules, whose turn it is, and the block a
//! validator should sign.

use super::ChainQueries;
use crate::domain::accounting::Coins;
use crate::domain::chain::block::Block;
use crate::domain::chain::block::BlockError;
use crate::domain::chain::ReorgLimit;
use crate::domain::consensus::rules::ChainRules;
use crate::domain::consensus::Reveal;
use crate::domain::crypto::address::Address;

/// How the chain picks its next block, as of now.
pub struct ConsensusState {
    pub rules: ChainRules,
    /// Under proof of stake, who must forge next. `None` means anyone may.
    pub next_validator: Option<Address>,
    pub stakes: Vec<(Address, Coins)>,
    /// The randomness the next validator is drawn from.
    pub beacon: String,
    /// Proof of work: the hashes the next block should take to find, on average.
    pub next_work: u128,
    /// Proof of work with retargeting: the next height where the work is adjusted.
    pub next_retarget: Option<u64>,
}

impl ChainQueries {
    pub fn consensus(&self) -> ConsensusState {
        let chain = self.chain.lock().unwrap();
        ConsensusState {
            rules: chain.rules(),
            next_validator: chain.next_validator(),
            stakes: chain.stakes(),
            beacon: chain.beacon().to_string(),
            next_work: chain.next_work(),
            next_retarget: chain.next_retarget_height(),
        }
    }

    pub fn rules(&self) -> ChainRules {
        self.chain.lock().unwrap().rules()
    }

    /// How far back this node will follow a fork.
    pub fn reorg_limit(&self) -> ReorgLimit {
        self.chain.lock().unwrap().reorg_limit()
    }

    /// The unsigned block `validator` should sign next. Changes nothing.
    pub fn propose(
        &self,
        validator: &Address,
        reveal: Option<Reveal>,
    ) -> Result<Block, BlockError> {
        let capacity = self.rules().block_capacity;
        let pending = self.mempool.lock().unwrap().candidates(capacity);
        self.chain.lock().unwrap().propose(validator, reveal, &pending)
    }
}
