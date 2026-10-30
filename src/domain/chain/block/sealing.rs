//! How a block gets made: genesis, mined under proof of work, or forged under proof of stake,
//! all at once or proposed and then signed elsewhere.

use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use super::{merkle, Block, Transaction};
use crate::domain::chain::block::BlockHash;
use crate::domain::consensus::difficulty::meets_work;
use crate::domain::consensus::rules::{ChainRules, Consensus};
use crate::domain::consensus::Reveal;
use crate::domain::crypto::address::Address;
use crate::domain::crypto::Signature;

/// How many nonces a miner tries between asking whether to keep going.
const CHECK_EVERY: u64 = 1 << 14;

impl Block {
    /// The first block. Its timestamp is fixed, so every node with the same rules has the same one.
    pub fn genesis(rules: &ChainRules) -> Self {
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is a valid time");
        match rules.consensus {
            Consensus::ProofOfWork { .. } => {
                Self::mine(0, epoch, BlockHash::zero(), Vec::new(), rules.initial_work())
            }
            Consensus::ProofOfStake => {
                let mut block = Self::unsealed(0, epoch, BlockHash::zero(), Vec::new());
                block.hash = block.computed_hash();
                block
            }
        }
    }

    /// Proof of work: tries nonces from zero until the hash meets `work`, which takes `work`
    /// tries on average.
    pub fn mine(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: BlockHash,
        transactions: Vec<Transaction>,
        work: u128,
    ) -> Self {
        Self::mine_while(index, timestamp, prev_hash, transactions, work, || true)
            .expect("mining only stops when told to")
    }

    /// Like `mine`, but asks `keep_going` before the first try and every few thousand after, and
    /// gives up when it says no, for a miner whose block has gone stale.
    pub fn mine_while(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: BlockHash,
        transactions: Vec<Transaction>,
        work: u128,
        mut keep_going: impl FnMut() -> bool,
    ) -> Option<Self> {
        let mut block = Self::unsealed(index, timestamp, prev_hash, transactions);
        loop {
            if block.nonce.is_multiple_of(CHECK_EVERY) && !keep_going() {
                return None;
            }
            block.hash = block.computed_hash();
            if meets_work(block.hash.as_str(), work) {
                return Some(block);
            }
            block.nonce += 1;
        }
    }

    /// Proof of stake: names the validator and signs the hash with their key. No work needed.
    pub fn forge(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: BlockHash,
        transactions: Vec<Transaction>,
        key: &SigningKey,
        reveal: Option<Reveal>,
    ) -> Self {
        let validator = Address::of(key);
        let mut block =
            Self::propose(index, timestamp, prev_hash, transactions, &validator, reveal);
        block.sign(key);
        block
    }

    /// Proof of stake, step one: a block naming `validator`, hashed but not yet signed. Lets the
    /// validator sign somewhere else, such as in a browser, without handing over their key.
    pub fn propose(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: BlockHash,
        transactions: Vec<Transaction>,
        validator: &Address,
        reveal: Option<Reveal>,
    ) -> Self {
        let mut block = Self::unsealed(index, timestamp, prev_hash, transactions);
        block.validator = Some(validator.clone());
        block.reveal = reveal;
        block.hash = block.computed_hash();
        block
    }

    /// Proof of stake, step two: the validator signs the hash.
    pub fn sign(&mut self, key: &SigningKey) {
        self.signature = Some(Signature::sign(key, self.hash.as_str().as_bytes()));
    }

    fn unsealed(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: BlockHash,
        transactions: Vec<Transaction>,
    ) -> Self {
        Self {
            index,
            timestamp,
            prev_hash,
            merkle_root: merkle::root(&transactions),
            transactions,
            nonce: 0,
            hash: BlockHash::zero(),
            validator: None,
            reveal: None,
            signature: None,
        }
    }
}
