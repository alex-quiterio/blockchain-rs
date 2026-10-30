//! Adapters: for the domain's ports (storage, peers), for listening to the node (the log), and
//! for reading a node's settings from the environment (`Config`).

mod config;
mod event;
mod http;
mod json;
mod memory;

pub use config::Config;
pub use event::log_event;
pub use http::HttpPeerClient;
pub use json::ChainJsonFileRepository;
pub use memory::ChainInMemoryRepository;
