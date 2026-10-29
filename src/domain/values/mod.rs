//! Value objects: the domain's own kinds of value, each checked when it's made, so the rest of
//! the domain never handles a malformed one or mixes one kind up with another.

pub mod address;
mod coins;
mod hash;
mod party;
pub mod signature;

pub use coins::Coins;
pub use hash::{Beacon, BlockHash, Commitment, MerkleRoot, Reveal};
pub use party::{Destination, Source, COINBASE, STAKE, UNSTAKE};
pub use signature::Signature;
