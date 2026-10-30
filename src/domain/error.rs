//! Why the domain refused something, by what was refused: a value, a transaction, a block or a
//! chain. Each operation fails only in the ways of what it handles, so its signature says what
//! can go wrong; `DomainError` gathers them where several meet, such as at the HTTP boundary.

use std::fmt;

use super::address::Address;
use super::values::Coins;

/// A value that isn't one: the wrong shape for an address, a hash or a signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueError {
    InvalidAddress(String),
    /// Not 64 lowercase hex characters.
    InvalidHash(String),
    /// Not 128 lowercase hex characters.
    InvalidSignature(String),
}

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
    /// The exact transaction is already waiting in the mempool, usually because of gossip.
    AlreadyPending,
}

/// A block that can't be made or added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockError {
    /// Mining was asked of a proof-of-stake chain.
    NotProofOfWork,
    /// Forging was asked of a proof-of-work chain.
    NotProofOfStake,
    /// Someone other than the selected validator tried to forge.
    NotYourTurn { expected: Address },
    /// A forger's reveal doesn't open their commitment, or is missing or not due.
    BadReveal,
    /// It doesn't build on the current tip, usually because another block won.
    StaleBlock,
    /// It holds more transfers than the rules let a block hold.
    TooManyTransfers { max: u64, got: usize },
    /// It fails a check: its hash, signature, payout or transactions.
    InvalidBlock,
}

/// Blocks that can't become our chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainError {
    /// They don't validate, from genesis or from where they join ours.
    InvalidChain,
    /// A heavier fork would throw away more of our blocks than the rules allow.
    ReorgTooDeep { depth: usize, max: u64 },
}

/// Any of them, for where several kinds of refusal meet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    Value(ValueError),
    Transaction(TransactionError),
    Block(BlockError),
    Chain(ChainError),
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
            Self::AlreadyPending => write!(f, "transaction is already waiting to be mined"),
        }
    }
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
            Self::InvalidBlock => write!(f, "block does not validate"),
        }
    }
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidChain => write!(f, "chain does not validate"),
            Self::ReorgTooDeep { depth, max } => {
                write!(f, "switching forks would undo {depth} blocks; at most {max} may be undone")
            }
        }
    }
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

impl std::error::Error for ValueError {}
impl std::error::Error for TransactionError {}
impl std::error::Error for BlockError {}
impl std::error::Error for ChainError {}
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
