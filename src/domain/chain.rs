use std::cmp::Reverse;

use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use super::address::Address;
use super::block::{Block, Transaction};
use super::crypto;
use super::difficulty::{self, meets_work};
use super::error::ChainError;
use super::ledger::Ledger;
use super::ledger::ValidatorState;
use super::mempool::Mempool;
use super::randao;
use super::rules::{select_validator, ChainRules, Consensus};
use super::weight::ChainWeight;

/// The aggregate root: confirmed blocks, the mempool and the ledger they produce.
/// Every change goes through its methods, so the three never disagree.
#[derive(Clone)]
pub struct Blockchain {
    blocks: Vec<Block>,
    mempool: Mempool,
    /// What the confirmed blocks add up to.
    tally: Tally,
    rules: ChainRules,
}

/// What replaying blocks in order adds up to.
#[derive(Clone)]
struct Tally {
    /// Balances, stakes, nonces and the randomness beacon.
    ledger: Ledger,
    /// What the blocks are worth when forks compete.
    weight: ChainWeight,
    /// Proof of work: the work the latest block required, which the next one starts from.
    work: u128,
}

impl Tally {
    fn at_genesis(genesis: &Block, rules: &ChainRules) -> Self {
        Self {
            ledger: Ledger::starting_at(&genesis.hash),
            weight: ChainWeight::ZERO,
            work: rules.initial_work(),
        }
    }
}

impl Blockchain {
    pub fn new(rules: ChainRules) -> Self {
        let genesis = Block::genesis(&rules);
        let tally = Tally::at_genesis(&genesis, &rules);
        Self { mempool: Mempool::on(&tally.ledger), blocks: vec![genesis], tally, rules }
    }

    /// Rebuilds a chain from saved blocks. They are replayed in full, so tampered ones are refused.
    pub fn restore(blocks: Vec<Block>, rules: ChainRules) -> Result<Self, ChainError> {
        let tally = replay(&blocks, &rules).ok_or(ChainError::InvalidChain)?;
        Ok(Self { blocks, mempool: Mempool::on(&tally.ledger), tally, rules })
    }

