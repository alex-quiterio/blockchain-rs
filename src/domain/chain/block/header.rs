//! A block without its transactions, and the hash its header and the block share.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::Block;
use crate::domain::address::Address;
use crate::domain::chain::block::{BlockHash, MerkleRoot};
use crate::domain::consensus::Reveal;
use crate::domain::crypto::Signature;

impl Block {
    /// The block without its transactions.
    pub fn header(&self) -> BlockHeader {
        BlockHeader {
            index: self.index,
            timestamp: self.timestamp,
            prev_hash: self.prev_hash.clone(),
            merkle_root: self.merkle_root.clone(),
            nonce: self.nonce,
            hash: self.hash.clone(),
            validator: self.validator.clone(),
            reveal: self.reveal.clone(),
            signature: self.signature.clone(),
        }
    }
}

/// A block without its transactions: enough to check its hash, its proof of work or signature,
/// and Merkle proofs of what it holds. What a light client keeps instead of whole blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    pub index: u64,
    pub timestamp: DateTime<Utc>,
    pub prev_hash: BlockHash,
    pub merkle_root: MerkleRoot,
    pub nonce: u64,
    pub hash: BlockHash,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator: Option<Address>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal: Option<Reveal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

impl BlockHeader {
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
}
/// `index|timestamp|prev_hash|merkle_root|nonce`, then the validator and reveal if present.
pub(super) fn header_hash(
    index: u64,
    timestamp: &DateTime<Utc>,
    prev_hash: &BlockHash,
    merkle_root: &MerkleRoot,
    nonce: u64,
    extras: [Option<&str>; 2],
) -> BlockHash {
    let ts = timestamp.to_rfc3339_opts(SecondsFormat::Nanos, true);
    let mut fields = format!("{index}|{ts}|{prev_hash}|{merkle_root}|{nonce}");
    for extra in extras.into_iter().flatten() {
        fields.push('|');
        fields.push_str(extra);
    }
    BlockHash::of(fields.as_bytes())
}
