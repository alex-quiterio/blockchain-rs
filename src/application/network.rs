use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use super::NodeService;
use crate::domain::block::{Block, Transaction};
use crate::domain::chain::Extension;

pub type PeerResult<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// The header gossip carries the sender's URL in, so the receiver doesn't relay it straight back.
pub const PEER_HEADER: &str = "x-peer";

/// How this node talks to other nodes. The HTTP adapter lives in infrastructure.
pub trait PeerClient: Send + Sync {
    fn status<'a>(&'a self, peer: &'a str) -> PeerResult<'a, PeerTip>;
    /// The peer's blocks from `height` on.
    fn chain_from<'a>(&'a self, peer: &'a str, height: usize) -> PeerResult<'a, Vec<Block>>;
    fn pending<'a>(&'a self, peer: &'a str) -> PeerResult<'a, Vec<Transaction>>;
    /// Offers blocks from some height on. `me` tells the peer who sent them.
    fn offer_blocks<'a>(
        &'a self,
        peer: &'a str,
        blocks: &'a [Block],
        me: Option<&'a str>,
    ) -> PeerResult<'a, Extension>;
    fn offer_transaction<'a>(
        &'a self,
        peer: &'a str,
        tx: &'a Transaction,
        me: Option<&'a str>,
    ) -> PeerResult<'a, ()>;
    /// Asks `peer` to add `me` to its own peers.
    fn introduce<'a>(&'a self, peer: &'a str, me: &'a str) -> PeerResult<'a, ()>;
}

/// Where a peer's chain stands, from a request small enough to make on every sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerTip {
    pub height: u64,
    pub tip: String,
    pub weight: u128,
    pub pending: usize,
}

/// What this node last heard from a peer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerStatus {
    pub height: Option<usize>,
    pub last_seen: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// The peer whose heavier chain was adopted, if any.
    pub adopted_from: Option<String>,
    pub reached: usize,
    pub length: usize,
}

/// The other nodes this one keeps in step with. New blocks and transactions are pushed to peers
/// as they happen, and peers are polled now and then for anything the pushes missed. Only what a
/// peer may lack goes over the wire: a new block on its own, more history only when it asks.
pub struct Network {
    node: Arc<NodeService>,
    client: Box<dyn PeerClient>,
    /// This node's own URL, given to peers so they can reach back.
    me: Option<String>,
    peers: Mutex<BTreeMap<String, PeerStatus>>,
}

impl Network {
    pub fn new(node: Arc<NodeService>, client: Box<dyn PeerClient>, me: Option<String>) -> Self {
        let me = me.map(|url| normalize(&url));
        Self { node, client, me, peers: Mutex::default() }
    }

    pub fn node(&self) -> &Arc<NodeService> {
        &self.node
    }

    pub fn peers(&self) -> Vec<(String, PeerStatus)> {
        self.peers.lock().unwrap().iter().map(|(url, s)| (url.clone(), s.clone())).collect()
    }

