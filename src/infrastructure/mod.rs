//! Adapters for the domain's ports.

mod json_file;
mod memory;

pub use json_file::JsonFileRepository;
pub use memory::InMemoryRepository;
