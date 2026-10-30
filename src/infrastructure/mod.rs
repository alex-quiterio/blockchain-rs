//! Adapters: for the domain's ports (storage, peers), for listening to the node (the log), and
//! for reading a node's settings from the environment (`Config`).

mod config;
mod event_log;
mod http_peer;
mod json_file;
mod memory;

pub use config::Config;
pub use event_log::log_event;
pub use http_peer::HttpPeerClient;
pub use json_file::ChainJsonFileRepository;
pub use memory::ChainInMemoryRepository;
