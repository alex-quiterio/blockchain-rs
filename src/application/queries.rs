//! The read side of a node: what the chain holds and what it adds up to, for anyone who asks.
//! Nothing here changes the chain; `NodeService` makes every change.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use crate::domain::address::Address;
use crate::domain::block::{Block, BlockHeader, Transaction};
use crate::domain::chain::Blockchain;
use crate::domain::error::BlockError;
use crate::domain::ledger::ValidatorState;
use crate::domain::mempool::Mempool;
use crate::domain::merkle;
use crate::domain::rules::ChainRules;
use crate::domain::values::{BlockHash, Coins, Reveal};
use crate::domain::weight::ChainWeight;

pub struct TipStatus {
    pub height: u64,
    pub hash: BlockHash,
    pub weight: ChainWeight,
    pub pending: usize,
}

pub struct ChainStatus {
    pub valid: bool,
    pub length: usize,
    pub weight: ChainWeight,
}

pub struct Account {
    pub balance: Coins,
    pub staked: Coins,
    pub next_nonce: u64,
    pub validator: ValidatorState,
    /// If the next stake must commit, the epoch to commit for.
    pub commitment_due: Option<u64>,
}

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

    pub fn blocks(&self) -> Vec<Block> {
        self.chain.lock().unwrap().blocks().to_vec()
    }

    /// The blocks from `height` on, for clients catching up.
    pub fn blocks_from(&self, height: usize) -> Vec<Block> {
        let chain = self.chain.lock().unwrap();
        chain.blocks().get(height..).unwrap_or_default().to_vec()
    }

    /// Up to `limit` blocks from `height` on, without copying the rest.
    pub fn blocks_between(&self, height: usize, limit: usize) -> Vec<Block> {
        let chain = self.chain.lock().unwrap();
        let blocks = chain.blocks().get(height..).unwrap_or_default();
        blocks[..limit.min(blocks.len())].to_vec()
    }

    /// The headers from `height` on: everything a light client needs to follow the chain.
    pub fn headers_from(&self, height: usize) -> Vec<BlockHeader> {
        let chain = self.chain.lock().unwrap();
        chain.blocks().get(height..).unwrap_or_default().iter().map(Block::header).collect()
    }

    /// The block at a height, given as digits, or with a hash.
    pub fn block(&self, id: &str) -> Option<Block> {
        let chain = self.chain.lock().unwrap();
        let blocks = chain.blocks();
        match id.parse::<usize>() {
            Ok(height) => blocks.get(height).cloned(),
            Err(_) => blocks.iter().rev().find(|block| block.hash().as_str() == id).cloned(),
        }
    }

    /// Finds the transfer signed `signature`, confirmed or pending.
    pub fn locate(&self, signature: &str) -> Option<Located> {
        let chain = self.chain.lock().unwrap();
        if let Some((block, position)) = chain.find_transaction(signature) {
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
        let confirmed = chain.history_of(address).map(|(block, position)| Located {
            transaction: block.transactions()[position].clone(),
            block: Some(placement(&chain, block, position)),
        });
        pending.chain(confirmed).collect()
    }

    /// Proves that the confirmed transfer signed `signature` is in its block, or `None` if no
    /// block holds it.
    pub fn prove(&self, signature: &str) -> Option<TransactionProof> {
        let chain = self.chain.lock().unwrap();
        let (block, position) = chain.find_transaction(signature)?;
        let transaction = block.transactions()[position].clone();
        Some(TransactionProof {
            leaf: merkle::leaf(&transaction),
            steps: merkle::proof(block.transactions(), position)?,
            header: block.header(),
            confirmations: chain.tip().index() - block.index() + 1,
            transaction,
        })
    }

    /// Where the chain stands, without replaying it: cheap enough to poll every second.
    pub fn tip(&self) -> TipStatus {
        let chain = self.chain.lock().unwrap();
        TipStatus {
            height: chain.blocks().len() as u64 - 1,
            hash: chain.tip().hash().clone(),
            weight: chain.weight(),
            pending: self.mempool.lock().unwrap().transactions().len(),
        }
    }

    /// Whether the chain replays cleanly, how long it is and what it weighs in a fork.
    pub fn status(&self) -> ChainStatus {
        let chain = self.chain.lock().unwrap();
        ChainStatus {
            valid: chain.is_valid(),
            length: chain.blocks().len(),
            weight: chain.weight(),
        }
    }

    pub fn account(&self, address: &Address) -> Account {
        let chain = self.chain.lock().unwrap();
        let mempool = self.mempool.lock().unwrap();
        Account {
            balance: chain.balance(address),
            staked: chain.staked(address),
            next_nonce: mempool.next_nonce(address),
            validator: chain.validator_state(address),
            commitment_due: mempool.commitment_due(address),
        }
    }

    pub fn next_nonce(&self, address: &Address) -> u64 {
        self.mempool.lock().unwrap().next_nonce(address)
    }

    pub fn pending(&self) -> Vec<Transaction> {
        self.mempool.lock().unwrap().transactions().to_vec()
    }

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

    /// Proof of work: the work each block required, by height.
    pub fn work_history(&self) -> Vec<u128> {
        self.chain.lock().unwrap().work_history()
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

fn placement(chain: &Blockchain, block: &Block, position: usize) -> Placement {
    Placement {
        height: block.index(),
        hash: block.hash().clone(),
        position,
        timestamp: block.timestamp(),
        confirmations: chain.tip().index() - block.index() + 1,
    }
}
