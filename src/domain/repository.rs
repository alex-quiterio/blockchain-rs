use super::block::Block;
use super::chain::Snapshot;

pub type RepositoryError = Box<dyn std::error::Error + Send + Sync>;

/// What happened to the chain since it was last saved, so a store can write only what's new.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainChange {
    /// Nothing is known to be saved yet: the first save.
    Whole,
    /// New blocks were added on top: the last `added`.
    Grew { added: usize },
    /// A fork replaced the blocks from height `from` on.
    Reorganized { from: usize },
}

/// Where confirmed blocks are kept between runs. The mempool is not stored.
pub trait ChainRepository: Send + Sync {
    /// The saved blocks, or `None` if nothing has been saved yet.
    fn load(&self) -> Result<Option<Vec<Block>>, RepositoryError>;

    /// Saves the chain as `blocks`, after `change`.
    fn save(&self, blocks: &[Block], change: ChainChange) -> Result<(), RepositoryError>;

    /// The last snapshot saved, if this store keeps them. A store that doesn't returns `None`,
    /// and every start replays the whole chain.
    fn load_snapshot(&self) -> Result<Option<Snapshot>, RepositoryError> {
        Ok(None)
    }

    fn save_snapshot(&self, _snapshot: &Snapshot) -> Result<(), RepositoryError> {
        Ok(())
    }
}
