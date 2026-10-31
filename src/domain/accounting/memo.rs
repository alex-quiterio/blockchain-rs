//! What a transaction may say besides moving coins: never words, only the hash of words kept
//! somewhere else.

use crate::domain::crypto::sha256_type;

sha256_type!(
    /// The SHA-256 of a note kept off the chain. Once its transaction is in a block, the note is
    /// proven to have existed by the block's time and to be unchanged since, while the chain,
    /// and every peer it gossips to, learns nothing of what it says.
    Memo
);
