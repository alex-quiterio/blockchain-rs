//! Use cases of a node. `NodeService` changes the aggregate and saves it; `ChainQueries` answers
//! questions about it; `Network` keeps it in step with peers.

pub mod network;
mod node;
mod queries;

pub use network::{Network, PeerClient, PeerStatus, PeerTip, SyncReport, SYNC_PAGE};
pub use node::{ApplicationError, NodeError, NodeEvent, NodeService, Opening, SNAPSHOT_BLOCKS};
pub use queries::{
    Account, BlockFees, ChainQueries, ChainStatus, ConsensusState, FeeReport, Located, Placement,
    TipStatus, TransactionProof, RECENT_BLOCKS,
};

/// Runs `work` on a thread set aside for blocking work, and waits for it there. Changing the
/// chain replays blocks, checks signatures and writes to disk, and checking it replays it all; on
/// the async runtime, that would stall every request sharing the thread. A panic in `work` carries
/// on in the caller, as if it had run there.
pub async fn off_runtime<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}
