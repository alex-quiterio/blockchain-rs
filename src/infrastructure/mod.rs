//! Adapters: for the domain's ports (storage, peers), for listening to the node (the log), and
//! for reading a node's settings from the environment (`Config`).

mod config;
mod http;
mod logger;
pub mod repository;

pub use config::Config;
pub use http::HttpPeerClient;
pub use logger::log_event;
