use std::fmt;

use super::rules::{ChainRules, Consensus};

/// What a chain is worth when forks compete: the total expected work behind it under proof of
/// work, or the total stake behind its blocks under proof of stake. The heavier chain wins.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChainWeight(u128);

impl ChainWeight {
    pub const ZERO: Self = Self(0);

    pub fn value(self) -> u128 {
        self.0
    }

    /// This weight plus one more block's.
    pub fn with_block(self, rules: &ChainRules, producer_stake: u64) -> Self {
        Self(self.0.saturating_add(block_weight(rules, producer_stake)))
    }
}

impl fmt::Display for ChainWeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One block's weight. Proof of work: the hashes it takes on average to find, 16^difficulty.
/// Proof of stake: what its validator had staked going in, or 1 while nothing is staked, so
/// blocks forged by big stakers outweigh blocks anyone could have forged.
fn block_weight(rules: &ChainRules, producer_stake: u64) -> u128 {
    match rules.consensus {
        Consensus::ProofOfWork { difficulty } => {
            16u128.saturating_pow(difficulty.try_into().unwrap_or(u32::MAX))
        }
        Consensus::ProofOfStake => u128::from(producer_stake.max(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_grows_sixteenfold_per_zero() {
        let block = |d| ChainWeight::ZERO.with_block(&ChainRules::proof_of_work(d, 50), 0).value();
        assert_eq!(block(0), 1);
        assert_eq!(block(2), 256);
        assert_eq!(block(4), 65_536);
        assert_eq!(block(64), u128::MAX, "saturates instead of overflowing");
    }

    #[test]
    fn stake_weighs_what_the_validator_staked_and_at_least_one() {
        let pos = ChainRules::proof_of_stake(50);
        assert_eq!(ChainWeight::ZERO.with_block(&pos, 0).value(), 1);
        assert_eq!(ChainWeight::ZERO.with_block(&pos, 30).with_block(&pos, 10).value(), 40);
    }
}
