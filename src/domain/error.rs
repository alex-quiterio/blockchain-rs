use std::fmt;

/// Why the domain refused something. Every rule that can fail says so here.
#[derive(Debug, PartialEq, Eq)]
pub enum ChainError {
    InvalidAddress(String),
    ZeroAmount,
    BadSignature,
    BadNonce {
        expected: u64,
        got: u64,
    },
    InsufficientFunds {
        balance: u64,
        amount: u64,
    },
    InsufficientStake {
        staked: u64,
        amount: u64,
    },
    UnexpectedCoinbase,
    InvalidChain,
    /// Mining was asked of a proof-of-stake chain.
    NotProofOfWork,
    /// Forging was asked of a proof-of-work chain.
    NotProofOfStake,
    /// Someone other than the selected validator tried to forge.
    NotYourTurn {
        expected: String,
    },
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAddress(a) => write!(f, "invalid address: {a}"),
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
            Self::InvalidChain => write!(f, "chain does not validate"),
            Self::NotProofOfWork => write!(f, "this chain uses proof of stake: forge, don't mine"),
            Self::NotProofOfStake => write!(f, "this chain uses proof of work: mine, don't forge"),
            Self::NotYourTurn { expected } => write!(f, "the next block is {expected}'s to forge"),
        }
    }
}

impl std::error::Error for ChainError {}
