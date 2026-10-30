use std::fmt;

use serde::{Deserialize, Serialize};

/// An amount of coins: a balance, a transfer, a fee, a stake or a block reward. Sums are
/// checked, so an amount never wraps around: one too large to hold is refused, not mistaken for
/// a small one. On the wire and on disk it's the plain number, so signatures and hashes over it
/// don't change.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Coins(u64);

impl Coins {
    pub const ZERO: Self = Self(0);

    pub const fn new(amount: u64) -> Self {
        Self(amount)
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    pub const fn is_zero(&self) -> bool {
        self.0 == 0
    }

    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }

    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }

    /// For adding to a balance or stake: never more than all the coins there are, so it can't
    /// overflow, but it holds at the most rather than wrapping if it ever did.
    pub fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// What `amounts` add up to, or `None` if that's too large to hold.
    pub fn total(amounts: impl IntoIterator<Item = Self>) -> Option<Self> {
        amounts.into_iter().try_fold(Self::ZERO, Self::checked_add)
    }
}

impl fmt::Display for Coins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_never_wrap() {
        let most = Coins::new(u64::MAX);
        assert_eq!(Coins::new(2).checked_add(Coins::new(3)), Some(Coins::new(5)));
        assert_eq!(most.checked_add(Coins::new(1)), None);
        assert_eq!(Coins::new(1).checked_sub(Coins::new(2)), None);
        assert_eq!(most.saturating_add(Coins::new(1)), most);
        assert_eq!(Coins::total([Coins::new(1), Coins::new(2)]), Some(Coins::new(3)));
        assert_eq!(Coins::total([most, Coins::new(1)]), None);
        assert_eq!(Coins::total([]), Some(Coins::ZERO));
    }

    #[test]
    fn it_travels_and_prints_as_the_plain_number() {
        assert_eq!(serde_json::to_string(&Coins::new(42)).unwrap(), "42");
        assert_eq!(serde_json::from_str::<Coins>("42").unwrap(), Coins::new(42));
        assert_eq!(Coins::new(42).to_string(), "42");
    }
}
