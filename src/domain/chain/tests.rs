//! The chain aggregate's tests: mining, forging, forks, restoring and every rule a block is
//! checked by.

use super::validation::replay;
use super::*;
use crate::domain::accounting::TransactionError;
use crate::domain::accounting::{Destination, Source};
use crate::domain::consensus::difficulty::meets_work;
use crate::domain::consensus::BlockCapacity;
use crate::domain::crypto;
use crate::domain::mempool::Mempool;

const RULES: ChainRules = ChainRules::proof_of_work(2, 50);

/// A chain with a mempool beside it, coordinated as a node does, so a test can submit and
/// mine in one place. Reads go to the chain itself; what touches what's waiting is here.
#[derive(Clone)]
struct Pooled {
    chain: Blockchain,
    pool: Mempool,
}

impl std::ops::Deref for Pooled {
    type Target = Blockchain;

    fn deref(&self) -> &Blockchain {
        &self.chain
    }
}

impl std::ops::DerefMut for Pooled {
    fn deref_mut(&mut self) -> &mut Blockchain {
        &mut self.chain
    }
}

impl From<Blockchain> for Pooled {
    fn from(chain: Blockchain) -> Self {
        let pool = Mempool::on(chain.ledger());
        Self { chain, pool }
    }
}

impl Pooled {
    fn new(rules: ChainRules) -> Self {
        Blockchain::new(rules).into()
    }

    fn limited(rules: ChainRules, limit: ReorgLimit) -> Self {
        Blockchain::new(rules).with_reorg_limit(limit).into()
    }

    fn restore(blocks: Vec<Block>, rules: ChainRules) -> Result<Self, ChainError> {
        Blockchain::restore(blocks, rules).map(Self::from)
    }

    fn restore_from(
        blocks: Vec<Block>,
        rules: ChainRules,
        limit: ReorgLimit,
        snapshot: Option<Snapshot>,
    ) -> Result<(Self, Option<usize>), ChainError> {
        Blockchain::restore_from(blocks, rules, limit, snapshot).map(|(c, from)| (c.into(), from))
    }

    fn submit(&mut self, tx: Transaction) -> Result<(), TransactionError> {
        self.pool.admit(tx)
    }

    fn pending(&self) -> &[Transaction] {
        self.pool.transactions()
    }

    fn next_nonce(&self, address: &Address) -> u64 {
        self.pool.next_nonce(address)
    }

    fn commitment_due(&self, address: &Address) -> Option<u64> {
        self.pool.commitment_due(address)
    }

    fn candidates(&self) -> Vec<Transaction> {
        self.pool.candidates(self.chain.rules().block_capacity)
    }

    fn next_contents(&self, producer: &Address) -> (u64, Vec<Transaction>) {
        self.chain.next_contents(producer, &self.candidates())
    }

    fn mining_template(&self, miner: &Address) -> Result<MiningTemplate, BlockError> {
        self.chain.mining_template(miner, &self.candidates())
    }

    fn propose(&self, validator: &Address, reveal: Option<Reveal>) -> Result<Block, BlockError> {
        self.chain.propose(validator, reveal, &self.candidates())
    }

    fn mine(&mut self, miner: &Address) -> Result<&Block, BlockError> {
        let pending = self.candidates();
        self.settling(|chain| chain.mine(miner, &pending).map(drop))?;
        Ok(self.chain.tip())
    }

    fn mine_at(&mut self, miner: &Address, at: DateTime<Utc>) -> Result<&Block, BlockError> {
        self.settling(|chain| chain.mine_at(miner, at).map(drop))?;
        Ok(self.chain.tip())
    }

    fn forge(&mut self, key: &SigningKey) -> Result<&Block, BlockError> {
        let pending = self.candidates();
        self.settling(|chain| chain.forge(key, &pending).map(drop))?;
        Ok(self.chain.tip())
    }

    fn append_mined(&mut self, block: Block) -> Result<&Block, BlockError> {
        self.settling(|chain| chain.append_mined(block).map(drop))?;
        Ok(self.chain.tip())
    }

    fn append_forged(&mut self, block: Block) -> Result<&Block, BlockError> {
        self.settling(|chain| chain.append_forged(block).map(drop))?;
        Ok(self.chain.tip())
    }

    fn adopt_if_heavier(&mut self, blocks: Vec<Block>) -> Result<bool, ChainError> {
        self.settling(|chain| chain.adopt_if_heavier(blocks))
    }

    fn extend(&mut self, blocks: Vec<Block>) -> Result<Extension, ChainError> {
        self.settling(|chain| chain.extend(blocks))
    }

    /// Makes a change, then rebases the mempool on what it did, as a node does. The events
    /// stay for the test to take.
    fn settling<T, E>(
        &mut self,
        change: impl FnOnce(&mut Blockchain) -> Result<T, E>,
    ) -> Result<T, E> {
        // Blocks in a row merge into one event, so the tip says whether anything happened.
        let (tip, before) = (self.chain.tip().hash.clone(), self.chain.events.len());
        let result = change(&mut self.chain)?;
        if self.chain.tip().hash != tip {
            let returned = self.chain.events[before..].iter().filter_map(|event| match event {
                ChainEvent::Reorganized { returned, .. } => Some(returned.clone()),
                _ => None,
            });
            let returned = returned.flatten().collect();
            self.pool.rebase(self.chain.ledger(), returned);
        }
        Ok(result)
    }
}

fn wallet() -> (Address, SigningKey) {
    let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
    (Address::of(&key), key)
}

fn transfer(key: &SigningKey, to: &Address, amount: u64, nonce: u64) -> Transaction {
    Transaction::signed(key, to, Coins::new(amount), nonce, Coins::new(0))
}

fn chain() -> Pooled {
    Pooled::new(RULES)
}

#[test]
fn mining_pays_the_reward() {
    let mut chain = chain();
    let (miner, _) = wallet();
    let block = chain.mine(&miner).unwrap();
    assert!(block.hash.as_str().starts_with("00"));
    assert_eq!(chain.balance(&miner), Coins::new(50));
    assert!(chain.is_valid());
}

#[test]
fn transfer_moves_coins_once_mined() {
    let mut chain = chain();
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    chain.mine(&alice).unwrap();

    chain.submit(transfer(&alice_key, &bob, 20, 0)).unwrap();
    assert_eq!(chain.balance(&bob), Coins::new(0));
    assert_eq!(chain.next_nonce(&alice), 1);

    chain.mine(&bob).unwrap();
    assert_eq!(chain.balance(&alice), Coins::new(30));
    assert_eq!(chain.balance(&bob), Coins::new(70));
    assert!(chain.pending().is_empty());
    assert!(chain.is_valid());
}

