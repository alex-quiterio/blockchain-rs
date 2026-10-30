//! The hashes a block is identified and proven by: its own, its Merkle root, and the nodes of its
//! Merkle tree. Value objects, each checked when it's made, and each its own kind.

use crate::domain::crypto::sha256_type;

sha256_type!(
    /// A block's hash: the SHA-256 of its header. Blocks name the one before them by it.
    BlockHash
);

sha256_type!(
    /// The root of the Merkle tree over a block's transactions, which its header carries.
    MerkleRoot
);

sha256_type!(
    /// A hash inside a Merkle tree: a transaction's leaf, or two nodes below it hashed together.
    MerkleNode
);
