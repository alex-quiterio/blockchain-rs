//! Proof-of-work difficulty as *work*, the hashes a block takes to find on average, and how it
//! adjusts to keep blocks coming at a steady pace. A domain service: functions over a chain's
//! blocks, with no state of their own.
//!
//! A hash meets `work` when its first 128 bits, read as a number, are at most `u128::MAX / work`.
//! For `work = 16^d` that is exactly "starts with d hex zeros", so chains mined before difficulty
//! could adjust read the same, but in between any value is possible: twice as hard, a third
//! easier.

use std::borrow::Borrow;

use chrono::{DateTime, TimeDelta, Utc};

use super::rules::Retarget;
use crate::domain::chain::block::Block;
use crate::domain::chain::block::BlockError;

/// The most one adjustment may change the work by, either way, as in Bitcoin.
pub const MAX_STEP: i64 = 4;
/// A block must be later than the median of this many blocks before it.
pub const MEDIAN_OF: usize = 11;
/// How far ahead of this node's clock a block's timestamp may be.
pub const MAX_FUTURE: TimeDelta = TimeDelta::hours(2);

pub fn meets_work(hash: &str, work: u128) -> bool {
    let Some(prefix) = hash.get(..32).and_then(|h| u128::from_str_radix(h, 16).ok()) else {
        return false;
    };
    prefix <= u128::MAX / work.max(1)
}

/// The work required of the block after `blocks`, given that the last one required `tip_work`.
///
/// Every `interval` blocks, the time the last `interval` blocks took is compared with the time
/// they should have taken, and the work is scaled by the difference, at most 4x either way. The
/// first adjustment waits until there are `interval` blocks after genesis to measure, since
/// genesis is stamped at the Unix epoch.
pub fn next_work(
    blocks: &[impl Borrow<Block>],
    tip_work: u128,
    retarget: Option<Retarget>,
) -> u128 {
    let Some(r) = retarget.filter(|r| r.interval > 0 && r.target_block_secs > 0) else {
        return tip_work;
    };
    let height = blocks.len() as u64;
    if height < 2 * r.interval || !height.is_multiple_of(r.interval) {
        return tip_work;
    }
    let first = blocks[(height - 1 - r.interval) as usize].borrow();
    let last = blocks[(height - 1) as usize].borrow();
    let expected = (r.target_block_secs * r.interval * 1000) as i64;
    let took = (last.timestamp - first.timestamp).num_milliseconds();
    let took = took.clamp((expected / MAX_STEP).max(1), expected * MAX_STEP);
    let scaled = tip_work.saturating_mul(expected as u128) / took as u128;
    // Clamping the time keeps the step within 4x, but rounding down could still overshoot on
    // small work, so the cap is applied to the result as well.
    let step = MAX_STEP as u128;
    scaled.clamp(tip_work.div_ceil(step), tip_work.saturating_mul(step)).max(1)
}

/// A block must come after the median of the ones before it, so a miner can't backdate it to
/// fake a slow pace, and no more than two hours ahead of `now`, so they can't future-date it.
pub fn check_timestamp(
    before: &[impl Borrow<Block>],
    timestamp: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), BlockError> {
    let mut recent: Vec<_> =
        before.iter().rev().take(MEDIAN_OF).map(|b| b.borrow().timestamp).collect();
    recent.sort();
    if recent.get(recent.len() / 2).is_some_and(|median| timestamp <= *median) {
        return Err(BlockError::TooEarly);
    }
    if timestamp > now + MAX_FUTURE {
        return Err(BlockError::TooFarAhead);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_of_16_to_the_d_means_d_leading_zeros() {
        for d in 1..8 {
            let work = 16u128.pow(d as u32);
            let just_enough = format!("{}{}", "0".repeat(d), "f".repeat(64 - d));
            let one_short = format!("{}1{}", "0".repeat(d - 1), "0".repeat(64 - d));
            assert!(meets_work(&just_enough, work), "{d} zeros meet 16^{d}");
            assert!(!meets_work(&one_short, work), "{} zeros don't", d - 1);
        }
        assert!(meets_work(&"f".repeat(64), 1), "work 1 accepts anything");
        assert!(!meets_work("not hex", 1));
    }

    #[test]
    fn rounding_never_breaks_the_4x_cap() {
        use crate::domain::consensus::rules::Retarget;
        let retarget = Some(Retarget { target_block_secs: 10, interval: 4 });
        // Eight blocks, the last window 2.5x too slow: 5 hashes would round down to 1.
        let at = |secs| Utc::now() - TimeDelta::days(1) + TimeDelta::seconds(secs);
        let mut block =
            Block::genesis(&crate::domain::consensus::rules::ChainRules::proof_of_work(0, 50));
        let blocks: Vec<Block> = [0, 1, 2, 3, 4, 29, 54, 79]
            .iter()
            .map(|&secs| {
                block.timestamp = at(secs);
                block.clone()
            })
            .collect();
        assert_eq!(next_work(&blocks, 5, retarget), 2, "not 1: that would be 5x easier");
    }

    #[test]
    fn work_in_between_whole_zeros_is_possible() {
        // Twice the work of one zero: the first hex digit must be 0, and the second at most 7.
        assert!(meets_work(&format!("07{}", "f".repeat(62)), 32));
        assert!(!meets_work(&format!("08{}", "0".repeat(62)), 32));
    }
}