#[test]
fn a_full_block_takes_the_highest_fees_and_leaves_the_rest_waiting() {
    let mut chain = Pooled::new(RULES.with_block_capacity(BlockCapacity::at_most(2)));
    let (dave, _) = wallet();
    let senders: Vec<_> = (0..3).map(|_| wallet()).collect();
    for (address, _) in &senders {
        chain.mine(address).unwrap();
    }
    for ((_, key), fee) in senders.iter().zip([1, 5, 3]) {
        chain.submit(Transaction::signed(key, &dave, Coins::new(10), 0, Coins::new(fee))).unwrap();
    }

    let block = chain.mine(&dave).unwrap();
    let fees: Vec<_> = block.transactions[1..].iter().map(|tx| tx.fee().value()).collect();
    assert_eq!(fees, [5, 3]);
    assert_eq!(block.transactions[0].amount(), Coins::new(58), "the reward and both fees");
    assert_eq!(chain.pending().len(), 1, "the lowest fee waits for the next block");

    chain.mine(&dave).unwrap();
    assert!(chain.pending().is_empty());
    assert_eq!(chain.balance(&dave), Coins::new(30 + 58 + 51));
    assert!(chain.is_valid());
}

#[test]
fn a_block_takes_candidates_in_the_order_given_as_many_as_it_holds() {
    let mut chain = Blockchain::new(RULES.with_block_capacity(BlockCapacity::at_most(2)));
    let senders: Vec<_> = (0..3).map(|_| wallet()).collect();
    for (address, _) in &senders {
        chain.mine(address, &[]).unwrap();
    }
    let (to, _) = wallet();
    let offered: Vec<_> = senders.iter().map(|(_, key)| transfer(key, &to, 1, 0)).collect();
    let (_, transactions) = chain.next_contents(&to, &offered);
    assert_eq!(transactions[1..], offered[..2], "the chain doesn't reorder what it's offered");
}

#[test]
fn a_block_over_the_limit_is_refused() {
    let mut chain = chain();
    let (alice, key) = wallet();
    chain.mine(&alice).unwrap();
    for nonce in 0..3 {
        chain.submit(transfer(&key, &alice, 1, nonce)).unwrap();
    }
    chain.mine(&alice).unwrap();

    let limited = RULES.with_block_capacity(BlockCapacity::at_most(2));
    assert!(Blockchain::restore(chain.blocks().to_vec(), RULES).is_ok());
    assert_eq!(
        Blockchain::restore(chain.blocks().to_vec(), limited).err(),
        Some(ChainError::InvalidBlock {
            height: 2,
            reason: BlockError::TooManyTransfers { max: 2, got: 3 }
        })
    );
    let mut peer = Blockchain::restore(chain.blocks()[..2].to_vec(), limited).unwrap();
    assert_eq!(
        peer.extend(chain.blocks()[2..].to_vec()),
        Err(ChainError::InvalidBlock {
            height: 2,
            reason: BlockError::TooManyTransfers { max: 2, got: 3 }
        })
    );
    assert_eq!(
        peer.append_mined(chain.blocks()[2].clone()).err(),
        Some(BlockError::TooManyTransfers { max: 2, got: 3 }),
        "a block made elsewhere is told why"
    );
}

#[test]
fn rejects_forged_signature() {
    let mut chain = chain();
    let (alice, _) = wallet();
    let (_, mallory_key) = wallet();
    chain.mine(&alice).unwrap();

    let mut tx = transfer(&mallory_key, &Address::of(&mallory_key), 10, 0);
    tx.from = Source::Account(alice.clone());
    assert_eq!(chain.submit(tx), Err(TransactionError::BadSignature));
}

#[test]
fn rejects_overspend_across_mempool() {
    let mut chain = chain();
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    chain.mine(&alice).unwrap();

    chain.submit(transfer(&alice_key, &bob, 40, 0)).unwrap();
    assert_eq!(
        chain.submit(transfer(&alice_key, &bob, 20, 1)),
        Err(TransactionError::InsufficientFunds {
            balance: Coins::new(10),
            amount: Coins::new(20)
        })
    );
}

#[test]
fn rejects_replayed_transaction() {
    let mut chain = chain();
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    chain.mine(&alice).unwrap();

    let tx = transfer(&alice_key, &bob, 5, 0);
    chain.submit(tx.clone()).unwrap();
    chain.mine(&bob).unwrap();
    assert_eq!(chain.submit(tx), Err(TransactionError::BadNonce { expected: 1, got: 0 }));
}

#[test]
fn tampering_breaks_validity() {
    let mut chain = chain();
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    chain.mine(&alice).unwrap();
    chain.submit(transfer(&alice_key, &bob, 5, 0)).unwrap();
    chain.mine(&alice).unwrap();

    let mut blocks = chain.blocks().to_vec();
    blocks[2].transactions[1].amount = Coins::new(50);
    assert_eq!(
        validate_blocks(&blocks, &RULES),
        Err(ChainError::InvalidBlock { height: 2, reason: BlockError::MerkleRootMismatch })
    );

    let mut blocks = chain.blocks().to_vec();
    blocks[1].transactions[0].amount = Coins::new(1_000);
    blocks[1].hash = blocks[1].computed_hash();
    assert_eq!(
        validate_blocks(&blocks, &RULES),
        Err(ChainError::InvalidBlock { height: 1, reason: BlockError::MerkleRootMismatch })
    );
}

#[test]
fn transactions_must_match_the_merkle_root() {
    let mut chain = chain();
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    chain.mine(&alice).unwrap();
    chain.submit(transfer(&alice_key, &bob, 5, 0)).unwrap();
    chain.mine(&alice).unwrap();

    // The header, and so the hash and its proof of work, stays valid; the contents don't.
    let mut blocks = chain.blocks().to_vec();
    blocks[2].transactions.pop();
    blocks[2].transactions[0].amount = Coins::new(50);
    assert_eq!(blocks[2].hash, blocks[2].computed_hash());
    assert_eq!(
        validate_blocks(&blocks, &RULES),
        Err(ChainError::InvalidBlock { height: 2, reason: BlockError::MerkleRootMismatch })
    );

    let mut ours = Pooled::new(RULES);
    assert_eq!(
        ours.extend(blocks[1..].to_vec()),
        Err(ChainError::InvalidBlock { height: 2, reason: BlockError::MerkleRootMismatch })
    );
}

#[test]
fn restore_rebuilds_balances() {
    let mut chain = chain();
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    chain.mine(&alice).unwrap();
    chain.submit(transfer(&alice_key, &bob, 20, 0)).unwrap();
    chain.mine(&alice).unwrap();

    let restored = Pooled::restore(chain.blocks().to_vec(), RULES).unwrap();
    assert_eq!(restored.balance(&alice), Coins::new(80));
    assert_eq!(restored.balance(&bob), Coins::new(20));
    assert_eq!(restored.next_nonce(&alice), 1);
}

#[test]
fn restore_refuses_tampered_or_empty_blocks() {
    let mut chain = chain();
    chain.mine(&wallet().0).unwrap();
    let mut blocks = chain.blocks().to_vec();
    blocks[1].transactions[0].amount = Coins::new(1_000);
    assert_eq!(
        Pooled::restore(blocks, RULES).err(),
        Some(ChainError::InvalidBlock { height: 1, reason: BlockError::MerkleRootMismatch })
    );
    assert_eq!(Pooled::restore(Vec::new(), RULES).err(), Some(ChainError::Empty));
}

