//! Blocks, transactions and the rules for valid chains. No I/O lives here.

pub mod address;
pub mod block;
pub mod chain;
pub mod consensus;
pub mod crypto;
pub mod error;
pub mod ledger;
pub mod mempool;
pub mod merkle;
pub mod repository;

// The consensus modules live under `consensus/`; these keep their old paths working.
pub use consensus::{difficulty, randao, rules, weight};
