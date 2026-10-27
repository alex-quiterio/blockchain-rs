use std::fmt;
use std::sync::Mutex;

use crate::domain::block::{Block, Transaction};
use crate::domain::chain::{Blockchain, ChainError};
use crate::domain::crypto;
use crate::domain::repository::{ChainRepository, RepositoryError};

#[derive(Debug)]
pub enum NodeError {
    /// A domain rule refused the change.
    Chain(ChainError),
    /// The change was valid but could not be saved, so it was not applied.
    Storage(RepositoryError),
}

impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Chain(e) => e.fmt(f),
            Self::Storage(e) => write!(f, "could not save the chain: {e}"),
        }
    }
}

impl std::error::Error for NodeError {}

impl From<ChainError> for NodeError {
    fn from(err: ChainError) -> Self {
        Self::Chain(err)
    }
}

pub struct Account {
    pub balance: u64,
    pub next_nonce: u64,
}

pub struct NodeService {
    chain: Mutex<Blockchain>,
    repository: Box<dyn ChainRepository>,
}

impl NodeService {
    /// Restores the saved chain, or starts and saves a new one if nothing is saved yet.
    pub fn open(
        repository: Box<dyn ChainRepository>,
        difficulty: usize,
        reward: u64,
    ) -> Result<Self, NodeError> {
        let chain = match repository.load().map_err(NodeError::Storage)? {
            Some(blocks) => Blockchain::restore(blocks, difficulty, reward)?,
            None => {
                let chain = Blockchain::new(difficulty, reward);
                repository.save(chain.blocks()).map_err(NodeError::Storage)?;
                chain
            }
        };
        Ok(Self { chain: Mutex::new(chain), repository })
    }

    pub fn blocks(&self) -> Vec<Block> {
        self.chain.lock().unwrap().blocks().to_vec()
    }

    /// Whether the chain replays cleanly, and its length.
    pub fn validity(&self) -> (bool, usize) {
        let chain = self.chain.lock().unwrap();
        (chain.is_valid(), chain.blocks().len())
    }

    pub fn account(&self, address: &str) -> Result<Account, ChainError> {
        if !crypto::is_valid_address(address) {
            return Err(ChainError::InvalidAddress(address.to_string()));
        }
        let chain = self.chain.lock().unwrap();
        Ok(Account { balance: chain.balance(address), next_nonce: chain.next_nonce(address) })
    }

    pub fn next_nonce(&self, address: &str) -> u64 {
        self.chain.lock().unwrap().next_nonce(address)
    }

    pub fn pending(&self) -> Vec<Transaction> {
        self.chain.lock().unwrap().pending().to_vec()
    }

    /// Queues a transaction. The mempool isn't saved, so nothing is written.
    pub fn submit(&self, tx: Transaction) -> Result<(), ChainError> {
        self.chain.lock().unwrap().submit(tx)
    }

    /// Mines a block and saves it. Blocks for as long as the proof of work takes.
    pub fn mine(&self, miner: &str) -> Result<Block, NodeError> {
        self.commit(|chain| chain.mine(miner).cloned())
    }

    /// Offers another node's chain. Returns whether it was adopted and the length afterwards.
    pub fn replace(&self, blocks: Vec<Block>) -> Result<(bool, usize), NodeError> {
        self.commit(|chain| {
            let replaced = chain.replace(blocks)?;
            Ok((replaced, chain.blocks().len()))
        })
    }

    /// Applies `change` to a copy of the chain and swaps it in only once the new blocks are
    /// saved, so memory and storage never disagree.
    fn commit<T>(
        &self,
        change: impl FnOnce(&mut Blockchain) -> Result<T, ChainError>,
    ) -> Result<T, NodeError> {
        let mut chain = self.chain.lock().unwrap();
        let mut next = chain.clone();
        let result = change(&mut next)?;
        if next.blocks().last().map(|b| &b.hash) != chain.blocks().last().map(|b| &b.hash) {
            self.repository.save(next.blocks()).map_err(NodeError::Storage)?;
        }
        *chain = next;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::infrastructure::InMemoryRepository;

    /// Shares one in-memory store between service instances, like a file surviving a restart.
    #[derive(Clone, Default)]
    struct SharedRepository(Arc<InMemoryRepository>);

    impl ChainRepository for SharedRepository {
        fn load(&self) -> Result<Option<Vec<Block>>, RepositoryError> {
            self.0.load()
        }
        fn save(&self, blocks: &[Block]) -> Result<(), RepositoryError> {
            self.0.save(blocks)
        }
    }

    struct FailingRepository;

    impl ChainRepository for FailingRepository {
        fn load(&self) -> Result<Option<Vec<Block>>, RepositoryError> {
            Ok(None)
        }
        fn save(&self, blocks: &[Block]) -> Result<(), RepositoryError> {
            // Let the genesis block through so the node can start.
            if blocks.len() == 1 {
                Ok(())
            } else {
                Err("disk full".into())
            }
        }
    }

    fn node(repository: impl ChainRepository + 'static) -> NodeService {
        NodeService::open(Box::new(repository), 2, 50).unwrap()
    }

    #[test]
    fn a_new_node_saves_its_genesis_block() {
        let repo = SharedRepository::default();
        let node = node(repo.clone());
        assert_eq!(repo.load().unwrap().unwrap(), node.blocks());
    }

    #[test]
    fn mined_blocks_survive_a_restart() {
        let repo = SharedRepository::default();
        let (miner, _) = crypto::generate_keypair();
        let first = node(repo.clone());
        first.mine(&miner).unwrap();
        first.mine(&miner).unwrap();

        let second = node(repo);
        assert_eq!(second.blocks(), first.blocks());
        assert_eq!(second.account(&miner).unwrap().balance, 100);
    }

    #[test]
    fn a_tampered_store_refuses_to_start() {
        let repo = SharedRepository::default();
        node(repo.clone()).mine(&crypto::generate_keypair().0).unwrap();
        let mut blocks = repo.load().unwrap().unwrap();
        blocks[1].transactions[0].amount = 1_000;
        repo.save(&blocks).unwrap();

        let err = NodeService::open(Box::new(repo), 2, 50).err().unwrap();
        assert!(matches!(err, NodeError::Chain(ChainError::InvalidChain)));
    }

    #[test]
    fn a_failed_save_leaves_the_chain_unchanged() {
        let node = node(FailingRepository);
        let (miner, _) = crypto::generate_keypair();

        assert!(matches!(node.mine(&miner), Err(NodeError::Storage(_))));
        assert_eq!(node.blocks().len(), 1);
        assert_eq!(node.account(&miner).unwrap().balance, 0);
    }

    #[test]
    fn adopting_a_longer_chain_saves_it() {
        let repo = SharedRepository::default();
        let ours = node(repo.clone());
        let theirs = node(InMemoryRepository::default());
        let (miner, _) = crypto::generate_keypair();
        theirs.mine(&miner).unwrap();

        assert_eq!(ours.replace(theirs.blocks()).unwrap(), (true, 2));
        assert_eq!(repo.load().unwrap().unwrap(), theirs.blocks());
        assert_eq!(ours.replace(vec![]).unwrap(), (false, 2));
    }
}