#[test]
fn restore_refuses_different_rules() {
    let mut chain = chain();
    chain.mine(&wallet().0).unwrap();
    // No SHA-256 hash is all zeros, so these blocks can't meet a difficulty of 64.
    assert!(Pooled::restore(chain.blocks().to_vec(), ChainRules::proof_of_work(64, 50)).is_err());
    assert!(Pooled::restore(chain.blocks().to_vec(), ChainRules::proof_of_work(2, 25)).is_err());
}

#[test]
fn adopt_takes_a_heavier_valid_chain() {
    let mut ours = chain();
    let mut theirs = chain();
    let (alice, _) = wallet();
    let (bob, _) = wallet();
    ours.mine(&alice).unwrap();
    theirs.mine(&bob).unwrap();
    theirs.mine(&bob).unwrap();

    assert_eq!(ours.adopt_if_heavier(theirs.blocks().to_vec()), Ok(true));
    assert_eq!(ours.blocks(), theirs.blocks());
    assert_eq!(ours.balance(&alice), Coins::new(0));
    assert_eq!(ours.balance(&bob), Coins::new(100));
    assert!(ours.is_valid());
}

#[test]
fn adopt_keeps_ours_against_a_lighter_or_identical_chain() {
    let mut ours = chain();
    ours.mine(&wallet().0).unwrap();
    ours.mine(&wallet().0).unwrap();
    let before = ours.blocks().to_vec();

    assert_eq!(ours.adopt_if_heavier(before[..2].to_vec()), Ok(false));
    assert_eq!(ours.adopt_if_heavier(before.clone()), Ok(false));
    assert_eq!(ours.blocks(), before);
}

#[test]
fn a_block_mined_on_an_old_tip_is_refused() {
    let mut chain = chain();
    let (miner, _) = wallet();
    let stale = chain.mining_template(&miner).unwrap();
    chain.mine(&wallet().0).unwrap();

    let block = stale.mine_while(|| true).unwrap();
    assert_eq!(chain.append_mined(block), Err(BlockError::StaleBlock));
    assert_eq!(chain.blocks().len(), 2);
    assert_eq!(chain.balance(&miner), Coins::new(0));
}

#[test]
fn a_mined_block_keeps_transactions_that_arrived_meanwhile() {
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    let mut chain = chain();
    chain.mine(&alice).unwrap();
    let template = chain.mining_template(&alice).unwrap();
    let late = transfer(&alice_key, &bob, 5, 0);
    chain.submit(late.clone()).unwrap();

    chain.append_mined(template.mine_while(|| true).unwrap()).unwrap();
    assert_eq!(chain.pending(), [late], "waits for the next block");
}

#[test]
fn mining_stops_when_told_to() {
    let hard = ChainRules::proof_of_work(16, 50);
    let template = Pooled::new(RULES).mining_template(&wallet().0).unwrap();
    let template = MiningTemplate { work: hard.initial_work(), ..template };
    let mut asked = 0;
    assert_eq!(
        template.mine_while(|| {
            asked += 1;
            asked < 3
        }),
        None
    );
    assert_eq!(asked, 3);
}

#[test]
fn a_mempool_that_lags_the_chain_cannot_spoil_a_block() {
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    let mut chain = super::Blockchain::new(RULES);
    chain.mine(&alice, &[]).unwrap();
    let payment = transfer(&alice_key, &bob, 5, 0);
    chain.mine(&alice, std::slice::from_ref(&payment)).unwrap();

    // A mempool that hasn't caught up still offers the payment the chain just confirmed.
    let block = chain.mine(&alice, &[payment]).unwrap();
    assert_eq!(block.transactions.len(), 1, "only the reward: the stale candidate is left out");
    assert!(chain.is_valid());
    assert_eq!(chain.balance(&bob), Coins::new(5));
}

#[test]
fn every_change_says_what_happened() {
    let (rules, limit) = (RULES, ReorgLimit::new(5));
    let mut ours = Pooled::limited(rules, limit);
    assert_eq!(ours.take_events(), [ChainEvent::Started]);
    assert_eq!(ours.take_events(), [], "taken once");
    ours.mine(&wallet().0).unwrap();
    ours.mine(&wallet().0).unwrap();
    assert_eq!(ours.take_events(), [ChainEvent::Extended { from: 1, to: 2 }], "merged");

    // A restored chain starts with nothing to tell.
    let mut theirs = Pooled::restore(ours.blocks().to_vec(), rules).unwrap();
    assert_eq!(theirs.take_events(), []);
    theirs.mine(&wallet().0).unwrap();
    theirs.mine(&wallet().0).unwrap();

    // Theirs is ours with more on top: an extension, however it arrives.
    let mut whole = ours.clone();
    assert_eq!(whole.adopt_if_heavier(theirs.blocks().to_vec()), Ok(true));
    assert_eq!(whole.take_events(), [ChainEvent::Extended { from: 3, to: 4 }]);
    assert_eq!(ours.extend(theirs.blocks()[3..].to_vec()), Ok(Extension::Adopted));
    assert_eq!(ours.take_events(), [ChainEvent::Extended { from: 3, to: 4 }]);

    // A heavier fork from block 2 replaces three of ours with four of theirs.
    let mut fork = Pooled::restore(ours.blocks()[..2].to_vec(), rules).unwrap();
    for _ in 0..4 {
        fork.mine(&wallet().0).unwrap();
    }
    assert_eq!(ours.adopt_if_heavier(fork.blocks().to_vec()), Ok(true));
    // Their blocks held only rewards, which never go back: nothing is returned.
    let replaced = ChainEvent::Reorganized { from: 2, dropped: 3, added: 4, returned: vec![] };
    assert_eq!(ours.take_events(), [replaced]);

    // Nothing adopted, nothing happened.
    assert_eq!(ours.adopt_if_heavier(theirs.blocks().to_vec()), Ok(false));
    assert_eq!(ours.take_events(), []);
}

#[test]
fn extend_appends_new_blocks_on_our_tip() {
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    let mut ours = chain();
    ours.mine(&alice).unwrap();
    let mut theirs = ours.clone();
    let payment = transfer(&alice_key, &bob, 20, 0);
    ours.submit(payment.clone()).unwrap();
    theirs.submit(payment).unwrap();
    theirs.mine(&bob).unwrap();
    theirs.mine(&bob).unwrap();

    // Overlapping blocks we already have are skipped.
    assert_eq!(ours.extend(theirs.blocks()[1..].to_vec()), Ok(Extension::Adopted));
    assert_eq!(ours.blocks(), theirs.blocks());
    assert_eq!(ours.balance(&bob), Coins::new(120));
    assert!(ours.pending().is_empty(), "the confirmed payment left the mempool");
    assert_eq!(ours.extend(theirs.blocks()[2..].to_vec()), Ok(Extension::Ignored));
    assert!(ours.is_valid());
}

