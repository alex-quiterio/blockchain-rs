//! What happens to a chain, in the domain's own words. The aggregate records these as it changes,
//! and whoever made the change takes them: to save only what's new, to log, to tell others.

use crate::domain::block::Transaction;

/// Something that happened to the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainEvent {
    /// A new chain began, with only its genesis block.
    Started,
    /// Blocks were added on top of the tip: those at heights `from` to `to`.
    Extended { from: usize, to: usize },
    /// A heavier fork replaced the blocks from height `from` on: `dropped` of ours gave way to
    /// `added` of theirs. `returned` are the transfers of the blocks dropped, for a mempool to
    /// take back where they still apply.
    Reorganized { from: usize, dropped: usize, added: usize, returned: Vec<Transaction> },
}
