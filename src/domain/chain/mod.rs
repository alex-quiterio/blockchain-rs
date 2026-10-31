//! The chain, an aggregate: its root that every change goes through (`Blockchain`), its entity
//! the block (`block`), the rules each block is checked by (`validation`), and what the blocks
//! add up to (`tally`). A block can be built outside the chain, but it only becomes part of it
//! through the root, which checks it first. Its lock in a node is its own, so the mempool, the
//! other aggregate, never waits on it.

use std::cmp::Reverse;
use std::collections::VecDeque;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use crate::domain::accounting::ledger::Ledger;
use crate::domain::accounting::transaction::Transaction;
use crate::domain::accounting::Coins;
use crate::domain::chain::block::Block;
use crate::domain::chain::block::BlockError;
use crate::domain::chain::block::BlockHash;
use crate::domain::consensus::difficulty;
use crate::domain::consensus::randao;
use crate::domain::consensus::rules::ChainRules;
use crate::domain::consensus::turns::{opens_at, turn_order};
use crate::domain::consensus::validators::ValidatorState;
use crate::domain::consensus::weight::ChainWeight;
use crate::domain::consensus::{Beacon, Reveal};
use crate::domain::crypto::address::Address;

pub mod block;
mod error;
mod events;
mod headers;
mod reorg;
pub mod repository;
mod tally;
mod validation;

use tally::{Tally, TallyLog};
use validation::{apply_block, check_place, links_up, payout, replay_from, replay_kept};

pub use error::ChainError;
pub use events::ChainEvent;
pub use headers::HeaderVerdict;
pub use reorg::ReorgLimit;
pub use tally::Snapshot;
pub use validation::validate_blocks;

/// The aggregate root: confirmed blocks and the ledger they produce. Every change goes through
/// its methods, so the two never disagree. Waiting transactions are a `Mempool`'s, an aggregate
/// of its own; producing a block takes them as candidates.
#[derive(Clone)]
pub struct Blockchain {
    /// Shared behind `Arc`s: a block never changes once it's in a chain, so copying the chain,
    /// as every change in a node does, copies a pointer per block rather than the blocks.
    blocks: Vec<Arc<Block>>,
    /// What the chain added up to after each of its last `reorg_limit + 1` blocks; the last is
    /// what the whole chain adds up to.
    tallies: TallyLog,
    rules: ChainRules,
    /// How far back this node will follow a fork: its own choice, not one of the rules.
    reorg_limit: ReorgLimit,
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
    /// A new chain of just genesis, following forks back as far as `ReorgLimit::DEFAULT`.
    pub fn new(rules: ChainRules) -> Self {
        let genesis = Block::genesis(&rules);
        let tally = Tally::at_genesis(&genesis, &rules);
        let limit = ReorgLimit::DEFAULT;
        let mut chain =
            Self::from_parts(vec![Arc::new(genesis)], VecDeque::from([tally]), rules, limit);
        chain.events.push(ChainEvent::Started);
        chain
    }

    /// Follows forks back at most `limit` blocks from now on. Tallies already let go of don't
    /// come back, so a restored chain should be given its limit by `restore_from` instead.
    pub fn with_reorg_limit(self, limit: ReorgLimit) -> Self {
        let tallies = self.tallies.keeping(limit.tallies_to_keep());
        Self { tallies, reorg_limit: limit, ..self }
    }

    /// Rebuilds a chain from saved blocks. They are replayed in full, so tampered ones are refused.
    pub fn restore(blocks: Vec<Block>, rules: ChainRules) -> Result<Self, ChainError> {
        Ok(Self::restore_from(blocks, rules, ReorgLimit::DEFAULT, None)?.0)
    }

    /// Rebuilds a chain from saved blocks, replaying only those after `snapshot` when it fits
    /// them. Blocks up to the snapshot are checked only for being the ones it was taken after:
    /// their hashes, links and Merkle roots lead to its block. Everything after is replayed in
    /// full. Without a snapshot that fits, the whole chain is replayed. Returns the chain and
    /// the snapshot's height, if one was used.
    pub fn restore_from(
        blocks: Vec<Block>,
        rules: ChainRules,
        limit: ReorgLimit,
        snapshot: Option<Snapshot>,
    ) -> Result<(Self, Option<usize>), ChainError> {
        let keep = limit.tallies_to_keep();
        let fits = snapshot.filter(|s| {
            s.is_current()
                && s.rules == rules
                && blocks.get(s.height).is_some_and(|block| block.hash == s.hash)
                && links_up(&blocks[..=s.height], &rules)
        });
        let (kept, from) = match fits {
            Some(mut s) => {
                s.tally.ledger = s.tally.ledger.on_chain(&blocks[0].hash);
                (replay_from(&blocks, s.height + 1, s.tally, &rules, keep)?, Some(s.height))
            }
            None => (replay_kept(&blocks, &rules, keep)?, None),
        };
        let blocks = blocks.into_iter().map(Arc::new).collect();
        Ok((Self::from_parts(blocks, kept, rules, limit), from))
    }

