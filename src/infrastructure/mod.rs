//! Adapters: for the domain's ports (storage, peers), for listening to the node (the log), and
//! for reading a node's settings from the environment (`Config`).

mod config;
mod event;
mod http;
mod repository;

pub use config::Config;
pub use event::log_event;
pub use http::HttpPeerClient;
pub use repository::{ChainInMemoryRepository, ChainJsonFileRepository};
