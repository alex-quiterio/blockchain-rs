//! The rules every block must follow, and replaying a chain by them: links, hashes, Merkle roots,
//! the proof of work or the validator's signature, rewards, fees, nonces, balances and stakes.

use std::collections::VecDeque;

use chrono::Utc;

use super::tally::Tally;
use crate::domain::address::Address;
use crate::domain::block::{Block, Transaction};
use crate::domain::difficulty::{self, meets_work};
use crate::domain::ledger::Ledger;
use crate::domain::rules::{select_validator, ChainRules, Consensus};
use crate::domain::values::{BlockHash, Coins};

pub fn validate_blocks(blocks: &[Block], rules: &ChainRules) -> bool {
    replay(blocks, rules).is_some()
}

/// Checks every block in order and returns what they add up to, or `None` if any check fails.
pub(super) fn replay(blocks: &[Block], rules: &ChainRules) -> Option<Tally> {
    replay_kept(blocks, rules, 1)?.pop_back()
}

/// Replays the whole chain, keeping what it added up to after each of its last `keep` blocks.
pub(super) fn replay_kept(
    blocks: &[Block],
    rules: &ChainRules,
    keep: usize,
) -> Option<VecDeque<Tally>> {
    let genesis = blocks.first().filter(|genesis| is_genesis(genesis, rules))?;
    replay_from(blocks, 1, Tally::at_genesis(genesis, rules), rules, keep)
}

/// Checks `blocks[from..]` in order, starting from `tally`, what `blocks[..from]` add up to.
/// Returns what the chain added up to after each of its last `keep` blocks, the last one at
/// least, or `None` if a check fails. `from` is at least 1: genesis is never replayed.
pub(super) fn replay_from(
    blocks: &[Block],
    from: usize,
    mut tally: Tally,
    rules: &ChainRules,
    keep: usize,
) -> Option<VecDeque<Tally>> {
    let first_kept = blocks.len().saturating_sub(keep.max(1));
    let mut kept = VecDeque::new();
    if from > first_kept {
        kept.push_back(tally.clone());
    }
    for height in from..blocks.len() {
        if !apply_block(&blocks[..height], &blocks[height], &mut tally, rules) {
            return None;
        }
        if height >= first_kept {
            kept.push_back(tally.clone());
        }
    }
    Some(kept)
}

/// How many blocks' tallies to keep: enough to check any fork we may adopt from its split.
/// The reward plus every fee in `transfers`, or `None` if that's too large to hold, which no
/// valid block can be.
pub(super) fn payout(reward: Coins, transfers: &[Transaction]) -> Option<Coins> {
    Coins::total(transfers.iter().map(|tx| tx.fee))?.checked_add(reward)
}

pub(super) fn keep(rules: &ChainRules) -> usize {
    rules.max_reorg_depth as usize + 1
}

fn is_genesis(genesis: &Block, rules: &ChainRules) -> bool {
    genesis.index == 0
        && genesis.prev_hash == BlockHash::zero()
        && genesis.timestamp.timestamp() == 0
        && genesis.transactions.is_empty()
        && genesis.has_matching_root()
        && genesis.hash == genesis.computed_hash()
        && is_sealed_genesis(genesis, rules)
}

/// Whether `blocks` are a chain of hashes from our genesis: each block's hash covers its header,
/// its header covers its transactions through the Merkle root, and it names the block before it.
/// No signatures, work or balances are checked: that's for a snapshot taken after these very
/// blocks, which already did.
pub(super) fn links_up(blocks: &[Block], rules: &ChainRules) -> bool {
    blocks.first().is_some_and(|genesis| is_genesis(genesis, rules))
        && blocks
            .windows(2)
            .all(|pair| pair[1].index == pair[0].index + 1 && pair[1].prev_hash == pair[0].hash)
        && blocks
            .iter()
            .all(|block| block.has_matching_root() && block.hash == block.computed_hash())
}

/// Checks that `block` validly follows the blocks `before` it and adds it to `tally`. On `false`
/// the tally may be half-updated, so callers check a copy.
pub(super) fn apply_block(
    before: &[Block],
    block: &Block,
    tally: &mut Tally,
    rules: &ChainRules,
) -> bool {
    let prev = before.last().expect("genesis comes first");
    let work = difficulty::next_work(before, tally.work, rules.retarget());
    if block.index != prev.index + 1
        || block.prev_hash != prev.hash
        || !block.has_matching_root()
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
    let Some(producer) = coinbase.to.account().map(|a| a.as_str()) else {
        return false;
    };
    if !coinbase.is_coinbase()
        || Some(coinbase.amount) != payout(rules.reward, transfers)
        || !coinbase.fee.is_zero()
        || coinbase.nonce != block.index
        || block.validator.as_ref().is_some_and(|v| v.as_str() != producer)
    {
        return false;
    }
    if !reveals_as_required(block, ledger, rules) {
        return false;
    }
    tally.weight = tally.weight.with_block(rules, ledger.staked(producer), work);
    tally.work = work;
    ledger.credit(producer, coinbase.amount);
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
        (Some(validator), Some(reveal)) if selected => {
            ledger.apply_reveal(validator.as_str(), reveal)
        }
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
            unsigned && meets_work(genesis.hash.as_str(), rules.initial_work())
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
                && meets_work(block.hash.as_str(), work)
        }
        Consensus::ProofOfStake => {
            block.nonce == 0
                && block.is_signed_by_validator()
                && select_validator(&ledger.validator_stakes(), ledger.beacon()).is_none_or(
                    |expected| {
                        block.validator.as_ref().map(Address::as_str) == Some(expected.as_str())
                    },
                )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::crypto;

    #[test]
    fn fees_too_large_to_add_up_make_no_valid_payout() {
        let to = Address::parse(&crypto::generate_keypair().0).unwrap();
        let fee =
            |n| Transaction { fee: Coins::new(n), ..Transaction::coinbase(&to, Coins::ZERO, 1) };
        let reward = Coins::new(50);
        assert_eq!(payout(reward, &[fee(2), fee(3)]), Some(Coins::new(55)));
        // In plain u64 this wrapped to 48 in a release build, a payout a block could claim.
        assert_eq!(payout(reward, &[fee(u64::MAX), fee(u64::MAX)]), None);
        assert_eq!(payout(Coins::new(u64::MAX), &[fee(1)]), None);
    }
}