    fn from_parts(
        blocks: Vec<Arc<Block>>,
        kept: VecDeque<Tally>,
        rules: ChainRules,
        reorg_limit: ReorgLimit,
    ) -> Self {
        let tallies = TallyLog::new(blocks.len() - kept.len(), kept, reorg_limit.tallies_to_keep());
        Self { blocks, tallies, rules, reorg_limit, events: Vec::new() }
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

    pub fn reorg_limit(&self) -> ReorgLimit {
        self.reorg_limit
    }

    pub fn blocks(&self) -> &[Arc<Block>] {
        &self.blocks
    }

    pub fn weight(&self) -> ChainWeight {
        self.tally().weight
    }

    /// Proof of work: the hashes the next block should take to find, on average.
    pub fn next_work(&self) -> u128 {
        difficulty::next_work(&self.blocks[..], self.tally().work, self.rules.retarget())
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

    /// The confirmed ledger: balances, stakes, nonces and validators as the chain leaves them.
    /// A mempool rebases onto it whenever the chain changes.
    pub fn ledger(&self) -> &Ledger {
        &self.tally().ledger
    }

    pub fn balance(&self, address: &Address) -> Coins {
        self.tally().ledger.balance(address)
    }

    pub fn staked(&self, address: &Address) -> Coins {
        self.tally().ledger.staked(address)
    }

    /// Every confirmed stake, sorted by address.
    pub fn stakes(&self) -> Vec<(Address, Coins)> {
        self.tally().ledger.stakes()
    }

    /// Who is drawn to forge the next block, first in line. `None` on proof-of-work chains, and
    /// on proof-of-stake chains where nobody with reveals left has staked, when anyone may forge.
    ///
    /// The draw is seeded by the randomness beacon, which only validators' committed reveals
    /// move. The block hash isn't involved, so a forger can't steer the next turn by trying
    /// different timestamps or contents.
    pub fn next_validator(&self) -> Option<Address> {
        self.turn_order().into_iter().next()
    }

    /// The order validators may forge the next block in: the drawn one at once, then each one
    /// after it a turn later, in case those before it are away. Empty on proof-of-work chains,
    /// and while anyone may forge.
    pub fn turn_order(&self) -> Vec<Address> {
        if !self.rules.is_proof_of_stake() {
            return Vec::new();
        }
        turn_order(&self.tally().ledger.validator_stakes(), self.tally().ledger.beacon())
    }

    /// When each in `turn_order` may forge the next block: one turn apart from the tip's time.
    pub fn turns_open_at(&self) -> Vec<(Address, DateTime<Utc>)> {
        let after = self.tip().timestamp;
        let order = self.turn_order().into_iter().enumerate();
        order.map(|(place, validator)| (validator, opens_at(after, place))).collect()
    }

    /// The randomness beacon the next validator is drawn from.
    pub fn beacon(&self) -> &Beacon {
        self.tally().ledger.beacon()
    }

    /// Where `address` stands with its reveal chain.
    pub fn validator_state(&self, address: &Address) -> ValidatorState {
        self.tally().ledger.validator_state(address)
    }

    /// Proof of work: bundles the reward and those of `pending` that apply into a new block.
    pub fn mine(&mut self, miner: &Address, pending: &[Transaction]) -> Result<&Block, BlockError> {
        let template = self.mining_template(miner, pending)?;
        let block = template.mine_while(|| true).expect("never told to stop");
        self.append_mined(block)
    }

    /// Proof of work: what `miner`'s next block would hold, so it can be mined without holding
    /// the chain.
    pub fn mining_template(
        &self,
        miner: &Address,
        pending: &[Transaction],
    ) -> Result<MiningTemplate, BlockError> {
        if self.rules.is_proof_of_stake() {
            return Err(BlockError::NotProofOfWork);
        }
        let (index, transactions) = self.next_contents(miner, pending);
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
    pub fn append_mined(&mut self, block: Block) -> Result<&Block, BlockError> {
        if self.rules.is_proof_of_stake() {
            return Err(BlockError::NotProofOfWork);
        }
        self.append_sealed(block)
    }

    /// Proof of work at a chosen time, so tests can play out any pace of blocks.
    #[cfg(test)]
    fn mine_at(&mut self, miner: &Address, at: DateTime<Utc>) -> Result<&Block, BlockError> {
        let work = self.next_work();
        self.append(miner, &[], at, |index, at, prev, txs| Block::mine(index, at, prev, txs, work))
    }

    /// Proof of stake: a validator whose turn has come signs the next block with `key`,
    /// revealing the next link of their reveal chain, which `key` also lets us work out.
    pub fn forge(
        &mut self,
        key: &SigningKey,
        pending: &[Transaction],
    ) -> Result<&Block, BlockError> {
        self.forge_at(key, pending, Utc::now())
    }

    /// Proof of stake, as `forge` does, but stamped `at`: now, or any time in tests.
    fn forge_at(
        &mut self,
        key: &SigningKey,
        pending: &[Transaction],
        at: DateTime<Utc>,
    ) -> Result<&Block, BlockError> {
        let validator = Address::of(key);
        self.check_turn(&validator, at)?;
        let reveal = self
            .due_reveal(&validator)
            .map(|(epoch, remaining)| randao::reveal(key, epoch, remaining));
        self.append(&validator, pending, at, |index, at, prev, txs| {
            Block::forge(index, at, prev, txs, key, reveal)
        })
    }

    /// Proof of stake, without the key: the block `validator`, whose turn has come, should sign
    /// next, carrying their `reveal`. It changes nothing; sign its hash and hand it to
    /// `append_forged`.
    pub fn propose(
        &self,
        validator: &Address,
        reveal: Option<Reveal>,
        pending: &[Transaction],
    ) -> Result<Block, BlockError> {
        let now = Utc::now();
        self.check_turn(validator, now)?;
        match (self.due_reveal(validator), &reveal) {
            (Some(_), None) => return Err(BlockError::BadReveal),
            (None, Some(_)) => return Err(BlockError::BadReveal),
            (Some(_), Some(r)) if !self.tally().ledger.clone().apply_reveal(validator, r) => {
                return Err(BlockError::BadReveal)
            }
            _ => {}
        }
        let (index, transactions) = self.next_contents(validator, pending);
        let prev_hash = self.tip().hash.clone();
        Ok(Block::propose(index, now, prev_hash, transactions, validator, reveal))
    }

    /// The commitment epoch and reveals left that `validator`'s next block must reveal from, or
    /// `None` while anyone may forge and no reveal is due.
    fn due_reveal(&self, validator: &Address) -> Option<(u64, u32)> {
        self.next_validator()?;
        let state = self.tally().ledger.validator_state(validator);
        Some((state.next_epoch.checked_sub(1)?, state.reveals_left?))
    }

    /// Proof of stake: appends a block signed elsewhere, after checking it like any other block.
    pub fn append_forged(&mut self, block: Block) -> Result<&Block, BlockError> {
        let validator = block.validator.clone().ok_or(BlockError::BadSignature)?;
        self.check_turn(&validator, block.timestamp)?;
        self.append_sealed(block)
    }

    /// Appends a block made elsewhere on our tip, after checking it like any other block.
    fn append_sealed(&mut self, block: Block) -> Result<&Block, BlockError> {
        if block.prev_hash != self.tip().hash || block.index != self.blocks.len() as u64 {
            return Err(BlockError::StaleBlock);
        }
        let mut tally = self.tally().clone();
        apply_block(&self.blocks, &block, &mut tally, &self.rules)?;
        self.blocks.push(Arc::new(block));
        self.tallies.push(tally);
        self.extended(self.blocks.len() - 1, self.blocks.len() - 1);
        Ok(self.tip())
    }

    /// Under proof of stake, whether `validator`'s turn to forge the next block has come by `at`.
    fn check_turn(&self, validator: &Address, at: DateTime<Utc>) -> Result<(), BlockError> {
        if !self.rules.is_proof_of_stake() {
            return Err(BlockError::NotProofOfStake);
        }
        check_place(&self.turn_order(), validator, self.tip().timestamp, at).map(drop)
    }

    /// The next block's index and transactions: a payout of the reward plus fees to `producer`,
    /// then those of `pending` that apply on top of this chain, in the order given, as many as a
    /// block holds. Which to offer, and in what order, is the producer's choice; a mempool puts
    /// the highest fees first. It may lag the chain for a moment, so its candidates are checked
    /// here rather than trusted.
    fn next_contents(
        &self,
        producer: &Address,
        pending: &[Transaction],
    ) -> (u64, Vec<Transaction>) {
        let index = self.blocks.len() as u64;
        // Each is tried on top of the ones kept before it, and trying stops once the block is
        // full. A repeat of one already kept fails its nonce.
        let mut ledger = self.ledger().clone();
        let fitting: Vec<_> = pending
            .iter()
            .filter(|tx| ledger.apply_transfer(tx, Utc::now()).is_ok())
            .take(self.rules.block_capacity.limit())
            .cloned()
            .collect();
        // Only fees their senders can afford get this far, so they always add up.
        let payout = payout(self.rules.reward, &fitting).expect("affordable fees add up");
        let mut transactions = vec![Transaction::coinbase(producer, payout, index)];
        transactions.extend(fitting);
        (index, transactions)
    }

    /// Pays `producer` the reward plus fees, includes what of `pending` applies and appends the
    /// block `seal` makes, after checking it exactly as a peer replaying the chain would.
    fn append(
        &mut self,
        producer: &Address,
        pending: &[Transaction],
        at: DateTime<Utc>,
        seal: impl FnOnce(u64, DateTime<Utc>, BlockHash, Vec<Transaction>) -> Block,
    ) -> Result<&Block, BlockError> {
        let (index, transactions) = self.next_contents(producer, pending);
        let block = seal(index, at, self.tip().hash.clone(), transactions);
        let mut tally = self.tally().clone();
        apply_block(&self.blocks, &block, &mut tally, &self.rules)?;
        self.blocks.push(Arc::new(block));
        self.tallies.push(tally);
        self.extended(self.blocks.len() - 1, self.blocks.len() - 1);
        Ok(self.tip())
    }

    /// The fork-choice rule: adopts `blocks` if they form a valid chain heavier than ours. Exact
    /// ties go to the lower tip hash, so every node settles on the same fork. Never undoes more
    /// than `reorg_limit` of our blocks. The transfers of our blocks it drops travel in its
    /// `Reorganized` event, for a mempool to take back.
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
        let keep = self.reorg_limit.tallies_to_keep();
        let (chain, kept) = match shared.checked_sub(1).and_then(|h| self.tallies.after(h)) {
            Some(base) => {
                let base = base.clone();
                let mut chain = self.blocks[..shared].to_vec();
                chain.extend(blocks.into_iter().skip(shared).map(Arc::new));
                let kept = replay_from(&chain, shared, base, &self.rules, keep)?;
                (chain, kept)
            }
            None => {
                let kept = replay_kept(&blocks, &self.rules, keep)?;
                (blocks.into_iter().map(Arc::new).collect(), kept)
            }
        };
        let tally = kept.back().expect("replays keep at least the last").clone();
        let theirs =
            (tally.weight, Reverse(&chain.last().expect("replay refuses empty chains").hash));
        if theirs <= (self.tally().weight, Reverse(&self.tip().hash)) {
            return Ok(false);
        }

        let depth = self.blocks.len() - shared;
        if !self.reorg_limit.allows(depth) {
            return Err(ChainError::ReorgTooDeep { depth, max: self.reorg_limit.blocks() });
        }
        let returned: Vec<Transaction> = self.blocks[shared..]
            .iter()
            .flat_map(|block| block.transactions.iter().skip(1))
            .cloned()
            .collect();

        self.tallies.replace_from(shared, chain.len() - kept.len(), kept);
        let ours = self.blocks.len();
        self.blocks = chain;
        if shared == ours {
            // Theirs is ours with more on top.
            self.extended(shared, self.blocks.len() - 1);
        } else {
            let (dropped, added) = (ours - shared, self.blocks.len() - shared);
            let from = shared;
            self.events.push(ChainEvent::Reorganized { from, dropped, added, returned });
        }
        Ok(true)
    }

    /// Takes another node's blocks from some height on, so peers can send only what's new. Blocks
    /// that build on our tip are checked one by one, without replaying the rest of the chain. A
    /// stretch that forks off one of our blocks is weighed as a whole chain by `adopt_if_heavier`.
    /// Blocks from height 0 are a whole chain already.
    pub fn extend(&mut self, blocks: Vec<Block>) -> Result<Extension, ChainError> {
        let start = blocks.first().ok_or(ChainError::Empty)?.index as usize;
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
            let mut candidate: Vec<Block> =
                self.blocks[..start].iter().map(|block| Block::clone(block)).collect();
            candidate.extend(blocks);
            return self.adopt_if_heavier(candidate).map(Extension::from);
        }

        let mut tally = self.tally().clone();
        let height = self.blocks.len();
        let mut tallies = Vec::with_capacity(new.len());
        for block in new {
            if let Err(reason) = apply_block(&self.blocks, block, &mut tally, &self.rules) {
                let at = self.blocks.len() as u64;
                self.blocks.truncate(height);
                return Err(ChainError::InvalidBlock { height: at, reason });
            }
            self.blocks.push(Arc::new(block.clone()));
            tallies.push(tally.clone());
        }
        for tally in tallies {
            self.tallies.push(tally);
        }
        self.extended(height, self.blocks.len() - 1);
        Ok(Extension::Adopted)
    }

    /// Replays the whole chain: links, hashes, consensus, rewards, signatures and balances. Says
    /// which block first breaks a rule, and how.
    pub fn validate(&self) -> Result<(), ChainError> {
        validate_blocks(&self.blocks, &self.rules)
    }

    pub fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }
}

#[cfg(test)]
mod tests;
