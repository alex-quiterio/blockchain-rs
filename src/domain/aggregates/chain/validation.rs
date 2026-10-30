//! The rules every block must follow, and replaying a chain by them: links, hashes, Merkle roots,
//! the proof of work or the validator's signature, size, rewards, fees, nonces, balances and stakes.

use std::collections::VecDeque;

use chrono::Utc;

use super::tally::Tally;
use crate::domain::block::{Block, Transaction};
use crate::domain::difficulty::{self, meets_work};
use crate::domain::error::{BlockError, ChainError};
use crate::domain::ledger::Ledger;
use crate::domain::rules::{select_validator, ChainRules, Consensus};
use crate::domain::values::{BlockHash, Coins};

/// Replays `blocks` by every rule, or says which block first breaks one and how.
pub fn validate_blocks(blocks: &[Block], rules: &ChainRules) -> Result<(), ChainError> {
    replay(blocks, rules).map(|_| ())
}

/// Checks every block in order and returns what they add up to.
pub(super) fn replay(blocks: &[Block], rules: &ChainRules) -> Result<Tally, ChainError> {
    Ok(replay_kept(blocks, rules, 1)?.pop_back().expect("replays keep at least the last"))
}

/// Replays the whole chain, keeping what it added up to after each of its last `keep` blocks.
pub(super) fn replay_kept(
    blocks: &[Block],
    rules: &ChainRules,
    keep: usize,
) -> Result<VecDeque<Tally>, ChainError> {
    let genesis = blocks.first().ok_or(ChainError::Empty)?;
    check_genesis(genesis, rules)
        .map_err(|reason| ChainError::InvalidBlock { height: 0, reason })?;
    replay_from(blocks, 1, Tally::at_genesis(genesis, rules), rules, keep)
}

/// Checks `blocks[from..]` in order, starting from `tally`, what `blocks[..from]` add up to.
/// Returns what the chain added up to after each of its last `keep` blocks, the last one at
/// least. `from` is at least 1: genesis is never replayed.
pub(super) fn replay_from(
    blocks: &[Block],
    from: usize,
    mut tally: Tally,
    rules: &ChainRules,
    keep: usize,
) -> Result<VecDeque<Tally>, ChainError> {
    let first_kept = blocks.len().saturating_sub(keep.max(1));
    let mut kept = VecDeque::new();
    if from > first_kept {
        kept.push_back(tally.clone());
    }
    for height in from..blocks.len() {
        apply_block(&blocks[..height], &blocks[height], &mut tally, rules)
            .map_err(|reason| ChainError::InvalidBlock { height: height as u64, reason })?;
        if height >= first_kept {
            kept.push_back(tally.clone());
        }
    }
    Ok(kept)
}

/// The reward plus every fee in `transfers`, or `None` if that's too large to hold, which no
/// valid block can be.
pub(super) fn payout(reward: Coins, transfers: &[Transaction]) -> Option<Coins> {
    Coins::total(transfers.iter().map(|tx| tx.fee))?.checked_add(reward)
}

fn check_genesis(genesis: &Block, rules: &ChainRules) -> Result<(), BlockError> {
    let made_by_the_rules = genesis.index == 0
        && genesis.prev_hash == BlockHash::zero()
        && genesis.timestamp.timestamp() == 0
        && genesis.transactions.is_empty()
        && is_sealed_genesis(genesis, rules);
    if !made_by_the_rules {
        return Err(BlockError::BadGenesis);
    }
    check_hashes(genesis)
}

/// The block's header covers its transactions through the Merkle root, and its hash covers the
/// header.
fn check_hashes(block: &Block) -> Result<(), BlockError> {
    if !block.has_matching_root() {
        return Err(BlockError::MerkleRootMismatch);
    }
    if block.hash != block.computed_hash() {
        return Err(BlockError::HashMismatch);
    }
    Ok(())
}