    /// Adds a peer and introduces this node back to it. Returns whether it was new.
    pub async fn add_peer(&self, url: &str) -> Result<bool, String> {
        let url = normalize(url);
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(format!("peer must be an http(s) URL, not {url}"));
        }
        if Some(&url) == self.me.as_ref() {
            return Ok(false);
        }
        let added = self.peers.lock().unwrap().insert(url.clone(), PeerStatus::default()).is_none();
        if added {
            if let Some(me) = &self.me {
                // Best effort: a peer that's down now will still be pulled from later.
                let _ = self.client.introduce(&url, me).await;
            }
        }
        Ok(added)
    }

    /// Checks in with every peer once. Fetches blocks only from a peer whose chain could beat
    /// ours, and its mempool only when something is waiting in it.
    pub async fn sync(&self) -> SyncReport {
        let mut report = SyncReport::default();
        for url in self.peer_urls() {
            let status = match self.pull(&url).await {
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
        report.length = self.node.tip().height as usize + 1;
        report
    }

    /// Catches up with one peer. Returns its status and whether its chain was adopted.
    async fn pull(&self, url: &str) -> Result<(PeerTip, bool), String> {
        let theirs = self.client.status(url).await?;
        let ours = self.node.tip();
        let mut adopted = false;
        if theirs.tip != ours.hash && theirs.weight >= ours.weight.value() {
            let shorter = ours.height.min(theirs.height) as usize + 1;
            for from in self.fallbacks(shorter, false) {
                let blocks = self.client.chain_from(url, from).await?;
                match self.node.extend(blocks) {
                    Ok((Extension::MissingParent, _)) => continue,
                    Ok((extension, _)) => adopted = extension == Extension::Adopted,
                    Err(_) => {}
                }
                break;
            }
        }
        if theirs.pending > 0 {
            for tx in self.client.pending(url).await.unwrap_or_default() {
                let _ = self.node.submit(tx);
            }
        }
        Ok((theirs, adopted))
    }

    /// Tells every peer but `except` about this node's newest block, after it mined, forged or
    /// adopted one. Only the tip goes out. A peer that lacks its parent gets the blocks a fork
    /// could reach back to, and one further behind than that gets the whole chain.
    pub async fn broadcast_tip(&self, except: Option<&str>) {
        let length = self.node.tip().height as usize + 1;
        for url in self.peer_urls().into_iter().filter(|url| Some(url.as_str()) != except) {
            for from in self.fallbacks(length, true) {
                let blocks = self.node.blocks_from(from);
                match self.client.offer_blocks(&url, &blocks, self.me.as_deref()).await {
                    Ok(Extension::MissingParent) => continue,
                    _ => break,
                }
            }
        }
    }

    /// Passes a transaction this node accepted on to every peer but the one it came from.
    pub async fn broadcast_transaction(&self, tx: &Transaction, except: Option<&str>) {
        for url in self.peer_urls().into_iter().filter(|url| Some(url.as_str()) != except) {
            let _ = self.client.offer_transaction(&url, tx, self.me.as_deref()).await;
        }
    }

    /// Heights to send or fetch blocks from, nearest first, for chains whose shorter one has
    /// `length` blocks: just the tip (when pushing), then as far back as a fork may reach, then
    /// everything.
    fn fallbacks(&self, length: usize, tip_first: bool) -> Vec<usize> {
        let depth = self.node.rules().max_reorg_depth as usize;
        let recent = length.saturating_sub(depth + 1).max(1);
        let mut heights: Vec<usize> = Vec::new();
        for height in [tip_first.then(|| length - 1), Some(recent), Some(0)].into_iter().flatten() {
            if heights.last().is_none_or(|&last| height < last) {
                heights.push(height);
            }
        }
        heights
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
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::domain::address::Address;
    use crate::domain::crypto;
    use crate::domain::rules::ChainRules;
    use crate::infrastructure::InMemoryRepository;

    /// A whole network in one process: peer URLs map straight to other nodes' networks. Every
    /// message is logged as `peer what`, so tests can see what went over the wire.
    #[derive(Clone, Default)]
    struct Mesh {
        nodes: Arc<Mutex<HashMap<String, Arc<Network>>>>,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Mesh {
        fn get(&self, url: &str, what: String) -> Result<Arc<Network>, String> {
            self.log.lock().unwrap().push(format!("{url} {what}"));
            self.nodes.lock().unwrap().get(url).cloned().ok_or_else(|| format!("{url} is down"))
        }

        fn join(&self, url: &str) -> Arc<Network> {
            let node = NodeService::open(
                Box::new(InMemoryRepository::default()),
                ChainRules::proof_of_work(2, 50).with_max_reorg_depth(3),
            )
            .unwrap();
            let network =
                Arc::new(Network::new(Arc::new(node), Box::new(self.clone()), Some(url.into())));
            self.nodes.lock().unwrap().insert(url.to_string(), network.clone());
            network
        }

        /// The messages sent since the last call.
        fn sent(&self) -> Vec<String> {
            std::mem::take(&mut self.log.lock().unwrap())
        }
    }

    impl PeerClient for Mesh {
        fn status<'a>(&'a self, peer: &'a str) -> PeerResult<'a, PeerTip> {
            Box::pin(async move {
                let tip = self.get(peer, "status".into())?.node().tip();
                Ok(PeerTip {
                    height: tip.height,
                    tip: tip.hash,
                    weight: tip.weight.value(),
                    pending: tip.pending,
                })
            })
        }
        fn chain_from<'a>(&'a self, peer: &'a str, height: usize) -> PeerResult<'a, Vec<Block>> {
            Box::pin(async move {
                Ok(self.get(peer, format!("chain from {height}"))?.node().blocks_from(height))
            })
        }
        fn pending<'a>(&'a self, peer: &'a str) -> PeerResult<'a, Vec<Transaction>> {
            Box::pin(async move { Ok(self.get(peer, "pending".into())?.node().pending()) })
        }
        fn offer_blocks<'a>(
            &'a self,
            peer: &'a str,
            blocks: &'a [Block],
            _me: Option<&'a str>,
        ) -> PeerResult<'a, Extension> {
            Box::pin(async move {
                let what = format!("offered {} block(s)", blocks.len());
                let (extension, _) = self
                    .get(peer, what)?
                    .node()
                    .extend(blocks.to_vec())
                    .map_err(|e| e.to_string())?;
                Ok(extension)
            })
        }
        fn offer_transaction<'a>(
            &'a self,
            peer: &'a str,
            tx: &'a Transaction,
            _me: Option<&'a str>,
        ) -> PeerResult<'a, ()> {
            Box::pin(async move {
                self.get(peer, "offered a transaction".into())?
                    .node()
                    .submit(tx.clone())
                    .map_err(|e| e.to_string())
            })
        }
        fn introduce<'a>(&'a self, peer: &'a str, me: &'a str) -> PeerResult<'a, ()> {
            Box::pin(async move {
                self.get(peer, "introduced".into())?.add_peer(me).await?;
                Ok(())
            })
        }
    }

    fn miner() -> Address {
        Address::parse(&crypto::generate_keypair().0).unwrap()
    }

    #[tokio::test]
    async fn adding_a_peer_introduces_both_ways() {
        let mesh = Mesh::default();
        let a = mesh.join("http://a");
        let b = mesh.join("http://b");

        assert_eq!(a.add_peer("http://b/").await, Ok(true));
        assert_eq!(a.add_peer("http://b").await, Ok(false));
        assert_eq!(a.add_peer("http://a").await, Ok(false), "a node is not its own peer");
        assert_eq!(b.peers()[0].0, "http://a");
        assert!(a.add_peer("ftp://c").await.is_err());
    }

    #[tokio::test]
    async fn sync_adopts_the_heaviest_chain_and_pending_transactions() {
        let mesh = Mesh::default();
        let a = mesh.join("http://a");
        let b = mesh.join("http://b");
        a.add_peer("http://b").await.unwrap();

        let (_, secret) = crypto::generate_keypair();
        let key = crypto::parse_secret_key(&secret).unwrap();
        let rich = Address::of(&key);
        b.node().mine(&rich).unwrap();
        b.node().mine(&miner()).unwrap();
        b.node().submit(Transaction::signed(&key, miner().as_str(), 5, 0, 1)).unwrap();

        let report = a.sync().await;
        assert_eq!(report.adopted_from.as_deref(), Some("http://b"));
        assert_eq!((report.reached, report.length), (1, 3));
        assert_eq!(a.node().blocks(), b.node().blocks());
        assert_eq!(a.node().pending(), b.node().pending());
        assert_eq!(a.peers()[0].1.height, Some(3));
    }

    #[tokio::test]
    async fn broadcasting_spreads_new_blocks_and_transactions() {
        let mesh = Mesh::default();
        let a = mesh.join("http://a");
        let b = mesh.join("http://b");
        let c = mesh.join("http://c");
        a.add_peer("http://b").await.unwrap();
        a.add_peer("http://c").await.unwrap();

        let (_, secret) = crypto::generate_keypair();
        let key = crypto::parse_secret_key(&secret).unwrap();
        a.node().mine(&Address::of(&key)).unwrap();
        a.broadcast_tip(None).await;
        assert_eq!(b.node().blocks(), a.node().blocks());
        assert_eq!(c.node().blocks(), a.node().blocks());

        let tx = Transaction::signed(&key, miner().as_str(), 5, 0, 0);
        a.node().submit(tx.clone()).unwrap();
        a.broadcast_transaction(&tx, None).await;
        assert_eq!(c.node().pending(), [tx]);
    }

    #[tokio::test]
    async fn a_peer_in_step_gets_only_the_new_block() {
        let mesh = Mesh::default();
        let a = mesh.join("http://a");
        let b = mesh.join("http://b");
        let c = mesh.join("http://c");
        a.add_peer("http://b").await.unwrap();
        a.add_peer("http://c").await.unwrap();
        for _ in 0..5 {
            a.node().mine(&miner()).unwrap();
            a.broadcast_tip(None).await;
        }
        mesh.sent();

        a.node().mine(&miner()).unwrap();
        a.broadcast_tip(Some("http://c")).await;
        assert_eq!(mesh.sent(), ["http://b offered 1 block(s)"], "not back to where it came from");
        assert_eq!(b.node().blocks(), a.node().blocks());
        assert_eq!(c.node().blocks().len(), 6);
    }

    #[tokio::test]
    async fn a_peer_behind_gets_recent_blocks_then_the_whole_chain() {
        let mesh = Mesh::default();
        let a = mesh.join("http://a");
        let b = mesh.join("http://b");
        a.add_peer("http://b").await.unwrap();
        a.node().mine(&miner()).unwrap();
        a.node().mine(&miner()).unwrap();
        mesh.sent();

        a.broadcast_tip(None).await;
        assert_eq!(mesh.sent(), ["http://b offered 1 block(s)", "http://b offered 2 block(s)"]);
        assert_eq!(b.node().blocks(), a.node().blocks());

        // Further behind than a fork may reach (3 blocks here): the whole chain goes.
        for _ in 0..5 {
            a.node().mine(&miner()).unwrap();
        }
        a.broadcast_tip(None).await;
        assert_eq!(
            mesh.sent(),
            [
                "http://b offered 1 block(s)",
                "http://b offered 4 block(s)",
                "http://b offered 8 block(s)"
            ]
        );
        assert_eq!(b.node().blocks(), a.node().blocks());
    }

    #[tokio::test]
    async fn sync_only_checks_the_status_of_a_peer_in_step() {
        let mesh = Mesh::default();
        let a = mesh.join("http://a");
        let b = mesh.join("http://b");
        a.add_peer("http://b").await.unwrap();
        b.node().mine(&miner()).unwrap();
        a.sync().await;
        assert_eq!(a.node().blocks(), b.node().blocks());
        mesh.sent();

        let report = a.sync().await;
        assert_eq!(mesh.sent(), ["http://b status"]);
        assert_eq!((report.reached, report.adopted_from), (1, None));

        a.node().mine(&miner()).unwrap();
        a.sync().await;
        assert_eq!(mesh.sent(), ["http://b status"], "a lighter peer has nothing for us");
    }

    #[tokio::test]
    async fn an_unreachable_peer_is_reported_not_fatal() {
        let mesh = Mesh::default();
        let a = mesh.join("http://a");
        a.add_peer("http://gone").await.unwrap();

        let report = a.sync().await;
        assert_eq!(report.reached, 0);
        assert_eq!(a.peers()[0].1.error.as_deref(), Some("http://gone is down"));
    }
}
