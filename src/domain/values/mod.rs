//! Value objects: the domain's own kinds of value, each checked when it's made, so the rest of
//! the domain never handles a malformed one or mixes one kind up with another.

pub mod address;
mod capacity;
mod coins;
mod hash;
mod party;
mod proof;
pub mod signature;

pub use capacity::BlockCapacity;
pub use coins::Coins;
pub use hash::{Beacon, BlockHash, Commitment, MerkleNode, MerkleRoot, Reveal};
pub use party::{Destination, Source, COINBASE, STAKE, UNSTAKE};
pub use proof::{Side, Step};
pub use signature::Signature;
