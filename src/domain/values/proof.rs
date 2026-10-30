//! The steps of a Merkle proof: the sibling hash at each level from a transaction's leaf up to
//! its block's root, and which side it sits on.

use serde::{Deserialize, Serialize};

use super::MerkleNode;

/// Which side of the hash so far a proof's sibling sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

/// One level of a proof: the hash to combine with, and on which side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub hash: MerkleNode,
    pub side: Side,
}
