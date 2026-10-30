//! Discovery: meeting the peers of our peers, so a node given one peer to start with ends up
//! linked to the rest of the network directly, not only through whoever it was given.

use futures_util::future::join_all;

use super::Network;

/// Syncs between rounds of discovery. Who's on the network changes far more slowly than its
/// chain, so there's no need to ask every peer who it knows on every sync.
pub const DISCOVER_EVERY: u64 = 12;

/// The most peers a node seeks out by itself. Peers it's told to join, and peers that reach
/// out to it, are always added, so a link never goes only one way.
pub const MAX_PEERS: usize = 16;

impl Network {
    /// Asks every peer, all at once, which nodes it's in contact with, and adds those this node
    /// doesn't know yet, introducing itself to each, until it has `max_peers`. Returns how many
    /// were added. Peers only pass on nodes they've reached, so an address that never answered
    /// doesn't spread.
    pub async fn discover(&self) -> usize {
        let urls = self.peer_urls();
        let known = join_all(urls.iter().map(|url| self.client.peers(url))).await;
        let mut added = 0;
        for url in known.into_iter().flatten().flatten() {
            if self.peers.lock().unwrap().len() >= self.max_peers {
                break;
            }
            if let Ok(true) = self.add_peer(&url).await {
                tracing::info!("Discovered peer {url}");
                added += 1;
            }
        }
        added
    }
}
