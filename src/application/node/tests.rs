//! Opening, restarting, saving, mining and offering blocks, as one node sees them.

use crate::domain::accounting::Coins;
use crate::domain::chain::block::merkle;
use std::sync::Arc;

use super::*;
use crate::domain::chain::block::Block;
use crate::domain::chain::block::BlockError;
use crate::domain::chain::repository::ChainRepositoryError;
use crate::domain::chain::ChainError;
use crate::domain::chain::Snapshot;
use crate::domain::crypto;
use crate::domain::crypto::address::Address;
use crate::domain::mempool::MempoolEvent;
use crate::infrastructure::ChainInMemoryRepository;
use std::sync::atomic::AtomicBool;

const RULES: ChainRules = ChainRules::proof_of_work(2, 50);

fn new_address() -> Address {
    Address::parse(&crypto::generate_keypair().0).unwrap()
}

/// Shares one in-memory store between service instances, like a file surviving a restart.
#[derive(Clone, Default)]
struct SharedRepository(Arc<ChainInMemoryRepository>);

impl ChainRepository for SharedRepository {
    fn load(&self) -> Result<Option<Vec<Block>>, ChainRepositoryError> {
        self.0.load()
    }
    fn save(
        &self,
        blocks: &[Arc<Block>],
        events: &[ChainEvent],
    ) -> Result<(), ChainRepositoryError> {
        self.0.save(blocks, events)
    }
    fn load_snapshot(&self) -> Result<Option<Snapshot>, ChainRepositoryError> {
        self.0.load_snapshot()
    }
    fn save_snapshot(&self, snapshot: &Snapshot) -> Result<(), ChainRepositoryError> {
        self.0.save_snapshot(snapshot)
    }
}

struct FailingRepository;

impl ChainRepository for FailingRepository {
    fn load(&self) -> Result<Option<Vec<Block>>, ChainRepositoryError> {
        Ok(None)
    }
    fn save(
        &self,
        blocks: &[Arc<Block>],
        _events: &[ChainEvent],
    ) -> Result<(), ChainRepositoryError> {
        // Let the genesis block through so the node can start.
        if blocks.len() == 1 {
            Ok(())
        } else {
            Err("disk full".into())
        }
    }
}