#[test]
fn extend_asks_for_more_when_the_parent_is_missing() {
    let mut ours = chain();
    let mut theirs = chain();
    theirs.mine(&wallet().0).unwrap();
    theirs.mine(&wallet().0).unwrap();

    assert_eq!(ours.extend(theirs.blocks()[2..].to_vec()), Ok(Extension::MissingParent));
    assert_eq!(ours.extend(theirs.blocks()[1..].to_vec()), Ok(Extension::Adopted));
    assert_eq!(ours.blocks(), theirs.blocks());
}

#[test]
fn extend_weighs_a_fork_like_a_whole_chain() {
    let (alice, _) = wallet();
    let (bob, _) = wallet();
    let mut ours = chain();
    ours.mine(&alice).unwrap();
    let mut theirs = ours.clone();
    ours.mine(&alice).unwrap();
    theirs.mine(&bob).unwrap();
    theirs.mine(&bob).unwrap();

    assert_eq!(ours.extend(theirs.blocks()[2..].to_vec()), Ok(Extension::Adopted));
    assert_eq!(ours.blocks(), theirs.blocks());
    assert_eq!(ours.balance(&alice), Coins::new(50));
}

#[test]
fn extend_refuses_a_bad_block_and_keeps_ours() {
    let mut ours = chain();
    let mut theirs = chain();
    theirs.mine(&wallet().0).unwrap();
    let mut block = theirs.blocks()[1].clone();
    block.transactions[0].amount = Coins::new(1_000);
    block.hash = block.computed_hash();

    assert_eq!(
        ours.extend(vec![block]),
        Err(ChainError::InvalidBlock { height: 1, reason: BlockError::MerkleRootMismatch })
    );
    assert_eq!(ours.blocks().len(), 1);
    assert_eq!(ours.extend(vec![]), Err(ChainError::Empty));
}

/// The remembered tallies say what a full replay of the chain up to each of them would.
fn assert_remembers_correctly(chain: &Pooled, watched: &[Address]) {
    let heights = chain.tallies.heights();
    assert_eq!(heights.len(), chain.reorg_limit.tallies_to_keep().min(chain.blocks.len()));
    assert_eq!(heights.end, chain.blocks.len(), "the last is after the tip");
    for height in heights {
        let replayed = replay(&chain.blocks[..=height], &chain.rules).unwrap();
        let kept = chain.tallies.after(height).unwrap();
        assert_eq!((kept.weight, kept.work), (replayed.weight, replayed.work), "at {height}");
        for who in watched {
            let (k, r) = (kept.ledger.balance(who), replayed.ledger.balance(who));
            assert_eq!(k, r, "{who} at {height}");
        }
    }
}

#[test]
fn forks_are_checked_from_where_they_split() {
    let (rules, limit) = (RULES, ReorgLimit::new(3));
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    let mut ours = Pooled::limited(rules, limit);
    for _ in 0..6 {
        ours.mine(&alice).unwrap();
    }
    assert_remembers_correctly(&ours, &[alice.clone(), bob.clone()]);

    // Their fork splits two blocks back and carries a payment ours never saw.
    let mut theirs = Pooled::restore(ours.blocks()[..5].to_vec(), rules).unwrap();
    theirs.submit(transfer(&alice_key, &bob, 30, 0)).unwrap();
    for _ in 0..3 {
        theirs.mine(&bob).unwrap();
    }
    assert_eq!(ours.adopt_if_heavier(theirs.blocks().to_vec()), Ok(true));
    assert_eq!(ours.blocks(), theirs.blocks());
    assert_eq!((ours.balance(&alice), ours.balance(&bob)), (Coins::new(170), Coins::new(180)));
    assert_remembers_correctly(&ours, &[alice.clone(), bob.clone()]);

    // Blocks that arrive one by one after the fork are remembered too.
    let mut longer = theirs.clone();
    longer.mine(&alice).unwrap();
    longer.mine(&alice).unwrap();
    assert_eq!(ours.extend(longer.blocks()[8..].to_vec()), Ok(Extension::Adopted));
    assert_remembers_correctly(&ours, &[alice, bob]);
}

#[test]
fn a_restart_from_a_snapshot_replays_only_what_came_after() {
    let (rules, limit) = (RULES, ReorgLimit::new(2));
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    let mut chain = Pooled::limited(rules, limit);
    chain.mine(&alice).unwrap();
    chain.submit(transfer(&alice_key, &bob, 20, 0)).unwrap();
    for _ in 0..4 {
        chain.mine(&alice).unwrap();
    }
    let snapshot = chain.snapshot().unwrap();
    assert_eq!(snapshot.height, chain.blocks().len() - 3, "the oldest block no fork may undo");

    let (restored, from) =
        Pooled::restore_from(chain.blocks().to_vec(), rules, limit, Some(snapshot.clone()))
            .unwrap();
    assert_eq!(from, Some(snapshot.height));
    assert_eq!(
        (restored.balance(&alice), restored.balance(&bob)),
        (Coins::new(230), Coins::new(20))
    );
    assert_eq!((restored.weight(), restored.next_work()), (chain.weight(), chain.next_work()));
    assert_remembers_correctly(&restored, &[alice.clone(), bob.clone()]);

    // It's trusted for the blocks it covers: that's what it saves. Hence the checks below.
    let mut generous = snapshot.clone();
    generous.tally.ledger.credit(&bob, Coins::new(1_000));
    let (trusting, _) =
        Pooled::restore_from(chain.blocks().to_vec(), rules, limit, Some(generous)).unwrap();
    assert_eq!(trusting.balance(&bob), Coins::new(1_020));

    // Blocks it covers that were changed don't lead to its block, so it isn't used.
    let mut tampered = chain.blocks().to_vec();
    tampered[2].transactions[1].amount = Coins::new(25);
    assert!(Pooled::restore_from(tampered, rules, limit, Some(snapshot.clone())).is_err());

    // A node's own reorg limit isn't one of the rules: changing it keeps the snapshot.
    let wider = ReorgLimit::new(9);
    let (_, from) =
        Pooled::restore_from(chain.blocks().to_vec(), rules, wider, Some(snapshot.clone()))
            .unwrap();
    assert_eq!(from, Some(snapshot.height));

    // But it isn't used under other rules, or for a chain where its block was replaced.
    let other = rules.with_block_capacity(BlockCapacity::at_most(9));
    let (_, from) =
        Pooled::restore_from(chain.blocks().to_vec(), other, limit, Some(snapshot.clone()))
            .unwrap();
    assert_eq!(from, None);
    let replaced = Snapshot { hash: BlockHash::of(b"another block"), ..snapshot };
    let (full, from) =
        Pooled::restore_from(chain.blocks().to_vec(), rules, limit, Some(replaced)).unwrap();
    assert_eq!((from, full.balance(&bob)), (None, Coins::new(20)));
}

