//! Adapters for the domain's ports.

mod http_peer;
mod json_file;
mod memory;

pub use http_peer::HttpPeerClient;
pub use json_file::JsonFileRepository;
pub use memory::InMemoryRepository;
