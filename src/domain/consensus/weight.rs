use std::fmt;

use serde::{Deserialize, Serialize};

use super::rules::{ChainRules, Consensus};
use crate::domain::values::Coins;

/// What a chain is worth when forks compete: the total expected work behind it under proof of
/// work, or the total stake behind its blocks under proof of stake. The heavier chain wins.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChainWeight(u128);

impl ChainWeight {
    pub const ZERO: Self = Self(0);

    pub fn value(self) -> u128 {
        self.0
    }

    /// This weight plus one more block's: `work` under proof of work, `producer_stake` under
    /// proof of stake.
    pub fn with_block(self, rules: &ChainRules, producer_stake: Coins, work: u128) -> Self {
        Self(self.0.saturating_add(block_weight(rules, producer_stake, work)))
    }
}

impl fmt::Display for ChainWeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One block's weight. Proof of work: the work it required, the hashes it takes on average to
/// find. Proof of stake: what its validator had staked going in, or 1 while nothing is staked, so
/// blocks forged by big stakers outweigh blocks anyone could have forged.
fn block_weight(rules: &ChainRules, producer_stake: Coins, work: u128) -> u128 {
    match rules.consensus {
        Consensus::ProofOfWork { .. } => work,
        Consensus::ProofOfStake => u128::from(producer_stake.value().max(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_grows_sixteenfold_per_zero() {
        let block = |d| {
            let rules = ChainRules::proof_of_work(d, 50);
            ChainWeight::ZERO.with_block(&rules, Coins::ZERO, rules.initial_work()).value()
        };
        assert_eq!(block(0), 1);
        assert_eq!(block(2), 256);
        assert_eq!(block(4), 65_536);
        assert_eq!(block(64), u128::MAX, "saturates instead of overflowing");
    }

    #[test]
    fn stake_weighs_what_the_validator_staked_and_at_least_one() {
        let pos = ChainRules::proof_of_stake(50);
        assert_eq!(ChainWeight::ZERO.with_block(&pos, Coins::ZERO, 0).value(), 1);
        let (thirty, ten) = (Coins::new(30), Coins::new(10));
        assert_eq!(
            ChainWeight::ZERO.with_block(&pos, thirty, 0).with_block(&pos, ten, 0).value(),
            40
        );
    }
}
