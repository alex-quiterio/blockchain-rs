//! Blocks, transactions and the rules for valid chains. No I/O lives here.
//!
//! Organised by concept, as DDD modules, not by building block. Two of them are aggregates: the
//! chain (`chain/`, with its block) and the mempool (`mempool/`). Beside them, who owns what
//! (`accounting/`: coins, transactions and the ledger, which both aggregates work with), how
//! nodes agree (`consensus/`), and hashing and signing (`crypto/`), which every other concept is
//! built on. Each concept keeps its own value objects and its own errors, the ways its operations
//! can refuse. Each file names the building blocks it holds.

pub mod accounting;
pub mod chain;
pub mod consensus;
pub mod crypto;
pub mod mempool;
