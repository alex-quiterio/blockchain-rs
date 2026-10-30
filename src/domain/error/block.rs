//! A block that can't be made or added, by the rule it breaks.

use std::fmt;

use super::TransactionError;
use crate::domain::address::Address;
use crate::domain::values::Coins;

/// A block that can't be made or added, by the rule it breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockError {
    /// Mining was asked of a proof-of-stake chain.
    NotProofOfWork,
    /// Forging was asked of a proof-of-work chain.
    NotProofOfStake,
    /// Someone other than the selected validator tried to forge, or signed the block.
    NotYourTurn { expected: Address },
    /// A forger's reveal doesn't open their commitment, or is missing or not due.
    BadReveal,
    /// It doesn't build on the current tip, usually because another block won.
    StaleBlock,
    /// It holds more transfers than the rules let a block hold.
    TooManyTransfers { max: u64, got: usize },
    /// The first block isn't this chain's genesis: the one every node with these rules makes.
    BadGenesis,
    /// Its height or the hash it names as its parent don't follow the block before it.
    BrokenLink,
    /// Its Merkle root doesn't cover its transactions, so they were changed.
    MerkleRootMismatch,
    /// Its hash isn't the hash of its header, so the header was changed.
    HashMismatch,
    /// Not after the median of the blocks before it, so it may be backdated.
    TooEarly,
    /// More than two hours ahead of this node's clock.
    TooFarAhead,
    /// Proof of work: its hash doesn't meet the work required at its height.
    InsufficientWork,
    /// Sealed the way the other consensus seals blocks: a mined block that's signed, or a
    /// forged one with a nonce.
    WrongSeal,
    /// Proof of stake: it's unsigned, or the signature isn't its validator's.
    BadSignature,
    /// Its first transaction isn't a payout to whoever made it: missing, carrying a fee, or with
    /// a nonce other than the block's height.
    MalformedPayout,
    /// The payout isn't the reward plus the block's fees. `expected` is `None` when the fees are
    /// too large to add up, which no valid block's can be.
    WrongPayout { expected: Option<Coins>, got: Coins },
    /// A transfer doesn't apply after the ones before it. `position` counts the payout as 0.
    BadTransfer { position: usize, reason: TransactionError },
}

impl fmt::Display for BlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotProofOfWork => write!(f, "this chain uses proof of stake: forge, don't mine"),
            Self::NotProofOfStake => write!(f, "this chain uses proof of work: mine, don't forge"),
            Self::NotYourTurn { expected } => write!(f, "the next block is {expected}'s to forge"),
            Self::BadReveal => {
                write!(f, "the reveal doesn't open the validator's commitment, or isn't due")
            }
            Self::StaleBlock => write!(f, "another block took the tip first"),
            Self::TooManyTransfers { max, got } => {
                write!(f, "block holds {got} transfers; at most {max} fit in one")
            }
            Self::BadGenesis => write!(f, "not this chain's genesis block"),
            Self::BrokenLink => write!(f, "doesn't follow the block before it"),
            Self::MerkleRootMismatch => write!(f, "the Merkle root doesn't match the transactions"),
            Self::HashMismatch => write!(f, "the hash doesn't match the header"),
            Self::TooEarly => write!(f, "not after the median time of the blocks before it"),
            Self::TooFarAhead => write!(f, "more than two hours in the future"),
            Self::InsufficientWork => write!(f, "the hash doesn't meet the work required"),
            Self::WrongSeal => write!(f, "sealed the way the other consensus seals blocks"),
            Self::BadSignature => write!(f, "unsigned, or not signed by its validator"),
            Self::MalformedPayout => write!(f, "the first transaction isn't a valid payout"),
            Self::WrongPayout { expected: Some(expected), got } => {
                write!(f, "the payout is {got}; the reward plus fees is {expected}")
            }
            Self::WrongPayout { expected: None, got } => {
                write!(f, "the payout is {got}, but the fees are too large to add up")
            }
            Self::BadTransfer { position, reason } => write!(f, "transaction {position}: {reason}"),
        }
    }
}

impl std::error::Error for BlockError {}
