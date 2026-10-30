//! Declaring a kind of SHA-256 hash. Each concept declares its own with `sha256_type!`, so one
//! kind can never be passed where another is meant: a block's hash for a Merkle root, say.

/// Declares a SHA-256 hash type: 64 lowercase hex characters, checked on the way in. Each kind
/// of hash gets its own type, so one can't be passed where another is meant. On the wire and on
/// disk it's the plain hex string, so hashes computed over it don't change.
macro_rules! sha256_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, ::serde::Serialize, ::serde::Deserialize,
        )]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(hex: &str) -> Result<Self, $crate::domain::crypto::ValueError> {
                if hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                    Ok(Self(hex.to_string()))
                } else {
                    Err($crate::domain::crypto::ValueError::InvalidHash(hex.to_string()))
                }
            }

            /// The SHA-256 of `data`.
            pub fn of(data: &[u8]) -> Self {
                Self($crate::domain::crypto::sha256_hex(data))
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

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = $crate::domain::crypto::ValueError;

            fn try_from(hex: String) -> Result<Self, $crate::domain::crypto::ValueError> {
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

pub(crate) use sha256_type;

#[cfg(test)]
mod tests {
    sha256_type!(
        /// A kind of hash for these tests alone.
        TestHash
    );

    #[test]
    fn only_64_lowercase_hex_characters_are_a_hash() {
        use crate::domain::crypto::ValueError;
        let hash = TestHash::of(b"block");
        assert_eq!(TestHash::parse(hash.as_str()), Ok(hash.clone()));
        assert_eq!(TestHash::zero().as_str(), "0".repeat(64));
        let upper = hash.as_str().to_uppercase();
        for bad in ["", "nope", &hash.as_str()[..63], &format!("{hash}0"), &upper] {
            assert_eq!(TestHash::parse(bad), Err(ValueError::InvalidHash(bad.to_string())));
        }
    }

    #[test]
    fn it_travels_as_the_plain_hex_string() {
        let hash = TestHash::of(b"transactions");
        let json = serde_json::to_string(&hash).unwrap();
        assert_eq!(json, format!("\"{hash}\""));
        assert_eq!(serde_json::from_str::<TestHash>(&json).unwrap(), hash);
        assert!(serde_json::from_str::<TestHash>("\"not a hash\"").is_err());
    }
}
