//! Pulling: checking in with every peer, catching up with any whose chain could beat ours, and
//! doing that every few seconds for as long as the node runs. Catching up is headers first: a
//! peer's headers are checked and weighed before any of its blocks are fetched, and then only
//! from where its chain leaves ours.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures_util::future::join_all;

use super::{Network, PeerStatus, PeerTip, BAN_AFTER, DISCOVER_EVERY, HEADER_PAGE};
use crate::application::{off_runtime, ApplicationError, NodeError};
use crate::domain::chain::block::{Block, BlockHeader};
use crate::domain::chain::{ChainError, Extension, HeaderVerdict};

/// What a peer that answered sent: a chain that was adopted, nothing new, or one that breaks a
/// rule, and how.
enum Pulled {
    Adopted,
    Nothing,
    Invalid(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// The peer whose heavier chain was adopted, if any.
    pub adopted_from: Option<String>,
    pub reached: usize,
    pub length: usize,
}

impl Network {
    /// Joins `peers`, then pulls from every peer every `every`, for as long as the node runs.
    /// After adopting a heavier chain from one, tells the rest about it. Meets the peers of its
    /// peers after the first sync, once it has heard from them, and every `DISCOVER_EVERY` after.
    pub async fn keep_in_sync(self: Arc<Self>, peers: Vec<String>, every: Duration) {
        for url in &peers {
            match self.join(url).await {
                Ok(_) => tracing::info!("Peer {url}"),
                Err(e) => tracing::warn!("Skipping peer: {e}"),
            }
        }
        let mut ticker = tokio::time::interval(every);
        for round in (0..DISCOVER_EVERY).cycle() {
            ticker.tick().await;
            let report = self.sync().await;
            if let Some(peer) = report.adopted_from {
                tracing::info!("Adopted the heavier chain from {peer} ({} blocks)", report.length);
                self.broadcast_tip(Some(&peer)).await;
            }
            if round == 0 {
                self.discover().await;
            }
        }
    }

    /// Checks in with every peer once. Fetches blocks only from a peer whose chain could beat
    /// ours, and its mempool only when something is waiting in it.
    /// Every peer is reached at once, so a slow one doesn't hold up the rest. A peer that hasn't
    /// answered for `forget_after` is forgotten, unless it's one this node was told to join; it
    /// comes back if it reaches out again, or a peer that reached it passes it on.
    pub async fn sync(&self) -> SyncReport {
        let urls = self.peer_urls();
        let pulls = join_all(urls.iter().map(|url| self.pull(url))).await;
        let mut report = SyncReport::default();
        let now = Utc::now();
        for (url, pulled) in urls.into_iter().zip(pulls) {
            let status = match pulled {
                Ok((theirs, pulled)) => {
                    report.reached += 1;
                    let (mut strikes, mut banned_until, mut error) = (0, None, None);
                    match pulled {
                        Pulled::Adopted => report.adopted_from = Some(url.clone()),
                        Pulled::Nothing => {}
                        Pulled::Invalid(rule) => {
                            strikes = self.status_of(&url).strikes + 1;
                            if strikes >= BAN_AFTER {
                                let until = now + self.ban_for;
                                tracing::warn!("Banned peer {url} until {until}: {strikes} invalid chains in a row, the last {rule}");
                                (strikes, banned_until) = (0, Some(until));
                            }
                            error = Some(format!("sent a chain whose {rule}"));
                        }
                    }
                    PeerStatus {
                        height: Some(theirs.height as usize + 1),
                        last_seen: Some(now),
                        error,
                        failing_since: None,
                        strikes,
                        banned_until,
                    }
                }
                Err(error) => {
                    let before = self.status_of(&url);
                    let failing_since = before.failing_since.unwrap_or(now);
                    if now - failing_since >= self.forget_after && !self.kept(&url) {
                        tracing::info!("Forgot peer {url}: unreachable since {failing_since}");
                        self.peers.lock().unwrap().remove(&url);
                        continue;
                    }
                    PeerStatus { error: Some(error), failing_since: Some(failing_since), ..before }
                }
            };
            self.peers.lock().unwrap().insert(url, status);
        }
        report.length = self.queries.tip().height as usize + 1;
        report
    }

