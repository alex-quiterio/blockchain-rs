//! The node's use cases. Every change to the chain goes through `commit` (here): applied to a
//! copy, saved, then swapped in. Submitting transactions (`transactions`), adding and taking
//! blocks (`blocks`), snapshots (`snapshots`), what happened (`events`) and why a change failed
//! (`error`) each have a file of their own.

mod blocks;
mod error;
mod events;
mod snapshots;
mod transactions;

use std::sync::{Arc, Mutex};

use crate::domain::block::Transaction;
use crate::domain::chain::{Blockchain, ChainEvent, ReorgLimit};
use crate::domain::error::DomainError;
use crate::domain::mempool::Mempool;
use crate::domain::repository::ChainRepository;
use crate::domain::rules::ChainRules;

use super::queries::ChainQueries;

pub use error::NodeError;
use events::Listener;
pub use events::NodeEvent;

/// The node's use cases: every change to the chain goes through here, is saved, and is told to
/// listeners. Questions about the chain go to `ChainQueries`.
pub struct NodeService {
    chain: Arc<Mutex<Blockchain>>,
    /// What's waiting, an aggregate of its own, under its own lock: submitting a transaction
    /// doesn't wait for mining or a peer's blocks. Whenever both are locked, the chain is first.
    mempool: Arc<Mutex<Mempool>>,
    repository: Box<dyn ChainRepository>,
    /// Blocks between snapshots. 0 takes none.
    snapshot_every: usize,
    /// The height of the last snapshot saved or loaded.
    last_snapshot: Mutex<usize>,
    /// The snapshot the chain was restored from on opening, if one was used.
    restored_from: Option<usize>,
    /// Told what happened after every change.
    listeners: Mutex<Vec<Listener>>,
}

/// Blocks between snapshots, by default.
pub const SNAPSHOT_BLOCKS: usize = 100;

impl NodeService {
    /// Restores the saved chain, or starts and saves a new one if nothing is saved yet, under
    /// the network's `rules` and this node's own `reorg_limit`. A saved snapshot that fits the
    /// chain spares replaying the blocks before it.
    pub fn open(
        repository: Box<dyn ChainRepository>,
        rules: ChainRules,
        reorg_limit: ReorgLimit,
    ) -> Result<Self, NodeError> {
        let (chain, restored_from) = match repository.load().map_err(NodeError::Storage)? {
            Some(blocks) => {
                // A snapshot that can't be read is no worse than none: the chain is replayed.
                let snapshot = repository.load_snapshot().unwrap_or(None);
                Blockchain::restore_from(blocks, rules, reorg_limit, snapshot)?
            }
            None => {
                let mut chain = Blockchain::new(rules).with_reorg_limit(reorg_limit);
                let events = chain.take_events();
                repository.save(chain.blocks(), &events).map_err(NodeError::Storage)?;
                (chain, None)
            }
        };
        Ok(Self {
            mempool: Arc::new(Mutex::new(Mempool::on(chain.ledger()))),
            chain: Arc::new(Mutex::new(chain)),
            repository,
            snapshot_every: SNAPSHOT_BLOCKS,
            last_snapshot: Mutex::new(restored_from.unwrap_or(0)),
            restored_from,
            listeners: Mutex::default(),
        })
    }

    /// Saves a snapshot every `blocks` blocks instead of every `SNAPSHOT_BLOCKS`. 0 saves none.
    pub fn with_snapshots_every(self, blocks: usize) -> Self {
        Self { snapshot_every: blocks, ..self }
    }

    /// The height of the snapshot the chain was restored from on opening, if one was used.
    pub fn restored_from(&self) -> Option<usize> {
        self.restored_from
    }

    /// Answers questions about the same chain this service changes.
    pub fn queries(&self) -> ChainQueries {
        ChainQueries::new(self.chain.clone(), self.mempool.clone())
    }

    /// Applies `change` to a copy of the chain and swaps it in only once the new blocks are
    /// saved, so memory and storage never disagree.
    /// The chain's events are what's saved and, once the chain is free again, what listeners
    /// hear.
    fn commit<T, E: Into<DomainError>>(
        &self,
        change: impl FnOnce(&mut Blockchain) -> Result<T, E>,
    ) -> Result<T, NodeError> {
        let mut chain = self.chain.lock().unwrap();
        let mut next = chain.clone();
        let result = change(&mut next).map_err(|e| NodeError::Domain(e.into()))?;
        let events = next.take_events();
        let mut rebased = Vec::new();
        if !events.is_empty() {
            self.repository.save(next.blocks(), &events).map_err(NodeError::Storage)?;
            self.maybe_snapshot(&next);
            let mut mempool = self.mempool.lock().unwrap();
            mempool.rebase(next.ledger(), returned_by(&events));
            rebased = mempool.take_events();
        }
        *chain = next;
        drop(chain);
        let rebased = rebased.into_iter().map(|event| NodeEvent::Mempool { event, from: None });
        self.publish(events.into_iter().map(NodeEvent::Chain).chain(rebased));
        Ok(result)
    }
}

/// The transfers forks in `events` dropped, for the mempool to take back.
fn returned_by(events: &[ChainEvent]) -> Vec<Transaction> {
    let returned = events.iter().filter_map(|event| match event {
        ChainEvent::Reorganized { returned, .. } => Some(returned.iter().cloned()),
        _ => None,
    });
    returned.flatten().collect()
}

#[cfg(test)]
mod tests;
