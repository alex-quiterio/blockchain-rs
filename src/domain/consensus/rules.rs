use std::fmt;

use serde::{Deserialize, Serialize};

use crate::domain::crypto;
use crate::domain::values::{Beacon, Coins};

/// How a chain decides who may add the next block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Consensus {
    /// Anyone may add a block whose hash is small enough. The chain starts out needing
    /// `difficulty` leading hex zeros, 16^difficulty hashes on average, and with `retarget` the
    /// requirement follows the pace of blocks from there.
    ProofOfWork { difficulty: usize, retarget: Option<Retarget> },
    /// One staker per block, picked at random and weighted by stake, signs it. Costs no work,
    /// but a validator who cheats can be spotted from their signature.
    ProofOfStake,
}

/// How proof-of-work difficulty keeps blocks coming at a steady pace as miners join or leave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retarget {
    /// The time the chain aims to take per block.
    pub target_block_secs: u64,
    /// How many blocks pass between adjustments.
    pub interval: u64,
}

/// The consensus rules. Every node on a network has to share them, or their chains won't validate
/// on each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainRules {
    pub consensus: Consensus,
    /// Coins paid to whoever adds a block.
    pub reward: Coins,
    /// The most of its own blocks a node will throw away to switch forks. Anything deeper is
    /// treated as an attack rewriting old history, however heavy the other fork is.
    pub max_reorg_depth: u64,
}

pub const DEFAULT_MAX_REORG_DEPTH: u64 = 64;

impl ChainRules {
    pub const fn proof_of_work(difficulty: usize, reward: u64) -> Self {
        Self {
            consensus: Consensus::ProofOfWork { difficulty, retarget: None },
            reward: Coins::new(reward),
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
        }
    }

    pub const fn proof_of_stake(reward: u64) -> Self {
        Self {
            consensus: Consensus::ProofOfStake,
            reward: Coins::new(reward),
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
        }
    }

    pub const fn with_max_reorg_depth(self, max_reorg_depth: u64) -> Self {
        Self { max_reorg_depth, ..self }
    }

    /// Proof of work only: adjusts the difficulty every `interval` blocks to aim for one block
    /// every `target_block_secs`.
    pub const fn with_retarget(self, target_block_secs: u64, interval: u64) -> Self {
        match self.consensus {
            Consensus::ProofOfWork { difficulty, .. } => {
                let retarget = Some(Retarget { target_block_secs, interval });
                Self { consensus: Consensus::ProofOfWork { difficulty, retarget }, ..self }
            }
            Consensus::ProofOfStake => self,
        }
    }

    /// Proof of work: the expected hashes per block the chain starts out at, 16^difficulty.
    pub fn initial_work(&self) -> u128 {
        match self.consensus {
            Consensus::ProofOfWork { difficulty, .. } => {
                16u128.saturating_pow(difficulty.try_into().unwrap_or(u32::MAX))
            }
            Consensus::ProofOfStake => 0,
        }
    }

    pub fn retarget(&self) -> Option<Retarget> {
        match self.consensus {
            Consensus::ProofOfWork { retarget, .. } => retarget,
            Consensus::ProofOfStake => None,
        }
    }

    pub fn is_proof_of_stake(&self) -> bool {
        self.consensus == Consensus::ProofOfStake
    }
}

impl fmt::Display for ChainRules {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.consensus {
            Consensus::ProofOfWork { difficulty, retarget: None } => {
                write!(f, "proof of work, difficulty {difficulty}")?
            }
            Consensus::ProofOfWork { difficulty, retarget: Some(r) } => write!(
                f,
                "proof of work, starting difficulty {difficulty}, adjusting every {} blocks \
                 towards one block per {}s",
                r.interval, r.target_block_secs
            )?,
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

/// Picks who forges the block after `prev_hash`: a stake-weighted draw seeded by that hash, so
/// every node computes the same answer. `stakes` must be sorted by address. `None` while nothing
/// is staked, when anyone may forge.
pub fn select_validator(stakes: &[(String, Coins)], beacon: &Beacon) -> Option<String> {
    let total = Coins::total(stakes.iter().map(|(_, amount)| *amount))?.value();
    if total == 0 {
        return None;
    }
    let digest =
        hex::decode(crypto::sha256_hex(beacon.as_str().as_bytes())).expect("sha256 is hex");
    let seed = u64::from_be_bytes(digest[..8].try_into().expect("8 bytes")) % total;
    let mut cumulative = 0;
    stakes.iter().find_map(|(address, amount)| {
        cumulative += amount.value();
        (seed < cumulative).then(|| address.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nobody_is_selected_without_stake() {
        assert_eq!(select_validator(&[], &Beacon::of(b"abc")), None);
        assert_eq!(select_validator(&[("a".into(), Coins::ZERO)], &Beacon::of(b"abc")), None);
    }

    #[test]
    fn selection_is_deterministic_and_weighted_by_stake() {
        let stakes = [("alice".to_string(), Coins::new(90)), ("bob".to_string(), Coins::new(10))];
        assert_eq!(
            select_validator(&stakes, &Beacon::of(b"seed")),
            select_validator(&stakes, &Beacon::of(b"seed"))
        );

        let alice = (0..1_000)
            .filter(|i| {
                select_validator(&stakes, &Beacon::of(i.to_string().as_bytes())).as_deref()
                    == Some("alice")
            })
            .count();
        assert!((850..=950).contains(&alice), "alice was picked {alice} times out of 1000");
    }
}
