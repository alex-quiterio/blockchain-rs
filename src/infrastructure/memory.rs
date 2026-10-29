use std::sync::Mutex;

use crate::domain::block::Block;
use crate::domain::chain::Snapshot;
use crate::domain::repository::{ChainChange, ChainRepository, RepositoryError};

/// Keeps the chain in memory only, for tests and throwaway nodes.
#[derive(Default)]
pub struct InMemoryRepository {
    blocks: Mutex<Option<Vec<Block>>>,
    snapshot: Mutex<Option<Snapshot>>,
}

impl ChainRepository for InMemoryRepository {
    fn load(&self) -> Result<Option<Vec<Block>>, RepositoryError> {
        Ok(self.blocks.lock().unwrap().clone())
    }

    fn save(&self, blocks: &[Block], _change: ChainChange) -> Result<(), RepositoryError> {
        *self.blocks.lock().unwrap() = Some(blocks.to_vec());
        Ok(())
    }

    fn load_snapshot(&self) -> Result<Option<Snapshot>, RepositoryError> {
        Ok(self.snapshot.lock().unwrap().clone())
    }

    fn save_snapshot(&self, snapshot: &Snapshot) -> Result<(), RepositoryError> {
        *self.snapshot.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}
