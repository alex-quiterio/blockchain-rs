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
