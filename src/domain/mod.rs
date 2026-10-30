//! Blocks, transactions and the rules for valid chains. No I/O lives here.
//!
//! Organised by concept, as DDD modules: the two aggregates (`aggregates/`: the chain, with its
//! block, and the mempool), who owns what (`accounting/`: transactions and the ledger, which
//! both aggregates work with), and how nodes agree (`consensus/`). Beside them, the services that
//! belong to no one concept (`services/`) and the small checked values everything is made of
//! (`values/`). Each file names the building block it is.

pub mod accounting;
pub mod aggregates;
pub mod consensus;
pub mod error;
pub mod repository;
pub mod services;
pub mod values;

// These live under `accounting/`, `aggregates/`, `consensus/` and `services/`; the re-exports
// keep their old paths working.
pub use accounting::{ledger, transaction};
pub use aggregates::chain::block;
pub use aggregates::{chain, mempool};
pub use consensus::{difficulty, randao, rules, validators, weight};
pub use services::{crypto, merkle};
pub use values::address;
