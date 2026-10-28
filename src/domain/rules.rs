use std::fmt;

use super::crypto;

/// How a chain decides who may add the next block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consensus {
    /// Anyone may add a block whose hash starts with `difficulty` hex zeros. Finding one takes work.
    ProofOfWork { difficulty: usize },
    /// One staker per block, picked at random and weighted by stake, signs it. Costs no work,
    /// but a validator who cheats can be spotted from their signature.
    ProofOfStake,
}

/// The consensus rules. Every node on a network has to share them, or their chains won't validate
/// on each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainRules {
    pub consensus: Consensus,
    /// Coins paid to whoever adds a block.
    pub reward: u64,
    /// The most of its own blocks a node will throw away to switch forks. Anything deeper is
    /// treated as an attack rewriting old history, however heavy the other fork is.
    pub max_reorg_depth: u64,
}

pub const DEFAULT_MAX_REORG_DEPTH: u64 = 64;

impl ChainRules {
    pub const fn proof_of_work(difficulty: usize, reward: u64) -> Self {
        Self {
            consensus: Consensus::ProofOfWork { difficulty },
            reward,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
        }
    }

    pub const fn proof_of_stake(reward: u64) -> Self {
        Self {
            consensus: Consensus::ProofOfStake,
            reward,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
        }
    }

    pub const fn with_max_reorg_depth(self, max_reorg_depth: u64) -> Self {
        Self { max_reorg_depth, ..self }
    }

    pub fn is_proof_of_stake(&self) -> bool {
        self.consensus == Consensus::ProofOfStake
    }
}

impl fmt::Display for ChainRules {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.consensus {
            Consensus::ProofOfWork { difficulty } => {
                write!(f, "proof of work, difficulty {difficulty}")?
            }
            Consensus::ProofOfStake => write!(f, "proof of stake")?,
        }
        write!(f, ", reward {}", self.reward)
    }
}

impl Default for ChainRules {
    fn default() -> Self {
        Self::proof_of_work(4, 50)
    }
}

pub fn meets_difficulty(hash: &str, difficulty: usize) -> bool {
    hash.len() >= difficulty && hash.bytes().take(difficulty).all(|b| b == b'0')
}

/// Picks who forges the block after `prev_hash`: a stake-weighted draw seeded by that hash, so
/// every node computes the same answer. `stakes` must be sorted by address. `None` while nothing
/// is staked, when anyone may forge.
pub fn select_validator(stakes: &[(String, u64)], prev_hash: &str) -> Option<String> {
    let total: u64 = stakes.iter().map(|(_, amount)| amount).sum();
    if total == 0 {
        return None;
    }
    let digest = hex::decode(crypto::sha256_hex(prev_hash.as_bytes())).expect("sha256 is hex");
    let seed = u64::from_be_bytes(digest[..8].try_into().expect("8 bytes")) % total;
    let mut cumulative = 0;
    stakes.iter().find_map(|(address, amount)| {
        cumulative += amount;
        (seed < cumulative).then(|| address.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difficulty_counts_leading_zeros() {
        assert!(meets_difficulty("000abc", 3));
        assert!(meets_difficulty("0000ab", 3));
        assert!(!meets_difficulty("00abcd", 3));
        assert!(!meets_difficulty("00", 3));
    }

    #[test]
    fn nobody_is_selected_without_stake() {
        assert_eq!(select_validator(&[], "abc"), None);
        assert_eq!(select_validator(&[("a".into(), 0)], "abc"), None);
    }

    #[test]
    fn selection_is_deterministic_and_weighted_by_stake() {
        let stakes = [("alice".to_string(), 90), ("bob".to_string(), 10)];
        assert_eq!(select_validator(&stakes, "seed"), select_validator(&stakes, "seed"));

        let alice = (0..1_000)
            .filter(|i| select_validator(&stakes, &i.to_string()).as_deref() == Some("alice"))
            .count();
        assert!((850..=950).contains(&alice), "alice was picked {alice} times out of 1000");
    }
}
