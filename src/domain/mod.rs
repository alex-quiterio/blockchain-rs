//! Blocks, transactions and the rules for valid chains. No I/O lives here.
//!
//! The two aggregates, the chain and the mempool, live under `aggregates/`. A block is the chain's
//! entity, so it lives inside it; a transaction is shared by both, so it lives out here.

pub mod aggregates;
pub mod consensus;
pub mod crypto;
pub mod error;
pub mod ledger;
pub mod merkle;
pub mod repository;
pub mod transaction;
pub mod values;

// These live under `aggregates/` and `consensus/`; the re-exports keep their old paths working.
pub use aggregates::chain::block;
pub use aggregates::{chain, mempool};
pub use consensus::{difficulty, randao, rules, validators, weight};
pub use values::address;
