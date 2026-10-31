//! Accounting: who owns what, counted in `Coins` that move from a `Source` to a `Destination`
//! (value objects). A transaction moves coins, stakes or unstakes them, on
//! the chain it names (`ChainId`); the ledger is
//! what the confirmed ones add up to, and the rules each new one is checked by. Both aggregates
//! work with these: the chain's blocks carry transactions and add up to a ledger, and the
//! mempool holds transactions and projects its own ledger from them.

mod chain_id;
mod coins;
mod error;
pub mod ledger;
mod party;
pub mod transaction;

pub use chain_id::{chain_bound_since, ChainId};
pub use coins::Coins;
pub use party::{Destination, Source, COINBASE, STAKE, UNSTAKE};

pub use error::TransactionError;
