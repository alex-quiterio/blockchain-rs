use std::fmt;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ed25519_dalek::SigningKey;

use crate::domain::address::Address;
use crate::domain::block::{Block, Transaction};
use crate::domain::chain::{Blockchain, ChainEvent, Extension};
use crate::domain::error::{BlockError, ChainError, DomainError, TransactionError};
use crate::domain::repository::{ChainRepository, RepositoryError};
use crate::domain::rules::ChainRules;

use super::queries::ChainQueries;

#[derive(Debug)]
pub enum NodeError {
    /// A domain rule refused the change.
    Domain(DomainError),
    /// The change was valid but could not be saved, so it was not applied.
    Storage(RepositoryError),
}

impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain(e) => e.fmt(f),
            Self::Storage(e) => write!(f, "could not save the chain: {e}"),
        }
    }
}

impl std::error::Error for NodeError {}

impl<E: Into<DomainError>> From<E> for NodeError {
    fn from(err: E) -> Self {
        Self::Domain(err.into())
    }
}

/// The node's use cases: every change to the chain goes through here, is saved, and is told to
/// listeners. Questions about the chain go to `ChainQueries`.
pub struct NodeService {
    chain: Arc<Mutex<Blockchain>>,
    repository: Box<dyn ChainRepository>,
    /// Blocks between snapshots. 0 takes none.
    snapshot_every: usize,
    /// The height of the last snapshot saved or loaded.
    last_snapshot: Mutex<usize>,
    /// The snapshot the chain was restored from on opening, if one was used.
    restored_from: Option<usize>,
    /// Told what happened after every change.
    listeners: Mutex<Vec<Listener>>,
}

type Listener = Box<dyn Fn(&ChainEvent) + Send + Sync>;

/// Blocks between snapshots, by default.
pub const SNAPSHOT_BLOCKS: usize = 100;

impl NodeService {
    /// Restores the saved chain, or starts and saves a new one if nothing is saved yet. A saved
    /// snapshot that fits the chain spares replaying the blocks before it.
    pub fn open(
        repository: Box<dyn ChainRepository>,
        rules: ChainRules,
    ) -> Result<Self, NodeError> {
        let (chain, restored_from) = match repository.load().map_err(NodeError::Storage)? {
            Some(blocks) => {
                // A snapshot that can't be read is no worse than none: the chain is replayed.
                let snapshot = repository.load_snapshot().unwrap_or(None);
                Blockchain::restore_from(blocks, rules, snapshot)?
            }
            None => {
                let mut chain = Blockchain::new(rules);
                let events = chain.take_events();
                repository.save(chain.blocks(), &events).map_err(NodeError::Storage)?;
                (chain, None)
            }
        };
        Ok(Self {
            chain: Arc::new(Mutex::new(chain)),
            repository,
            snapshot_every: SNAPSHOT_BLOCKS,
            last_snapshot: Mutex::new(restored_from.unwrap_or(0)),
            restored_from,
            listeners: Mutex::default(),
        })
    }

    /// Saves a snapshot every `blocks` blocks instead of every `SNAPSHOT_BLOCKS`. 0 saves none.
    pub fn with_snapshots_every(self, blocks: usize) -> Self {
        Self { snapshot_every: blocks, ..self }
    }

    /// Calls `listener` with everything that happens to the chain from now on, after it's saved.
    pub fn subscribe(&self, listener: impl Fn(&ChainEvent) + Send + Sync + 'static) {
        self.listeners.lock().unwrap().push(Box::new(listener));
    }

    /// The height of the snapshot the chain was restored from on opening, if one was used.
    pub fn restored_from(&self) -> Option<usize> {
        self.restored_from
    }

    /// Answers questions about the same chain this service changes.
    pub fn queries(&self) -> ChainQueries {
        ChainQueries::new(self.chain.clone())
    }

    /// Queues a transaction. The mempool isn't saved, so nothing is written.
    pub fn submit(&self, tx: Transaction) -> Result<(), TransactionError> {
        self.chain.lock().unwrap().submit(tx)
    }