    /// Catches up with one peer. Returns its status and what its chain came to.
    async fn pull(&self, url: &str) -> Result<(PeerTip, Pulled), String> {
        let theirs = self.client.status(url, self.me.as_deref()).await?;
        let ours = self.queries.tip();
        let mut pulled = Pulled::Nothing;
        if theirs.tip != ours.hash && theirs.weight >= ours.weight.value() {
            // Headers from as far back as a fork may reach, which say whether the peer's chain
            // holds up and where it leaves ours, before a single block is fetched.
            let shorter = ours.height.min(theirs.height) as usize + 1;
            let headers = self.headers_from(url, self.recent_start(shorter)).await?;
            let queries = self.queries.clone();
            match off_runtime(move || queries.weigh_headers(&headers)).await {
                Ok(HeaderVerdict::FetchFrom(from)) => {
                    let ours = ours.height as usize + 1;
                    pulled = self.fetch_blocks(url, from, from < ours).await?;
                }
                Ok(HeaderVerdict::Ignore | HeaderVerdict::MissingParent) => {}
                Err(refused) if breaks_a_rule(&refused) => {
                    tracing::warn!("Not fetching from {url}: its headers {refused}");
                    pulled = Pulled::Invalid(format!("headers {refused}"));
                }
                Err(refused) => tracing::warn!("Not fetching from {url}: its headers {refused}"),
            }
        }
        if theirs.pending > 0 {
            let waiting = self.client.pending(url).await.unwrap_or_default();
            let (node, from) = (self.node.clone(), url.to_owned());
            off_runtime(move || {
                for tx in waiting {
                    let _ = node.submit_from(tx, Some(&from));
                }
            })
            .await;
        }
        Ok((theirs, pulled))
    }
}

impl Network {
    fn kept(&self, url: &str) -> bool {
        self.kept.lock().unwrap().contains(url)
    }

    /// Every header the peer has from `height` on, a page at a time.
    async fn headers_from(&self, url: &str, height: usize) -> Result<Vec<BlockHeader>, String> {
        let mut headers = Vec::new();
        loop {
            let page = self.client.headers_from(url, height + headers.len(), HEADER_PAGE).await?;
            let fetched = page.len();
            headers.extend(page);
            if fetched < HEADER_PAGE {
                return Ok(headers);
            }
        }
    }

    /// Fetches the peer's blocks from `from` on and offers them to the node. Returns whether
    /// they were adopted, or the rule one broke. Blocks on top of our tip are offered a page at a time, each checked as
    /// it comes, so a node far behind makes progress and no response grows with the chain. A
    /// `fork` is fetched whole first: a page of it alone may weigh less than the blocks of ours
    /// it would replace, and the next page wouldn't build on anything we have.
    async fn fetch_blocks(&self, url: &str, from: usize, fork: bool) -> Result<Pulled, String> {
        let mut adopted = false;
        let mut whole: Vec<Block> = Vec::new();
        let mut next = from;
        loop {
            let page = self.client.chain_from(url, next, self.page).await?;
            let fetched = page.len();
            next += fetched;
            if fork {
                whole.extend(page);
            } else {
                match self.offer(page).await {
                    Ok(Some(taken)) => adopted |= taken,
                    Ok(None) => break,
                    Err(rule) => return Ok(Pulled::Invalid(rule)),
                }
            }
            if fetched < self.page {
                break;
            }
        }
        if fork && !whole.is_empty() {
            match self.offer(whole).await {
                Ok(taken) => adopted = taken.unwrap_or(false),
                Err(rule) => return Ok(Pulled::Invalid(rule)),
            }
        }
        Ok(if adopted { Pulled::Adopted } else { Pulled::Nothing })
    }

    /// Offers blocks to the node. Returns whether they were adopted, `None` when it's no use
    /// going on because they build on a block we don't have, or the rule one breaks.
    async fn offer(&self, blocks: Vec<Block>) -> Result<Option<bool>, String> {
        let node = self.node.clone();
        match off_runtime(move || node.extend(blocks)).await {
            Ok((extension, _)) => Ok(match extension {
                Extension::Adopted => Some(true),
                Extension::Ignored => Some(false),
                Extension::MissingParent => None,
            }),
            Err(NodeError::Domain(ApplicationError::Chain(refused))) if breaks_a_rule(&refused) => {
                Err(format!("blocks {refused}"))
            }
            Err(NodeError::Domain(ApplicationError::Block(refused))) => {
                Err(format!("blocks {refused}"))
            }
            Err(_) => Ok(None),
        }
    }
}

/// Whether a refusal is the peer's fault: a block or header that breaks a rule. A fork deeper
/// than we follow is only a different view of the chain, and an empty answer only a short one.
fn breaks_a_rule(refused: &ChainError) -> bool {
    matches!(refused, ChainError::InvalidBlock { .. })
}
