//! What a chain is worth when forks compete. `ChainWeight` is a value object: a block adds its
//! work, or the stake behind its turn, to it, and the heavier fork wins.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::rules::{ChainRules, Consensus};
use crate::domain::accounting::Coins;

/// What a chain is worth when forks compete: the total expected work behind it under proof of
/// work, or the total stake behind its blocks' turns under proof of stake. The heavier chain
/// wins.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChainWeight(u128);

impl ChainWeight {
    pub const ZERO: Self = Self(0);

    pub fn value(self) -> u128 {
        self.0
    }

    /// This weight plus one more block's: `work` under proof of work. Under proof of stake,
    /// `in_line`, the stake of every validator the block's forger was drawn from, and `place`,
    /// the forger's place in line: 0 for the one drawn first.
    pub fn with_block(self, rules: &ChainRules, in_line: Coins, place: usize, work: u128) -> Self {
        Self(self.0.saturating_add(block_weight(rules, in_line, place, work)))
    }
}

impl fmt::Display for ChainWeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// How many turns skipped a proof-of-stake block's weight tells apart. Past this many, blocks
/// weigh the same however late in line their forger was.
const WEIGHED_PLACES: usize = 32;

/// One block's weight. Proof of work: the work it required, the hashes it takes on average to
/// find. Proof of stake: the stake in line, halved for each turn skipped, or 1 while nothing is
/// staked. So a block forged in turn outweighs one by any stand-in at the same height: nobody
/// gains by waiting out the drawn validator, or by forking around a block that came in time.
fn block_weight(rules: &ChainRules, in_line: Coins, place: usize, work: u128) -> u128 {
    match rules.consensus {
        Consensus::ProofOfWork { .. } => work,
        Consensus::ProofOfStake if in_line.is_zero() => 1,
        Consensus::ProofOfStake => {
            let halvings = WEIGHED_PLACES - place.min(WEIGHED_PLACES);
            u128::from(in_line.value()) << halvings
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_grows_sixteenfold_per_zero() {
        let block = |d| {
            let rules = ChainRules::proof_of_work(d, 50);
            ChainWeight::ZERO.with_block(&rules, Coins::ZERO, 0, rules.initial_work()).value()
        };
        assert_eq!(block(0), 1);
        assert_eq!(block(2), 256);
        assert_eq!(block(4), 65_536);
        assert_eq!(block(64), u128::MAX, "saturates instead of overflowing");
    }

    #[test]
    fn stake_weighs_the_stake_in_line_and_one_while_anyone_may_forge() {
        let pos = ChainRules::proof_of_stake(50);
        assert_eq!(ChainWeight::ZERO.with_block(&pos, Coins::ZERO, 0, 0).value(), 1);
        let (thirty, ten) = (Coins::new(30), Coins::new(10));
        let both = ChainWeight::ZERO.with_block(&pos, thirty, 0, 0).with_block(&pos, ten, 0, 0);
        assert_eq!(both.value(), 40 << WEIGHED_PLACES);
    }

    #[test]
    fn a_block_in_turn_outweighs_any_stand_ins() {
        let pos = ChainRules::proof_of_stake(50);
        let weigh = |place| ChainWeight::ZERO.with_block(&pos, Coins::new(1), place, 0);
        assert!(weigh(0) > weigh(1) && weigh(1) > weigh(2));
        let two_stand_ins = weigh(1).with_block(&pos, Coins::new(1), 1, 0);
        assert!(weigh(0).with_block(&pos, Coins::new(1), 0, 0) > two_stand_ins);
        assert_eq!(weigh(WEIGHED_PLACES), weigh(WEIGHED_PLACES + 5), "late enough is all one");
        assert_eq!(weigh(usize::MAX).value(), 1);
    }
}