#[test]
fn a_short_chain_has_no_snapshot_yet() {
    let mut chain = Pooled::limited(RULES, ReorgLimit::new(3));
    chain.mine(&wallet().0).unwrap();
    assert!(chain.snapshot().is_none());
}

#[test]
fn weight_grows_with_every_block() {
    let mut chain = chain();
    assert_eq!(chain.weight(), ChainWeight::ZERO, "genesis weighs nothing");
    chain.mine(&wallet().0).unwrap();
    chain.mine(&wallet().0).unwrap();
    assert_eq!(chain.weight().value(), 2 * 256);
    let restored = Pooled::restore(chain.blocks().to_vec(), RULES).unwrap();
    assert_eq!(restored.weight(), chain.weight());
}

#[test]
fn equal_forks_settle_on_the_lower_tip_hash_from_either_side() {
    let mut a = chain();
    let mut b = chain();
    a.mine(&wallet().0).unwrap();
    b.mine(&wallet().0).unwrap();
    assert_eq!(a.weight(), b.weight());

    let winner = if a.tip().hash < b.tip().hash { a.blocks() } else { b.blocks() }.to_vec();
    let a_switched = a.adopt_if_heavier(b.blocks().to_vec()).unwrap();
    let b_switched = b.adopt_if_heavier(winner.clone()).unwrap();
    assert_ne!(a_switched, b_switched, "exactly one side gives way");
    assert_eq!(a.blocks(), winner);
    assert_eq!(b.blocks(), winner);
}

#[test]
fn a_deep_reorg_is_refused_however_heavy() {
    let (rules, limit) = (RULES, ReorgLimit::new(2));
    let mut ours = Pooled::limited(rules, limit);
    let mut theirs = Pooled::limited(rules, limit);
    for _ in 0..3 {
        ours.mine(&wallet().0).unwrap();
    }
    for _ in 0..5 {
        theirs.mine(&wallet().0).unwrap();
    }
    assert_eq!(
        ours.adopt_if_heavier(theirs.blocks().to_vec()),
        Err(ChainError::ReorgTooDeep { depth: 3, max: 2 })
    );

    let mut shallow = Pooled::limited(rules, limit);
    shallow.adopt_if_heavier(ours.blocks()[..2].to_vec()).unwrap();
    shallow.mine(&wallet().0).unwrap();
    shallow.mine(&wallet().0).unwrap();
    shallow.mine(&wallet().0).unwrap();
    assert_eq!(ours.adopt_if_heavier(shallow.blocks().to_vec()), Ok(true), "undoes only 2");
}

#[test]
fn adopt_refuses_an_invalid_longer_chain() {
    let mut ours = chain();
    let mut theirs = chain();
    let (alice, _) = wallet();
    ours.mine(&alice).unwrap();
    theirs.mine(&wallet().0).unwrap();
    theirs.mine(&wallet().0).unwrap();
    let mut forged = theirs.blocks().to_vec();
    forged[2].transactions[0].amount = Coins::new(1_000);

    assert_eq!(
        ours.adopt_if_heavier(forged),
        Err(ChainError::InvalidBlock { height: 2, reason: BlockError::MerkleRootMismatch })
    );
    assert_eq!(ours.balance(&alice), Coins::new(50));
}

#[test]
fn adopt_returns_dropped_transfers_to_the_mempool() {
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    let mut ours = chain();
    ours.mine(&alice).unwrap();
    let mut theirs = ours.clone();

    // Our fork confirms Alice's payment; their heavier fork never saw it.
    let payment = transfer(&alice_key, &bob, 20, 0);
    ours.submit(payment.clone()).unwrap();
    ours.mine(&alice).unwrap();
    theirs.mine(&bob).unwrap();
    theirs.mine(&bob).unwrap();

    assert_eq!(ours.adopt_if_heavier(theirs.blocks().to_vec()), Ok(true));
    assert_eq!(ours.pending(), [payment]);
    assert_eq!(ours.balance(&alice), Coins::new(50));
    assert_eq!(ours.next_nonce(&alice), 1);
}

#[test]
fn adopt_drops_transfers_the_new_chain_already_confirmed() {
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    let mut ours = chain();
    ours.mine(&alice).unwrap();
    let mut theirs = ours.clone();

    let payment = transfer(&alice_key, &bob, 20, 0);
    ours.submit(payment.clone()).unwrap();
    theirs.submit(payment).unwrap();
    theirs.mine(&bob).unwrap();
    theirs.mine(&bob).unwrap();

    assert_eq!(ours.adopt_if_heavier(theirs.blocks().to_vec()), Ok(true));
    assert!(ours.pending().is_empty());
    assert_eq!(ours.balance(&bob), Coins::new(120));
}

// ---- Proof of stake --------------------------------------------------------------------

const POS: ChainRules = ChainRules::proof_of_stake(50);

/// A first stake, committing to the key's reveal chain for epoch 0.
fn stake(key: &SigningKey, amount: u64, nonce: u64) -> Transaction {
    Transaction::stake(
        key,
        Coins::new(amount),
        nonce,
        Coins::new(0),
        Some(randao::commitment(key, 0)),
    )
}

/// The reveal `key`'s next block must carry, as a wallet would work it out.
fn reveal_for(chain: &Pooled, key: &SigningKey) -> Option<Reveal> {
    chain.next_validator()?;
    let state = chain.validator_state(&Address::of(key));
    Some(randao::reveal(key, state.next_epoch - 1, state.reveals_left?))
}

#[test]
fn anyone_may_forge_until_something_is_staked() {
    let mut chain = Pooled::new(POS);
    let (alice, alice_key) = wallet();
    assert_eq!(chain.next_validator(), None);

    let block = chain.forge(&alice_key).unwrap();
    assert_eq!(block.nonce, 0);
    assert_eq!(block.validator.as_ref(), Some(&alice));
    assert!(block.is_signed_by_validator());
    assert_eq!(chain.balance(&alice), Coins::new(50));
    assert!(chain.is_valid());
}

#[test]
fn staking_locks_coins_and_makes_a_validator() {
    let mut chain = Pooled::new(POS);
    let (alice, alice_key) = wallet();
    let (_, bob_key) = wallet();
    chain.forge(&alice_key).unwrap();
    chain.submit(stake(&alice_key, 30, 0)).unwrap();
    chain.forge(&bob_key).unwrap();

    assert_eq!(chain.balance(&alice), Coins::new(20));
    assert_eq!(chain.staked(&alice), Coins::new(30));
    assert_eq!(chain.next_validator(), Some(alice.clone()));
    assert_eq!(
        chain.forge(&bob_key).err(),
        Some(BlockError::NotYourTurn { expected: alice.clone() })
    );
    chain.forge(&alice_key).unwrap();
    assert!(chain.is_valid());
}

