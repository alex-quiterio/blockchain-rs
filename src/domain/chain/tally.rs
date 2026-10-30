//! What a chain adds up to, and how much of that it remembers: the tally after each recent block,
//! and now and then a snapshot of an old one. The chain's rules decide what goes in a tally; this
//! module only keeps them.

use std::collections::VecDeque;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::domain::accounting::ledger::Ledger;
use crate::domain::chain::block::Block;
use crate::domain::chain::block::BlockHash;
use crate::domain::consensus::rules::ChainRules;
use crate::domain::consensus::weight::ChainWeight;

/// What replaying blocks in order adds up to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Tally {
    /// Balances, stakes, nonces and the randomness beacon.
    pub(super) ledger: Ledger,
    /// What the blocks are worth when forks compete.
    pub(super) weight: ChainWeight,
    /// Proof of work: the work the latest block required, which the next one starts from.
    pub(super) work: u128,
}

impl Tally {
    pub(super) fn at_genesis(genesis: &Block, rules: &ChainRules) -> Self {
        Self {
            ledger: Ledger::starting_at(&genesis.hash),
            weight: ChainWeight::ZERO,
            work: rules.initial_work(),
        }
    }
}

/// The tally after each of a chain's last few blocks, oldest first, the last being its tip's:
/// enough to check any fork the chain may adopt from where it splits off, instead of from
/// genesis. Shared behind `Arc`s, so copying a chain doesn't copy them.
#[derive(Debug, Clone)]
pub(super) struct TallyLog {
    /// The height of the block the oldest tally is after.
    first: usize,
    tallies: VecDeque<Arc<Tally>>,
    /// How many to keep: one more than the deepest fork the chain may adopt.
    keep: usize,
}

impl TallyLog {
    /// A log of `tallies`, the first being after the block at height `first`, the last after the
    /// tip. Only the last `keep` are kept.
    pub(super) fn new(first: usize, tallies: impl IntoIterator<Item = Tally>, keep: usize) -> Self {
        let tallies = tallies.into_iter().map(Arc::new).collect();
        let mut log = Self { first, tallies, keep: keep.max(1) };
        log.trim();
        assert!(!log.tallies.is_empty(), "a chain has at least genesis");
        log
    }

    /// The same log, keeping the last `keep` from now on.
    pub(super) fn keeping(mut self, keep: usize) -> Self {
        self.keep = keep.max(1);
        self.trim();
        self
    }

    /// The tally after the tip: what the whole chain adds up to.
    pub(super) fn latest(&self) -> &Tally {
        self.tallies.back().expect("never empty")
    }

    /// The tally after the block at `height`, if that's recent enough to be kept.
    pub(super) fn after(&self, height: usize) -> Option<&Tally> {
        height.checked_sub(self.first).and_then(|i| self.tallies.get(i)).map(Arc::as_ref)
    }

    /// Records the tally after one more block on top.
    pub(super) fn push(&mut self, tally: Tally) {
        self.tallies.push_back(Arc::new(tally));
        self.trim();
    }

    /// For a fork that replaced the blocks from height `split` on: keeps ours before it, then
    /// records `theirs`, the first being after the block at height `first_theirs`. If either
    /// side doesn't reach the split, theirs alone are kept.
    pub(super) fn replace_from(
        &mut self,
        split: usize,
        first_theirs: usize,
        theirs: VecDeque<Tally>,
    ) {
        if first_theirs > split || self.first > split {
            *self = Self::new(first_theirs, theirs, self.keep);
            return;
        }
        self.tallies.truncate(split - self.first);
        self.tallies.extend(theirs.into_iter().skip(split - first_theirs).map(Arc::new));
        self.trim();
    }

    /// A snapshot after the oldest block kept, once the log is full: that block is as deep as
    /// any fork may reach, so no fork can undo it.
    pub(super) fn snapshot(&self, blocks: &[Arc<Block>], rules: ChainRules) -> Option<Snapshot> {
        if self.tallies.len() < self.keep {
            return None;
        }
        Some(Snapshot {
            height: self.first,
            hash: blocks[self.first].hash.clone(),
            rules,
            tally: self.tallies[0].as_ref().clone(),
        })
    }

    /// The heights of the blocks a tally is kept after.
    #[cfg(test)]
    pub(super) fn heights(&self) -> std::ops::Range<usize> {
        self.first..self.first + self.tallies.len()
    }

    fn trim(&mut self) {
        while self.tallies.len() > self.keep {
            self.tallies.pop_front();
            self.first += 1;
        }
    }
}

/// What the chain adds up to after a block no fork may undo any more, with that block's height
/// and hash. Saved now and then, so a restart replays only the blocks after it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub height: usize,
    pub hash: BlockHash,
    /// The rules it was made under; a snapshot is only used under the same ones.
    pub(super) rules: ChainRules,
    pub(super) tally: Tally,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tally told apart by its work, which here just labels the block it's after.
    fn after(height: u128) -> Tally {
        Tally { ledger: Ledger::default(), weight: ChainWeight::ZERO, work: height }
    }

    fn labels(log: &TallyLog) -> Vec<u128> {
        log.heights().map(|h| log.after(h).unwrap().work).collect()
    }

    #[test]
    fn keeps_only_the_last_few_and_knows_their_heights() {
        let mut log = TallyLog::new(0, [after(0), after(1)], 3);
        log.push(after(2));
        log.push(after(3));
        assert_eq!(log.heights(), 1..4);
        assert_eq!(labels(&log), [1, 2, 3]);
        assert_eq!(log.latest().work, 3);
        assert!(log.after(0).is_none(), "too old to keep");
        assert!(log.after(4).is_none(), "not there yet");
    }

    #[test]
    fn a_fork_keeps_ours_before_the_split_and_theirs_after() {
        let mut log = TallyLog::new(0, (0..5).map(after), 5);
        // Their blocks replace ours from height 3 on; they kept tallies from height 2.
        log.replace_from(3, 2, [after(2), after(30), after(40), after(50)].into());
        assert_eq!(log.heights(), 1..6);
        assert_eq!(labels(&log), [1, 2, 30, 40, 50]);
    }

    #[test]
    fn a_fork_that_reaches_past_what_either_side_kept_starts_over() {
        let mut log = TallyLog::new(4, (4..7).map(after), 3);
        log.replace_from(5, 6, [after(60), after(70), after(80)].into());
        assert_eq!(log.heights(), 6..9);
        assert_eq!(labels(&log), [60, 70, 80]);
    }

    #[test]
    fn a_snapshot_comes_once_the_log_is_full() {
        let rules = ChainRules::proof_of_work(1, 50);
        let blocks: Vec<Arc<Block>> = (0..3).map(|_| Arc::new(Block::genesis(&rules))).collect();
        let mut log = TallyLog::new(0, [after(0), after(1)], 3);
        assert!(log.snapshot(&blocks, rules).is_none());
        log.push(after(2));
        let snapshot = log.snapshot(&blocks, rules).unwrap();
        assert_eq!((snapshot.height, snapshot.tally.work), (0, 0));
    }
}
