//! The adapters of the chain aggregate's repository, `ChainRepository`: one that keeps the chain
//! in a JSON file between runs, and one that keeps it in memory, for tests.

mod json;
mod memory;

pub use json::ChainJsonRepository;
pub use memory::ChainInMemoryRepository;
