//! Where the chain aggregate is kept between runs: a port, a repository in DDD terms, that
//! infrastructure implements. One per aggregate, and only the chain has one: the mempool isn't
//! stored.

use super::block::Block;
use super::{ChainEvent, Snapshot};

pub type RepositoryError = Box<dyn std::error::Error + Send + Sync>;

/// Where confirmed blocks are kept between runs. The mempool is not stored.
pub trait ChainRepository: Send + Sync {
    /// The saved blocks, or `None` if nothing has been saved yet.
    fn load(&self) -> Result<Option<Vec<Block>>, RepositoryError>;

    /// Saves the chain as `blocks`, after `events`, what happened to it since the last save, so a
    /// store can write only what's new.
    fn save(&self, blocks: &[Block], events: &[ChainEvent]) -> Result<(), RepositoryError>;

    /// The last snapshot saved, if this store keeps them. A store that doesn't returns `None`,
    /// and every start replays the whole chain.
    fn load_snapshot(&self) -> Result<Option<Snapshot>, RepositoryError> {
        Ok(None)
    }

    fn save_snapshot(&self, _snapshot: &Snapshot) -> Result<(), RepositoryError> {
        Ok(())
    }
}
