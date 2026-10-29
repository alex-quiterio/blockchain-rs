use std::fmt;

use serde::{Deserialize, Serialize};

use super::address::Address;
use crate::domain::error::ValueError;

/// The sender a block reward names, since its coins come from nowhere.
pub const COINBASE: &str = "coinbase";
/// The recipient that locks coins up as stake, for proof of stake.
pub const STAKE: &str = "stake";
/// The recipient that unlocks staked coins back into the sender's balance.
pub const UNSTAKE: &str = "unstake";

/// Where a transaction's coins come from: an account, or nowhere, for the new coins a block
/// pays its producer. On the wire it's the address, or `"coinbase"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Source {
    NewCoins,
    Account(Address),
}

/// Where a transaction's coins go: an account, or into or out of its sender's stake. On the wire
/// it's the address, `"stake"` or `"unstake"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Destination {
    Account(Address),
    Stake,
    Unstake,
}

impl Source {
    pub fn parse(from: &str) -> Result<Self, ValueError> {
        match from {
            COINBASE => Ok(Self::NewCoins),
            _ => Address::parse(from).map(Self::Account),
        }
    }

    /// The account, unless the coins are new.
    pub fn account(&self) -> Option<&Address> {
        match self {
            Self::Account(address) => Some(address),
            Self::NewCoins => None,
        }
    }
}

impl Destination {
    pub fn parse(to: &str) -> Result<Self, ValueError> {
        match to {
            STAKE => Ok(Self::Stake),
            UNSTAKE => Ok(Self::Unstake),
            _ => Address::parse(to).map(Self::Account),
        }
    }

    /// The account, unless the coins go into or out of a stake.
    pub fn account(&self) -> Option<&Address> {
        match self {
            Self::Account(address) => Some(address),
            Self::Stake | Self::Unstake => None,
        }
    }
}

impl From<Address> for Destination {
    fn from(address: Address) -> Self {
        Self::Account(address)
    }
}

impl From<&Address> for Destination {
    fn from(address: &Address) -> Self {
        Self::Account(address.clone())
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NewCoins => f.write_str(COINBASE),
            Self::Account(address) => address.fmt(f),
        }
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Account(address) => address.fmt(f),
            Self::Stake => f.write_str(STAKE),
            Self::Unstake => f.write_str(UNSTAKE),
        }
    }
}

impl TryFrom<String> for Source {
    type Error = ValueError;

    fn try_from(from: String) -> Result<Self, ValueError> {
        Self::parse(&from)
    }
}

impl TryFrom<String> for Destination {
    type Error = ValueError;

    fn try_from(to: String) -> Result<Self, ValueError> {
        Self::parse(&to)
    }
}

impl From<Source> for String {
    fn from(source: Source) -> String {
        source.to_string()
    }
}

impl From<Destination> for String {
    fn from(destination: Destination) -> String {
        destination.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::crypto;

    #[test]
    fn they_travel_as_the_same_strings_as_before() {
        let address = Address::parse(&crypto::generate_keypair().0).unwrap();
        let cases = [
            (serde_json::to_string(&Source::NewCoins), "\"coinbase\"".to_string()),
            (serde_json::to_string(&Destination::Stake), "\"stake\"".to_string()),
            (serde_json::to_string(&Destination::Unstake), "\"unstake\"".to_string()),
            (serde_json::to_string(&Source::Account(address.clone())), format!("\"{address}\"")),
        ];
        for (json, expected) in cases {
            assert_eq!(json.unwrap(), expected);
        }
        let back: Destination = serde_json::from_str(&format!("\"{address}\"")).unwrap();
        assert_eq!(back, Destination::Account(address));
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in ["", "nobody", "Coinbase", "STAKE"] {
            assert!(Source::parse(bad).is_err());
            assert!(Destination::parse(bad).is_err());
        }
        assert!(Source::parse(STAKE).is_err(), "coins don't come from a stake");
        assert!(Destination::parse(COINBASE).is_err(), "nor go to the coinbase");
    }
}
