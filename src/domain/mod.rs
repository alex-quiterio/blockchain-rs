//! Blocks, transactions and the rules for valid chains. No I/O lives here.
//!
//! Organised by concept, as DDD modules, not by building block. Two of them are aggregates: the
//! chain (`chain/`, with its block) and the mempool (`mempool/`). Beside them, who owns what
//! (`accounting/`: transactions and the ledger, which both aggregates work with), how nodes agree
//! (`consensus/`), hashing and signing (`crypto`), and the shared vocabulary every other module is
//! built from (`values/`). Each concept keeps its own errors; `error` gathers them where they
//! meet. Each file names the building blocks it holds.

pub mod accounting;
pub mod chain;
pub mod consensus;
pub mod crypto;
pub mod error;
pub mod mempool;
pub mod repository;
pub mod values;

// These live deeper in their concept; the re-exports keep their old paths working.
pub use accounting::{ledger, transaction};
pub use chain::block;
pub use chain::block::merkle;
pub use consensus::{difficulty, randao, rules, validators, weight};
pub use values::address;
