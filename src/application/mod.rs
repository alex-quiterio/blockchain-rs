//! Use cases of a node. Each one loads the aggregate, changes it and saves it.

pub mod network;
mod node;

pub use network::{Network, PeerClient, PeerStatus, PeerTip, SyncReport, PEER_HEADER, SYNC_PAGE};
pub use node::{
    Account, ChainStatus, ConsensusState, Located, NodeError, NodeService, Placement, TipStatus,
    TransactionProof,
};
