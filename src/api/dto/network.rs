//! Bodies for peers, syncing and the node's build.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PeerRequest {
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PeerResponse {
    pub url: String,
    /// The peer's chain length when last reached.
    pub height: Option<usize>,
    pub last_seen: Option<DateTime<Utc>>,
    /// Why the last attempt to reach it failed, if it did.
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AddPeerResponse {
    pub url: String,
    pub added: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SyncResponse {
    pub adopted_from: Option<String>,
    pub reached: usize,
    pub length: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NodeResponse {
    pub version: String,
    /// The git commit the node was built from. `-dirty` means uncommitted changes.
    pub commit: String,
    pub started_at: DateTime<Utc>,
}
