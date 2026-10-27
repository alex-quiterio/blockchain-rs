use super::block::Block;

pub type RepositoryError = Box<dyn std::error::Error + Send + Sync>;

/// Where confirmed blocks are kept between runs. The mempool is not stored.
pub trait ChainRepository: Send + Sync {
    /// The saved blocks, or `None` if nothing has been saved yet.
    fn load(&self) -> Result<Option<Vec<Block>>, RepositoryError>;

    /// Replaces whatever was saved with `blocks`.
    fn save(&self, blocks: &[Block]) -> Result<(), RepositoryError>;
}
