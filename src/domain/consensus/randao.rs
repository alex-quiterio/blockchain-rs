//! Commit-reveal randomness for picking validators, after Ethereum's RANDAO.
//!
//! A validator commits to the end of a hash chain when they stake: `H(H(…H(seed)))`, `LENGTH`
//! times. Each block they forge reveals the previous link, which anyone can check by hashing it
//! once. The reveals are fixed the moment the commitment is made, so a forger has nothing to
//! choose and nothing to grind. Each reveal is mixed into a running beacon, and the beacon, not
//! the block hash, picks the next validator.

use ed25519_dalek::SigningKey;

use crate::domain::crypto;
use crate::domain::values::{Beacon, Commitment, Reveal};

/// Reveals one commitment covers. Staking again after they run out makes a new one.
pub const LENGTH: u32 = 1_000;

/// A validator's secret starting point for commitment number `epoch`. Derived from their key, so
/// they can recompute any reveal without storing anything, and nobody else can.
pub fn seed(key: &SigningKey, epoch: u64) -> String {
    crypto::sha256_hex(
        format!("blockchain-rs/randao|{}|{epoch}", hex::encode(key.to_bytes())).as_bytes(),
    )
}

/// What a validator publishes when staking: the seed hashed `LENGTH` times.
pub fn commitment(key: &SigningKey, epoch: u64) -> Commitment {
    Commitment::parse(&hash_times(seed(key, epoch), LENGTH)).expect("SHA-256 is hex")
}

/// The value to reveal when `remaining` reveals are left: one hash short of the last one shown.
pub fn reveal(key: &SigningKey, epoch: u64, remaining: u32) -> Reveal {
    Reveal::parse(&hash_times(seed(key, epoch), remaining.saturating_sub(1)))
        .expect("SHA-256 is hex")
}

/// Whether `reveal` is the link before `head`, i.e. hashes to it.
pub fn opens(reveal: &Reveal, head: &Reveal) -> bool {
    crypto::sha256_hex(reveal.as_str().as_bytes()) == head.as_str()
}

/// The first link a validator's chain shows: their commitment, its far end.
pub fn first_link(commitment: &Commitment) -> Reveal {
    Reveal::parse(commitment.as_str()).expect("both are SHA-256 hex")
}

/// The beacon after mixing in a reveal.
pub fn mix(beacon: &Beacon, reveal: &Reveal) -> Beacon {
    Beacon::of(format!("{beacon}|{reveal}").as_bytes())
}

fn hash_times(mut value: String, times: u32) -> String {
    for _ in 0..times {
        value = crypto::sha256_hex(value.as_bytes());
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    #[test]
    fn each_reveal_opens_the_one_before() {
        let mut head = first_link(&commitment(&key(), 0));
        for remaining in (1..=LENGTH).rev().take(5) {
            let next = reveal(&key(), 0, remaining);
            assert!(opens(&next, &head));
            assert!(!opens(&head, &next), "the chain only runs one way");
            head = next;
        }
    }

    #[test]
    fn commitments_differ_by_key_and_epoch() {
        let other = SigningKey::from_bytes(&[8; 32]);
        assert_ne!(commitment(&key(), 0), commitment(&key(), 1));
        assert_ne!(commitment(&key(), 0), commitment(&other, 0));
        assert!(!opens(&reveal(&other, 0, LENGTH), &first_link(&commitment(&key(), 0))));
    }

    #[test]
    fn mixing_depends_on_both_inputs() {
        let (a, b) = (Beacon::of(b"a"), Beacon::of(b"b"));
        let (x, y) = (Reveal::of(b"x"), Reveal::of(b"y"));
        assert_ne!(mix(&a, &x), mix(&a, &y));
        assert_ne!(mix(&a, &x), mix(&b, &x));
    }
}
