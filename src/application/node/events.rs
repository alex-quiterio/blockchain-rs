//! What happened in a node, and telling whoever listens.

use super::NodeService;
use crate::domain::chain::ChainEvent;
use crate::domain::mempool::MempoolEvent;

pub(super) type Listener = Box<dyn Fn(&NodeEvent) + Send + Sync>;

/// What happened in a node, for its listeners: to its chain, or to its mempool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeEvent {
    Chain(ChainEvent),
    /// `from` is the peer a transaction arrived from, when one did, so it isn't sent back.
    Mempool {
        event: MempoolEvent,
        from: Option<String>,
    },
}

impl NodeService {
    /// Calls `listener` with everything that happens to the chain from now on, after it's saved.
    pub fn subscribe(&self, listener: impl Fn(&NodeEvent) + Send + Sync + 'static) {
        self.listeners.lock().unwrap().push(Box::new(listener));
    }

    /// Tells every listener, in order. Called with no lock held, so a listener may call back in.
    pub(super) fn publish(&self, events: impl IntoIterator<Item = NodeEvent>) {
        for event in events {
            for listener in self.listeners.lock().unwrap().iter() {
                listener(&event);
            }
        }
    }
}
