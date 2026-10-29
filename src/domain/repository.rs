use super::block::Block;
use super::chain::Snapshot;

pub type RepositoryError = Box<dyn std::error::Error + Send + Sync>;

/// Where confirmed blocks are kept between runs. The mempool is not stored.
pub trait ChainRepository: Send + Sync {
    /// The saved blocks, or `None` if nothing has been saved yet.
    fn load(&self) -> Result<Option<Vec<Block>>, RepositoryError>;

    /// Replaces whatever was saved with `blocks`. The first `unchanged` of them are the same as
    /// the last save's, so a store may write only the rest: usually just the new block.
    fn save(&self, blocks: &[Block], unchanged: usize) -> Result<(), RepositoryError>;

    /// The last snapshot saved, if this store keeps them. Stores that don't replay the whole
    /// chain on every start.
    fn load_snapshot(&self) -> Result<Option<Snapshot>, RepositoryError> {
        Ok(None)
    }

    fn save_snapshot(&self, _snapshot: &Snapshot) -> Result<(), RepositoryError> {
        Ok(())
    }
}
