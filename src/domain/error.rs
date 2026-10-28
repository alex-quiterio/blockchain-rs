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
    /// A first stake must commit to a reveal chain.
    MissingCommitment,
    /// Only a first stake carries a commitment.
    UnexpectedCommitment,
    /// A forger's reveal doesn't open their commitment, or is missing or not due.
    BadReveal,
    /// The exact transaction is already waiting in the mempool, usually because of gossip.
    AlreadyPending,
    /// A submitted block doesn't build on the current tip, usually because another block won.
    StaleBlock,
    /// A submitted block fails a check: its hash, signature, payout or transactions.
    InvalidBlock,
    /// A heavier fork would throw away more of our blocks than the rules allow.
    ReorgTooDeep {
        depth: usize,
        max: u64,
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
            Self::MissingCommitment => {
                write!(f, "a first stake needs a commitment: the end of a 64-hex reveal chain")
            }
            Self::UnexpectedCommitment => {
                write!(f, "only a first stake carries a commitment; this one is still in use")
            }
            Self::BadReveal => {
                write!(f, "the reveal doesn't open the validator's commitment, or isn't due")
            }
            Self::AlreadyPending => write!(f, "transaction is already waiting to be mined"),
            Self::StaleBlock => write!(f, "another block took the tip first"),
            Self::InvalidBlock => write!(f, "block does not validate"),
            Self::ReorgTooDeep { depth, max } => {
                write!(f, "switching forks would undo {depth} blocks; at most {max} may be undone")
            }
        }
    }
}

impl std::error::Error for ChainError {}