#[test]
fn unstaking_returns_coins_but_never_more_than_staked() {
    let mut chain = Pooled::new(POS);
    let (alice, alice_key) = wallet();
    chain.forge(&alice_key).unwrap();
    chain.submit(stake(&alice_key, 30, 0)).unwrap();
    chain.forge(&alice_key).unwrap();

    assert_eq!(
        chain.submit(Transaction::signed(
            &alice_key,
            Destination::Unstake,
            Coins::new(31),
            1,
            Coins::new(0)
        )),
        Err(TransactionError::InsufficientStake { staked: Coins::new(30), amount: Coins::new(31) })
    );
    chain
        .submit(Transaction::signed(
            &alice_key,
            Destination::Unstake,
            Coins::new(30),
            1,
            Coins::new(0),
        ))
        .unwrap();
    chain.forge(&alice_key).unwrap();
    assert_eq!(chain.staked(&alice), Coins::new(0));
    assert_eq!(chain.balance(&alice), Coins::new(150));
}

#[test]
fn each_consensus_refuses_the_other_ones_blocks() {
    let (alice, alice_key) = wallet();
    assert_eq!(Pooled::new(POS).mine(&alice).err(), Some(BlockError::NotProofOfWork));
    assert_eq!(chain().forge(&alice_key).err(), Some(BlockError::NotProofOfStake));

    let mut pos = Pooled::new(POS);
    pos.forge(&alice_key).unwrap();
    assert!(Pooled::restore(pos.blocks().to_vec(), RULES).is_err());
    let mut pow = chain();
    pow.mine(&alice).unwrap();
    assert!(Pooled::restore(pow.blocks().to_vec(), POS).is_err());
}

#[test]
fn forged_blocks_cannot_be_tampered_with_or_reassigned() {
    let mut chain = Pooled::new(POS);
    let (_, alice_key) = wallet();
    let (mallory, _) = wallet();
    chain.forge(&alice_key).unwrap();

    let mut blocks = chain.blocks().to_vec();
    blocks[1].validator = Some(mallory.clone());
    blocks[1].hash = blocks[1].computed_hash();
    assert_eq!(
        validate_blocks(&blocks, &POS),
        Err(ChainError::InvalidBlock { height: 1, reason: BlockError::BadSignature }),
        "signature no longer matches the validator"
    );

    let mut blocks = chain.blocks().to_vec();
    blocks[1].signature = None;
    assert_eq!(
        validate_blocks(&blocks, &POS),
        Err(ChainError::InvalidBlock { height: 1, reason: BlockError::BadSignature })
    );
}

#[test]
fn only_the_selected_validator_may_forge_on_a_replayed_chain() {
    let mut chain = Pooled::new(POS);
    let (alice, alice_key) = wallet();
    let (_, bob_key) = wallet();
    chain.forge(&alice_key).unwrap();
    chain.submit(stake(&alice_key, 10, 0)).unwrap();
    chain.forge(&alice_key).unwrap();
    assert_eq!(chain.next_validator(), Some(alice.clone()));

    // Bob forges a block out of turn by going around `forge`.
    let mut blocks = chain.blocks().to_vec();
    let reward = Transaction::coinbase(&Address::of(&bob_key), Coins::new(50), 3);
    let bob_block =
        Block::forge(3, Utc::now(), blocks[2].hash.clone(), vec![reward], &bob_key, None);
    blocks.push(bob_block);
    assert_eq!(
        validate_blocks(&blocks, &POS),
        Err(ChainError::InvalidBlock {
            height: 3,
            reason: BlockError::NotYourTurn { expected: alice }
        })
    );
}

#[test]
fn a_short_fork_backed_by_stake_beats_a_long_one_nobody_staked() {
    let (_, alice_key) = wallet();
    let (_, nobody_key) = wallet();

    // Four blocks anyone could forge, with nothing staked: weight 4.
    let mut unstaked = Pooled::new(POS);
    for _ in 0..4 {
        unstaked.forge(&nobody_key).unwrap();
    }

    // Three blocks, the last forged by Alice with 30 staked: weight 1 + 1 + 30.
    let mut staked = Pooled::new(POS);
    staked.forge(&alice_key).unwrap();
    staked.submit(stake(&alice_key, 30, 0)).unwrap();
    staked.forge(&alice_key).unwrap();
    staked.forge(&alice_key).unwrap();

    assert_eq!(unstaked.weight().value(), 4);
    assert_eq!(staked.weight().value(), 32);
    assert_eq!(unstaked.adopt_if_heavier(staked.blocks().to_vec()), Ok(true));
    assert_eq!(unstaked.blocks().len(), 4, "the shorter chain won");
}

#[test]
fn a_proposal_signed_elsewhere_is_appended_like_a_forged_block() {
    let mut chain = Pooled::new(POS);
    let (_, alice_key) = wallet();
    let (bob, bob_key) = wallet();
    chain.forge(&alice_key).unwrap();
    chain.submit(Transaction::signed(&alice_key, &bob, Coins::new(5), 0, Coins::new(2))).unwrap();

    let before = chain.blocks().len();
    let mut block = chain.propose(&bob, None).unwrap();
    assert_eq!(chain.blocks().len(), before, "proposing changes nothing");
    assert_eq!(block.signature, None);
    block.sign(&bob_key);

    chain.append_forged(block).unwrap();
    assert!(chain.pending().is_empty(), "the confirmed transfer left the mempool");
    assert_eq!(chain.balance(&bob), Coins::new(5 + 52));
    assert!(chain.is_valid());
}

#[test]
fn forged_submissions_are_checked() {
    let mut chain = Pooled::new(POS);
    let (alice, alice_key) = wallet();
    let (_, mallory_key) = wallet();
    chain.forge(&alice_key).unwrap();
    chain.submit(stake(&alice_key, 10, 0)).unwrap();
    chain.forge(&alice_key).unwrap();

    let proposal = chain.propose(&alice, reveal_for(&chain, &alice_key)).unwrap();

    let mut unsigned = proposal.clone();
    assert_eq!(chain.append_forged(unsigned.clone()), Err(BlockError::BadSignature));
    unsigned.sign(&mallory_key);
    assert_eq!(chain.append_forged(unsigned).err(), Some(BlockError::BadSignature));

    let mut padded = proposal.clone();
    padded.transactions[0].amount =
        padded.transactions[0].amount.checked_add(Coins::new(1)).unwrap();
    padded.hash = padded.computed_hash();
    padded.sign(&alice_key);
    assert_eq!(chain.append_forged(padded).err(), Some(BlockError::MerkleRootMismatch));

    let mut signed = proposal;
    signed.sign(&alice_key);
    chain.append_forged(signed.clone()).unwrap();
    assert_eq!(chain.append_forged(signed).err(), Some(BlockError::StaleBlock));
    assert!(chain.is_valid());
}

