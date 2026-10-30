//! The domain's two aggregates: clusters changed only through their root, each keeping its own
//! invariants.
//!
//! - `chain`: the `Blockchain`, root of the blocks and of the ledger they add up to. A `Block` is
//!   its entity: one can be built outside the chain, but it only becomes part of it through the
//!   root, which checks it first.
//! - `mempool`: the `Mempool`, the transfers waiting for a block. Together they always apply on
//!   top of the chain's ledger.
//!
//! Each has its own lock in a node, so neither waits on the other. What they share, such as a
//! `Transaction`, lives outside both.

pub mod chain;
pub mod mempool;
