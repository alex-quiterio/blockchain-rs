//! Use cases of a node. `NodeService` changes the aggregate and saves it; `ChainQueries` answers
//! questions about it; `Network` keeps it in step with peers.

pub mod network;
mod node;
mod queries;

pub use network::{Network, PeerClient, PeerStatus, PeerTip, SyncReport, SYNC_PAGE};
pub use node::{NodeError, NodeEvent, NodeService, SNAPSHOT_BLOCKS};
pub use queries::{
    Account, ChainQueries, ChainStatus, ConsensusState, Located, Placement, TipStatus,
    TransactionProof,
};
