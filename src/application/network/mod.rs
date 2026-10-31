//! The other nodes this one keeps in step with: who they are (here), the port to reach them
//! (`client`), pulling what they have (`sync`), pushing what's new (`gossip`) and meeting the
//! peers of our peers (`discovery`).

mod client;
mod discovery;
mod gossip;
mod sync;

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use super::{ChainQueries, NodeService};

pub use client::{PeerClient, PeerResult, PeerTip};
pub use discovery::{DISCOVER_EVERY, MAX_PEERS};
pub use sync::SyncReport;

/// What this node last heard from a peer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerStatus {
    pub height: Option<usize>,
    pub last_seen: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// When the attempts that have failed since it last answered began.
    pub failing_since: Option<DateTime<Utc>>,
    /// How many syncs in a row it answered with headers or blocks that break a rule.
    pub strikes: u32,
    /// Until when it's ignored, after `BAN_AFTER` strikes: not pulled from, gossiped to or asked
    /// for its peers, and its pushes are refused.
    pub banned_until: Option<DateTime<Utc>>,
}

impl PeerStatus {
    pub fn is_banned(&self, now: DateTime<Utc>) -> bool {
        self.banned_until.is_some_and(|until| until > now)
    }
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
    /// The peers it was told to join on startup, which it never forgets.
    kept: Mutex<BTreeSet<String>>,
    /// How many blocks to fetch per request when catching up.
    page: usize,
    /// The most peers discovery adds up to.
    max_peers: usize,
    /// How long a peer may go unanswered before it's forgotten.
    forget_after: TimeDelta,
    /// How long a peer that keeps sending invalid chains is ignored for.
    ban_for: TimeDelta,
}

/// Blocks per request when catching up: small enough to arrive well within a request's timeout,
/// large enough that a node far behind needs few of them.
pub const SYNC_PAGE: usize = 500;

/// How long a peer may go without answering before it's forgotten: long enough to ride out a
/// restart or a deploy, short enough that a node gone for good stops costing a request per sync.
pub const FORGET_AFTER: Duration = Duration::from_secs(600);

/// Syncs in a row a peer may answer with an invalid chain before it's banned. Only what this
/// node fetched counts: a push names its sender in a header anyone could write, so counting
/// those would let a node get an honest peer banned by sending garbage in its name.
pub const BAN_AFTER: u32 = 3;

/// How long a banned peer is ignored for: long enough that a node feeding us garbage stops
/// costing us a fetch per sync, short enough that one fixed by a restart is soon back.
pub const BAN_FOR: Duration = Duration::from_secs(3600);

/// Headers per request when catching up. A header is a fraction of a block, so more fit.
pub const HEADER_PAGE: usize = 2000;

impl Network {
    pub fn new(node: Arc<NodeService>, client: Box<dyn PeerClient>, me: Option<String>) -> Self {
        let me = me.map(|url| normalize(&url));
        let queries = node.queries();
        Self {
            node,
            queries,
            client,
            me,
            peers: Mutex::default(),
            kept: Mutex::default(),
            page: SYNC_PAGE,
            max_peers: MAX_PEERS,
            forget_after: TimeDelta::from_std(FORGET_AFTER).unwrap(),
            ban_for: TimeDelta::from_std(BAN_FOR).unwrap(),
        }
    }

    /// Fetches `page` blocks per request when catching up, instead of `SYNC_PAGE`.
    pub fn with_sync_page(self, page: usize) -> Self {
        Self { page: page.max(1), ..self }
    }

    /// Stops discovering peers once it has `max_peers`, instead of `MAX_PEERS`.
    pub fn with_max_peers(self, max_peers: usize) -> Self {
        Self { max_peers, ..self }
    }

    /// Forgets a peer that hasn't answered for `forget_after`, instead of `FORGET_AFTER`.
    pub fn with_forget_after(self, forget_after: Duration) -> Self {
        Self { forget_after: TimeDelta::from_std(forget_after).unwrap_or(TimeDelta::MAX), ..self }
    }

    /// Ignores a peer that keeps sending invalid chains for `ban_for`, instead of `BAN_FOR`.
    pub fn with_ban_for(self, ban_for: Duration) -> Self {
        Self { ban_for: TimeDelta::from_std(ban_for).unwrap_or(TimeDelta::MAX), ..self }
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

    /// Adds a peer, as `add_peer` does, that's never forgotten however long it's unreachable.
    /// For the peers a node is told to join: if one of those restarted and forgot this node,
    /// forgetting it back would leave the two apart for good.
    pub async fn join(&self, url: &str) -> Result<bool, String> {
        self.kept.lock().unwrap().insert(normalize(url));
        self.add_peer(url).await
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

    /// Whether `url` is a peer this node is ignoring for now.
    pub fn is_banned(&self, url: &str) -> bool {
        let peers = self.peers.lock().unwrap();
        peers.get(&normalize(url)).is_some_and(|status| status.is_banned(Utc::now()))
    }

    /// The peers to talk to: every one but those banned for now.
    fn peer_urls(&self) -> Vec<String> {
        let (peers, now) = (self.peers.lock().unwrap(), Utc::now());
        peers
            .iter()
            .filter(|(_, status)| !status.is_banned(now))
            .map(|(url, _)| url.clone())
            .collect()
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
