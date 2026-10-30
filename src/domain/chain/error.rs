//! Blocks that can't become our chain.

use std::fmt;

use super::block::BlockError;

/// Blocks that can't become our chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainError {
    /// There were no blocks.
    Empty,
    /// The block at `height` breaks a rule, so neither it nor anything after it can be ours.
    InvalidBlock { height: u64, reason: BlockError },
    /// A heavier fork would throw away more of our blocks than the rules allow.
    ReorgTooDeep { depth: usize, max: u64 },
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "no blocks"),
            Self::InvalidBlock { height, reason } => write!(f, "block {height}: {reason}"),
            Self::ReorgTooDeep { depth, max } => {
                write!(f, "switching forks would undo {depth} blocks; at most {max} may be undone")
            }
        }
    }
}

impl std::error::Error for ChainError {}
