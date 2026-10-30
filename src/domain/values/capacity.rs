use std::fmt;
use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

/// How many transfers a block may hold besides its payout: at most some number, or any. A block
/// that may hold none would never confirm anything, so there's no such capacity: `0` means any,
/// as it does when a node is configured. On the wire and on disk it's the number, or `null` for
/// any.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockCapacity(Option<NonZeroU64>);

impl BlockCapacity {
    pub const ANY: Self = Self(None);

    /// At most `max` transfers a block, or any number for `0`.
    pub const fn at_most(max: u64) -> Self {
        Self(NonZeroU64::new(max))
    }

    /// The most a block may hold, or `None` for any number.
    pub fn max(self) -> Option<u64> {
        self.0.map(NonZeroU64::get)
    }

    /// Whether a block may hold `transfers` transfers besides its payout.
    pub fn holds(self, transfers: usize) -> bool {
        self.0.is_none_or(|max| u64::try_from(transfers).is_ok_and(|n| n <= max.get()))
    }
}

impl fmt::Display for BlockCapacity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(max) => write!(f, "at most {max} transfers a block"),
            None => write!(f, "any number of transfers a block"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_holds_up_to_its_max() {
        let two = BlockCapacity::at_most(2);
        assert!(two.holds(0) && two.holds(2));
        assert!(!two.holds(3));
        assert_eq!(two.max(), Some(2));
    }

    #[test]
    fn zero_means_any_number() {
        assert_eq!(BlockCapacity::at_most(0), BlockCapacity::ANY);
        assert!(BlockCapacity::ANY.holds(usize::MAX));
        assert_eq!(BlockCapacity::ANY.max(), None);
    }

    #[test]
    fn it_travels_as_the_plain_number_or_null() {
        let two = BlockCapacity::at_most(2);
        assert_eq!(serde_json::to_string(&two).unwrap(), "2");
        assert_eq!(serde_json::to_string(&BlockCapacity::ANY).unwrap(), "null");
        assert_eq!(serde_json::from_str::<BlockCapacity>("2").unwrap(), two);
        assert_eq!(serde_json::from_str::<BlockCapacity>("null").unwrap(), BlockCapacity::ANY);
        assert!(serde_json::from_str::<BlockCapacity>("0").is_err(), "a capacity of none");
    }
}
