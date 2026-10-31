//! Proof of stake's turns: the order validators may forge the next block in, and when each one
//! may. A domain service: every node works out the same order from the same stakes and beacon,
//! and nothing here keeps state.
//!
//! The first in line is the validator the beacon draws, and it may forge at once. If it hasn't
//! after `TURN_SECS`, the next in line may too, and so on, so a validator who's offline delays
//! the chain by a turn instead of stopping it for good. A block's timestamp says whose turn it
//! came in; it can't be put far ahead, so nobody can skip the line by claiming a turn early.

use chrono::{DateTime, TimeDelta, Utc};

use super::rules::select_validator;
use crate::domain::accounting::Coins;
use crate::domain::consensus::Beacon;
use crate::domain::crypto::address::Address;

/// How long each validator in line has to forge before the next one may too.
pub const TURN_SECS: i64 = 30;

/// How far ahead of a node's clock a stand-in's block may claim its turn came. Clocks drift a
/// little; any further and a stand-in could jump the line by stamping its block late enough.
pub const STAND_IN_DRIFT: TimeDelta = TimeDelta::seconds(15);

/// The order `stakes` may forge the block after `beacon` in: each drawn by stake from those not
/// drawn yet, the first exactly as `select_validator` draws, so a chain whose validators were
/// all there forges as it always did. Empty while nothing is staked, when anyone may forge.
pub fn turn_order(stakes: &[(Address, Coins)], beacon: &Beacon) -> Vec<Address> {
    let (mut left, mut seed, mut order) = (stakes.to_vec(), beacon.clone(), Vec::new());
    while let Some(next) = select_validator(&left, &seed) {
        left.retain(|(address, _)| *address != next);
        order.push(next);
        seed = Beacon::of(format!("{seed}|next in line").as_bytes());
    }
    order
}

/// How many in line may forge a block made at `at`, after a parent made at `after`: the first at
/// once, and one more for every `TURN_SECS` since.
pub fn places_open(after: DateTime<Utc>, at: DateTime<Utc>) -> usize {
    let turns = (at - after).num_seconds().max(0) / TURN_SECS;
    usize::try_from(turns).unwrap_or(usize::MAX).saturating_add(1)
}

/// When the validator at `place` in line may forge, after a parent made at `after`.
pub fn opens_at(after: DateTime<Utc>, place: usize) -> DateTime<Utc> {
    let turns = i64::try_from(place).unwrap_or(i64::MAX);
    after + TimeDelta::seconds(TURN_SECS.saturating_mul(turns))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::crypto;

    fn stakes(amounts: &[u64]) -> Vec<(Address, Coins)> {
        let mut stakes: Vec<_> = amounts
            .iter()
            .map(|&n| (Address::parse(&crypto::generate_keypair().0).unwrap(), Coins::new(n)))
            .collect();
        stakes.sort();
        stakes
    }

    #[test]
    fn the_line_starts_with_the_drawn_validator_and_holds_everyone_once() {
        let stakes = stakes(&[5, 30, 1, 64]);
        for seed in 0..20u8 {
            let beacon = Beacon::of(&[seed]);
            let order = turn_order(&stakes, &beacon);
            assert_eq!(order.first(), select_validator(&stakes, &beacon).as_ref());
            let mut sorted = order.clone();
            sorted.sort();
            assert_eq!(sorted, stakes.iter().map(|(a, _)| a.clone()).collect::<Vec<_>>());
        }
        assert!(turn_order(&[], &Beacon::of(b"x")).is_empty(), "anyone may forge");
    }

    #[test]
    fn one_more_in_line_may_forge_each_turn() {
        let after = DateTime::<Utc>::from_timestamp(1_000, 0).unwrap();
        let at = |secs| after + TimeDelta::seconds(secs);
        assert_eq!(places_open(after, at(-5)), 1, "a block before its parent opens nothing more");
        assert_eq!(places_open(after, at(0)), 1);
        assert_eq!(places_open(after, at(TURN_SECS - 1)), 1);
        assert_eq!(places_open(after, at(TURN_SECS)), 2);
        assert_eq!(places_open(after, at(3 * TURN_SECS + 1)), 4);
        assert_eq!(opens_at(after, 0), after);
        assert_eq!(opens_at(after, 2), at(2 * TURN_SECS));
        assert_eq!(places_open(after, opens_at(after, 7)), 8);
    }
}
