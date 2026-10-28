use std::cmp::Reverse;

use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use super::address::Address;
use super::block::{Block, Transaction};
use super::crypto;
use super::error::ChainError;
use super::ledger::Ledger;
use super::mempool::Mempool;
use super::rules::{meets_difficulty, select_validator, ChainRules, Consensus};
use super::weight::ChainWeight;

/// The aggregate root: confirmed blocks, the mempool and the ledger they produce.
/// Every change goes through its methods, so the three never disagree.
#[derive(Clone)]
pub struct Blockchain {
    blocks: Vec<Block>,
    mempool: Mempool,
    /// State after every confirmed block.
    ledger: Ledger,
    /// What the blocks are worth when forks compete.
    weight: ChainWeight,
    rules: ChainRules,
}

impl Blockchain {
    pub fn new(rules: ChainRules) -> Self {
        Self {
            blocks: vec![Block::genesis(&rules)],
            mempool: Mempool::default(),
            ledger: Ledger::default(),
            weight: ChainWeight::ZERO,
            rules,
        }
    }

    /// Rebuilds a chain from saved blocks. They are replayed in full, so tampered ones are refused.
    pub fn restore(blocks: Vec<Block>, rules: ChainRules) -> Result<Self, ChainError> {
        let (ledger, weight) = replay(&blocks, &rules).ok_or(ChainError::InvalidChain)?;
        Ok(Self { blocks, mempool: Mempool::default(), ledger, weight, rules })
    }

