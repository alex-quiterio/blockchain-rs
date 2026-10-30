//! An adapter that listens to the node and says in the log what happened to its chain and
//! mempool. Subscribe it with `NodeService::subscribe`.

use crate::application::NodeEvent;
use crate::domain::chain::ChainEvent;
use crate::domain::mempool::MempoolEvent;

/// Says in the log what happened to the chain and its mempool.
pub fn log_event(event: &NodeEvent) {
    match event {
        NodeEvent::Chain(ChainEvent::Started) => tracing::info!("Started a new chain"),
        NodeEvent::Chain(ChainEvent::Extended { from, to }) if from == to => {
            tracing::info!("Added block {to}")
        }
        NodeEvent::Chain(ChainEvent::Extended { from, to }) => {
            tracing::info!("Added blocks {from} to {to}")
        }
        NodeEvent::Chain(ChainEvent::Reorganized { from, dropped, added, returned }) => {
            tracing::info!(
                "Switched to a heavier fork: {dropped} block(s) from height {from} replaced by \
                 {added}; {} transfer(s) back to the mempool",
                returned.len()
            )
        }
        // One per transfer: too many to log by default.
        NodeEvent::Mempool { event: MempoolEvent::Accepted(tx), from } => tracing::debug!(
            "Accepted a transfer of {} from {}",
            tx.amount(),
            from.as_deref().unwrap_or("a client")
        ),
        NodeEvent::Mempool { event: MempoolEvent::Rebased { kept, dropped, returned }, .. } => {
            if dropped + returned > 0 {
                tracing::info!(
                    "Mempool: {dropped} transfer(s) left with the chain, {returned} came back \
                     from a fork, {kept} still waiting"
                )
            }
        }
    }
}
