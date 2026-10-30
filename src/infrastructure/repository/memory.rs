use std::sync::{Arc, Mutex};

use crate::domain::chain::block::{owned, Block};
use crate::domain::chain::repository::{ChainRepository, ChainRepositoryError};
use crate::domain::chain::{ChainEvent, Snapshot};

/// Keeps the chain in memory only, for tests and throwaway nodes. It shares the chain's blocks
/// rather than copying them.
#[derive(Default)]
pub struct ChainInMemoryRepository {
    blocks: Mutex<Option<Vec<Arc<Block>>>>,
    snapshot: Mutex<Option<Snapshot>>,
}

impl ChainRepository for ChainInMemoryRepository {
    fn load(&self) -> Result<Option<Vec<Block>>, ChainRepositoryError> {
        let blocks = self.blocks.lock().unwrap();
        Ok(blocks.as_deref().map(owned))
    }

    fn save(
        &self,
        blocks: &[Arc<Block>],
        _events: &[ChainEvent],
    ) -> Result<(), ChainRepositoryError> {
        *self.blocks.lock().unwrap() = Some(blocks.to_vec());
        Ok(())
    }

    fn load_snapshot(&self) -> Result<Option<Snapshot>, ChainRepositoryError> {
        Ok(self.snapshot.lock().unwrap().clone())
    }

    fn save_snapshot(&self, snapshot: &Snapshot) -> Result<(), ChainRepositoryError> {
        *self.snapshot.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}
