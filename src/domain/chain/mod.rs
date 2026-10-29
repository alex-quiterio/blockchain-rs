//! The chain: the aggregate root that every change goes through (`Blockchain`), the rules each
//! block is checked by (`validation`), and what the blocks add up to (`tally`).

use std::cmp::Reverse;
use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use crate::domain::address::Address;
use crate::domain::block::{Block, Transaction};
use crate::domain::difficulty;
use crate::domain::error::ChainError;
use crate::domain::ledger::ValidatorState;
use crate::domain::mempool::Mempool;
use crate::domain::randao;
use crate::domain::rules::{select_validator, ChainRules};
use crate::domain::values::{BlockHash, Coins};
use crate::domain::weight::ChainWeight;

mod events;
mod tally;
mod validation;

use tally::{Tally, TallyLog};
use validation::{apply_block, keep, links_up, payout, replay_from, replay_kept};

pub use events::ChainEvent;
pub use tally::Snapshot;
pub use validation::validate_blocks;

/// The aggregate root: confirmed blocks, the mempool and the ledger they produce.
/// Every change goes through its methods, so the three never disagree.
#[derive(Clone)]
pub struct Blockchain {
    blocks: Vec<Block>,
    mempool: Mempool,
    /// What the chain added up to after each of its last `max_reorg_depth + 1` blocks; the last
    /// is what the whole chain adds up to.
    tallies: TallyLog,
    rules: ChainRules,
    /// What happened since they were last taken.
    events: Vec<ChainEvent>,
}

/// Proof of work: the contents of the next block, waiting for a nonce.
pub struct MiningTemplate {
    pub index: u64,
    pub timestamp: DateTime<Utc>,
    pub prev_hash: BlockHash,
    pub transactions: Vec<Transaction>,
    pub work: u128,
}

impl MiningTemplate {
    /// Searches for the nonce. Gives up when `keep_going` says no.
    pub fn mine_while(self, keep_going: impl FnMut() -> bool) -> Option<Block> {
        let Self { index, timestamp, prev_hash, transactions, work } = self;
        Block::mine_while(index, timestamp, prev_hash, transactions, work, keep_going)
    }
}

/// What came of offering a chain some blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extension {
    /// They made a heavier chain, which is now ours.
    Adopted,
    /// We already had them, or our chain is heavier.
    Ignored,
    /// They build on a block we don't have, so the sender should send more from further back.
    MissingParent,
}

impl From<bool> for Extension {
    fn from(adopted: bool) -> Self {
        if adopted {
            Self::Adopted
        } else {
            Self::Ignored
        }
    }
}

impl Blockchain {
    pub fn new(rules: ChainRules) -> Self {
        let genesis = Block::genesis(&rules);
        let tally = Tally::at_genesis(&genesis, &rules);
        let mut chain = Self::from_parts(vec![genesis], VecDeque::from([tally]), rules);
        chain.events.push(ChainEvent::Started);
        chain
    }

    /// Rebuilds a chain from saved blocks. They are replayed in full, so tampered ones are refused.
    pub fn restore(blocks: Vec<Block>, rules: ChainRules) -> Result<Self, ChainError> {
        Ok(Self::restore_from(blocks, rules, None)?.0)
    }

    /// Rebuilds a chain from saved blocks, replaying only those after `snapshot` when it fits
    /// them. Blocks up to the snapshot are checked only for being the ones it was taken after:
    /// their hashes, links and Merkle roots lead to its block. Everything after is replayed in
    /// full. Without a snapshot that fits, the whole chain is replayed. Returns the chain and
    /// the snapshot's height, if one was used.
    pub fn restore_from(
        blocks: Vec<Block>,
        rules: ChainRules,
        snapshot: Option<Snapshot>,
    ) -> Result<(Self, Option<usize>), ChainError> {
        let keep = keep(&rules);
        let fits = snapshot.filter(|s| {
            s.rules == rules
                && blocks.get(s.height).is_some_and(|block| block.hash == s.hash)
                && links_up(&blocks[..=s.height], &rules)
        });
        let (kept, from) = match fits {
            Some(s) => (replay_from(&blocks, s.height + 1, s.tally, &rules, keep), Some(s.height)),
            None => (replay_kept(&blocks, &rules, keep), None),
        };
        let kept = kept.ok_or(ChainError::InvalidChain)?;
        Ok((Self::from_parts(blocks, kept, rules), from))
    }

