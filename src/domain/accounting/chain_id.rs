//! Which chain a transfer is for, so one signed for another network can't be replayed on this.

use chrono::{DateTime, TimeZone, Utc};

use crate::domain::crypto::sha256_type;

sha256_type!(
    /// The chain a transfer is signed for: the hash of its genesis block, which every network
    /// with different rules has its own of. A transfer names it, and the signature covers it,
    /// so a transfer from another network fails here even when its nonce is free.
    ChainId
);

/// Transfers in blocks from this time on must name their chain. Earlier blocks were made before
/// transfers did, and are still taken as they are.
pub fn chain_bound_since() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).single().expect("a valid date")
}
