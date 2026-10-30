//! The other nodes this one keeps in step with: who they are (here), the port to reach them
//! (`client`), pulling what they have (`sync`) and pushing what's new (`gossip`).

mod client;
mod gossip;
mod sync;

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use super::{ChainQueries, NodeService};

pub use client::{PeerClient, PeerResult, PeerTip};
pub use sync::SyncReport;

/// What this node last heard from a peer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerStatus {
    pub height: Option<usize>,
    pub last_seen: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

/// The other nodes this one keeps in step with. New blocks and transactions are pushed to peers
/// as they happen, and peers are polled now and then for anything the pushes missed. Only what a
/// peer may lack goes over the wire: a new block on its own, more history only when it asks.
pub struct Network {
    node: Arc<NodeService>,
    queries: ChainQueries,
    client: Box<dyn PeerClient>,
    /// This node's own URL, given to peers so they can reach back.
    me: Option<String>,
    peers: Mutex<BTreeMap<String, PeerStatus>>,
    /// How many blocks to fetch per request when catching up.
    page: usize,
}

/// Blocks per request when catching up: small enough to arrive well within a request's timeout,
/// large enough that a node far behind needs few of them.
pub const SYNC_PAGE: usize = 500;

/// Headers per request when catching up. A header is a fraction of a block, so more fit.
pub const HEADER_PAGE: usize = 2000;

impl Network {
    pub fn new(node: Arc<NodeService>, client: Box<dyn PeerClient>, me: Option<String>) -> Self {
        let me = me.map(|url| normalize(&url));
        let queries = node.queries();
        Self { node, queries, client, me, peers: Mutex::default(), page: SYNC_PAGE }
    }

    /// Fetches `page` blocks per request when catching up, instead of `SYNC_PAGE`.
    pub fn with_sync_page(self, page: usize) -> Self {
        Self { page: page.max(1), ..self }
    }

    pub fn node(&self) -> &Arc<NodeService> {
        &self.node
    }

    pub fn queries(&self) -> &ChainQueries {
        &self.queries
    }

    pub fn peers(&self) -> Vec<(String, PeerStatus)> {
        self.peers.lock().unwrap().iter().map(|(url, s)| (url.clone(), s.clone())).collect()
    }

    /// Adds a peer and introduces this node back to it. Returns whether it was new. A peer that's
    /// already known keeps what we last heard from it.
    ///
    /// Nodes also call this for anyone who reaches out to them, so a link that only goes one way
    /// heals the next time the two talk: say one side restarted and forgot the other, or started
    /// before the other was up to hear its introduction.
    pub async fn add_peer(&self, url: &str) -> Result<bool, String> {
        let url = normalize(url);
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(format!("peer must be an http(s) URL, not {url}"));
        }
        if Some(&url) == self.me.as_ref() {
            return Ok(false);
        }
        let added = match self.peers.lock().unwrap().entry(url.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(PeerStatus::default());
                true
            }
            Entry::Occupied(_) => false,
        };
        if added {
            if let Some(me) = &self.me {
                // Best effort: a peer that's down now will still be pulled from later.
                let _ = self.client.introduce(&url, me).await;
            }
        }
        Ok(added)
    }

    /// Where the blocks a fork could reach back to begin, for chains whose shorter one has
    /// `length` blocks. Never genesis, which every node shares.
    fn recent_start(&self, length: usize) -> usize {
        let depth = usize::try_from(self.queries.reorg_limit().blocks()).unwrap_or(usize::MAX);
        length.saturating_sub(depth.saturating_add(1)).max(1)
    }

    fn peers_except(&self, except: Option<&str>) -> Vec<String> {
        self.peer_urls().into_iter().filter(|url| Some(url.as_str()) != except).collect()
    }

    fn peer_urls(&self) -> Vec<String> {
        self.peers.lock().unwrap().keys().cloned().collect()
    }

    fn status_of(&self, url: &str) -> PeerStatus {
        self.peers.lock().unwrap().get(url).cloned().unwrap_or_default()
    }
}

fn normalize(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests;