    fn from_parts(blocks: Vec<Block>, kept: VecDeque<Tally>, rules: ChainRules) -> Self {
        let tallies = TallyLog::new(blocks.len() - kept.len(), kept, keep(&rules));
        let mempool = Mempool::on(&tallies.latest().ledger);
        Self { mempool, blocks, tallies, rules, events: Vec::new() }
    }

    /// A snapshot after the oldest block no fork may undo any more, once the chain is long
    /// enough to have one.
    pub fn snapshot(&self) -> Option<Snapshot> {
        self.tallies.snapshot(&self.blocks, self.rules)
    }

    /// What happened to the chain since this was last called, oldest first.
    pub fn take_events(&mut self) -> Vec<ChainEvent> {
        std::mem::take(&mut self.events)
    }

    /// Records blocks added on top, merged with blocks added just before.
    fn extended(&mut self, from: usize, to: usize) {
        match self.events.last_mut() {
            Some(ChainEvent::Extended { to: last, .. }) if *last + 1 == from => *last = to,
            _ => self.events.push(ChainEvent::Extended { from, to }),
        }
    }

    /// What the whole chain adds up to.
    fn tally(&self) -> &Tally {
        self.tallies.latest()
    }

    pub fn rules(&self) -> ChainRules {
        self.rules
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn weight(&self) -> ChainWeight {
        self.tally().weight
    }

    /// Proof of work: the hashes the next block should take to find, on average.
    pub fn next_work(&self) -> u128 {
        difficulty::next_work(&self.blocks, self.tally().work, self.rules.retarget())
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

    pub fn balance(&self, address: &Address) -> Coins {
        self.tally().ledger.balance(address.as_str())
    }

    pub fn staked(&self, address: &Address) -> Coins {
        self.tally().ledger.staked(address.as_str())
    }

    /// Every confirmed stake, sorted by address.
    pub fn stakes(&self) -> Vec<(String, Coins)> {
        self.tally().ledger.stakes()
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
        let selected = select_validator(
            &self.tally().ledger.validator_stakes(),
            self.tally().ledger.beacon(),
        )?;
        Some(Address::parse(&selected).expect("only valid senders can stake"))
    }

    /// The randomness beacon the next validator is drawn from.
    pub fn beacon(&self) -> &str {
        self.tally().ledger.beacon()
    }

    /// Where `address` stands with its reveal chain.
    pub fn validator_state(&self, address: &Address) -> ValidatorState {
        self.tally().ledger.validator_state(address.as_str())
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
        let block = self.mining_template(miner)?.mine_while(|| true).expect("never told to stop");
        self.append_mined(block)
    }

    /// Proof of work: what `miner`'s next block would hold, so it can be mined without holding
    /// the chain.
    pub fn mining_template(&self, miner: &Address) -> Result<MiningTemplate, ChainError> {
        if self.rules.is_proof_of_stake() {
            return Err(ChainError::NotProofOfWork);
        }
        let (index, transactions) = self.next_contents(miner);
        Ok(MiningTemplate {
            index,
            timestamp: Utc::now(),
            prev_hash: self.tip().hash.clone(),
            transactions,
            work: self.next_work(),
        })
    }

    /// Proof of work: appends a block mined from a template, unless another block took the tip
    /// in the meantime.
    pub fn append_mined(&mut self, block: Block) -> Result<&Block, ChainError> {
        if self.rules.is_proof_of_stake() {
            return Err(ChainError::NotProofOfWork);
        }
        self.append_sealed(block)
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
                if !self.tally().ledger.clone().apply_reveal(validator.as_str(), r) =>
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
        let state = self.tally().ledger.validator_state(validator.as_str());
        Some((state.next_epoch.checked_sub(1)?, state.reveals_left?))
    }

    /// Proof of stake: appends a block signed elsewhere, after checking it like any other block.
    /// Transactions it confirmed leave the mempool; the rest stay if they still apply.
    pub fn append_forged(&mut self, block: Block) -> Result<&Block, ChainError> {
        let validator = block.validator.as_deref().ok_or(ChainError::InvalidBlock)?;
        self.check_turn(&Address::parse(validator)?)?;
        self.append_sealed(block)
    }

    /// Appends a block made elsewhere on our tip, after checking it like any other block.
    /// Transactions it confirmed leave the mempool; the rest stay if they still apply.
    fn append_sealed(&mut self, block: Block) -> Result<&Block, ChainError> {
        if block.prev_hash != self.tip().hash || block.index != self.blocks.len() as u64 {
            return Err(ChainError::StaleBlock);
        }
        let mut tally = self.tally().clone();
        if !apply_block(&self.blocks, &block, &mut tally, &self.rules) {
            return Err(ChainError::InvalidBlock);
        }
        let waiting = self.mempool.transactions().to_vec();
        self.mempool = Mempool::rebuilt(waiting, &tally.ledger);
        self.blocks.push(block);
        self.tallies.push(tally);
        self.extended(self.blocks.len() - 1, self.blocks.len() - 1);
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
        // The mempool only takes fees their senders can afford, so they always add up.
        let payout = payout(self.rules.reward, pending).expect("affordable fees add up");
        let mut transactions = vec![Transaction::coinbase(producer, payout, index)];
        transactions.extend(pending.iter().cloned());
        (index, transactions)
    }

    /// Pays `producer` the reward plus fees, includes the mempool and appends the block `seal`
    /// makes, after checking it exactly as a peer replaying the chain would.
    fn append(
        &mut self,
        producer: &Address,
        seal: impl FnOnce(u64, DateTime<Utc>, BlockHash, Vec<Transaction>) -> Block,
    ) -> Result<&Block, ChainError> {
        let (index, transactions) = self.next_contents(producer);
        let block = seal(index, Utc::now(), self.tip().hash.clone(), transactions);
        let mut tally = self.tally().clone();
        if !apply_block(&self.blocks, &block, &mut tally, &self.rules) {
            return Err(ChainError::InvalidBlock);
        }
        self.blocks.push(block);
        self.mempool = Mempool::on(&tally.ledger);
        self.tallies.push(tally);
        self.extended(self.blocks.len() - 1, self.blocks.len() - 1);
        Ok(self.tip())
    }

    /// The fork-choice rule: adopts `blocks` if they form a valid chain heavier than ours. Exact
    /// ties go to the lower tip hash, so every node settles on the same fork. Never undoes more
    /// than `max_reorg_depth` of our blocks. Transfers from our blocks that `blocks` drops go back
    /// to the mempool if they still apply.
    ///
    /// Only their blocks after the split are checked, starting from what our chain added up to
    /// there; before it, their blocks are ours hash for hash, and our copies are kept. A split
    /// deeper than any fork we may take is replayed in full, then refused below.
    pub fn adopt_if_heavier(&mut self, blocks: Vec<Block>) -> Result<bool, ChainError> {
        if blocks.last().is_some_and(|tip| tip.hash == self.tip().hash) {
            return Ok(false);
        }
        let shared = self
            .blocks
            .iter()
            .zip(&blocks)
            .take_while(|(ours, theirs)| ours.hash == theirs.hash)
            .count();
        let keep = keep(&self.rules);
        let (chain, kept) = match shared.checked_sub(1).and_then(|h| self.tallies.after(h)) {
            Some(base) => {
                let base = base.clone();
                let mut chain = self.blocks[..shared].to_vec();
                chain.extend(blocks.into_iter().skip(shared));
                let kept = replay_from(&chain, shared, base, &self.rules, keep);
                (chain, kept)
            }
            None => {
                let kept = replay_kept(&blocks, &self.rules, keep);
                (blocks, kept)
            }
        };
        let kept = kept.ok_or(ChainError::InvalidChain)?;
        let tally = kept.back().expect("replays keep at least the last").clone();
        let theirs =
            (tally.weight, Reverse(&chain.last().expect("replay refuses empty chains").hash));
        if theirs <= (self.tally().weight, Reverse(&self.tip().hash)) {
            return Ok(false);
        }

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
        self.tallies.replace_from(shared, chain.len() - kept.len(), kept);
        let ours = self.blocks.len();
        self.blocks = chain;
        if shared == ours {
            // Theirs is ours with more on top.
            self.extended(shared, self.blocks.len() - 1);
        } else {
            let (dropped, added) = (ours - shared, self.blocks.len() - shared);
            self.events.push(ChainEvent::Reorganized { from: shared, dropped, added });
        }
        Ok(true)
    }

    /// Takes another node's blocks from some height on, so peers can send only what's new. Blocks
    /// that build on our tip are checked one by one, without replaying the rest of the chain. A
    /// stretch that forks off one of our blocks is weighed as a whole chain by `adopt_if_heavier`.
    /// Blocks from height 0 are a whole chain already.
    pub fn extend(&mut self, blocks: Vec<Block>) -> Result<Extension, ChainError> {
        let start = blocks.first().ok_or(ChainError::InvalidChain)?.index as usize;
        if start == 0 {
            return self.adopt_if_heavier(blocks).map(Extension::from);
        }
        match self.blocks.get(start - 1) {
            Some(parent) if parent.hash == blocks[0].prev_hash => {}
            _ => return Ok(Extension::MissingParent),
        }
        let known = self.blocks[start..]
            .iter()
            .zip(&blocks)
            .take_while(|(ours, theirs)| ours.hash == theirs.hash)
            .count();
        let new = &blocks[known..];
        if new.is_empty() {
            return Ok(Extension::Ignored);
        }
        if start + known < self.blocks.len() {
            let mut candidate = self.blocks[..start].to_vec();
            candidate.extend(blocks);
            return self.adopt_if_heavier(candidate).map(Extension::from);
        }

        let mut tally = self.tally().clone();
        let height = self.blocks.len();
        let mut tallies = Vec::with_capacity(new.len());
        for block in new {
            if !apply_block(&self.blocks, block, &mut tally, &self.rules) {
                self.blocks.truncate(height);
                return Err(ChainError::InvalidBlock);
            }
            self.blocks.push(block.clone());
            tallies.push(tally.clone());
        }
        let waiting = self.mempool.transactions().to_vec();
        self.mempool = Mempool::rebuilt(waiting, &tally.ledger);
        for tally in tallies {
            self.tallies.push(tally);
        }
        self.extended(height, self.blocks.len() - 1);
        Ok(Extension::Adopted)
    }

    /// The transfer signed `signature`, if it's waiting in the mempool.
    pub fn pending_transaction(&self, signature: &str) -> Option<&Transaction> {
        self.mempool
            .transactions()
            .iter()
            .find(|tx| !signature.is_empty() && tx.signature == signature)
    }

    /// Transactions to or from `address` waiting in the mempool.
    pub fn pending_of<'a>(&'a self, address: &'a Address) -> impl Iterator<Item = &'a Transaction> {
        self.mempool.transactions().iter().filter(move |tx| tx.involves(address))
    }

    /// Confirmed transactions to or from `address`, block rewards included, newest first: each
    /// with its block and its place there.
    pub fn history_of<'a>(
        &'a self,
        address: &'a Address,
    ) -> impl Iterator<Item = (&'a Block, usize)> {
        self.blocks.iter().rev().flat_map(move |block| {
            let positions = block.transactions.iter().enumerate();
            positions
                .filter(|(_, tx)| tx.involves(address))
                .map(move |(position, _)| (block, position))
        })
    }

    /// The confirmed block holding the transfer signed `signature`, and its place in the block.
    pub fn find_transaction(&self, signature: &str) -> Option<(&Block, usize)> {
        if signature.is_empty() {
            return None;
        }
        self.blocks.iter().rev().find_map(|block| {
            let position = block.transactions.iter().position(|tx| tx.signature == signature)?;
            Some((block, position))
        })
    }

    /// Replays the whole chain: links, hashes, consensus, rewards, signatures and balances.
    pub fn is_valid(&self) -> bool {
        validate_blocks(&self.blocks, &self.rules)
    }
}

#[cfg(test)]
mod tests {
    use super::validation::replay;
    use super::*;
    use crate::domain::block::UNSTAKE;
    use crate::domain::crypto;
    use crate::domain::difficulty::meets_work;