#[test]
fn only_the_selected_validator_gets_a_proposal() {
    let mut chain = Pooled::new(POS);
    let (_, alice_key) = wallet();
    let (bob, _) = wallet();
    chain.forge(&alice_key).unwrap();
    chain.submit(stake(&alice_key, 10, 0)).unwrap();
    chain.forge(&alice_key).unwrap();
    assert!(matches!(chain.propose(&bob, None), Err(BlockError::NotYourTurn { .. })));
    let pow = Pooled::new(RULES);
    assert_eq!(pow.propose(&bob, None).err(), Some(BlockError::NotProofOfStake));
}

// ---- Validator randomness ----------------------------------------------------------------

/// A chain where Alice and Bob have both staked and validators are being drawn.
fn staked_chain() -> (Pooled, [SigningKey; 2]) {
    let mut chain = Pooled::new(POS);
    let (_, alice_key) = wallet();
    let (_, bob_key) = wallet();
    chain.forge(&alice_key).unwrap();
    chain.forge(&bob_key).unwrap();
    chain.submit(stake(&alice_key, 30, 0)).unwrap();
    chain.submit(stake(&bob_key, 30, 0)).unwrap();
    chain.forge(&alice_key).unwrap();
    (chain, [alice_key, bob_key])
}

fn key_of(keys: &[SigningKey; 2], address: &Address) -> SigningKey {
    keys.iter().find(|k| Address::of(k) == *address).unwrap().clone()
}

#[test]
fn a_forger_cannot_steer_who_forges_next() {
    let (chain, keys) = staked_chain();
    let forger = key_of(&keys, &chain.next_validator().unwrap());
    let (_, carol_key) = wallet();

    // The same forger tries many versions of their block: other timestamps, other contents.
    let mut next = std::collections::HashSet::new();
    let mut tips = std::collections::HashSet::new();
    for i in 0..20 {
        let mut attempt = chain.clone();
        if i % 2 == 1 {
            // Small enough for either validator: each holds at least 20 unstaked.
            let to = Address::of(&carol_key);
            attempt
                .submit(Transaction::signed(&forger, &to, Coins::new(1), 1, Coins::new(i / 2)))
                .unwrap();
        }
        attempt.forge(&forger).unwrap();
        tips.insert(attempt.tip().hash.clone());
        next.insert(attempt.next_validator());
    }
    assert_eq!(tips.len(), 20, "every attempt made a different block");
    assert_eq!(next.len(), 1, "but none of them changed who's next");
}

#[test]
fn a_reveal_must_open_the_validators_commitment() {
    let (mut chain, keys) = staked_chain();
    let validator = chain.next_validator().unwrap();
    let key = key_of(&keys, &validator);
    let due = reveal_for(&chain, &key).unwrap();

    assert_eq!(chain.propose(&validator, None).err(), Some(BlockError::BadReveal));
    assert_eq!(chain.propose(&validator, Some(Reveal::zero())).err(), Some(BlockError::BadReveal));
    let skipped_ahead = randao::reveal(&key, 0, randao::LENGTH - 2);
    assert_eq!(chain.propose(&validator, Some(skipped_ahead)).err(), Some(BlockError::BadReveal));

    // A validly signed block with a made-up reveal still fails validation.
    let (index, txs) = chain.next_contents(&validator);
    let mut forged = Block::forge(index, Utc::now(), chain.tip().hash.clone(), txs, &key, None);
    forged.reveal = Some(Reveal::of(b"a link of nobody's chain"));
    forged.hash = forged.computed_hash();
    forged.sign(&key);
    assert_eq!(chain.append_forged(forged).err(), Some(BlockError::BadReveal));

    let beacon = chain.beacon().clone();
    let mut block = chain.propose(&validator, Some(due.clone())).unwrap();
    block.sign(&key);
    chain.append_forged(block).unwrap();
    assert_eq!(*chain.beacon(), randao::mix(&beacon, &due));
    assert_eq!(chain.validator_state(&validator).reveals_left, Some(randao::LENGTH - 1));
    assert!(chain.is_valid());
}

#[test]
fn only_a_first_stake_commits() {
    let mut chain = Pooled::new(POS);
    let (alice, alice_key) = wallet();
    chain.forge(&alice_key).unwrap();
    chain.forge(&alice_key).unwrap();

    assert_eq!(
        chain.submit(Transaction::stake(&alice_key, Coins::new(10), 0, Coins::new(0), None)),
        Err(TransactionError::MissingCommitment)
    );
    assert_eq!(chain.commitment_due(&alice), Some(0));
    chain.submit(stake(&alice_key, 10, 0)).unwrap();
    assert_eq!(chain.commitment_due(&alice), None, "the waiting stake already commits");
    assert_eq!(
        chain.submit(Transaction::stake(
            &alice_key,
            Coins::new(5),
            1,
            Coins::ZERO,
            Some(randao::commitment(&alice_key, 1))
        )),
        Err(TransactionError::UnexpectedCommitment)
    );
    chain.submit(Transaction::stake(&alice_key, Coins::new(5), 1, Coins::new(0), None)).unwrap();
    chain.forge(&alice_key).unwrap();
    assert_eq!(chain.staked(&alice), Coins::new(15));

    // Unstaking everything ends the chain; staking again needs commitment number 1.
    chain
        .submit(Transaction::signed(
            &alice_key,
            Destination::Unstake,
            Coins::new(15),
            2,
            Coins::new(0),
        ))
        .unwrap();
    chain.forge(&alice_key).unwrap();
    assert_eq!(chain.validator_state(&alice).reveals_left, None);
    assert_eq!(chain.commitment_due(&alice), Some(1));
    let again = Transaction::stake(
        &alice_key,
        Coins::new(10),
        3,
        Coins::new(0),
        Some(randao::commitment(&alice_key, 1)),
    );
    chain.submit(again).unwrap();
    chain.forge(&alice_key).unwrap();
    chain.forge(&alice_key).unwrap();
    assert!(chain.is_valid());
}

#[test]
fn the_beacon_only_moves_with_reveals() {
    let mut pow = chain();
    let genesis_beacon = pow.beacon().clone();
    pow.mine(&wallet().0).unwrap();
    assert_eq!(*pow.beacon(), genesis_beacon, "mining reveals nothing");

    let mut pos = Pooled::new(POS);
    let start = pos.beacon().clone();
    pos.forge(&wallet().1).unwrap();
    assert_eq!(*pos.beacon(), start, "nobody can steer it while anyone may forge");
}

// ---- Fees -----------------------------------------------------------------------------

#[test]
fn fees_go_to_whoever_adds_the_block() {
    let mut chain = chain();
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    let (carol, _) = wallet();
    chain.mine(&alice).unwrap();
    chain.submit(Transaction::signed(&alice_key, &bob, Coins::new(20), 0, Coins::new(3))).unwrap();

    let block = chain.mine(&carol).unwrap();
    assert_eq!(block.transactions[0].amount, Coins::new(53));
    assert_eq!(chain.balance(&alice), Coins::new(27));
    assert_eq!(chain.balance(&bob), Coins::new(20));
    assert_eq!(chain.balance(&carol), Coins::new(53));
    assert!(chain.is_valid());
}

