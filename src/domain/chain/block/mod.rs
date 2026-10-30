//! The chain aggregate's entity: a block itself (here), how one is made (`sealing`) and its
//! header (`header`). The transactions it carries are shared with the mempool, so they live in
//! `accounting::transaction`.

mod error;
mod hash;
mod header;
pub mod merkle;
mod proof;
mod sealing;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::consensus::Reveal;
use crate::domain::crypto::address::Address;
use crate::domain::crypto::Signature;

use crate::domain::accounting::transaction::Transaction;
pub use error::BlockError;
pub use hash::{BlockHash, MerkleNode, MerkleRoot};
use header::header_hash;
pub use header::BlockHeader;
pub use proof::{Side, Step};

/// A block of transactions. Only the domain changes one; everywhere else it can only be read:
///
/// ```
/// # fn read(block: &blockchain::domain::chain::block::Block) -> u64 {
/// block.nonce()
/// # }
/// ```
///
/// ```compile_fail
/// # fn tamper(block: &mut blockchain::domain::chain::block::Block) {
/// block.nonce += 1;
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Block {
    pub(in crate::domain) index: u64,
    pub(in crate::domain) timestamp: DateTime<Utc>,
    pub(in crate::domain) prev_hash: BlockHash,
    pub(in crate::domain) transactions: Vec<Transaction>,
    /// The root of the Merkle tree over `transactions`. The hash covers it instead of the
    /// transactions, so a transaction can be proven to be in the block from its header alone.
    pub(in crate::domain) merkle_root: MerkleRoot,
    /// Proof-of-work counter. Always 0 under proof of stake.
    pub(in crate::domain) nonce: u64,
    pub(in crate::domain) hash: BlockHash,
    /// Proof of stake only: who forged the block. Left out of proof-of-work blocks entirely, so
    /// their JSON and hashes are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub(in crate::domain) validator: Option<Address>,
    /// Proof of stake only: the next link of the validator's reveal chain, mixed into the beacon
    /// that picks who forges next. Absent while nothing is staked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub(in crate::domain) reveal: Option<Reveal>,
    /// Proof of stake only: the validator's signature over `hash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub(in crate::domain) signature: Option<Signature>,
}

impl Block {
    // Read access for code outside the domain, which may look at a block but not change it.

    pub fn index(&self) -> u64 {
        self.index
    }

    pub fn timestamp(&self) -> DateTime<Utc> {
        self.timestamp
    }

    pub fn prev_hash(&self) -> &BlockHash {
        &self.prev_hash
    }

    pub fn transactions(&self) -> &[Transaction] {
        &self.transactions
    }

    pub fn merkle_root(&self) -> &MerkleRoot {
        &self.merkle_root
    }

    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    pub fn hash(&self) -> &BlockHash {
        &self.hash
    }

    /// Proof of stake only.
    pub fn validator(&self) -> Option<&Address> {
        self.validator.as_ref()
    }

    /// Proof of stake only.
    pub fn reveal(&self) -> Option<&Reveal> {
        self.reveal.as_ref()
    }

    /// Proof of stake only.
    pub fn signature(&self) -> Option<&Signature> {
        self.signature.as_ref()
    }

    /// The hash of the header: every field except `transactions`, which `merkle_root` stands
    /// for, `hash` and `signature`.
    pub fn computed_hash(&self) -> BlockHash {
        header_hash(
            self.index,
            &self.timestamp,
            &self.prev_hash,
            &self.merkle_root,
            self.nonce,
            [
                self.validator.as_ref().map(Address::as_str),
                self.reveal.as_ref().map(Reveal::as_str),
            ],
        )
    }

    /// Whether `merkle_root` is the root of the transactions the block carries.
    pub fn has_matching_root(&self) -> bool {
        self.merkle_root == merkle::root(&self.transactions)
    }

    /// Whether `signature` is the named validator's signature over `hash`.
    pub fn is_signed_by_validator(&self) -> bool {
        match (&self.validator, &self.signature) {
            (Some(validator), Some(signature)) => {
                signature.is_by(validator, self.hash.as_str().as_bytes())
            }
            _ => false,
        }
    }
}
