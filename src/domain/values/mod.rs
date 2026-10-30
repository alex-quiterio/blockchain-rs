//! Value objects: the domain's own kinds of value, each checked when it's made, so the rest of
//! the domain never handles a malformed one or mixes one kind up with another.
//!
//! The one module grouped by building block, on purpose: these are the shared vocabulary every
//! other module is built from, coins, addresses, hashes and signatures, too basic to belong to any
//! one of them.

pub mod address;
mod capacity;
mod coins;
mod error;
mod hash;
mod party;
mod proof;
pub mod signature;

pub use capacity::BlockCapacity;
pub use coins::Coins;
pub use error::ValueError;
pub use hash::{Beacon, BlockHash, Commitment, MerkleNode, MerkleRoot, Reveal};
pub use party::{Destination, Source, COINBASE, STAKE, UNSTAKE};
pub use proof::{Side, Step};
pub use signature::Signature;