/// Whether `blocks` are a chain of hashes from our genesis: each block's hash covers its header,
/// its header covers its transactions through the Merkle root, and it names the block before it.
/// No signatures, work or balances are checked: that's for a snapshot taken after these very
/// blocks, which already did.
pub(super) fn links_up(blocks: &[Block], rules: &ChainRules) -> bool {
    blocks.first().is_some_and(|genesis| check_genesis(genesis, rules).is_ok())
        && blocks
            .windows(2)
            .all(|pair| pair[1].index == pair[0].index + 1 && pair[1].prev_hash == pair[0].hash)
        && blocks.iter().all(|block| check_hashes(block).is_ok())
}

/// Checks that `block` validly follows the blocks `before` it and adds it to `tally`, or says
/// which rule it breaks. On an error the tally may be half-updated, so callers check a copy.
pub(super) fn apply_block(
    before: &[Block],
    block: &Block,
    tally: &mut Tally,
    rules: &ChainRules,
) -> Result<(), BlockError> {
    let prev = before.last().expect("genesis comes first");
    let work = difficulty::next_work(before, tally.work, rules.retarget());
    if block.index != prev.index + 1 || block.prev_hash != prev.hash {
        return Err(BlockError::BrokenLink);
    }
    check_hashes(block)?;
    difficulty::check_timestamp(before, block.timestamp, Utc::now())?;
    check_seal(block, &tally.ledger, rules, work)?;

    let ledger = &mut tally.ledger;
    let (coinbase, transfers) =
        block.transactions.split_first().ok_or(BlockError::MalformedPayout)?;
    let producer = coinbase.to.account().ok_or(BlockError::MalformedPayout)?;
    if !coinbase.is_coinbase()
        || !coinbase.fee.is_zero()
        || coinbase.nonce != block.index
        || block.validator.as_ref().is_some_and(|v| v != producer)
    {
        return Err(BlockError::MalformedPayout);
    }
    let capacity = rules.block_capacity;
    if let (false, Some(max)) = (capacity.holds(transfers.len()), capacity.max()) {
        return Err(BlockError::TooManyTransfers { max, got: transfers.len() });
    }
    let expected = payout(rules.reward, transfers);
    if expected != Some(coinbase.amount) {
        return Err(BlockError::WrongPayout { expected, got: coinbase.amount });
    }
    check_reveal(block, ledger, rules)?;

    tally.weight = tally.weight.with_block(rules, ledger.staked(producer), work);
    tally.work = work;
    ledger.credit(producer, coinbase.amount);
    for (position, tx) in block.transactions.iter().enumerate().skip(1) {
        ledger.apply_transfer(tx).map_err(|reason| BlockError::BadTransfer { position, reason })?;
    }
    Ok(())
}

/// Proof of stake: when a validator was selected, the block must open their commitment, which
/// moves their reveal chain along and mixes the reveal into the beacon. While anyone may forge,
/// there's nothing to reveal and the beacon stays put, so nobody can steer it then either.
fn check_reveal(block: &Block, ledger: &mut Ledger, rules: &ChainRules) -> Result<(), BlockError> {
    if !rules.is_proof_of_stake() {
        return Ok(());
    }
    let selected = select_validator(&ledger.validator_stakes(), ledger.beacon()).is_some();
    let opened = match (&block.validator, &block.reveal) {
        (Some(validator), Some(reveal)) if selected => ledger.apply_reveal(validator, reveal),
        (_, None) => !selected,
        _ => false,
    };
    opened.then_some(()).ok_or(BlockError::BadReveal)
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
fn check_seal(
    block: &Block,
    ledger: &Ledger,
    rules: &ChainRules,
    work: u128,
) -> Result<(), BlockError> {
    match rules.consensus {
        Consensus::ProofOfWork { .. } => {
            if block.validator.is_some() || block.signature.is_some() || block.reveal.is_some() {
                return Err(BlockError::WrongSeal);
            }
            meets_work(block.hash.as_str(), work).then_some(()).ok_or(BlockError::InsufficientWork)
        }
        Consensus::ProofOfStake => {
            if block.nonce != 0 {
                return Err(BlockError::WrongSeal);
            }
            if !block.is_signed_by_validator() {
                return Err(BlockError::BadSignature);
            }
            match select_validator(&ledger.validator_stakes(), ledger.beacon()) {
                Some(expected) if block.validator.as_ref() != Some(&expected) => {
                    Err(BlockError::NotYourTurn { expected })
                }
                _ => Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::address::Address;
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
