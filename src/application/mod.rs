//! Use cases of a node. Each one loads the aggregate, changes it and saves it.

pub mod network;
mod node;

pub use network::{Network, PeerClient, PeerStatus, PeerTip, SyncReport, PEER_HEADER};
pub use node::{
    Account, ChainStatus, ConsensusState, NodeError, NodeService, TipStatus, TransactionProof,
};
