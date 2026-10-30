//! When to save a snapshot of the balances, which spares the next start replaying the chain.

use super::NodeService;
use crate::domain::chain::Blockchain;

impl NodeService {
    /// Saves a snapshot once the chain has moved `snapshot_every` blocks past the last one. It
    /// only saves time on the next start, so a failure is reported and otherwise ignored.
    pub(super) fn maybe_snapshot(&self, chain: &Blockchain) {
        let Some(snapshot) = chain.snapshot().filter(|_| self.snapshot_every > 0) else {
            return;
        };
        let mut last = self.last_snapshot.lock().unwrap();
        if snapshot.height < *last + self.snapshot_every {
            return;
        }
        match self.repository.save_snapshot(&snapshot) {
            Ok(()) => *last = snapshot.height,
            Err(e) => {
                tracing::warn!("Could not save a snapshot at height {}: {e}", snapshot.height)
            }
        }
    }
}
