//! Use cases of a node. Each one loads the aggregate, changes it and saves it.

pub mod network;
mod node;

pub use network::{Network, PeerClient, PeerStatus, SyncReport};
pub use node::{Account, ConsensusState, NodeError, NodeService};
