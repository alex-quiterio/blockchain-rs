//! Adapters: for the domain's ports (storage, peers), for listening to the node (the log), and
//! for reading a node's settings from the environment (`Config`).

mod config;
mod event_log;
mod http_peer;
mod json;
mod memory;

pub use config::Config;
pub use event_log::log_event;
pub use http_peer::HttpPeerClient;
pub use json::ChainJsonFileRepository;
pub use memory::ChainInMemoryRepository;