    const RULES: ChainRules = ChainRules::proof_of_work(2, 50);

    fn wallet() -> (Address, SigningKey) {
        let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
        (Address::of(&key), key)
    }

    fn transfer(key: &SigningKey, to: &Address, amount: u64, nonce: u64) -> Transaction {
        Transaction::signed(key, to.as_str(), Coins::new(amount), nonce, Coins::new(0))
    }

    fn chain() -> Blockchain {
        Blockchain::new(RULES)
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
            Err(ChainError::InsufficientFunds { balance: Coins::new(10), amount: Coins::new(20) })
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
        blocks[2].transactions[1].amount = Coins::new(50);
        assert!(!validate_blocks(&blocks, &RULES));

        let mut blocks = chain.blocks().to_vec();
        blocks[1].transactions[0].amount = Coins::new(1_000);
        blocks[1].hash = blocks[1].computed_hash();
        assert!(!validate_blocks(&blocks, &RULES));
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
        assert!(!validate_blocks(&blocks, &RULES));

        let mut ours = Blockchain::new(RULES);
        assert_eq!(ours.extend(blocks[1..].to_vec()), Err(ChainError::InvalidBlock));
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
        assert_eq!(chain.append_mined(block), Err(ChainError::StaleBlock));
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
        let template = Blockchain::new(RULES).mining_template(&wallet().0).unwrap();
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
    fn every_change_says_what_happened() {
        let rules = RULES.with_max_reorg_depth(5);
        let mut ours = Blockchain::new(rules);
        assert_eq!(ours.take_events(), [ChainEvent::Started]);
        assert_eq!(ours.take_events(), [], "taken once");
        ours.mine(&wallet().0).unwrap();
        ours.mine(&wallet().0).unwrap();
        assert_eq!(ours.take_events(), [ChainEvent::Extended { from: 1, to: 2 }], "merged");

        // A restored chain starts with nothing to tell.
        let mut theirs = Blockchain::restore(ours.blocks().to_vec(), rules).unwrap();
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
        let mut fork = Blockchain::restore(ours.blocks()[..2].to_vec(), rules).unwrap();
        for _ in 0..4 {
            fork.mine(&wallet().0).unwrap();
        }
        assert_eq!(ours.adopt_if_heavier(fork.blocks().to_vec()), Ok(true));
        let replaced = ChainEvent::Reorganized { from: 2, dropped: 3, added: 4 };
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

        assert_eq!(ours.extend(vec![block]), Err(ChainError::InvalidBlock));
        assert_eq!(ours.blocks().len(), 1);
        assert_eq!(ours.extend(vec![]), Err(ChainError::InvalidChain));
    }

    /// The remembered tallies say what a full replay of the chain up to each of them would.
    fn assert_remembers_correctly(chain: &Blockchain, watched: &[Address]) {
        let heights = chain.tallies.heights();
        assert_eq!(heights.len(), keep(&chain.rules).min(chain.blocks.len()));
        assert_eq!(heights.end, chain.blocks.len(), "the last is after the tip");
        for height in heights {
            let replayed = replay(&chain.blocks[..=height], &chain.rules).unwrap();
            let kept = chain.tallies.after(height).unwrap();
            assert_eq!((kept.weight, kept.work), (replayed.weight, replayed.work), "at {height}");
            for who in watched {
                let (k, r) =
                    (kept.ledger.balance(who.as_str()), replayed.ledger.balance(who.as_str()));
                assert_eq!(k, r, "{who} at {height}");
            }
        }
    }

    #[test]
    fn forks_are_checked_from_where_they_split() {
        let rules = RULES.with_max_reorg_depth(3);
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        let mut ours = Blockchain::new(rules);
        for _ in 0..6 {
            ours.mine(&alice).unwrap();
        }
        assert_remembers_correctly(&ours, &[alice.clone(), bob.clone()]);

        // Their fork splits two blocks back and carries a payment ours never saw.
        let mut theirs = Blockchain::restore(ours.blocks()[..5].to_vec(), rules).unwrap();
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
        let rules = RULES.with_max_reorg_depth(2);
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        let mut chain = Blockchain::new(rules);
        chain.mine(&alice).unwrap();
        chain.submit(transfer(&alice_key, &bob, 20, 0)).unwrap();
        for _ in 0..4 {
            chain.mine(&alice).unwrap();
        }
        let snapshot = chain.snapshot().unwrap();
        assert_eq!(snapshot.height, chain.blocks().len() - 3, "the oldest block no fork may undo");

        let (restored, from) =
            Blockchain::restore_from(chain.blocks().to_vec(), rules, Some(snapshot.clone()))
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
        generous.tally.ledger.credit(bob.as_str(), Coins::new(1_000));
        let (trusting, _) =
            Blockchain::restore_from(chain.blocks().to_vec(), rules, Some(generous)).unwrap();
        assert_eq!(trusting.balance(&bob), Coins::new(1_020));

        // Blocks it covers that were changed don't lead to its block, so it isn't used.
        let mut tampered = chain.blocks().to_vec();
        tampered[2].transactions[1].amount = Coins::new(25);
        assert!(Blockchain::restore_from(tampered, rules, Some(snapshot.clone())).is_err());

        // Nor under other rules, or for a chain where its block was replaced.
        let other = rules.with_max_reorg_depth(9);
        let (_, from) =
            Blockchain::restore_from(chain.blocks().to_vec(), other, Some(snapshot.clone()))
                .unwrap();
        assert_eq!(from, None);
        let replaced = Snapshot { hash: BlockHash::of(b"another block"), ..snapshot };
        let (full, from) =
            Blockchain::restore_from(chain.blocks().to_vec(), rules, Some(replaced)).unwrap();
        assert_eq!((from, full.balance(&bob)), (None, Coins::new(20)));
    }

    #[test]
    fn a_short_chain_has_no_snapshot_yet() {
        let mut chain = Blockchain::new(RULES.with_max_reorg_depth(3));
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
        forged[2].transactions[0].amount = Coins::new(1_000);

        assert_eq!(ours.adopt_if_heavier(forged), Err(ChainError::InvalidChain));
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
        assert_eq!(chain.balance(&alice), Coins::new(50));
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

        assert_eq!(chain.balance(&alice), Coins::new(20));
        assert_eq!(chain.staked(&alice), Coins::new(30));
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
            chain.submit(Transaction::signed(
                &alice_key,
                UNSTAKE,
                Coins::new(31),
                1,
                Coins::new(0)
            )),
            Err(ChainError::InsufficientStake { staked: Coins::new(30), amount: Coins::new(31) })
        );
        chain
            .submit(Transaction::signed(&alice_key, UNSTAKE, Coins::new(30), 1, Coins::new(0)))
            .unwrap();
        chain.forge(&alice_key).unwrap();
        assert_eq!(chain.staked(&alice), Coins::new(0));
        assert_eq!(chain.balance(&alice), Coins::new(150));
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
        let reward = Transaction::coinbase(&Address::of(&bob_key), Coins::new(50), 3);
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
        chain
            .submit(Transaction::signed(&alice_key, bob.as_str(), Coins::new(5), 0, Coins::new(2)))
            .unwrap();

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
        padded.transactions[0].amount =
            padded.transactions[0].amount.checked_add(Coins::new(1)).unwrap();
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
                attempt
                    .submit(Transaction::signed(
                        &forger,
                        to.as_str(),
                        Coins::new(1),
                        1,
                        Coins::new(i / 2),
                    ))
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
            chain.submit(Transaction::stake(&alice_key, Coins::new(10), 0, Coins::new(0), None)),
            Err(ChainError::MissingCommitment)
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
            Err(ChainError::UnexpectedCommitment)
        );
        chain
            .submit(Transaction::stake(&alice_key, Coins::new(5), 1, Coins::new(0), None))
            .unwrap();
        chain.forge(&alice_key).unwrap();
        assert_eq!(chain.staked(&alice), Coins::new(15));