    /// Appends a proof-of-stake block signed elsewhere and saves it.
    pub fn append_forged(&self, block: Block) -> Result<Block, NodeError> {
        self.commit(|chain| chain.append_forged(block).cloned())
    }

    /// Forges a proof-of-stake block signed with `key` and saves it.
    pub fn forge(&self, key: &SigningKey) -> Result<Block, NodeError> {
        self.commit(|chain| chain.forge(key).cloned())
    }

    /// Mines a block and saves it. The chain stays free while the proof of work is searched for,
    /// so the node keeps answering and taking peers' blocks. If one of those takes the tip first,
    /// the race is lost and mining stops with `StaleBlock`.
    pub fn mine(&self, miner: &Address) -> Result<Block, NodeError> {
        self.mine_unless(miner, &AtomicBool::new(false))
    }

    /// Like `mine`, but also stops with `StaleBlock` once `cancelled` is set, say because whoever
    /// asked for the block has stopped waiting for it.
    pub fn mine_unless(&self, miner: &Address, cancelled: &AtomicBool) -> Result<Block, NodeError> {
        let template = self.chain.lock().unwrap().mining_template(miner)?;
        let tip = template.prev_hash.clone();
        let keep_going = || {
            !cancelled.load(Ordering::Relaxed) && *self.chain.lock().unwrap().tip().hash() == tip
        };
        let block = template.mine_while(keep_going).ok_or(BlockError::StaleBlock)?;
        self.commit(|chain| chain.append_mined(block).cloned())
    }

    /// Offers another node's chain. Returns whether it was adopted and the length afterwards.
    pub fn adopt_if_heavier(&self, blocks: Vec<Block>) -> Result<(bool, usize), NodeError> {
        self.commit(|chain| {
            let adopted = chain.adopt_if_heavier(blocks)?;
            Ok::<_, ChainError>((adopted, chain.blocks().len()))
        })
    }

    /// Offers another node's blocks from some height on. Returns what came of it and the length
    /// afterwards.
    pub fn extend(&self, blocks: Vec<Block>) -> Result<(Extension, usize), NodeError> {
        self.commit(|chain| {
            let extension = chain.extend(blocks)?;
            Ok::<_, ChainError>((extension, chain.blocks().len()))
        })
    }

    /// Applies `change` to a copy of the chain and swaps it in only once the new blocks are
    /// saved, so memory and storage never disagree.
    /// The chain's events are what's saved and, once the chain is free again, what listeners
    /// hear.
    fn commit<T, E: Into<DomainError>>(
        &self,
        change: impl FnOnce(&mut Blockchain) -> Result<T, E>,
    ) -> Result<T, NodeError> {
        let mut chain = self.chain.lock().unwrap();
        let mut next = chain.clone();
        let result = change(&mut next).map_err(|e| NodeError::Domain(e.into()))?;
        let events = next.take_events();
        if !events.is_empty() {
            self.repository.save(next.blocks(), &events).map_err(NodeError::Storage)?;
            self.maybe_snapshot(&next);
        }
        *chain = next;
        drop(chain);
        for event in &events {
            for listener in self.listeners.lock().unwrap().iter() {
                listener(event);
            }
        }
        Ok(result)
    }
}

