//! Submitting a transaction to the mempool, from a client or a peer.

use super::{NodeEvent, NodeService};
use crate::domain::accounting::transaction::Transaction;
use crate::domain::accounting::TransactionError;

impl NodeService {
    /// Queues a transaction, checked against the confirmed ledger and everything already
    /// waiting. The mempool isn't saved, so nothing is written.
    pub fn submit(&self, tx: Transaction) -> Result<(), TransactionError> {
        self.submit_from(tx, None)
    }

    /// Like `submit`, for a transaction that arrived from the peer at `from`.
    pub fn submit_from(&self, tx: Transaction, from: Option<&str>) -> Result<(), TransactionError> {
        let events = {
            let mut mempool = self.mempool.lock().unwrap();
            mempool.admit(tx)?;
            mempool.take_events()
        };
        let from = from.map(str::to_string);
        self.publish(
            events.into_iter().map(|event| NodeEvent::Mempool { event, from: from.clone() }),
        );
        Ok(())
    }
}
