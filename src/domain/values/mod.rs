//! Value objects: the domain's own kinds of value, each checked when it's made, so the rest of
//! the domain never handles a malformed one or mixes one kind up with another.

pub mod address;
mod coins;
mod hash;
mod party;

pub use coins::Coins;
pub use hash::{BlockHash, MerkleRoot};
pub use party::{Destination, Source, COINBASE, STAKE, UNSTAKE};