        // Unstaking everything ends the chain; staking again needs commitment number 1.
        chain
            .submit(Transaction::signed(&alice_key, UNSTAKE, Coins::new(15), 2, Coins::new(0)))
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
        chain
            .submit(Transaction::signed(&alice_key, bob.as_str(), Coins::new(20), 0, Coins::new(3)))
            .unwrap();

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
            chain.submit(Transaction::signed(
                &alice_key,
                bob.as_str(),
                Coins::new(50),
                0,
                Coins::new(1)
            )),
            Err(ChainError::InsufficientFunds { balance: Coins::new(50), amount: Coins::new(51) })
        );
    }

    #[test]
    fn fees_are_signed_and_counted_in_the_payout() {
        let mut chain = chain();
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        chain.mine(&alice).unwrap();
        chain
            .submit(Transaction::signed(&alice_key, bob.as_str(), Coins::new(5), 0, Coins::new(2)))
            .unwrap();
        chain.mine(&alice).unwrap();

        let mut blocks = chain.blocks().to_vec();
        blocks[2].transactions[1].fee = Coins::new(0);
        blocks[2].hash = blocks[2].computed_hash();
        assert!(!validate_blocks(&blocks, &RULES), "the signature covers the fee");

        let mut blocks = chain.blocks().to_vec();
        blocks[2].transactions[0].amount = Coins::new(60);
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
            .find(|b| !meets_work(b.hash.as_str(), 64))
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
