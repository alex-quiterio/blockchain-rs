use std::sync::Mutex;

use crate::domain::block::Block;
use crate::domain::repository::{ChainRepository, RepositoryError};

/// Keeps the chain in memory only, for tests and throwaway nodes.
#[derive(Default)]
pub struct InMemoryRepository {
    blocks: Mutex<Option<Vec<Block>>>,
}

impl ChainRepository for InMemoryRepository {
    fn load(&self) -> Result<Option<Vec<Block>>, RepositoryError> {
        Ok(self.blocks.lock().unwrap().clone())
    }

    fn save(&self, blocks: &[Block], _unchanged: usize) -> Result<(), RepositoryError> {
        *self.blocks.lock().unwrap() = Some(blocks.to_vec());
        Ok(())
    }
}
