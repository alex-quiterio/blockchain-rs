//! A value that isn't one.

use std::fmt;

/// A value that isn't one: the wrong shape for an address, a hash or a signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueError {
    InvalidAddress(String),
    /// Not 64 lowercase hex characters.
    InvalidHash(String),
    /// Not 128 lowercase hex characters.
    InvalidSignature(String),
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAddress(a) => write!(f, "invalid address: {a}"),
            Self::InvalidHash(h) => write!(f, "not a hash: {h}"),
            Self::InvalidSignature(s) => write!(f, "not a signature: {s}"),
        }
    }
}

impl std::error::Error for ValueError {}
