//! Accounting: who owns what. A transaction moves coins, stakes or unstakes them; the ledger is
//! what the confirmed ones add up to, and the rules each new one is checked by. Both aggregates
//! work with these: the chain's blocks carry transactions and add up to a ledger, and the
//! mempool holds transactions and projects its own ledger from them.

pub mod ledger;
pub mod transaction;
