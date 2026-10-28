use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use super::NodeService;
use crate::domain::block::{Block, Transaction};

pub type PeerResult<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// How this node talks to other nodes. The HTTP adapter lives in infrastructure.
pub trait PeerClient: Send + Sync {
    fn chain<'a>(&'a self, peer: &'a str) -> PeerResult<'a, Vec<Block>>;
    fn pending<'a>(&'a self, peer: &'a str) -> PeerResult<'a, Vec<Transaction>>;
    fn offer_chain<'a>(&'a self, peer: &'a str, blocks: &'a [Block]) -> PeerResult<'a, ()>;
    fn offer_transaction<'a>(&'a self, peer: &'a str, tx: &'a Transaction) -> PeerResult<'a, ()>;
    /// Asks `peer` to add `me` to its own peers.
    fn introduce<'a>(&'a self, peer: &'a str, me: &'a str) -> PeerResult<'a, ()>;
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
    /// The peer whose longer chain was adopted, if any.
    pub adopted_from: Option<String>,
    pub reached: usize,
    pub length: usize,
}

/// The other nodes this one keeps in step with: it pulls their chains and mempools, adopts the
/// longest valid chain, and gossips its own new blocks and transactions.
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

    /// Pulls every peer's chain and mempool once. Adopts the longest valid chain on offer and
    /// queues any of their pending transactions that are valid here.
    pub async fn sync(&self) -> SyncReport {
        let mut report = SyncReport::default();
        for url in self.peer_urls() {
            let status = match self.client.chain(&url).await {
                Ok(blocks) => {
                    report.reached += 1;
                    let height = blocks.len();
                    if let Ok((true, _)) = self.node.adopt_if_longer(blocks) {
                        report.adopted_from = Some(url.clone());
                    }
                    for tx in self.client.pending(&url).await.unwrap_or_default() {
                        let _ = self.node.submit(tx);
                    }
                    PeerStatus { height: Some(height), last_seen: Some(Utc::now()), error: None }
                }
                Err(error) => PeerStatus { error: Some(error), ..self.status_of(&url) },
            };
            self.peers.lock().unwrap().insert(url, status);
        }
        report.length = self.node.validity().1;
        report
    }

    /// Offers this node's chain to every peer, after it mined, forged or adopted a block.
    pub async fn broadcast_chain(&self) {
        let blocks = self.node.blocks();
        for url in self.peer_urls() {
            let _ = self.client.offer_chain(&url, &blocks).await;
        }
    }

    /// Passes a transaction this node accepted on to every peer.
    pub async fn broadcast_transaction(&self, tx: &Transaction) {
        for url in self.peer_urls() {
            let _ = self.client.offer_transaction(&url, tx).await;
        }
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

    /// A whole network in one process: peer URLs map straight to other nodes' networks.
    #[derive(Clone, Default)]
    struct Mesh(Arc<Mutex<HashMap<String, Arc<Network>>>>);

    impl Mesh {
        fn get(&self, url: &str) -> Result<Arc<Network>, String> {
            self.0.lock().unwrap().get(url).cloned().ok_or_else(|| format!("{url} is down"))
        }

        fn join(&self, url: &str) -> Arc<Network> {
            let node = NodeService::open(
                Box::new(InMemoryRepository::default()),
                ChainRules::proof_of_work(2, 50),
            )
            .unwrap();
            let network =
                Arc::new(Network::new(Arc::new(node), Box::new(self.clone()), Some(url.into())));
            self.0.lock().unwrap().insert(url.to_string(), network.clone());
            network
        }
    }

    impl PeerClient for Mesh {
        fn chain<'a>(&'a self, peer: &'a str) -> PeerResult<'a, Vec<Block>> {
            Box::pin(async move { Ok(self.get(peer)?.node().blocks()) })
        }
        fn pending<'a>(&'a self, peer: &'a str) -> PeerResult<'a, Vec<Transaction>> {
            Box::pin(async move { Ok(self.get(peer)?.node().pending()) })
        }
        fn offer_chain<'a>(&'a self, peer: &'a str, blocks: &'a [Block]) -> PeerResult<'a, ()> {
            Box::pin(async move {
                self.get(peer)?
                    .node()
                    .adopt_if_longer(blocks.to_vec())
                    .map_err(|e| e.to_string())?;
                Ok(())
            })
        }
        fn offer_transaction<'a>(
            &'a self,
            peer: &'a str,
            tx: &'a Transaction,
        ) -> PeerResult<'a, ()> {
            Box::pin(
                async move { self.get(peer)?.node().submit(tx.clone()).map_err(|e| e.to_string()) },
            )
        }
        fn introduce<'a>(&'a self, peer: &'a str, me: &'a str) -> PeerResult<'a, ()> {
            Box::pin(async move {
                self.get(peer)?.add_peer(me).await?;
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
    async fn sync_adopts_the_longest_chain_and_pending_transactions() {
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
        a.broadcast_chain().await;
        assert_eq!(b.node().blocks(), a.node().blocks());
        assert_eq!(c.node().blocks(), a.node().blocks());

        let tx = Transaction::signed(&key, miner().as_str(), 5, 0, 0);
        a.node().submit(tx.clone()).unwrap();
        a.broadcast_transaction(&tx).await;
        assert_eq!(c.node().pending(), [tx]);
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
