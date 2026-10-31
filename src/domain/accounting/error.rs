//! A transaction the ledger or the mempool won't take.

use std::fmt;

use crate::domain::accounting::Coins;

/// A transaction the ledger or the mempool won't take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionError {
    ZeroAmount,
    BadSignature,
    BadNonce {
        expected: u64,
        got: u64,
    },
    InsufficientFunds {
        balance: Coins,
        amount: Coins,
    },
    InsufficientStake {
        staked: Coins,
        amount: Coins,
    },
    /// Block rewards are made by mining or forging, never submitted.
    UnexpectedCoinbase,
    /// A first stake must commit to a reveal chain.
    MissingCommitment,
    /// Only a first stake carries a commitment.
    UnexpectedCommitment,
    /// It names another chain than this one.
    WrongChain,
    /// It names no chain, which only transfers in blocks from before `chain_bound_since` may.
    NoChain,
    /// The exact transaction is already waiting in the mempool, usually because of gossip.
    AlreadyPending,
    /// The mempool holds all it may. `needed` is the fee that would push out the cheapest
    /// transfer waiting, or `None` when only the sender's own are there to push out.
    MempoolFull {
        needed: Option<Coins>,
    },
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroAmount => write!(f, "amount must be greater than zero"),
            Self::BadSignature => write!(f, "signature does not match sender"),
            Self::BadNonce { expected, got } => write!(f, "nonce must be {expected}, got {got}"),
            Self::InsufficientFunds { balance, amount } => {
                write!(f, "balance {balance} is less than {amount}")
            }
            Self::InsufficientStake { staked, amount } => {
                write!(f, "stake {staked} is less than {amount}")
            }
            Self::UnexpectedCoinbase => write!(f, "coinbase transactions are only made by mining"),
            Self::MissingCommitment => {
                write!(f, "a first stake needs a commitment: the end of a 64-hex reveal chain")
            }
            Self::UnexpectedCommitment => {
                write!(f, "only a first stake carries a commitment; this one is still in use")
            }
            Self::WrongChain => write!(f, "transaction is signed for another chain"),
            Self::NoChain => {
                write!(f, "transaction must name its chain: the genesis hash, under `chain`")
            }
            Self::AlreadyPending => write!(f, "transaction is already waiting to be mined"),
            Self::MempoolFull { needed: Some(fee) } => {
                write!(f, "the mempool is full: a fee of at least {fee} gets a place")
            }
            Self::MempoolFull { needed: None } => {
                write!(f, "the mempool is full of this sender's own transfers; wait for a block")
            }
        }
    }
}

impl std::error::Error for TransactionError {}
