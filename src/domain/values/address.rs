use std::fmt;

use ed25519_dalek::SigningKey;

use crate::domain::crypto;
use crate::domain::error::ChainError;

/// An account: a hex-encoded ed25519 public key. Holding one means it has already been checked.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Address(String);

impl Address {
    pub fn parse(address: &str) -> Result<Self, ChainError> {
        if crypto::is_valid_address(address) {
            Ok(Self(address.to_string()))
        } else {
            Err(ChainError::InvalidAddress(address.to_string()))
        }
    }

    /// The address that `key` signs for.
    pub fn of(key: &SigningKey) -> Self {
        Self(crypto::address_of(key))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_public_keys() {
        let (hex, secret) = crypto::generate_keypair();
        let key = crypto::parse_secret_key(&secret).unwrap();
        assert_eq!(Address::parse(&hex), Ok(Address::of(&key)));
        for bad in ["", "nobody", "coinbase", &hex[..62], &format!("{hex}00")] {
            assert_eq!(Address::parse(bad), Err(ChainError::InvalidAddress(bad.to_string())));
        }
    }
}