    pub fn rules(&self) -> ChainRules {
        self.rules
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn weight(&self) -> ChainWeight {
        self.tally.weight
    }

    /// Proof of work: the hashes the next block should take to find, on average.
    pub fn next_work(&self) -> u128 {
        difficulty::next_work(&self.blocks, self.tally.work, self.rules.retarget())
    }

    /// Proof of work: the work each block required, by height. Genesis needs the starting work.
    pub fn work_history(&self) -> Vec<u128> {
        let mut work = self.rules.initial_work();
        let mut history = vec![work];
        for height in 1..self.blocks.len() {
            work = difficulty::next_work(&self.blocks[..height], work, self.rules.retarget());
            history.push(work);
        }
        history
    }

    /// Proof of work with retargeting: the height of the next block whose work is adjusted.
    pub fn next_retarget_height(&self) -> Option<u64> {
        let interval = self.rules.retarget()?.interval.max(1);
        let height = self.blocks.len() as u64;
        Some((height.div_ceil(interval) * interval).max(2 * interval))
    }

    pub fn tip(&self) -> &Block {
        self.blocks.last().expect("a chain always has its genesis block")
    }

    pub fn pending(&self) -> &[Transaction] {
        self.mempool.transactions()
    }

    pub fn balance(&self, address: &Address) -> u64 {
        self.tally.ledger.balance(address.as_str())
    }

    pub fn staked(&self, address: &Address) -> u64 {
        self.tally.ledger.staked(address.as_str())
    }

    /// Every confirmed stake, sorted by address.
    pub fn stakes(&self) -> Vec<(String, u64)> {
        self.tally.ledger.stakes()
    }

    /// The nonce a new transaction from `address` must carry, counting the mempool.
    pub fn next_nonce(&self, address: &Address) -> u64 {
        self.mempool.projected().next_nonce(address.as_str())
    }

    /// Who must forge the next block. `None` on proof-of-work chains, and on proof-of-stake chains
    /// where nobody with reveals left has staked, when anyone may forge.
    ///
    /// The draw is seeded by the randomness beacon, which only validators' committed reveals
    /// move. The block hash isn't involved, so a forger can't steer the next turn by trying
    /// different timestamps or contents.
    pub fn next_validator(&self) -> Option<Address> {
        if !self.rules.is_proof_of_stake() {
            return None;
        }
        let selected =
            select_validator(&self.tally.ledger.validator_stakes(), self.tally.ledger.beacon())?;
        Some(Address::parse(&selected).expect("only valid senders can stake"))
    }

    /// The randomness beacon the next validator is drawn from.
    pub fn beacon(&self) -> &str {
        self.tally.ledger.beacon()
    }

    /// Where `address` stands with its reveal chain.
    pub fn validator_state(&self, address: &Address) -> ValidatorState {
        self.tally.ledger.validator_state(address.as_str())
    }

    /// If `address`'s next stake must carry a commitment, the epoch to make it for. Counts stakes
    /// already waiting in the mempool, so a second stake in a row doesn't commit again.
    pub fn commitment_due(&self, address: &Address) -> Option<u64> {
        let state = self.mempool.projected().validator_state(address.as_str());
        state.reveals_left.is_none().then_some(state.next_epoch)
    }

    /// Validates a transaction against confirmed state plus the mempool, then queues it.
    pub fn submit(&mut self, tx: Transaction) -> Result<(), ChainError> {
        self.mempool.admit(tx)
    }

    /// Proof of work: bundles the reward and every pending transaction into a new block.
    pub fn mine(&mut self, miner: &Address) -> Result<&Block, ChainError> {
        if self.rules.is_proof_of_stake() {
            return Err(ChainError::NotProofOfWork);
        }
        let work = self.next_work();
        self.append(miner, |index, now, prev, txs| Block::mine(index, now, prev, txs, work))
    }

    /// Proof of work at a chosen time, so tests can play out any pace of blocks.
    #[cfg(test)]
    fn mine_at(&mut self, miner: &Address, at: DateTime<Utc>) -> Result<&Block, ChainError> {
        let work = self.next_work();
        self.append(miner, |index, _, prev, txs| Block::mine(index, at, prev, txs, work))
    }

    /// Proof of stake: the selected validator signs the next block with `key`, revealing the
    /// next link of their reveal chain, which `key` also lets us work out.
    pub fn forge(&mut self, key: &SigningKey) -> Result<&Block, ChainError> {
        let validator = Address::of(key);
        self.check_turn(&validator)?;
        let reveal = self
            .due_reveal(&validator)
            .map(|(epoch, remaining)| randao::reveal(key, epoch, remaining));
        self.append(&validator, |index, now, prev, txs| {
            Block::forge(index, now, prev, txs, key, reveal)
        })
    }

    /// Proof of stake, without the key: the block the selected `validator` should sign next,
    /// carrying their `reveal`. It changes nothing; sign its hash and hand it to `append_forged`.
    pub fn propose(
        &self,
        validator: &Address,
        reveal: Option<String>,
    ) -> Result<Block, ChainError> {
        self.check_turn(validator)?;
        match (self.due_reveal(validator), &reveal) {
            (Some(_), None) => return Err(ChainError::BadReveal),
            (None, Some(_)) => return Err(ChainError::BadReveal),
            (Some(_), Some(r))
                if !self.tally.ledger.clone().apply_reveal(validator.as_str(), r) =>
            {
                return Err(ChainError::BadReveal)
            }
            _ => {}
        }
        let (index, transactions) = self.next_contents(validator);
        let prev_hash = self.tip().hash.clone();
        Ok(Block::propose(index, Utc::now(), prev_hash, transactions, validator, reveal))
    }

    /// The commitment epoch and reveals left that `validator`'s next block must reveal from, or
    /// `None` while anyone may forge and no reveal is due.
    fn due_reveal(&self, validator: &Address) -> Option<(u64, u32)> {
        self.next_validator()?;
        let state = self.tally.ledger.validator_state(validator.as_str());
        Some((state.next_epoch.checked_sub(1)?, state.reveals_left?))
    }

    /// Proof of stake: appends a block signed elsewhere, after checking it like any other block.
    /// Transactions it confirmed leave the mempool; the rest stay if they still apply.
    pub fn append_forged(&mut self, block: Block) -> Result<&Block, ChainError> {
        let validator = block.validator.as_deref().ok_or(ChainError::InvalidBlock)?;
        self.check_turn(&Address::parse(validator)?)?;
        if block.prev_hash != self.tip().hash || block.index != self.blocks.len() as u64 {
            return Err(ChainError::StaleBlock);
        }
        let mut tally = self.tally.clone();
        if !apply_block(&self.blocks, &block, &mut tally, &self.rules) {
            return Err(ChainError::InvalidBlock);
        }
        let waiting = self.mempool.transactions().to_vec();
        self.mempool = Mempool::rebuilt(waiting, &tally.ledger);
        self.blocks.push(block);
        self.tally = tally;
        Ok(self.tip())
    }

    /// Under proof of stake, whether it's `validator`'s turn to forge.
    fn check_turn(&self, validator: &Address) -> Result<(), ChainError> {
        if !self.rules.is_proof_of_stake() {
            return Err(ChainError::NotProofOfStake);
        }
        match self.next_validator() {
            Some(expected) if expected != *validator => {
                Err(ChainError::NotYourTurn { expected: expected.to_string() })
            }
            _ => Ok(()),
        }
    }

    /// The next block's index and transactions: a payout of the reward plus fees to `producer`,
    /// then everything in the mempool.
    fn next_contents(&self, producer: &Address) -> (u64, Vec<Transaction>) {
        let index = self.blocks.len() as u64;
        let pending = self.mempool.transactions();
        let payout = self.rules.reward + pending.iter().map(|tx| tx.fee).sum::<u64>();
        let mut transactions = vec![Transaction::coinbase(producer, payout, index)];
        transactions.extend(pending.iter().cloned());
        (index, transactions)
    }

    /// Pays `producer` the reward plus fees, includes the mempool and appends the block `seal`
    /// makes, after checking it exactly as a peer replaying the chain would.
    fn append(
        &mut self,
        producer: &Address,
        seal: impl FnOnce(u64, DateTime<Utc>, String, Vec<Transaction>) -> Block,
    ) -> Result<&Block, ChainError> {
        let (index, transactions) = self.next_contents(producer);
        let block = seal(index, Utc::now(), self.tip().hash.clone(), transactions);
        let mut tally = self.tally.clone();
        if !apply_block(&self.blocks, &block, &mut tally, &self.rules) {
            return Err(ChainError::InvalidBlock);
        }
        self.blocks.push(block);
        self.mempool = Mempool::on(&tally.ledger);
        self.tally = tally;
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
        let tally = replay(&blocks, &self.rules).ok_or(ChainError::InvalidChain)?;
        let theirs =
            (tally.weight, Reverse(&blocks.last().expect("replay refuses empty chains").hash));
        if theirs <= (self.tally.weight, Reverse(&self.tip().hash)) {
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

        self.mempool = Mempool::rebuilt(orphaned, &tally.ledger);
        self.blocks = blocks;
        self.tally = tally;
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

/// Checks every block in order and returns what they add up to, or `None` if any check fails.
fn replay(blocks: &[Block], rules: &ChainRules) -> Option<Tally> {
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
    let mut tally = Tally::at_genesis(genesis, rules);
    for height in 1..blocks.len() {
        if !apply_block(&blocks[..height], &blocks[height], &mut tally, rules) {
            return None;
        }
    }
    Some(tally)
}

/// Checks that `block` validly follows the blocks `before` it and adds it to `tally`. On `false`
/// the tally may be half-updated, so callers check a copy.
fn apply_block(before: &[Block], block: &Block, tally: &mut Tally, rules: &ChainRules) -> bool {
    let prev = before.last().expect("genesis comes first");
    let work = difficulty::next_work(before, tally.work, rules.retarget());
    if block.index != prev.index + 1
        || block.prev_hash != prev.hash
        || block.hash != block.computed_hash()
        || !difficulty::timestamp_fits(before, block.timestamp, Utc::now())
        || !is_sealed_by_consensus(block, &tally.ledger, rules, work)
    {
        return false;
    }
    let ledger = &mut tally.ledger;
    let Some((coinbase, transfers)) = block.transactions.split_first() else {
        return false;
    };
    if !coinbase.is_coinbase()
        || coinbase.amount != rules.reward + transfers.iter().map(|tx| tx.fee).sum::<u64>()
        || coinbase.fee != 0
        || coinbase.nonce != block.index
        || !crypto::is_valid_address(&coinbase.to)
        || block.validator.as_ref().is_some_and(|v| *v != coinbase.to)
    {
        return false;
    }
    if !reveals_as_required(block, ledger, rules) {
        return false;
    }
    tally.weight = tally.weight.with_block(rules, ledger.staked(&coinbase.to), work);
    tally.work = work;
    ledger.credit(&coinbase.to, coinbase.amount);
    transfers.iter().all(|tx| ledger.apply_transfer(tx).is_ok())
}

/// Proof of stake: when a validator was selected, the block must open their commitment, which
/// moves their reveal chain along and mixes the reveal into the beacon. While anyone may forge,
/// there's nothing to reveal and the beacon stays put, so nobody can steer it then either.
fn reveals_as_required(block: &Block, ledger: &mut Ledger, rules: &ChainRules) -> bool {
    if !rules.is_proof_of_stake() {
        return true;
    }
    let selected = select_validator(&ledger.validator_stakes(), ledger.beacon()).is_some();
    match (&block.validator, &block.reveal) {
        (Some(validator), Some(reveal)) if selected => ledger.apply_reveal(validator, reveal),
        (_, None) => !selected,
        _ => false,
    }
}

/// Genesis is never signed. Under proof of work it still needs the work.
fn is_sealed_genesis(genesis: &Block, rules: &ChainRules) -> bool {
    let unsigned =
        genesis.validator.is_none() && genesis.signature.is_none() && genesis.reveal.is_none();
    match rules.consensus {
        Consensus::ProofOfWork { .. } => {
            unsigned && meets_work(&genesis.hash, rules.initial_work())
        }
        Consensus::ProofOfStake => unsigned && genesis.nonce == 0,
    }
}

/// Proof of work: the hash meets the work required at its height and nobody signed it. Proof of
/// stake: the validator drawn from the stakes and beacon before this block signed it.
fn is_sealed_by_consensus(block: &Block, ledger: &Ledger, rules: &ChainRules, work: u128) -> bool {
    match rules.consensus {
        Consensus::ProofOfWork { .. } => {
            block.validator.is_none()
                && block.signature.is_none()
                && block.reveal.is_none()
                && meets_work(&block.hash, work)
        }
        Consensus::ProofOfStake => {
            block.nonce == 0
                && block.is_signed_by_validator()
                && select_validator(&ledger.validator_stakes(), ledger.beacon())
                    .is_none_or(|expected| block.validator.as_deref() == Some(&expected))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::block::UNSTAKE;

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

    /// A first stake, committing to the key's reveal chain for epoch 0.
    fn stake(key: &SigningKey, amount: u64, nonce: u64) -> Transaction {
        Transaction::stake(key, amount, nonce, 0, Some(randao::commitment(key, 0)))
    }

    /// The reveal `key`'s next block must carry, as a wallet would work it out.
    fn reveal_for(chain: &Blockchain, key: &SigningKey) -> Option<String> {
        chain.next_validator()?;
        let state = chain.validator_state(&Address::of(key));
        Some(randao::reveal(key, state.next_epoch - 1, state.reveals_left?))
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
        let bob_block =
            Block::forge(3, Utc::now(), blocks[2].hash.clone(), vec![reward], &bob_key, None);
        blocks.push(bob_block);
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

    #[test]
    fn a_proposal_signed_elsewhere_is_appended_like_a_forged_block() {
        let mut chain = Blockchain::new(POS);
        let (_, alice_key) = wallet();
        let (bob, bob_key) = wallet();
        chain.forge(&alice_key).unwrap();
        chain.submit(Transaction::signed(&alice_key, bob.as_str(), 5, 0, 2)).unwrap();

        let before = chain.blocks().len();
        let mut block = chain.propose(&bob, None).unwrap();
        assert_eq!(chain.blocks().len(), before, "proposing changes nothing");
        assert_eq!(block.signature, None);
        block.sign(&bob_key);

        chain.append_forged(block).unwrap();
        assert!(chain.pending().is_empty(), "the confirmed transfer left the mempool");
        assert_eq!(chain.balance(&bob), 5 + 52);
        assert!(chain.is_valid());
    }

    #[test]
    fn forged_submissions_are_checked() {
        let mut chain = Blockchain::new(POS);
        let (alice, alice_key) = wallet();
        let (_, mallory_key) = wallet();
        chain.forge(&alice_key).unwrap();
        chain.submit(stake(&alice_key, 10, 0)).unwrap();
        chain.forge(&alice_key).unwrap();

        let proposal = chain.propose(&alice, reveal_for(&chain, &alice_key)).unwrap();

        let mut unsigned = proposal.clone();
        assert_eq!(chain.append_forged(unsigned.clone()), Err(ChainError::InvalidBlock));
        unsigned.sign(&mallory_key);
        assert_eq!(chain.append_forged(unsigned).err(), Some(ChainError::InvalidBlock));

        let mut padded = proposal.clone();
        padded.transactions[0].amount += 1;
        padded.hash = padded.computed_hash();
        padded.sign(&alice_key);
        assert_eq!(chain.append_forged(padded).err(), Some(ChainError::InvalidBlock));

        let mut signed = proposal;
        signed.sign(&alice_key);
        chain.append_forged(signed.clone()).unwrap();
        assert_eq!(chain.append_forged(signed).err(), Some(ChainError::StaleBlock));
        assert!(chain.is_valid());
    }

    #[test]
    fn only_the_selected_validator_gets_a_proposal() {
        let mut chain = Blockchain::new(POS);
        let (_, alice_key) = wallet();
        let (bob, _) = wallet();
        chain.forge(&alice_key).unwrap();
        chain.submit(stake(&alice_key, 10, 0)).unwrap();
        chain.forge(&alice_key).unwrap();
        assert!(matches!(chain.propose(&bob, None), Err(ChainError::NotYourTurn { .. })));
        let pow = Blockchain::new(RULES);
        assert_eq!(pow.propose(&bob, None).err(), Some(ChainError::NotProofOfStake));
    }

    // ---- Validator randomness ----------------------------------------------------------------

    /// A chain where Alice and Bob have both staked and validators are being drawn.
    fn staked_chain() -> (Blockchain, [SigningKey; 2]) {
        let mut chain = Blockchain::new(POS);
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
                attempt.submit(Transaction::signed(&forger, to.as_str(), 1, 1, i / 2)).unwrap();
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

        assert_eq!(chain.propose(&validator, None).err(), Some(ChainError::BadReveal));
        assert_eq!(
            chain.propose(&validator, Some("0".repeat(64))).err(),
            Some(ChainError::BadReveal)
        );
        let skipped_ahead = randao::reveal(&key, 0, randao::LENGTH - 2);
        assert_eq!(
            chain.propose(&validator, Some(skipped_ahead)).err(),
            Some(ChainError::BadReveal)
        );

        // A validly signed block with a made-up reveal still fails validation.
        let (index, txs) = chain.next_contents(&validator);
        let mut forged = Block::forge(index, Utc::now(), chain.tip().hash.clone(), txs, &key, None);
        forged.reveal = Some("f".repeat(64));
        forged.hash = forged.computed_hash();
        forged.sign(&key);
        assert_eq!(chain.append_forged(forged).err(), Some(ChainError::InvalidBlock));

        let beacon = chain.beacon().to_string();
        let mut block = chain.propose(&validator, Some(due.clone())).unwrap();
        block.sign(&key);
        chain.append_forged(block).unwrap();
        assert_eq!(chain.beacon(), randao::mix(&beacon, &due));
        assert_eq!(chain.validator_state(&validator).reveals_left, Some(randao::LENGTH - 1));
        assert!(chain.is_valid());
    }

    #[test]
    fn only_a_first_stake_commits() {
        let mut chain = Blockchain::new(POS);
        let (alice, alice_key) = wallet();
        chain.forge(&alice_key).unwrap();
        chain.forge(&alice_key).unwrap();

        assert_eq!(
            chain.submit(Transaction::stake(&alice_key, 10, 0, 0, None)),
            Err(ChainError::MissingCommitment)
        );
        assert_eq!(chain.commitment_due(&alice), Some(0));
        chain.submit(stake(&alice_key, 10, 0)).unwrap();
        assert_eq!(chain.commitment_due(&alice), None, "the waiting stake already commits");
        assert_eq!(
            chain.submit(Transaction::stake(
                &alice_key,
                5,
                1,
                0,
                Some(randao::commitment(&alice_key, 1))
            )),
            Err(ChainError::UnexpectedCommitment)
        );
        chain.submit(Transaction::stake(&alice_key, 5, 1, 0, None)).unwrap();
        chain.forge(&alice_key).unwrap();
        assert_eq!(chain.staked(&alice), 15);

        // Unstaking everything ends the chain; staking again needs commitment number 1.
        chain.submit(Transaction::signed(&alice_key, UNSTAKE, 15, 2, 0)).unwrap();
        chain.forge(&alice_key).unwrap();
        assert_eq!(chain.validator_state(&alice).reveals_left, None);
        assert_eq!(chain.commitment_due(&alice), Some(1));
        let again =
            Transaction::stake(&alice_key, 10, 3, 0, Some(randao::commitment(&alice_key, 1)));
        chain.submit(again).unwrap();
        chain.forge(&alice_key).unwrap();
        chain.forge(&alice_key).unwrap();
        assert!(chain.is_valid());
    }

    #[test]
    fn the_beacon_only_moves_with_reveals() {
        let mut pow = chain();
        let genesis_beacon = pow.beacon().to_string();
        pow.mine(&wallet().0).unwrap();
        assert_eq!(pow.beacon(), genesis_beacon, "mining reveals nothing");

        let mut pos = Blockchain::new(POS);
        let start = pos.beacon().to_string();
        pos.forge(&wallet().1).unwrap();
        assert_eq!(pos.beacon(), start, "nobody can steer it while anyone may forge");
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

    // ---- Difficulty adjustment -----------------------------------------------------------

    /// Starts at 16 hashes a block, aiming for one block every 10s, adjusting every 5 blocks.
    const ADJUSTING: ChainRules = ChainRules::proof_of_work(1, 50).with_retarget(10, 5);

    /// Mines `count` blocks `secs` apart, starting where the chain's tip left off.
    fn mine_paced(chain: &mut Blockchain, count: usize, secs: i64) {
        let miner = wallet().0;
        for _ in 0..count {
            let last = chain.tip().timestamp.max(Utc::now() - chrono::TimeDelta::days(1));
            chain.mine_at(&miner, last + chrono::TimeDelta::seconds(secs)).unwrap();
        }
    }

    #[test]
    fn blocks_that_come_too_fast_make_mining_harder_and_too_slow_easier() {
        let mut chain = Blockchain::new(ADJUSTING);
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
        let restored = Blockchain::restore(chain.blocks().to_vec(), ADJUSTING).unwrap();
        assert_eq!(restored.next_work(), chain.next_work());
    }

    #[test]
    fn the_adjustment_is_proportional_inside_the_cap() {
        let mut chain = Blockchain::new(ADJUSTING);
        mine_paced(&mut chain, 9, 5);
        assert_eq!(chain.next_work(), 32, "twice as fast as the target: twice the work");
        mine_paced(&mut chain, 5, 20);
        assert_eq!(chain.next_work(), 16, "twice as slow: half the work");
        assert_eq!(chain.next_retarget_height(), Some(15), "and the next block is adjusted again");
    }

    #[test]
    fn a_block_below_the_required_work_is_rejected() {
        let mut chain = Blockchain::new(ADJUSTING);
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
            .find(|b| !meets_work(&b.hash, 64))
            .unwrap();
        let mut blocks = chain.blocks().to_vec();
        blocks.push(easy);
        assert!(!validate_blocks(&blocks, &ADJUSTING));
    }

    #[test]
    fn timestamps_cannot_be_backdated_or_far_in_the_future() {
        let mut chain = Blockchain::new(ADJUSTING);
        mine_paced(&mut chain, 12, 10);
        let miner = wallet().0;
        let median = chain.blocks()[chain.blocks().len() - 6].timestamp;
        assert_eq!(chain.mine_at(&miner, median).err(), Some(ChainError::InvalidBlock));
        let future = Utc::now() + chrono::TimeDelta::hours(3);
        assert_eq!(chain.mine_at(&miner, future).err(), Some(ChainError::InvalidBlock));
        chain.mine_at(&miner, median + chrono::TimeDelta::seconds(1)).unwrap();
    }

    #[test]
    fn a_shorter_but_harder_fork_beats_a_longer_easier_one() {
        // 10 blocks a second apart push the work to 64, so 5 more weigh 320.
        let mut hard = Blockchain::new(ADJUSTING);
        mine_paced(&mut hard, 15, 1);
        // 17 slow blocks: every block after the first adjustment needs a quarter of the work.
        let mut easy = Blockchain::new(ADJUSTING);
        mine_paced(&mut easy, 17, 100);

        assert!(hard.blocks().len() < easy.blocks().len());
        assert!(hard.weight() > easy.weight());
        assert_eq!(easy.adopt_if_heavier(hard.blocks().to_vec()), Ok(true));
    }
}
