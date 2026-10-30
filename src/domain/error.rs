//! Why the domain refused something, by what was refused: a value, a transaction, a block or a
//! chain. Each operation fails only in the ways of what it handles, so its signature says what
//! can go wrong; `DomainError` gathers them where several meet, such as at the HTTP boundary.
//!
//! Each kind lives with the concept it refuses for: `ValueError` in `values`, `TransactionError`
//! in `accounting`, `BlockError` with the block and `ChainError` with the chain. They're all
//! re-exported here, so whoever handles refusals finds them in one place.

use std::fmt;

pub use crate::domain::accounting::TransactionError;
pub use crate::domain::chain::block::BlockError;
pub use crate::domain::chain::ChainError;
pub use crate::domain::values::ValueError;

/// Any of them, for where several kinds of refusal meet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    Value(ValueError),
    Transaction(TransactionError),
    Block(BlockError),
    Chain(ChainError),
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(e) => e.fmt(f),
            Self::Transaction(e) => e.fmt(f),
            Self::Block(e) => e.fmt(f),
            Self::Chain(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for DomainError {}

impl From<ValueError> for DomainError {
    fn from(e: ValueError) -> Self {
        Self::Value(e)
    }
}

impl From<TransactionError> for DomainError {
    fn from(e: TransactionError) -> Self {
        Self::Transaction(e)
    }
}

impl From<BlockError> for DomainError {
    fn from(e: BlockError) -> Self {
        Self::Block(e)
    }
}

impl From<ChainError> for DomainError {
    fn from(e: ChainError) -> Self {
        Self::Chain(e)
    }
}
