//! Pulling: checking in with every peer, catching up with any whose chain could beat ours, and
//! doing that every few seconds for as long as the node runs.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures_util::future::join_all;

use super::{Network, PeerStatus, PeerTip};
use crate::domain::chain::Extension;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// The peer whose heavier chain was adopted, if any.
    pub adopted_from: Option<String>,
    pub reached: usize,
    pub length: usize,
}

impl Network {
    /// Joins `peers`, then pulls from every peer every `every`, for as long as the node runs.
    /// After adopting a heavier chain from one, tells the rest about it.
    pub async fn keep_in_sync(self: Arc<Self>, peers: Vec<String>, every: Duration) {
        for url in &peers {
            match self.add_peer(url).await {
                Ok(_) => tracing::info!("Peer {url}"),
                Err(e) => tracing::warn!("Skipping peer: {e}"),
            }
        }
        let mut ticker = tokio::time::interval(every);
        loop {
            ticker.tick().await;
            let report = self.sync().await;
            if let Some(peer) = report.adopted_from {
                tracing::info!("Adopted the heavier chain from {peer} ({} blocks)", report.length);
                self.broadcast_tip(Some(&peer)).await;
            }
        }
    }

    /// Checks in with every peer once. Fetches blocks only from a peer whose chain could beat
    /// ours, and its mempool only when something is waiting in it.
    /// Every peer is reached at once, so a slow one doesn't hold up the rest.
    pub async fn sync(&self) -> SyncReport {
        let urls = self.peer_urls();
        let pulls = join_all(urls.iter().map(|url| self.pull(url))).await;
        let mut report = SyncReport::default();
        for (url, pulled) in urls.into_iter().zip(pulls) {
            let status = match pulled {
                Ok((theirs, adopted)) => {
                    report.reached += 1;
                    if adopted {
                        report.adopted_from = Some(url.clone());
                    }
                    PeerStatus {
                        height: Some(theirs.height as usize + 1),
                        last_seen: Some(Utc::now()),
                        error: None,
                    }
                }
                Err(error) => PeerStatus { error: Some(error), ..self.status_of(&url) },
            };
            self.peers.lock().unwrap().insert(url, status);
        }
        report.length = self.queries.tip().height as usize + 1;
        report
    }

    /// Catches up with one peer. Returns its status and whether its chain was adopted.
    async fn pull(&self, url: &str) -> Result<(PeerTip, bool), String> {
        let theirs = self.client.status(url, self.me.as_deref()).await?;
        let ours = self.queries.tip();
        let mut adopted = false;
        if theirs.tip != ours.hash && theirs.weight >= ours.weight.value() {
            // From as far back as a fork may reach, a page at a time. Each page builds on the
            // last, so it's checked block by block, and no single response grows with the chain.
            let shorter = ours.height.min(theirs.height) as usize + 1;
            let mut from = self.recent_start(shorter);
            loop {
                let page = self.client.chain_from(url, from, self.page).await?;
                let fetched = page.len();
                match self.node.extend(page) {
                    Ok((Extension::Adopted, _)) => adopted = true,
                    Ok((Extension::Ignored, _)) => {}
                    // They fork off further back than we may undo, or sent a bad block.
                    Ok((Extension::MissingParent, _)) | Err(_) => break,
                }
                if fetched < self.page {
                    break;
                }
                from += fetched;
            }
        }
        if theirs.pending > 0 {
            for tx in self.client.pending(url).await.unwrap_or_default() {
                let _ = self.node.submit_from(tx, Some(url));
            }
        }
        Ok((theirs, adopted))
    }
}
