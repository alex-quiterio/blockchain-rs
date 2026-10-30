//! Why a change to the node didn't happen: a domain rule refused it, or it couldn't be saved.

use std::fmt;

use crate::domain::accounting::TransactionError;
use crate::domain::chain::block::BlockError;
use crate::domain::chain::repository::RepositoryError;
use crate::domain::chain::ChainError;
use crate::domain::crypto::ValueError;

#[derive(Debug)]
pub enum NodeError {
    /// A domain rule refused the change.
    Domain(ApplicationError),
    /// The change was valid but could not be saved, so it was not applied.
    Storage(RepositoryError),
}

impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain(e) => e.fmt(f),
            Self::Storage(e) => write!(f, "could not save the chain: {e}"),
        }
    }
}

impl std::error::Error for NodeError {}

impl<E: Into<ApplicationError>> From<E> for NodeError {
    fn from(err: E) -> Self {
        Self::Domain(err.into())
    }
}

/// A refusal from the domain, of whichever kind: what a use case returns when it may fail in
/// several ways, such as `commit`, which takes any change to the chain. The domain itself never
/// needs one; each of its operations fails only in the ways of what it handles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationError {
    Value(ValueError),
    Transaction(TransactionError),
    Block(BlockError),
    Chain(ChainError),
}

impl fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(e) => e.fmt(f),
            Self::Transaction(e) => e.fmt(f),
            Self::Block(e) => e.fmt(f),
            Self::Chain(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ApplicationError {}

impl From<ValueError> for ApplicationError {
    fn from(e: ValueError) -> Self {
        Self::Value(e)
    }
}

impl From<TransactionError> for ApplicationError {
    fn from(e: TransactionError) -> Self {
        Self::Transaction(e)
    }
}

impl From<BlockError> for ApplicationError {
    fn from(e: BlockError) -> Self {
        Self::Block(e)
    }
}

impl From<ChainError> for ApplicationError {
    fn from(e: ChainError) -> Self {
        Self::Chain(e)
    }
}