    pub fn rules(&self) -> ChainRules {
        self.rules
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn weight(&self) -> ChainWeight {
        self.weight
    }

    pub fn tip(&self) -> &Block {
        self.blocks.last().expect("a chain always has its genesis block")
    }

    pub fn pending(&self) -> &[Transaction] {
        self.mempool.transactions()
    }

    pub fn balance(&self, address: &Address) -> u64 {
        self.ledger.balance(address.as_str())
    }

    pub fn staked(&self, address: &Address) -> u64 {
        self.ledger.staked(address.as_str())
    }

    /// Every confirmed stake, sorted by address.
    pub fn stakes(&self) -> Vec<(String, u64)> {
        self.ledger.stakes()
    }

    /// The nonce a new transaction from `address` must carry, counting the mempool.
    pub fn next_nonce(&self, address: &Address) -> u64 {
        self.ledger.next_nonce(address.as_str()) + self.mempool.sent_by(address.as_str())
    }

    /// Who must forge the next block. `None` on proof-of-work chains, and on proof-of-stake chains
    /// where nothing is staked yet, when anyone may forge.
    pub fn next_validator(&self) -> Option<Address> {
        if !self.rules.is_proof_of_stake() {
            return None;
        }
        let selected = select_validator(&self.ledger.stakes(), &self.tip().hash)?;
        Some(Address::parse(&selected).expect("only valid senders can stake"))
    }

    /// Validates a transaction against confirmed state plus the mempool, then queues it.
    pub fn submit(&mut self, tx: Transaction) -> Result<(), ChainError> {
        self.mempool.admit(tx, &self.ledger)
    }

    /// Proof of work: bundles the reward and every pending transaction into a new block.
    pub fn mine(&mut self, miner: &Address) -> Result<&Block, ChainError> {
        let Consensus::ProofOfWork { difficulty } = self.rules.consensus else {
            return Err(ChainError::NotProofOfWork);
        };
        self.append(miner, |index, now, prev, txs| Block::mine(index, now, prev, txs, difficulty))
    }

    /// Proof of stake: the selected validator signs the next block with `key`.
    pub fn forge(&mut self, key: &SigningKey) -> Result<&Block, ChainError> {
        if !self.rules.is_proof_of_stake() {
            return Err(ChainError::NotProofOfStake);
        }
        let validator = Address::of(key);
        if let Some(expected) = self.next_validator() {
            if expected != validator {
                return Err(ChainError::NotYourTurn { expected: expected.to_string() });
            }
        }
        self.append(&validator, |index, now, prev, txs| Block::forge(index, now, prev, txs, key))
    }

    /// Pays `producer` the reward plus fees, applies the mempool and appends the block `seal` makes.
    fn append(
        &mut self,
        producer: &Address,
        seal: impl FnOnce(u64, DateTime<Utc>, String, Vec<Transaction>) -> Block,
    ) -> Result<&Block, ChainError> {
        let index = self.blocks.len() as u64;
        let pending = self.mempool.transactions();
        let payout = self.rules.reward + pending.iter().map(|tx| tx.fee).sum::<u64>();
        let mut transactions = vec![Transaction::coinbase(producer, payout, index)];
        transactions.extend(pending.iter().cloned());

        let mut ledger = self.ledger.clone();
        ledger.credit(producer.as_str(), payout);
        for tx in &transactions[1..] {
            ledger.apply_transfer(tx)?;
        }

        let prev_hash = self.tip().hash.clone();
        self.blocks.push(seal(index, Utc::now(), prev_hash, transactions));
        self.weight = self.weight.with_block(&self.rules, self.ledger.staked(producer.as_str()));
        self.mempool.clear();
        self.ledger = ledger;
        Ok(self.tip())
    }

    /// The fork-choice rule: adopts `blocks` if they form a valid chain heavier than ours. Exact
    /// ties go to the lower tip hash, so every node settles on the same fork. Never undoes more
    /// than `max_reorg_depth` of our blocks. Transfers from our blocks that `blocks` drops go back
    /// to the mempool if they still apply.
    pub fn adopt_if_heavier(&mut self, blocks: Vec<Block>) -> Result<bool, ChainError> {
        if blocks.last().is_some_and(|tip| tip.hash == self.tip().hash) {
            return Ok(false);
        }
        let (ledger, weight) = replay(&blocks, &self.rules).ok_or(ChainError::InvalidChain)?;
        let theirs = (weight, Reverse(&blocks.last().expect("replay refuses empty chains").hash));
        if theirs <= (self.weight, Reverse(&self.tip().hash)) {
            return Ok(false);
        }

        let shared = self
            .blocks
            .iter()
            .zip(&blocks)
            .take_while(|(ours, theirs)| ours.hash == theirs.hash)
            .count();
        let depth = self.blocks.len() - shared;
        if depth as u64 > self.rules.max_reorg_depth {
            return Err(ChainError::ReorgTooDeep { depth, max: self.rules.max_reorg_depth });
        }
        let orphaned: Vec<Transaction> = self.blocks[shared..]
            .iter()
            .flat_map(|block| block.transactions.iter().skip(1))
            .chain(self.mempool.transactions())
            .cloned()
            .collect();

        self.mempool = Mempool::rebuilt(orphaned, &ledger);
        self.blocks = blocks;
        self.ledger = ledger;
        self.weight = weight;
        Ok(true)
    }

    /// Replays the whole chain: links, hashes, consensus, rewards, signatures and balances.
    pub fn is_valid(&self) -> bool {
        validate_blocks(&self.blocks, &self.rules)
    }
}

pub fn validate_blocks(blocks: &[Block], rules: &ChainRules) -> bool {
    replay(blocks, rules).is_some()
}

/// Checks every block in order and returns the resulting ledger and the chain's weight, or `None`
/// if any check fails.
fn replay(blocks: &[Block], rules: &ChainRules) -> Option<(Ledger, ChainWeight)> {
    let genesis = blocks.first()?;
    if genesis.index != 0
        || genesis.prev_hash != "0".repeat(64)
        || genesis.timestamp.timestamp() != 0
        || !genesis.transactions.is_empty()
        || genesis.hash != genesis.computed_hash()
        || !is_sealed_genesis(genesis, rules)
    {
        return None;
    }
    let mut ledger = Ledger::default();
    let mut weight = ChainWeight::ZERO;
    for (prev, block) in blocks.iter().zip(&blocks[1..]) {
        if block.index != prev.index + 1
            || block.prev_hash != prev.hash
            || block.hash != block.computed_hash()
            || !is_sealed_by_consensus(block, prev, &ledger, rules)
        {
            return None;
        }
        let (coinbase, transfers) = block.transactions.split_first()?;
        if !coinbase.is_coinbase()
            || coinbase.amount != rules.reward + transfers.iter().map(|tx| tx.fee).sum::<u64>()
            || coinbase.fee != 0
            || coinbase.nonce != block.index
            || !crypto::is_valid_address(&coinbase.to)
            || block.validator.as_ref().is_some_and(|v| *v != coinbase.to)
        {
            return None;
        }
        weight = weight.with_block(rules, ledger.staked(&coinbase.to));
        ledger.credit(&coinbase.to, coinbase.amount);
        if transfers.iter().any(|tx| ledger.apply_transfer(tx).is_err()) {
            return None;
        }
    }
    Some((ledger, weight))
}

/// Genesis is never signed. Under proof of work it still needs the work.
fn is_sealed_genesis(genesis: &Block, rules: &ChainRules) -> bool {
    let unsigned = genesis.validator.is_none() && genesis.signature.is_none();
    match rules.consensus {
        Consensus::ProofOfWork { difficulty } => {
            unsigned && meets_difficulty(&genesis.hash, difficulty)
        }
        Consensus::ProofOfStake => unsigned && genesis.nonce == 0,
    }
}

/// Proof of work: the hash has enough zeros and nobody signed it. Proof of stake: the validator
/// selected from the stakes before this block signed it.
fn is_sealed_by_consensus(
    block: &Block,
    prev: &Block,
    ledger: &Ledger,
    rules: &ChainRules,
) -> bool {
    match rules.consensus {
        Consensus::ProofOfWork { difficulty } => {
            block.validator.is_none()
                && block.signature.is_none()
                && meets_difficulty(&block.hash, difficulty)
        }
        Consensus::ProofOfStake => {
            block.nonce == 0
                && block.is_signed_by_validator()
                && select_validator(&ledger.stakes(), &prev.hash)
                    .is_none_or(|expected| block.validator.as_deref() == Some(&expected))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::block::{STAKE, UNSTAKE};

    const RULES: ChainRules = ChainRules::proof_of_work(2, 50);

    fn wallet() -> (Address, SigningKey) {
        let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
        (Address::of(&key), key)
    }

    fn transfer(key: &SigningKey, to: &Address, amount: u64, nonce: u64) -> Transaction {
        Transaction::signed(key, to.as_str(), amount, nonce, 0)
    }

    fn chain() -> Blockchain {
        Blockchain::new(RULES)
    }

    #[test]
    fn mining_pays_the_reward() {
        let mut chain = chain();
        let (miner, _) = wallet();
        let block = chain.mine(&miner).unwrap();
        assert!(block.hash.starts_with("00"));
        assert_eq!(chain.balance(&miner), 50);
        assert!(chain.is_valid());
    }

    #[test]
    fn transfer_moves_coins_once_mined() {
        let mut chain = chain();
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        chain.mine(&alice).unwrap();

        chain.submit(transfer(&alice_key, &bob, 20, 0)).unwrap();
        assert_eq!(chain.balance(&bob), 0);
        assert_eq!(chain.next_nonce(&alice), 1);

        chain.mine(&bob).unwrap();
        assert_eq!(chain.balance(&alice), 30);
        assert_eq!(chain.balance(&bob), 70);
        assert!(chain.pending().is_empty());
        assert!(chain.is_valid());
    }

    #[test]
    fn rejects_forged_signature() {
        let mut chain = chain();
        let (alice, _) = wallet();
        let (_, mallory_key) = wallet();
        chain.mine(&alice).unwrap();

        let mut tx = transfer(&mallory_key, &Address::of(&mallory_key), 10, 0);
        tx.from = alice.to_string();
        assert_eq!(chain.submit(tx), Err(ChainError::BadSignature));
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
            Err(ChainError::InsufficientFunds { balance: 10, amount: 20 })
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
        assert_eq!(chain.submit(tx), Err(ChainError::BadNonce { expected: 1, got: 0 }));
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
        blocks[2].transactions[1].amount = 50;
        assert!(!validate_blocks(&blocks, &RULES));

        let mut blocks = chain.blocks().to_vec();
        blocks[1].transactions[0].amount = 1_000;
        blocks[1].hash = blocks[1].computed_hash();
        assert!(!validate_blocks(&blocks, &RULES));
    }

    #[test]
    fn restore_rebuilds_balances() {
        let mut chain = chain();
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        chain.mine(&alice).unwrap();
        chain.submit(transfer(&alice_key, &bob, 20, 0)).unwrap();
        chain.mine(&alice).unwrap();

        let restored = Blockchain::restore(chain.blocks().to_vec(), RULES).unwrap();
        assert_eq!(restored.balance(&alice), 80);
        assert_eq!(restored.balance(&bob), 20);
        assert_eq!(restored.next_nonce(&alice), 1);
    }

    #[test]
    fn restore_refuses_tampered_or_empty_blocks() {
        let mut chain = chain();
        chain.mine(&wallet().0).unwrap();
        let mut blocks = chain.blocks().to_vec();
        blocks[1].transactions[0].amount = 1_000;
        assert_eq!(Blockchain::restore(blocks, RULES).err(), Some(ChainError::InvalidChain));
        assert_eq!(Blockchain::restore(Vec::new(), RULES).err(), Some(ChainError::InvalidChain));
    }

    #[test]
    fn restore_refuses_different_rules() {
        let mut chain = chain();
        chain.mine(&wallet().0).unwrap();
        // No SHA-256 hash is all zeros, so these blocks can't meet a difficulty of 64.
        assert!(Blockchain::restore(chain.blocks().to_vec(), ChainRules::proof_of_work(64, 50))
            .is_err());
        assert!(
            Blockchain::restore(chain.blocks().to_vec(), ChainRules::proof_of_work(2, 25)).is_err()
        );
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
        assert_eq!(ours.balance(&alice), 0);
        assert_eq!(ours.balance(&bob), 100);
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
    fn weight_grows_with_every_block() {
        let mut chain = chain();
        assert_eq!(chain.weight(), ChainWeight::ZERO, "genesis weighs nothing");
        chain.mine(&wallet().0).unwrap();
        chain.mine(&wallet().0).unwrap();
        assert_eq!(chain.weight().value(), 2 * 256);
        let restored = Blockchain::restore(chain.blocks().to_vec(), RULES).unwrap();
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
        let rules = RULES.with_max_reorg_depth(2);
        let mut ours = Blockchain::new(rules);
        let mut theirs = Blockchain::new(rules);
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

        let mut shallow = Blockchain::new(rules);
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
        forged[2].transactions[0].amount = 1_000;

        assert_eq!(ours.adopt_if_heavier(forged), Err(ChainError::InvalidChain));
        assert_eq!(ours.balance(&alice), 50);
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
        assert_eq!(ours.balance(&alice), 50);
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
        assert_eq!(ours.balance(&bob), 120);
    }

    // ---- Proof of stake --------------------------------------------------------------------

    const POS: ChainRules = ChainRules::proof_of_stake(50);

    fn stake(key: &SigningKey, amount: u64, nonce: u64) -> Transaction {
        Transaction::signed(key, STAKE, amount, nonce, 0)
    }

    #[test]
    fn anyone_may_forge_until_something_is_staked() {
        let mut chain = Blockchain::new(POS);
        let (alice, alice_key) = wallet();
        assert_eq!(chain.next_validator(), None);

        let block = chain.forge(&alice_key).unwrap();
        assert_eq!(block.nonce, 0);
        assert_eq!(block.validator.as_deref(), Some(alice.as_str()));
        assert!(block.is_signed_by_validator());
        assert_eq!(chain.balance(&alice), 50);
        assert!(chain.is_valid());
    }

    #[test]
    fn staking_locks_coins_and_makes_a_validator() {
        let mut chain = Blockchain::new(POS);
        let (alice, alice_key) = wallet();
        let (_, bob_key) = wallet();
        chain.forge(&alice_key).unwrap();
        chain.submit(stake(&alice_key, 30, 0)).unwrap();
        chain.forge(&bob_key).unwrap();

        assert_eq!(chain.balance(&alice), 20);
        assert_eq!(chain.staked(&alice), 30);
        assert_eq!(chain.next_validator(), Some(alice.clone()));
        assert_eq!(
            chain.forge(&bob_key).err(),
            Some(ChainError::NotYourTurn { expected: alice.to_string() })
        );
        chain.forge(&alice_key).unwrap();
        assert!(chain.is_valid());
    }

    #[test]
    fn unstaking_returns_coins_but_never_more_than_staked() {
        let mut chain = Blockchain::new(POS);
        let (alice, alice_key) = wallet();
        chain.forge(&alice_key).unwrap();
        chain.submit(stake(&alice_key, 30, 0)).unwrap();
        chain.forge(&alice_key).unwrap();

        assert_eq!(
            chain.submit(Transaction::signed(&alice_key, UNSTAKE, 31, 1, 0)),
            Err(ChainError::InsufficientStake { staked: 30, amount: 31 })
        );
        chain.submit(Transaction::signed(&alice_key, UNSTAKE, 30, 1, 0)).unwrap();
        chain.forge(&alice_key).unwrap();
        assert_eq!(chain.staked(&alice), 0);
        assert_eq!(chain.balance(&alice), 150);
    }

    #[test]
    fn each_consensus_refuses_the_other_ones_blocks() {
        let (alice, alice_key) = wallet();
        assert_eq!(Blockchain::new(POS).mine(&alice).err(), Some(ChainError::NotProofOfWork));
        assert_eq!(chain().forge(&alice_key).err(), Some(ChainError::NotProofOfStake));

        let mut pos = Blockchain::new(POS);
        pos.forge(&alice_key).unwrap();
        assert!(Blockchain::restore(pos.blocks().to_vec(), RULES).is_err());
        let mut pow = chain();
        pow.mine(&alice).unwrap();
        assert!(Blockchain::restore(pow.blocks().to_vec(), POS).is_err());
    }

    #[test]
    fn forged_blocks_cannot_be_tampered_with_or_reassigned() {
        let mut chain = Blockchain::new(POS);
        let (_, alice_key) = wallet();
        let (mallory, _) = wallet();
        chain.forge(&alice_key).unwrap();

        let mut blocks = chain.blocks().to_vec();
        blocks[1].validator = Some(mallory.to_string());
        blocks[1].hash = blocks[1].computed_hash();
        assert!(!validate_blocks(&blocks, &POS), "signature no longer matches the validator");

        let mut blocks = chain.blocks().to_vec();
        blocks[1].signature = None;
        assert!(!validate_blocks(&blocks, &POS));
    }

    #[test]
    fn only_the_selected_validator_may_forge_on_a_replayed_chain() {
        let mut chain = Blockchain::new(POS);
        let (alice, alice_key) = wallet();
        let (_, bob_key) = wallet();
        chain.forge(&alice_key).unwrap();
        chain.submit(stake(&alice_key, 10, 0)).unwrap();
        chain.forge(&alice_key).unwrap();
        assert_eq!(chain.next_validator(), Some(alice));

        // Bob forges a block out of turn by going around `forge`.
        let mut blocks = chain.blocks().to_vec();
        let reward = Transaction::coinbase(&Address::of(&bob_key), 50, 3);
        blocks.push(Block::forge(3, Utc::now(), blocks[2].hash.clone(), vec![reward], &bob_key));
        assert!(!validate_blocks(&blocks, &POS));
    }

    #[test]
    fn a_short_fork_backed_by_stake_beats_a_long_one_nobody_staked() {
        let (_, alice_key) = wallet();
        let (_, nobody_key) = wallet();

        // Four blocks anyone could forge, with nothing staked: weight 4.
        let mut unstaked = Blockchain::new(POS);
        for _ in 0..4 {
            unstaked.forge(&nobody_key).unwrap();
        }

        // Three blocks, the last forged by Alice with 30 staked: weight 1 + 1 + 30.
        let mut staked = Blockchain::new(POS);
        staked.forge(&alice_key).unwrap();
        staked.submit(stake(&alice_key, 30, 0)).unwrap();
        staked.forge(&alice_key).unwrap();
        staked.forge(&alice_key).unwrap();

        assert_eq!(unstaked.weight().value(), 4);
        assert_eq!(staked.weight().value(), 32);
        assert_eq!(unstaked.adopt_if_heavier(staked.blocks().to_vec()), Ok(true));
        assert_eq!(unstaked.blocks().len(), 4, "the shorter chain won");
    }

    // ---- Fees -----------------------------------------------------------------------------

    #[test]
    fn fees_go_to_whoever_adds_the_block() {
        let mut chain = chain();
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        let (carol, _) = wallet();
        chain.mine(&alice).unwrap();
        chain.submit(Transaction::signed(&alice_key, bob.as_str(), 20, 0, 3)).unwrap();

        let block = chain.mine(&carol).unwrap();
        assert_eq!(block.transactions[0].amount, 53);
        assert_eq!(chain.balance(&alice), 27);
        assert_eq!(chain.balance(&bob), 20);
        assert_eq!(chain.balance(&carol), 53);
        assert!(chain.is_valid());
    }

    #[test]
    fn the_fee_must_be_affordable_too() {
        let mut chain = chain();
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        chain.mine(&alice).unwrap();
        assert_eq!(
            chain.submit(Transaction::signed(&alice_key, bob.as_str(), 50, 0, 1)),
            Err(ChainError::InsufficientFunds { balance: 50, amount: 51 })
        );
    }

    #[test]
    fn fees_are_signed_and_counted_in_the_payout() {
        let mut chain = chain();
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        chain.mine(&alice).unwrap();
        chain.submit(Transaction::signed(&alice_key, bob.as_str(), 5, 0, 2)).unwrap();
        chain.mine(&alice).unwrap();

        let mut blocks = chain.blocks().to_vec();
        blocks[2].transactions[1].fee = 0;
        blocks[2].hash = blocks[2].computed_hash();
        assert!(!validate_blocks(&blocks, &RULES), "the signature covers the fee");

        let mut blocks = chain.blocks().to_vec();
        blocks[2].transactions[0].amount = 60;
        assert!(!validate_blocks(&blocks, &RULES), "the payout must be reward plus fees");
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
}