#[test]
fn the_fee_must_be_affordable_too() {
    let mut chain = chain();
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    chain.mine(&alice).unwrap();
    assert_eq!(
        chain.submit(Transaction::signed(&alice_key, &bob, Coins::new(50), 0, Coins::new(1))),
        Err(TransactionError::InsufficientFunds {
            balance: Coins::new(50),
            amount: Coins::new(51)
        })
    );
}

#[test]
fn fees_are_signed_and_counted_in_the_payout() {
    let mut chain = chain();
    let (alice, alice_key) = wallet();
    let (bob, _) = wallet();
    chain.mine(&alice).unwrap();
    chain.submit(Transaction::signed(&alice_key, &bob, Coins::new(5), 0, Coins::new(2))).unwrap();
    chain.mine(&alice).unwrap();

    // A peer rewrites the block and does all the work again, so only the rules can catch it.
    let remined = |change: fn(&mut Vec<Transaction>)| {
        let mut blocks = chain.blocks().to_vec();
        let mut txs = blocks[2].transactions.clone();
        change(&mut txs);
        let (prev, at) = (blocks[1].hash.clone(), blocks[2].timestamp);
        blocks[2] = Block::mine(2, at, prev, txs, RULES.initial_work());
        validate_blocks(&blocks, &RULES)
    };
    let refused = |reason| Err(ChainError::InvalidBlock { height: 2, reason });
    assert_eq!(
        remined(|txs| {
            txs[1].fee = Coins::new(0);
            txs[0].amount = Coins::new(50);
        }),
        refused(BlockError::BadTransfer { position: 1, reason: TransactionError::BadSignature }),
        "the signature covers the fee"
    );
    assert_eq!(
        remined(|txs| txs[0].amount = Coins::new(60)),
        refused(BlockError::WrongPayout { expected: Some(Coins::new(52)), got: Coins::new(60) }),
        "the payout must be reward plus fees"
    );
}

#[test]
fn fee_free_transactions_and_mined_blocks_keep_their_original_json() {
    let mut chain = chain();
    let (alice, alice_key) = wallet();
    chain.mine(&alice).unwrap();
    let json = serde_json::to_value(transfer(&alice_key, &alice, 1, 0)).unwrap();
    assert!(json.get("fee").is_none());
    let json = serde_json::to_value(chain.tip()).unwrap();
    assert!(json.get("validator").is_none() && json.get("signature").is_none());
}

// ---- Difficulty adjustment -----------------------------------------------------------

/// Starts at 16 hashes a block, aiming for one block every 10s, adjusting every 5 blocks.
const ADJUSTING: ChainRules = ChainRules::proof_of_work(1, 50).with_retarget(10, 5);

/// Mines `count` blocks `secs` apart, starting where the chain's tip left off.
fn mine_paced(chain: &mut Pooled, count: usize, secs: i64) {
    let miner = wallet().0;
    for _ in 0..count {
        let last = chain.tip().timestamp.max(Utc::now() - chrono::TimeDelta::days(1));
        chain.mine_at(&miner, last + chrono::TimeDelta::seconds(secs)).unwrap();
    }
}

#[test]
fn blocks_that_come_too_fast_make_mining_harder_and_too_slow_easier() {
    let mut chain = Pooled::new(ADJUSTING);
    mine_paced(&mut chain, 8, 1);
    assert_eq!(chain.next_work(), 16, "no adjustment until height 10: genesis isn't timed");
    mine_paced(&mut chain, 1, 1);
    assert_eq!(chain.next_work(), 64, "1s blocks against a 10s target: 4x harder, the cap");

    mine_paced(&mut chain, 5, 100);
    assert_eq!(chain.next_work(), 16, "100s blocks: 4x easier, the cap");
    assert!(chain.is_valid());

    let history = chain.work_history();
    assert_eq!(history[..10], [16; 10]);
    assert_eq!(history[10..15], [64; 5]);
    let total: u128 = history[1..].iter().sum();
    assert_eq!(chain.weight().value(), total, "weight is the work each block required");
    let restored = Pooled::restore(chain.blocks().to_vec(), ADJUSTING).unwrap();
    assert_eq!(restored.next_work(), chain.next_work());
}

#[test]
fn the_adjustment_is_proportional_inside_the_cap() {
    let mut chain = Pooled::new(ADJUSTING);
    mine_paced(&mut chain, 9, 5);
    assert_eq!(chain.next_work(), 32, "twice as fast as the target: twice the work");
    mine_paced(&mut chain, 5, 20);
    assert_eq!(chain.next_work(), 16, "twice as slow: half the work");
    assert_eq!(chain.next_retarget_height(), Some(15), "and the next block is adjusted again");
}

#[test]
fn a_block_below_the_required_work_is_rejected() {
    let mut chain = Pooled::new(ADJUSTING);
    mine_paced(&mut chain, 10, 1);
    assert_eq!(chain.next_work(), 64);

    // Mine at the old, easier work until we get a hash that misses the new one.
    let (index, txs) = chain.next_contents(&wallet().0);
    let at = chain.tip().timestamp + chrono::TimeDelta::seconds(1);
    let easy = (0..)
        .map(|i| {
            Block::mine(
                index,
                at + chrono::TimeDelta::nanoseconds(i),
                chain.tip().hash.clone(),
                txs.clone(),
                16,
            )
        })
        .find(|b| !meets_work(b.hash.as_str(), 64))
        .unwrap();
    let mut blocks = chain.blocks().to_vec();
    blocks.push(easy);
    assert_eq!(
        validate_blocks(&blocks, &ADJUSTING),
        Err(ChainError::InvalidBlock { height: 11, reason: BlockError::InsufficientWork })
    );
}

#[test]
fn timestamps_cannot_be_backdated_or_far_in_the_future() {
    let mut chain = Pooled::new(ADJUSTING);
    mine_paced(&mut chain, 12, 10);
    let miner = wallet().0;
    let median = chain.blocks()[chain.blocks().len() - 6].timestamp;
    assert_eq!(chain.mine_at(&miner, median).err(), Some(BlockError::TooEarly));
    let future = Utc::now() + chrono::TimeDelta::hours(3);
    assert_eq!(chain.mine_at(&miner, future).err(), Some(BlockError::TooFarAhead));
    chain.mine_at(&miner, median + chrono::TimeDelta::seconds(1)).unwrap();
}

#[test]
fn a_shorter_but_harder_fork_beats_a_longer_easier_one() {
    // 10 blocks a second apart push the work to 64, so 5 more weigh 320.
    let mut hard = Pooled::new(ADJUSTING);
    mine_paced(&mut hard, 15, 1);
    // 17 slow blocks: every block after the first adjustment needs a quarter of the work.
    let mut easy = Pooled::new(ADJUSTING);
    mine_paced(&mut easy, 17, 100);

    assert!(hard.blocks().len() < easy.blocks().len());
    assert!(hard.weight() > easy.weight());
    assert_eq!(easy.adopt_if_heavier(hard.blocks().to_vec()), Ok(true));
}