impl NodeService {
    /// Saves a snapshot once the chain has moved `snapshot_every` blocks past the last one. It
    /// only saves time on the next start, so a failure is reported and otherwise ignored.
    fn maybe_snapshot(&self, chain: &Blockchain) {
        let Some(snapshot) = chain.snapshot().filter(|_| self.snapshot_every > 0) else {
            return;
        };
        let mut last = self.last_snapshot.lock().unwrap();
        if snapshot.height < *last + self.snapshot_every {
            return;
        }
        match self.repository.save_snapshot(&snapshot) {
            Ok(()) => *last = snapshot.height,
            Err(e) => {
                tracing::warn!("Could not save a snapshot at height {}: {e}", snapshot.height)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::merkle;
    use crate::domain::values::Coins;
    use std::sync::Arc;

    use super::*;
    use crate::domain::chain::Snapshot;
    use crate::domain::crypto;
    use crate::infrastructure::InMemoryRepository;

    const RULES: ChainRules = ChainRules::proof_of_work(2, 50);

    fn new_address() -> Address {
        Address::parse(&crypto::generate_keypair().0).unwrap()
    }

    /// Shares one in-memory store between service instances, like a file surviving a restart.
    #[derive(Clone, Default)]
    struct SharedRepository(Arc<InMemoryRepository>);

    impl ChainRepository for SharedRepository {
        fn load(&self) -> Result<Option<Vec<Block>>, RepositoryError> {
            self.0.load()
        }
        fn save(&self, blocks: &[Block], events: &[ChainEvent]) -> Result<(), RepositoryError> {
            self.0.save(blocks, events)
        }
        fn load_snapshot(&self) -> Result<Option<Snapshot>, RepositoryError> {
            self.0.load_snapshot()
        }
        fn save_snapshot(&self, snapshot: &Snapshot) -> Result<(), RepositoryError> {
            self.0.save_snapshot(snapshot)
        }
    }

    struct FailingRepository;

    impl ChainRepository for FailingRepository {
        fn load(&self) -> Result<Option<Vec<Block>>, RepositoryError> {
            Ok(None)
        }
        fn save(&self, blocks: &[Block], _events: &[ChainEvent]) -> Result<(), RepositoryError> {
            // Let the genesis block through so the node can start.
            if blocks.len() == 1 {
                Ok(())
            } else {
                Err("disk full".into())
            }
        }
    }

    fn node(repository: impl ChainRepository + 'static) -> NodeService {
        NodeService::open(Box::new(repository), RULES).unwrap()
    }

    #[test]
    fn a_new_node_saves_its_genesis_block() {
        let repo = SharedRepository::default();
        let node = node(repo.clone());
        assert_eq!(repo.load().unwrap().unwrap(), node.queries().blocks());
    }

    #[test]
    fn mined_blocks_survive_a_restart() {
        let repo = SharedRepository::default();
        let miner = new_address();
        let first = node(repo.clone());
        first.mine(&miner).unwrap();
        first.mine(&miner).unwrap();

        let second = node(repo);
        assert_eq!(second.queries().blocks(), first.queries().blocks());
        assert_eq!(second.queries().account(&miner).balance, Coins::new(100));
    }

    #[test]
    fn a_restart_picks_up_from_the_last_snapshot() {
        let rules = RULES.with_max_reorg_depth(2);
        let repo = SharedRepository::default();
        let miner = new_address();
        let first =
            NodeService::open(Box::new(repo.clone()), rules).unwrap().with_snapshots_every(3);
        assert_eq!(first.restored_from(), None);
        for _ in 0..8 {
            first.mine(&miner).unwrap();
        }

        // Nine blocks; the last two may still be undone, so the snapshot is after block 6.
        let second = NodeService::open(Box::new(repo.clone()), rules).unwrap();
        assert_eq!(second.restored_from(), Some(6));
        assert_eq!(second.queries().blocks(), first.queries().blocks());
        assert_eq!(second.queries().account(&miner).balance, Coins::new(400));

        let none =
            NodeService::open(Box::new(repo.clone()), rules).unwrap().with_snapshots_every(0);
        for _ in 0..4 {
            none.mine(&miner).unwrap();
        }
        assert_eq!(repo.load_snapshot().unwrap().unwrap().height, 6, "0 saves no more of them");
    }

    #[test]
    fn a_tampered_store_refuses_to_start() {
        let repo = SharedRepository::default();
        node(repo.clone()).mine(&new_address()).unwrap();
        // Outside the domain a block can't be changed, only rewritten as JSON, as on disk.
        let mut json = serde_json::to_value(repo.load().unwrap().unwrap()).unwrap();
        json[1]["transactions"][0]["amount"] = 1_000.into();
        let blocks: Vec<Block> = serde_json::from_value(json).unwrap();
        repo.save(&blocks, &[ChainEvent::Started]).unwrap();

        let err = NodeService::open(Box::new(repo), RULES).err().unwrap();
        assert!(matches!(err, NodeError::Domain(DomainError::Chain(ChainError::InvalidChain))));
    }

    #[test]
    fn a_failed_save_leaves_the_chain_unchanged() {
        let node = node(FailingRepository);
        let miner = new_address();

        assert!(matches!(node.mine(&miner), Err(NodeError::Storage(_))));
        assert_eq!(node.queries().blocks().len(), 1);
        assert_eq!(node.queries().account(&miner).balance, Coins::new(0));
    }

    #[test]
    fn mining_gives_up_when_a_peer_takes_the_tip() {
        let rules = ChainRules::proof_of_work(4, 50);
        let ours =
            Arc::new(NodeService::open(Box::new(InMemoryRepository::default()), rules).unwrap());
        let theirs = NodeService::open(Box::new(InMemoryRepository::default()), rules).unwrap();
        let theirs_block = theirs.mine(&new_address()).unwrap();

        let miner = new_address();
        let mining = {
            let ours = ours.clone();
            let miner = miner.clone();
            std::thread::spawn(move || ours.mine(&miner))
        };
        // Once ours is searching, the chain still answers and takes their block.
        std::thread::sleep(std::time::Duration::from_millis(50));
        ours.extend(vec![theirs_block.clone()]).unwrap();
        match mining.join().unwrap() {
            Err(NodeError::Domain(DomainError::Block(BlockError::StaleBlock))) => {
                assert_eq!(ours.queries().blocks()[1], theirs_block);
                assert_eq!(ours.queries().account(&miner).balance, Coins::new(0));
            }
            // Ours happened to finish first; their block then lost or won the tie.
            Ok(block) => assert_eq!(block.index(), 1),
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn mining_stops_when_cancelled() {
        let node = node(InMemoryRepository::default());
        let cancelled = AtomicBool::new(true);
        assert!(matches!(
            node.mine_unless(&new_address(), &cancelled),
            Err(NodeError::Domain(DomainError::Block(BlockError::StaleBlock)))
        ));
        assert_eq!(node.queries().blocks().len(), 1);
    }

    #[test]
    fn a_confirmed_transfer_can_be_proven_from_its_header() {
        let node = node(InMemoryRepository::default());
        let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
        let alice = Address::of(&key);
        node.mine(&alice).unwrap();
        let mut payments = Vec::new();
        for nonce in 0..5 {
            let tx = Transaction::signed(&key, new_address(), Coins::new(1), nonce, Coins::new(0));
            node.submit(tx.clone()).unwrap();
            payments.push(tx);
        }
        assert!(
            node.queries().prove(payments[3].signature().unwrap().as_str()).is_none(),
            "not in a block yet"
        );
        node.mine(&alice).unwrap();
        node.mine(&alice).unwrap();

        let proof = node.queries().prove(payments[3].signature().unwrap().as_str()).unwrap();
        assert_eq!(proof.transaction, payments[3]);
        assert_eq!((proof.header.index, proof.confirmations), (2, 2));
        assert_eq!(proof.header.hash, proof.header.computed_hash());
        assert_eq!(proof.header, node.queries().headers_from(2)[0]);
        assert_eq!(proof.leaf, merkle::leaf(&payments[3]));
        assert!(merkle::verify(&proof.leaf, &proof.steps, &proof.header.merkle_root));
        assert!(node.queries().prove("").is_none(), "block rewards have no signature to look up");
    }

    #[test]
    fn transactions_are_found_pending_then_confirmed_and_in_histories() {
        let node = node(InMemoryRepository::default());
        let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
        let (alice, bob) = (Address::of(&key), new_address());
        node.mine(&alice).unwrap();
        let payment = Transaction::signed(&key, &bob, Coins::new(7), 0, Coins::new(1));
        node.submit(payment.clone()).unwrap();

        let pending = node.queries().locate(payment.signature().unwrap().as_str()).unwrap();
        assert_eq!((pending.transaction, pending.block), (payment.clone(), None));
        assert_eq!(node.queries().history(&bob).len(), 1);

        let block = node.mine(&alice).unwrap();
        let placement =
            node.queries().locate(payment.signature().unwrap().as_str()).unwrap().block.unwrap();
        assert_eq!((placement.height, placement.position, placement.confirmations), (2, 1, 1));
        assert_eq!(node.queries().block("2"), Some(block.clone()));
        assert_eq!(node.queries().block(block.hash().as_str()), Some(block));
        assert_eq!(node.queries().block("9"), None);
        assert!(node.queries().locate("nope").is_none());

        // Alice: the payment, then two block rewards, newest first.
        let heights: Vec<_> = node
            .queries()
            .history(&alice)
            .iter()
            .map(|l| l.block.as_ref().unwrap().height)
            .collect();
        assert_eq!(heights, [2, 2, 1]);
    }

    /// Records the events each save was given.
    #[derive(Clone, Default)]
    struct RecordingRepository(Arc<Mutex<Vec<Vec<ChainEvent>>>>);

    impl ChainRepository for RecordingRepository {
        fn load(&self) -> Result<Option<Vec<Block>>, RepositoryError> {
            Ok(None)
        }
        fn save(&self, _blocks: &[Block], events: &[ChainEvent]) -> Result<(), RepositoryError> {
            self.0.lock().unwrap().push(events.to_vec());
            Ok(())
        }
    }

    #[test]
    fn what_happened_is_saved_and_then_told_to_listeners() {
        let repo = RecordingRepository::default();
        let ours = node(repo.clone());
        let heard = Arc::new(Mutex::new(Vec::new()));
        let log = heard.clone();
        ours.subscribe(move |event| log.lock().unwrap().push(event.clone()));
        ours.mine(&new_address()).unwrap();
        ours.mine(&new_address()).unwrap();
        let theirs = node(InMemoryRepository::default());
        theirs.extend(ours.queries().blocks()[1..2].to_vec()).unwrap();
        theirs.mine(&new_address()).unwrap();
        theirs.mine(&new_address()).unwrap();
        ours.extend(theirs.queries().blocks()[2..].to_vec()).unwrap();

        // Genesis, two new blocks, then a fork that replaces the second with two of theirs.
        let replaced = ChainEvent::Reorganized { from: 2, dropped: 1, added: 2 };
        let saved = repo.0.lock().unwrap().clone();
        assert_eq!(
            saved,
            [
                vec![ChainEvent::Started],
                vec![ChainEvent::Extended { from: 1, to: 1 }],
                vec![ChainEvent::Extended { from: 2, to: 2 }],
                vec![replaced.clone()],
            ]
        );
        let told = heard.lock().unwrap().clone();
        assert_eq!(told, saved[1..].concat(), "listeners hear what happens after they subscribe");

        // Blocks we already have change nothing, so nothing is saved or told.
        assert!(ours.extend(theirs.queries().blocks()[2..].to_vec()).is_ok());
        assert_eq!(repo.0.lock().unwrap().len(), 4);
    }

    #[test]
    fn adopting_a_heavier_chain_saves_it() {
        let repo = SharedRepository::default();
        let ours = node(repo.clone());
        let theirs = node(InMemoryRepository::default());
        theirs.mine(&new_address()).unwrap();

        assert_eq!(ours.adopt_if_heavier(theirs.queries().blocks()).unwrap(), (true, 2));
        assert_eq!(repo.load().unwrap().unwrap(), theirs.queries().blocks());
        assert_eq!(
            ours.adopt_if_heavier(theirs.queries().blocks()[..1].to_vec()).unwrap(),
            (false, 2)
        );
        assert!(matches!(
            ours.adopt_if_heavier(vec![]),
            Err(NodeError::Domain(DomainError::Chain(ChainError::InvalidChain)))
        ));
    }
}