fn node(repository: impl ChainRepository + 'static) -> NodeService {
    NodeService::open(Box::new(repository), RULES, ReorgLimit::DEFAULT).unwrap()
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
    let (rules, limit) = (RULES, ReorgLimit::new(2));
    let repo = SharedRepository::default();
    let miner = new_address();
    let first =
        NodeService::open(Box::new(repo.clone()), rules, limit).unwrap().with_snapshots_every(3);
    assert_eq!(first.restored_from(), None);
    for _ in 0..8 {
        first.mine(&miner).unwrap();
    }

    // Nine blocks; the last two may still be undone, so the snapshot is after block 6.
    let second = NodeService::open(Box::new(repo.clone()), rules, limit).unwrap();
    assert_eq!(second.restored_from(), Some(6));
    assert_eq!(second.queries().blocks(), first.queries().blocks());
    assert_eq!(second.queries().account(&miner).balance, Coins::new(400));

    // The reorg limit is this node's own choice, not a rule, so changing it keeps the snapshot.
    let wider = NodeService::open(Box::new(repo.clone()), rules, ReorgLimit::new(5)).unwrap();
    assert_eq!(wider.restored_from(), Some(6));

    let none =
        NodeService::open(Box::new(repo.clone()), rules, limit).unwrap().with_snapshots_every(0);
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
    let blocks: Vec<Arc<Block>> = blocks.into_iter().map(Arc::new).collect();
    repo.save(&blocks, &[ChainEvent::Started]).unwrap();

    let err = NodeService::open(Box::new(repo), RULES, ReorgLimit::DEFAULT).err().unwrap();
    assert!(matches!(
        err,
        NodeError::Domain(ApplicationError::Chain(ChainError::InvalidBlock {
            height: 1,
            reason: BlockError::MerkleRootMismatch
        }))
    ));
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
    let ours = Arc::new(
        NodeService::open(Box::new(ChainInMemoryRepository::default()), rules, ReorgLimit::DEFAULT)
            .unwrap(),
    );
    let theirs =
        NodeService::open(Box::new(ChainInMemoryRepository::default()), rules, ReorgLimit::DEFAULT)
            .unwrap();
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
        Err(NodeError::Domain(ApplicationError::Block(BlockError::StaleBlock))) => {
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
    let node = node(ChainInMemoryRepository::default());
    let cancelled = AtomicBool::new(true);
    assert!(matches!(
        node.mine_unless(&new_address(), &cancelled),
        Err(NodeError::Domain(ApplicationError::Block(BlockError::StaleBlock)))
    ));
    assert_eq!(node.queries().blocks().len(), 1);
}

#[test]
fn a_confirmed_transfer_can_be_proven_from_its_header() {
    let node = node(ChainInMemoryRepository::default());
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
    let node = node(ChainInMemoryRepository::default());
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
    let heights: Vec<_> =
        node.queries().history(&alice).iter().map(|l| l.block.as_ref().unwrap().height).collect();
    assert_eq!(heights, [2, 2, 1]);
}

/// Records the events each save was given.
#[derive(Clone, Default)]
struct RecordingRepository(Arc<Mutex<Vec<Vec<ChainEvent>>>>);

impl ChainRepository for RecordingRepository {
    fn load(&self) -> Result<Option<Vec<Block>>, ChainRepositoryError> {
        Ok(None)
    }
    fn save(
        &self,
        _blocks: &[Arc<Block>],
        events: &[ChainEvent],
    ) -> Result<(), ChainRepositoryError> {
        self.0.lock().unwrap().push(events.to_vec());
        Ok(())
    }
}

#[test]
fn a_fork_gives_the_mempool_back_what_its_dropped_block_carried() {
    let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
    let alice = Address::of(&key);
    let ours = node(ChainInMemoryRepository::default());
    ours.mine(&alice).unwrap();
    let theirs = node(ChainInMemoryRepository::default());
    theirs.extend(ours.queries().blocks()[1..].to_vec()).unwrap();

    // Our next block confirms Alice's payment; their heavier fork never saw it.
    let payment = Transaction::signed(&key, new_address(), Coins::new(5), 0, Coins::ZERO);
    ours.submit(payment.clone()).unwrap();
    ours.mine(&alice).unwrap();
    assert!(ours.queries().pending().is_empty(), "confirmed, so no longer waiting");
    theirs.mine(&new_address()).unwrap();
    theirs.mine(&new_address()).unwrap();

    ours.extend(theirs.queries().blocks()[2..].to_vec()).unwrap();
    assert_eq!(ours.queries().pending(), [payment], "waiting again, to be mined anew");
    assert_eq!(ours.queries().next_nonce(&alice), 1);
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
    let theirs = node(ChainInMemoryRepository::default());
    theirs.extend(ours.queries().blocks()[1..2].to_vec()).unwrap();
    theirs.mine(&new_address()).unwrap();
    theirs.mine(&new_address()).unwrap();
    ours.extend(theirs.queries().blocks()[2..].to_vec()).unwrap();

    // Genesis, two new blocks, then a fork that replaces the second with two of theirs.
    let replaced = ChainEvent::Reorganized { from: 2, dropped: 1, added: 2, returned: vec![] };
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
    // Listeners hear what happens after they subscribe: each change to the chain, then the
    // mempool following it, with nothing waiting here.
    let rebased = NodeEvent::Mempool {
        event: MempoolEvent::Rebased { kept: 0, dropped: 0, returned: 0 },
        from: None,
    };
    let expected: Vec<_> = saved[1..]
        .concat()
        .into_iter()
        .flat_map(|e| [NodeEvent::Chain(e), rebased.clone()])
        .collect();
    assert_eq!(*heard.lock().unwrap(), expected);

    // Blocks we already have change nothing, so nothing is saved or told.
    assert!(ours.extend(theirs.queries().blocks()[2..].to_vec()).is_ok());
    assert_eq!(repo.0.lock().unwrap().len(), 4);
}

#[test]
fn a_listener_given_before_opening_hears_a_new_chain_start() {
    let repo = SharedRepository::default();
    let heard = Arc::new(Mutex::new(Vec::new()));
    let log = heard.clone();
    let opening = NodeService::opening(Box::new(repo.clone()), RULES, ReorgLimit::DEFAULT);
    let new = opening.listening(move |event| log.lock().unwrap().push(event.clone())).open();
    assert!(new.unwrap().started());
    assert_eq!(*heard.lock().unwrap(), [NodeEvent::Chain(ChainEvent::Started)]);

    // Opening what was saved restores it: nothing new starts, so there's nothing to hear.
    let log = heard.clone();
    let opening = NodeService::opening(Box::new(repo), RULES, ReorgLimit::DEFAULT);
    let restored = opening.listening(move |event| log.lock().unwrap().push(event.clone())).open();
    assert!(!restored.unwrap().started());
    assert_eq!(heard.lock().unwrap().len(), 1);
}

#[test]
fn adopting_a_heavier_chain_saves_it() {
    let repo = SharedRepository::default();
    let ours = node(repo.clone());
    let theirs = node(ChainInMemoryRepository::default());
    theirs.mine(&new_address()).unwrap();

    assert_eq!(ours.adopt_if_heavier(theirs.queries().blocks()).unwrap(), (true, 2));
    assert_eq!(repo.load().unwrap().unwrap(), theirs.queries().blocks());
    assert_eq!(ours.adopt_if_heavier(theirs.queries().blocks()[..1].to_vec()).unwrap(), (false, 2));
    assert!(matches!(
        ours.adopt_if_heavier(vec![]),
        Err(NodeError::Domain(ApplicationError::Chain(ChainError::Empty)))
    ));
}
