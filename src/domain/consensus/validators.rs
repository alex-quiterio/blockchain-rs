//! Proof of stake's randomness: each validator's reveal chain, and the beacon their reveals mix
//! into. Stakes decide how likely a validator is to be drawn; this decides who can be drawn at
//! all, and the draw's seed.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::randao;
use crate::domain::address::Address;
use crate::domain::values::{Beacon, Coins, Commitment, Reveal};

/// Every validator's reveal chain and the beacon. Part of the ledger, since stakes and
/// commitments change together, but its own model: nothing here moves coins.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Validators {
    /// Each validator's reveal chain: the link they showed last and how many are left.
    commitments: HashMap<Address, RevealChain>,
    /// How many commitments each address has ever made, which numbers the next one.
    epochs: HashMap<Address, u64>,
    /// The randomness that picks the next validator, mixed with every reveal.
    beacon: Beacon,
}

/// A validator's reveal chain as far as it's been shown: the last link, and how many are left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RevealChain {
    head: Reveal,
    remaining: u32,
}

/// Where an address stands with its reveal chain, so a wallet can compute what to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatorState {
    /// The epoch of the commitment it would make next. The active one, if any, is one less.
    pub next_epoch: u64,
    /// Reveals left on the active commitment. `None` if there isn't one.
    pub reveals_left: Option<u32>,
}

impl Validators {
    /// No validators yet, and a beacon that starts at `beacon`.
    pub fn starting_at(beacon: Beacon) -> Self {
        Self { beacon, ..Self::default() }
    }

    pub fn beacon(&self) -> &Beacon {
        &self.beacon
    }

    pub fn state(&self, address: &Address) -> ValidatorState {
        ValidatorState {
            next_epoch: self.epochs.get(address).copied().unwrap_or(0),
            reveals_left: self.active(address).map(|c| c.remaining),
        }
    }

    /// Whether `address` has a reveal chain with links left to show. Its first stake, or its
    /// first since the last one ran out or it unstaked everything, must commit to a new one.
    pub fn is_committed(&self, address: &Address) -> bool {
        self.active(address).is_some()
    }

    /// Of `stakes`, those who can be drawn: whose reveal chain isn't used up.
    pub fn eligible(&self, stakes: Vec<(Address, Coins)>) -> Vec<(Address, Coins)> {
        stakes.into_iter().filter(|(address, _)| self.is_committed(address)).collect()
    }

    fn active(&self, address: &Address) -> Option<&RevealChain> {
        self.commitments.get(address).filter(|c| c.remaining > 0)
    }

    /// `address` commits to the reveal chain ending in `commitment`, its next epoch's.
    pub(in crate::domain) fn commit(&mut self, address: &Address, commitment: &Commitment) {
        let head = randao::first_link(commitment);
        self.commitments.insert(address.clone(), RevealChain { head, remaining: randao::LENGTH });
        *self.epochs.entry(address.clone()).or_insert(0) += 1;
    }

    /// `address` unstaked everything: its reveal chain ends, and staking again needs a new one.
    pub(in crate::domain) fn retire(&mut self, address: &Address) {
        self.commitments.remove(address);
    }

    /// Checks that `reveal` opens `validator`'s commitment, then moves their chain along by one
    /// and mixes the reveal into the beacon.
    pub(in crate::domain) fn apply_reveal(&mut self, validator: &Address, reveal: &Reveal) -> bool {
        let Some(chain) = self.commitments.get_mut(validator) else {
            return false;
        };
        if chain.remaining == 0 || !randao::opens(reveal, &chain.head) {
            return false;
        }
        chain.head = reveal.clone();
        chain.remaining -= 1;
        self.beacon = randao::mix(&self.beacon, reveal);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::crypto;

    fn validator() -> (ed25519_dalek::SigningKey, Address) {
        let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
        let address = Address::of(&key);
        (key, address)
    }

    #[test]
    fn a_commitment_is_opened_one_link_at_a_time() {
        let (key, alice) = validator();
        let mut validators = Validators::starting_at(Beacon::of(b"genesis"));
        assert!(!validators.is_committed(&alice));
        validators.commit(&alice, &randao::commitment(&key, 0));
        assert_eq!(
            validators.state(&alice),
            ValidatorState { next_epoch: 1, reveals_left: Some(randao::LENGTH) }
        );

        let before = validators.beacon().clone();
        let skipped = randao::reveal(&key, 0, randao::LENGTH - 1);
        assert!(!validators.apply_reveal(&alice, &skipped), "links can't be skipped");
        assert!(validators.apply_reveal(&alice, &randao::reveal(&key, 0, randao::LENGTH)));
        assert_ne!(*validators.beacon(), before, "the reveal is mixed in");
        assert_eq!(validators.state(&alice).reveals_left, Some(randao::LENGTH - 1));
    }

    #[test]
    fn only_the_committed_can_be_drawn_and_retiring_ends_a_chain() {
        let ((key, alice), (_, bob)) = (validator(), validator());
        let mut validators = Validators::default();
        validators.commit(&alice, &randao::commitment(&key, 0));
        let stakes = vec![(alice.clone(), Coins::new(5)), (bob, Coins::new(9))];
        assert_eq!(validators.eligible(stakes.clone()), [(alice.clone(), Coins::new(5))]);

        validators.retire(&alice);
        assert!(validators.eligible(stakes).is_empty());
        assert_eq!(
            validators.state(&alice),
            ValidatorState { next_epoch: 1, reveals_left: None },
            "the next commitment is a new epoch"
        );
    }
}
