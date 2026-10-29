use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use ed25519_dalek::SigningKey;

use super::address::Address;
use crate::domain::crypto;
use crate::domain::error::ValueError;

/// An ed25519 signature: 128 lowercase hex characters. On the wire it's the hex string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Signature(String);

impl Signature {
    pub fn parse(hex: &str) -> Result<Self, ValueError> {
        if hex.len() == 128 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(Self(hex.to_string()))
        } else {
            Err(ValueError::InvalidSignature(hex.to_string()))
        }
    }

    /// `key`'s signature over `payload`.
    pub fn sign(key: &SigningKey, payload: &[u8]) -> Self {
        Self(crypto::sign(key, payload))
    }

    /// Whether this is `address`'s signature over `payload`.
    pub fn is_by(&self, address: &Address, payload: &[u8]) -> bool {
        crypto::verify(address.as_str(), payload, &self.0)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Signature {
    type Error = ValueError;

    fn try_from(hex: String) -> Result<Self, ValueError> {
        Self::parse(&hex)
    }
}

impl From<Signature> for String {
    fn from(signature: Signature) -> String {
        signature.0
    }
}

/// For a field that may be unsigned: `None` travels as `""`, as a block reward's always has.
pub mod or_empty {
    use super::*;

    pub fn serialize<S: Serializer>(
        signature: &Option<Signature>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        s.serialize_str(signature.as_ref().map_or("", Signature::as_str))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Signature>, D::Error> {
        let hex = String::deserialize(d)?;
        if hex.is_empty() {
            return Ok(None);
        }
        Signature::parse(&hex).map(Some).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Signed {
        #[serde(with = "or_empty")]
        signature: Option<Signature>,
    }

    #[test]
    fn unsigned_travels_as_an_empty_string_and_anything_else_must_be_a_signature() {
        let unsigned = Signed { signature: None };
        assert_eq!(serde_json::to_string(&unsigned).unwrap(), r#"{"signature":""}"#);
        assert_eq!(serde_json::from_str::<Signed>(r#"{"signature":""}"#).unwrap(), unsigned);
        let hex = "ab".repeat(64);
        let signed: Signed = serde_json::from_str(&format!(r#"{{"signature":"{hex}"}}"#)).unwrap();
        assert_eq!(signed.signature.unwrap().as_str(), hex);
        assert!(serde_json::from_str::<Signed>(r#"{"signature":"nope"}"#).is_err());
        let short = &hex[..126];
        assert_eq!(Signature::parse(short), Err(ValueError::InvalidSignature(short.to_string())));
    }
}
