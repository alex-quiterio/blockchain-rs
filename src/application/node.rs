use std::fmt;

use chrono::{DateTime, Utc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use ed25519_dalek::SigningKey;

use crate::domain::address::Address;
use crate::domain::block::{Block, BlockHeader, Transaction};
use crate::domain::chain::{Blockchain, ChainEvent, Extension};
use crate::domain::error::ChainError;
use crate::domain::ledger::ValidatorState;
use crate::domain::merkle;
use crate::domain::repository::{ChainRepository, RepositoryError};
use crate::domain::rules::ChainRules;
use crate::domain::values::{BlockHash, Coins};
use crate::domain::weight::ChainWeight;

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

pub struct TipStatus {
    pub height: u64,
    pub hash: BlockHash,
    pub weight: ChainWeight,
    pub pending: usize,
}

pub struct ChainStatus {
    pub valid: bool,
    pub length: usize,
    pub weight: ChainWeight,
}

pub struct Account {
    pub balance: Coins,
    pub staked: Coins,
    pub next_nonce: u64,
    pub validator: ValidatorState,
    /// If the next stake must commit, the epoch to commit for.
    pub commitment_due: Option<u64>,
}

/// Evidence that a transaction is in a block: its leaf, the sibling hashes up to the block's
/// Merkle root, and the block's header, whose hash covers the root.
pub struct TransactionProof {
    pub transaction: Transaction,
    pub leaf: String,
    pub steps: Vec<merkle::Step>,
    pub header: BlockHeader,
    /// Blocks on top of it, counting its own.
    pub confirmations: u64,
}

/// Where a transaction is: in a block, or still waiting in the mempool.
pub struct Located {
    pub transaction: Transaction,
    /// The block holding it and its place there. `None` while it's pending.
    pub block: Option<Placement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub height: u64,
    pub hash: BlockHash,
    pub position: usize,
    pub timestamp: DateTime<Utc>,
    /// Blocks on top of it, counting its own.
    pub confirmations: u64,
}

/// How the chain picks its next block, as of now.
pub struct ConsensusState {
    pub rules: ChainRules,
    /// Under proof of stake, who must forge next. `None` means anyone may.
    pub next_validator: Option<Address>,
    pub stakes: Vec<(String, Coins)>,
    /// The randomness the next validator is drawn from.
    pub beacon: String,
    /// Proof of work: the hashes the next block should take to find, on average.
    pub next_work: u128,
    /// Proof of work with retargeting: the next height where the work is adjusted.
    pub next_retarget: Option<u64>,
}

pub struct NodeService {
    chain: Mutex<Blockchain>,
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
            chain: Mutex::new(chain),
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

    pub fn blocks(&self) -> Vec<Block> {
        self.chain.lock().unwrap().blocks().to_vec()
    }

    /// The blocks from `height` on, for clients catching up.
    pub fn blocks_from(&self, height: usize) -> Vec<Block> {
        let chain = self.chain.lock().unwrap();
        chain.blocks().get(height..).unwrap_or_default().to_vec()
    }

    /// Up to `limit` blocks from `height` on, without copying the rest.
    pub fn blocks_between(&self, height: usize, limit: usize) -> Vec<Block> {
        let chain = self.chain.lock().unwrap();
        let blocks = chain.blocks().get(height..).unwrap_or_default();
        blocks[..limit.min(blocks.len())].to_vec()
    }

    /// The headers from `height` on: everything a light client needs to follow the chain.
    pub fn headers_from(&self, height: usize) -> Vec<BlockHeader> {
        let chain = self.chain.lock().unwrap();
        chain.blocks().get(height..).unwrap_or_default().iter().map(Block::header).collect()
    }

    /// The block at a height, given as digits, or with a hash.
    pub fn block(&self, id: &str) -> Option<Block> {
        let chain = self.chain.lock().unwrap();
        let blocks = chain.blocks();
        match id.parse::<usize>() {
            Ok(height) => blocks.get(height).cloned(),
            Err(_) => blocks.iter().rev().find(|block| block.hash.as_str() == id).cloned(),
        }
    }

    /// Finds the transfer signed `signature`, confirmed or pending.
    pub fn locate(&self, signature: &str) -> Option<Located> {
        let chain = self.chain.lock().unwrap();
        if let Some((block, position)) = chain.find_transaction(signature) {
            return Some(Located {
                transaction: block.transactions[position].clone(),
                block: Some(placement(&chain, block, position)),
            });
        }
        let pending = chain.pending_transaction(signature)?;
        Some(Located { transaction: pending.clone(), block: None })
    }

    /// Every transaction to or from `address`, block rewards included: pending ones first, then
    /// confirmed ones, newest first.
    pub fn history(&self, address: &Address) -> Vec<Located> {
        let chain = self.chain.lock().unwrap();
        let pending =
            chain.pending_of(address).map(|tx| Located { transaction: tx.clone(), block: None });
        let confirmed = chain.history_of(address).map(|(block, position)| Located {
            transaction: block.transactions[position].clone(),
            block: Some(placement(&chain, block, position)),
        });
        pending.chain(confirmed).collect()
    }

    /// Proves that the confirmed transfer signed `signature` is in its block, or `None` if no
    /// block holds it.
    pub fn prove(&self, signature: &str) -> Option<TransactionProof> {
        let chain = self.chain.lock().unwrap();
        let (block, position) = chain.find_transaction(signature)?;
        let transaction = block.transactions[position].clone();
        Some(TransactionProof {
            leaf: merkle::leaf(&transaction),
            steps: merkle::proof(&block.transactions, position)?,
            header: block.header(),
            confirmations: chain.tip().index - block.index + 1,
            transaction,
        })
    }

    /// Where the chain stands, without replaying it: cheap enough to poll every second.
    pub fn tip(&self) -> TipStatus {
        let chain = self.chain.lock().unwrap();
        TipStatus {
            height: chain.blocks().len() as u64 - 1,
            hash: chain.tip().hash.clone(),
            weight: chain.weight(),
            pending: chain.pending().len(),
        }
    }

    /// Whether the chain replays cleanly, how long it is and what it weighs in a fork.
    pub fn status(&self) -> ChainStatus {
        let chain = self.chain.lock().unwrap();
        ChainStatus {
            valid: chain.is_valid(),
            length: chain.blocks().len(),
            weight: chain.weight(),
        }
    }

    pub fn account(&self, address: &Address) -> Account {
        let chain = self.chain.lock().unwrap();
        Account {
            balance: chain.balance(address),
            staked: chain.staked(address),
            next_nonce: chain.next_nonce(address),
            validator: chain.validator_state(address),
            commitment_due: chain.commitment_due(address),
        }
    }

    pub fn next_nonce(&self, address: &Address) -> u64 {
        self.chain.lock().unwrap().next_nonce(address)
    }

    pub fn pending(&self) -> Vec<Transaction> {
        self.chain.lock().unwrap().pending().to_vec()
    }

    /// Queues a transaction. The mempool isn't saved, so nothing is written.
    pub fn submit(&self, tx: Transaction) -> Result<(), ChainError> {
        self.chain.lock().unwrap().submit(tx)
    }

    pub fn consensus(&self) -> ConsensusState {
        let chain = self.chain.lock().unwrap();
        ConsensusState {
            rules: chain.rules(),
            next_validator: chain.next_validator(),
            stakes: chain.stakes(),
            beacon: chain.beacon().to_string(),
            next_work: chain.next_work(),
            next_retarget: chain.next_retarget_height(),
        }
    }

    pub fn rules(&self) -> ChainRules {
        self.chain.lock().unwrap().rules()
    }

    /// Proof of work: the work each block required, by height.
    pub fn work_history(&self) -> Vec<u128> {
        self.chain.lock().unwrap().work_history()
    }

    /// The unsigned block `validator` should sign next. Changes nothing.
    pub fn propose(
        &self,
        validator: &Address,
        reveal: Option<String>,
    ) -> Result<Block, ChainError> {
        self.chain.lock().unwrap().propose(validator, reveal)
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
        let keep_going =
            || !cancelled.load(Ordering::Relaxed) && self.chain.lock().unwrap().tip().hash == tip;
        let block = template.mine_while(keep_going).ok_or(ChainError::StaleBlock)?;
        self.commit(|chain| chain.append_mined(block).cloned())
    }

    /// Offers another node's chain. Returns whether it was adopted and the length afterwards.
    pub fn adopt_if_heavier(&self, blocks: Vec<Block>) -> Result<(bool, usize), NodeError> {
        self.commit(|chain| {
            let adopted = chain.adopt_if_heavier(blocks)?;
            Ok((adopted, chain.blocks().len()))
        })
    }

    /// Offers another node's blocks from some height on. Returns what came of it and the length
    /// afterwards.
    pub fn extend(&self, blocks: Vec<Block>) -> Result<(Extension, usize), NodeError> {
        self.commit(|chain| {
            let extension = chain.extend(blocks)?;
            Ok((extension, chain.blocks().len()))
        })
    }

    /// Applies `change` to a copy of the chain and swaps it in only once the new blocks are
    /// saved, so memory and storage never disagree.
    /// The chain's events are what's saved and, once the chain is free again, what listeners
    /// hear.
    fn commit<T>(
        &self,
        change: impl FnOnce(&mut Blockchain) -> Result<T, ChainError>,
    ) -> Result<T, NodeError> {
        let mut chain = self.chain.lock().unwrap();
        let mut next = chain.clone();
        let result = change(&mut next)?;
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

fn placement(chain: &Blockchain, block: &Block, position: usize) -> Placement {
    Placement {
        height: block.index,
        hash: block.hash.clone(),
        position,
        timestamp: block.timestamp,
        confirmations: chain.tip().index - block.index + 1,
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(repo.load().unwrap().unwrap(), node.blocks());
    }

    #[test]
    fn mined_blocks_survive_a_restart() {
        let repo = SharedRepository::default();
        let miner = new_address();
        let first = node(repo.clone());
        first.mine(&miner).unwrap();
        first.mine(&miner).unwrap();

        let second = node(repo);
        assert_eq!(second.blocks(), first.blocks());
        assert_eq!(second.account(&miner).balance, Coins::new(100));
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
        assert_eq!(second.blocks(), first.blocks());
        assert_eq!(second.account(&miner).balance, Coins::new(400));

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
        let mut blocks = repo.load().unwrap().unwrap();
        blocks[1].transactions[0].amount = Coins::new(1_000);
        repo.save(&blocks, &[ChainEvent::Started]).unwrap();

        let err = NodeService::open(Box::new(repo), RULES).err().unwrap();
        assert!(matches!(err, NodeError::Chain(ChainError::InvalidChain)));
    }

    #[test]
    fn a_failed_save_leaves_the_chain_unchanged() {
        let node = node(FailingRepository);
        let miner = new_address();

        assert!(matches!(node.mine(&miner), Err(NodeError::Storage(_))));
        assert_eq!(node.blocks().len(), 1);
        assert_eq!(node.account(&miner).balance, Coins::new(0));
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
            Err(NodeError::Chain(ChainError::StaleBlock)) => {
                assert_eq!(ours.blocks()[1], theirs_block);
                assert_eq!(ours.account(&miner).balance, Coins::new(0));
            }
            // Ours happened to finish first; their block then lost or won the tie.
            Ok(block) => assert_eq!(block.index, 1),
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn mining_stops_when_cancelled() {
        let node = node(InMemoryRepository::default());
        let cancelled = AtomicBool::new(true);
        assert!(matches!(
            node.mine_unless(&new_address(), &cancelled),
            Err(NodeError::Chain(ChainError::StaleBlock))
        ));
        assert_eq!(node.blocks().len(), 1);
    }

    #[test]
    fn a_confirmed_transfer_can_be_proven_from_its_header() {
        let node = node(InMemoryRepository::default());
        let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
        let alice = Address::of(&key);
        node.mine(&alice).unwrap();
        let mut payments = Vec::new();
        for nonce in 0..5 {
            let tx = Transaction::signed(
                &key,
                new_address().as_str(),
                Coins::new(1),
                nonce,
                Coins::new(0),
            );
            node.submit(tx.clone()).unwrap();
            payments.push(tx);
        }
        assert!(node.prove(&payments[3].signature).is_none(), "not in a block yet");
        node.mine(&alice).unwrap();
        node.mine(&alice).unwrap();

        let proof = node.prove(&payments[3].signature).unwrap();
        assert_eq!(proof.transaction, payments[3]);
        assert_eq!((proof.header.index, proof.confirmations), (2, 2));
        assert_eq!(proof.header.hash, proof.header.computed_hash());
        assert_eq!(proof.header, node.headers_from(2)[0]);
        assert_eq!(proof.leaf, merkle::leaf(&payments[3]));
        assert!(merkle::verify(&proof.leaf, &proof.steps, &proof.header.merkle_root));
        assert!(node.prove("").is_none(), "block rewards have no signature to look up");
    }

    #[test]
    fn transactions_are_found_pending_then_confirmed_and_in_histories() {
        let node = node(InMemoryRepository::default());
        let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
        let (alice, bob) = (Address::of(&key), new_address());
        node.mine(&alice).unwrap();
        let payment = Transaction::signed(&key, bob.as_str(), Coins::new(7), 0, Coins::new(1));
        node.submit(payment.clone()).unwrap();

        let pending = node.locate(&payment.signature).unwrap();
        assert_eq!((pending.transaction, pending.block), (payment.clone(), None));
        assert_eq!(node.history(&bob).len(), 1);

        let block = node.mine(&alice).unwrap();
        let placement = node.locate(&payment.signature).unwrap().block.unwrap();
        assert_eq!((placement.height, placement.position, placement.confirmations), (2, 1, 1));
        assert_eq!(node.block("2"), Some(block.clone()));
        assert_eq!(node.block(block.hash.as_str()), Some(block));
        assert_eq!(node.block("9"), None);
        assert!(node.locate("nope").is_none());

        // Alice: the payment, then two block rewards, newest first.
        let heights: Vec<_> =
            node.history(&alice).iter().map(|l| l.block.as_ref().unwrap().height).collect();
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
        theirs.extend(ours.blocks()[1..2].to_vec()).unwrap();
        theirs.mine(&new_address()).unwrap();
        theirs.mine(&new_address()).unwrap();
        ours.extend(theirs.blocks()[2..].to_vec()).unwrap();

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
        assert!(ours.extend(theirs.blocks()[2..].to_vec()).is_ok());
        assert_eq!(repo.0.lock().unwrap().len(), 4);
    }

    #[test]
    fn adopting_a_heavier_chain_saves_it() {
        let repo = SharedRepository::default();
        let ours = node(repo.clone());
        let theirs = node(InMemoryRepository::default());
        theirs.mine(&new_address()).unwrap();

        assert_eq!(ours.adopt_if_heavier(theirs.blocks()).unwrap(), (true, 2));
        assert_eq!(repo.load().unwrap().unwrap(), theirs.blocks());
        assert_eq!(ours.adopt_if_heavier(theirs.blocks()[..1].to_vec()).unwrap(), (false, 2));
        assert!(matches!(
            ours.adopt_if_heavier(vec![]),
            Err(NodeError::Chain(ChainError::InvalidChain))
        ));
    }
}
