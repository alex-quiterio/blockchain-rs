use std::fmt;

use serde::{Deserialize, Serialize};

use crate::domain::crypto;
use crate::domain::error::ValueError;

/// Declares a SHA-256 hash type: 64 lowercase hex characters, checked on the way in. Each kind
/// of hash gets its own type, so one can't be passed where another is meant. On the wire and on
/// disk it's the plain hex string, so hashes computed over it don't change.
macro_rules! sha256_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(hex: &str) -> Result<Self, ValueError> {
                if hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                    Ok(Self(hex.to_string()))
                } else {
                    Err(ValueError::InvalidHash(hex.to_string()))
                }
            }

            /// The SHA-256 of `data`.
            pub fn of(data: &[u8]) -> Self {
                Self(crypto::sha256_hex(data))
            }

            /// All zeros: what nothing hashes to.
            pub fn zero() -> Self {
                Self("0".repeat(64))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        /// All zeros, until there's something to hash.
        impl Default for $name {
            fn default() -> Self {
                Self::zero()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = ValueError;

            fn try_from(hex: String) -> Result<Self, ValueError> {
                Self::parse(&hex)
            }
        }

        impl From<$name> for String {
            fn from(hash: $name) -> String {
                hash.0
            }
        }
    };
}

sha256_type!(
    /// A block's hash: the SHA-256 of its header. Blocks name the one before them by it.
    BlockHash
);

sha256_type!(
    /// The root of the Merkle tree over a block's transactions, which its header carries.
    MerkleRoot
);

sha256_type!(
    /// A hash inside a Merkle tree: a transaction's leaf, or two nodes below it hashed together.
    MerkleNode
);

sha256_type!(
    /// A link of a validator's reveal chain, shown in a block it forges. Hashing it once gives the
    /// link shown before it, so anyone can check it, and nobody can show the next one early.
    Reveal
);

sha256_type!(
    /// The end of a validator's reveal chain, published in their first stake.
    Commitment
);

sha256_type!(
    /// The randomness that picks who forges next, which only validators' reveals move.
    Beacon
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_64_lowercase_hex_characters_are_a_hash() {
        let hash = BlockHash::of(b"block");
        assert_eq!(BlockHash::parse(hash.as_str()), Ok(hash.clone()));
        assert_eq!(BlockHash::zero().as_str(), "0".repeat(64));
        let upper = hash.as_str().to_uppercase();
        for bad in ["", "nope", &hash.as_str()[..63], &format!("{hash}0"), &upper] {
            assert_eq!(BlockHash::parse(bad), Err(ValueError::InvalidHash(bad.to_string())));
        }
    }

    #[test]
    fn it_travels_as_the_plain_hex_string() {
        let hash = MerkleRoot::of(b"transactions");
        let json = serde_json::to_string(&hash).unwrap();
        assert_eq!(json, format!("\"{hash}\""));
        assert_eq!(serde_json::from_str::<MerkleRoot>(&json).unwrap(), hash);
        assert!(serde_json::from_str::<MerkleRoot>("\"not a hash\"").is_err());
    }
}
