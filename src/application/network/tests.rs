//! A whole network in one process, and what goes over the wire as nodes join, sync and gossip.

use crate::domain::accounting::{ChainId, Coins};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use super::*;
use crate::domain::accounting::transaction::Transaction;
use crate::domain::chain::block::{Block, BlockHeader};
use crate::domain::chain::Extension;
use crate::domain::chain::ReorgLimit;
use crate::domain::consensus::rules::ChainRules;
use crate::domain::crypto;
use crate::domain::crypto::address::Address;
use crate::infrastructure::ChainInMemoryRepository;

/// A whole network in one process: peer URLs map straight to other nodes' networks. Every
/// message is logged as `peer what`, so tests can see what went over the wire.
#[derive(Clone, Default)]
struct Mesh {
    nodes: Arc<Mutex<HashMap<String, Arc<Network>>>>,
    log: Arc<Mutex<Vec<String>>>,
    /// Peers that take a second to answer anything.
    slow: Arc<Mutex<HashSet<String>>>,
    /// Peers that claim far more weight than their chain has.
    boasting: Arc<Mutex<HashSet<String>>>,
    /// Peers whose headers are tampered with on the way out, so they break a rule.
    lying: Arc<Mutex<HashSet<String>>>,
}

impl Mesh {
    async fn reach(&self, url: &str, what: String) -> Result<Arc<Network>, String> {
        self.log.lock().unwrap().push(format!("{url} {what}"));
        if self.slow.lock().unwrap().contains(url) {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        self.nodes.lock().unwrap().get(url).cloned().ok_or_else(|| format!("{url} is down"))
    }

    fn join(&self, url: &str) -> Arc<Network> {
        self.join_fetching(url, SYNC_PAGE)
    }

    /// A node that fetches `page` blocks per request when catching up.
    fn join_fetching(&self, url: &str, page: usize) -> Arc<Network> {
        let node = NodeService::open(
            Box::new(ChainInMemoryRepository::default()),
            ChainRules::proof_of_work(2, 50),
            ReorgLimit::new(3),
        )
        .unwrap();
        let network = Network::new(Arc::new(node), Box::new(self.clone()), Some(url.into()));
        let network = Arc::new(network.with_sync_page(page));
        self.nodes.lock().unwrap().insert(url.to_string(), network.clone());
        network
    }

    /// The messages sent since the last call.
    fn sent(&self) -> Vec<String> {
        std::mem::take(&mut self.log.lock().unwrap())
    }
}

impl PeerClient for Mesh {
    fn status<'a>(&'a self, peer: &'a str, me: Option<&'a str>) -> PeerResult<'a, PeerTip> {
        Box::pin(async move {
            let network = self.reach(peer, "status".into()).await?;
            // As the HTTP handler does: whoever asks is added back if they're new.
            if let Some(me) = me {
                network.add_peer(me).await?;
            }
            let tip = network.queries().tip();
            let boast = if self.boasting.lock().unwrap().contains(peer) { 1_000_000 } else { 0 };
            Ok(PeerTip {
                height: tip.height,
                tip: tip.hash,
                weight: tip.weight.value() + boast,
                pending: tip.pending,
            })
        })
    }
    fn headers_from<'a>(
        &'a self,
        peer: &'a str,
        height: usize,
        limit: usize,
    ) -> PeerResult<'a, Vec<BlockHeader>> {
        Box::pin(async move {
            let network = self.reach(peer, format!("headers from {height}")).await?;
            let mut headers = network.queries().headers_between(height, limit);
            if let Some(last) =
                headers.last_mut().filter(|_| self.lying.lock().unwrap().contains(peer))
            {
                last.timestamp += chrono::TimeDelta::seconds(1);
            }
            Ok(headers)
        })
    }
    fn chain_from<'a>(
        &'a self,
        peer: &'a str,
        height: usize,
        limit: usize,
    ) -> PeerResult<'a, Vec<Block>> {
        Box::pin(async move {
            let network = self.reach(peer, format!("chain from {height}")).await?;
            Ok(network.queries().blocks_from(height).into_iter().take(limit).collect())
        })
    }
    fn pending<'a>(&'a self, peer: &'a str) -> PeerResult<'a, Vec<Transaction>> {
        Box::pin(async move { Ok(self.reach(peer, "pending".into()).await?.queries().pending()) })
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
                .reach(peer, what)
                .await?
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
            self.reach(peer, "offered a transaction".into())
                .await?
                .node()
                .submit(tx.clone())
                .map_err(|e| e.to_string())
        })
    }
    fn peers<'a>(&'a self, peer: &'a str) -> PeerResult<'a, Vec<String>> {
        Box::pin(async move {
            let network = self.reach(peer, "peers".into()).await?;
            // As the HTTP adapter does: only peers it reached the last time it tried.
            let reached = network.peers().into_iter();
            let reached = reached.filter(|(_, s)| s.last_seen.is_some() && s.error.is_none());
            Ok(reached.map(|(url, _)| url).collect())
        })
    }
    fn introduce<'a>(&'a self, peer: &'a str, me: &'a str) -> PeerResult<'a, ()> {
        Box::pin(async move {
            self.reach(peer, "introduced".into()).await?.add_peer(me).await?;
            Ok(())
        })
    }
}

/// The chain a transfer under `rules` names: the hash of their genesis block.
fn id(rules: ChainRules) -> ChainId {
    ChainId::parse(Block::genesis(&rules).hash().as_str()).unwrap()
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
    b.node()
        .submit(Transaction::signed(
            &key,
            &id(ChainRules::proof_of_work(2, 50)),
            miner(),
            Coins::new(5),
            0,
            Coins::new(1),
        ))
        .unwrap();

    let report = a.sync().await;
    assert_eq!(report.adopted_from.as_deref(), Some("http://b"));
    assert_eq!((report.reached, report.length), (1, 3));
    assert_eq!(a.queries().blocks(), b.queries().blocks());
    assert_eq!(a.queries().pending(), b.queries().pending());
    assert_eq!(a.peers()[0].1.height, Some(3));
}

#[tokio::test]
async fn an_accepted_transaction_travels_on_but_not_back() {
    let mesh = Mesh::default();
    let a = mesh.join("http://a");
    let b = mesh.join("http://b");
    let c = mesh.join("http://c");
    a.add_peer("http://b").await.unwrap();
    a.add_peer("http://c").await.unwrap();
    a.gossip_accepted();
    let (_, secret) = crypto::generate_keypair();
    let key = crypto::parse_secret_key(&secret).unwrap();
    for network in [&a, &b, &c] {
        network.node().mine(&Address::of(&key)).unwrap();
    }
    mesh.sent();

    // It came from b, so only c hears of it from a.
    let tx = Transaction::signed(
        &key,
        &id(ChainRules::proof_of_work(2, 50)),
        miner(),
        Coins::new(5),
        0,
        Coins::ZERO,
    );
    a.node().submit_from(tx.clone(), Some("http://b")).unwrap();
    for _ in 0..100 {
        if !c.queries().pending().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(c.queries().pending(), [tx]);
    assert_eq!(mesh.sent(), ["http://c offered a transaction"]);
    assert!(b.queries().pending().is_empty());
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
    assert_eq!(b.queries().blocks(), a.queries().blocks());
    assert_eq!(c.queries().blocks(), a.queries().blocks());

    let tx = Transaction::signed(
        &key,
        &id(ChainRules::proof_of_work(2, 50)),
        miner(),
        Coins::new(5),
        0,
        Coins::new(0),
    );
    a.node().submit(tx.clone()).unwrap();
    a.broadcast_transaction(&tx, None).await;
    assert_eq!(c.queries().pending(), [tx]);
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
    assert_eq!(b.queries().blocks(), a.queries().blocks());
    assert_eq!(c.queries().blocks().len(), 6);
}

#[tokio::test]
async fn a_peer_behind_gets_recent_blocks_and_pulls_the_rest() {
    let mesh = Mesh::default();
    let a = mesh.join("http://a");
    let b = mesh.join("http://b");
    a.add_peer("http://b").await.unwrap();
    a.node().mine(&miner()).unwrap();
    a.node().mine(&miner()).unwrap();
    mesh.sent();

    a.broadcast_tip(None).await;
    assert_eq!(mesh.sent(), ["http://b offered 1 block(s)", "http://b offered 2 block(s)"]);
    assert_eq!(b.queries().blocks(), a.queries().blocks());

    // Further behind than a fork may reach (3 blocks here): a push never sends the whole
    // chain, so b catches up when it next pulls.
    for _ in 0..5 {
        a.node().mine(&miner()).unwrap();
    }
    a.broadcast_tip(None).await;
    assert_eq!(mesh.sent(), ["http://b offered 1 block(s)", "http://b offered 4 block(s)"]);
    assert_eq!(b.queries().blocks().len(), 3);

    // Its headers show b already has blocks 1 and 2, so only the rest are fetched.
    b.sync().await;
    assert_eq!(
        mesh.sent(),
        ["http://a status", "http://a headers from 1", "http://a chain from 3"]
    );
    assert_eq!(b.queries().blocks(), a.queries().blocks());
}

#[tokio::test]
async fn a_node_far_behind_catches_up_a_page_at_a_time() {
    let mesh = Mesh::default();
    let a = mesh.join("http://a");
    for _ in 0..7 {
        a.node().mine(&miner()).unwrap();
    }
    let fresh = Arc::new(
        Network::new(
            Arc::new(
                NodeService::open(
                    Box::new(ChainInMemoryRepository::default()),
                    ChainRules::proof_of_work(2, 50),
                    ReorgLimit::new(3),
                )
                .unwrap(),
            ),
            Box::new(mesh.clone()),
            Some("http://fresh".into()),
        )
        .with_sync_page(3),
    );
    mesh.nodes.lock().unwrap().insert("http://fresh".into(), fresh.clone());
    fresh.add_peer("http://a").await.unwrap();
    mesh.sent();

    let report = fresh.sync().await;
    assert_eq!(report.adopted_from.as_deref(), Some("http://a"));
    assert_eq!(fresh.queries().blocks(), a.queries().blocks());
    assert_eq!(
        mesh.sent(),
        [
            "http://a status",
            "http://a headers from 1",
            "http://a chain from 1",
            "http://a chain from 4",
            "http://a chain from 7"
        ]
    );
}

#[tokio::test]
async fn a_peer_claiming_more_weight_than_its_headers_show_sends_no_blocks() {
    let mesh = Mesh::default();
    let a = mesh.join("http://a");
    let b = mesh.join("http://b");
    for _ in 0..3 {
        a.node().mine(&miner()).unwrap();
    }
    b.node().mine(&miner()).unwrap();
    a.add_peer("http://b").await.unwrap();
    mesh.boasting.lock().unwrap().insert("http://b".into());
    let before = a.queries().blocks();
    mesh.sent();

    let report = a.sync().await;
    assert_eq!(mesh.sent(), ["http://b status", "http://b headers from 1"], "no blocks fetched");
    assert_eq!((report.adopted_from, a.queries().blocks()), (None, before));
}

#[tokio::test]
async fn a_heavier_fork_longer_than_a_page_is_fetched_whole() {
    let mesh = Mesh::default();
    // A page of b's fork weighs less than a's three blocks, so offered a page at a time, a
    // would refuse the first and the next wouldn't build on anything it has.
    let a = mesh.join_fetching("http://a", 2);
    let b = mesh.join("http://b");
    for _ in 0..3 {
        a.node().mine(&miner()).unwrap();
    }
    for _ in 0..5 {
        b.node().mine(&miner()).unwrap();
    }
    a.add_peer("http://b").await.unwrap();
    mesh.sent();

    let report = a.sync().await;
    assert_eq!(report.adopted_from.as_deref(), Some("http://b"));
    assert_eq!(a.queries().blocks(), b.queries().blocks());
    assert_eq!(
        mesh.sent(),
        [
            "http://b status",
            "http://b headers from 1",
            "http://b chain from 1",
            "http://b chain from 3",
            "http://b chain from 5"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_slow_peer_holds_up_nobody() {
    let mesh = Mesh::default();
    let a = mesh.join("http://a");
    for url in ["http://b", "http://c", "http://d"] {
        mesh.join(url);
        a.add_peer(url).await.unwrap();
        mesh.slow.lock().unwrap().insert(url.to_string());
    }
    a.node().mine(&miner()).unwrap();

    // Each peer takes a second; one after another would take three.
    let started = tokio::time::Instant::now();
    a.broadcast_tip(None).await;
    assert_eq!(started.elapsed(), Duration::from_secs(1));
    let tx = Transaction::signed(
        &crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap(),
        &id(ChainRules::proof_of_work(2, 50)),
        miner(),
        Coins::new(1),
        0,
        Coins::ZERO,
    );
    let started = tokio::time::Instant::now();
    a.broadcast_transaction(&tx, None).await;
    a.sync().await;
    assert_eq!(started.elapsed(), Duration::from_secs(2));
}

#[tokio::test]
async fn sync_only_checks_the_status_of_a_peer_in_step() {
    let mesh = Mesh::default();
    let a = mesh.join("http://a");
    let b = mesh.join("http://b");
    a.add_peer("http://b").await.unwrap();
    b.node().mine(&miner()).unwrap();
    a.sync().await;
    assert_eq!(a.queries().blocks(), b.queries().blocks());
    mesh.sent();

    let report = a.sync().await;
    assert_eq!(mesh.sent(), ["http://b status"]);
    assert_eq!((report.reached, report.adopted_from), (1, None));

    a.node().mine(&miner()).unwrap();
    a.sync().await;
    assert_eq!(mesh.sent(), ["http://b status"], "a lighter peer has nothing for us");
}

#[tokio::test]
async fn a_link_that_went_one_way_heals_on_the_next_sync() {
    let mesh = Mesh::default();
    let a = mesh.join("http://a");
    let c = mesh.join("http://c");
    // c starts before b is up, so b never hears c's introduction.
    c.add_peer("http://b").await.unwrap();
    let b = mesh.join("http://b");
    a.add_peer("http://b").await.unwrap();
    assert_eq!(b.peers().len(), 1, "b only knows a");

    c.sync().await;
    let mut known: Vec<_> = b.peers().into_iter().map(|(url, _)| url).collect();
    known.sort();
    assert_eq!(known, ["http://a", "http://c"]);

    // b restarts and forgets both; each comes back as soon as it syncs.
    let b = mesh.join("http://b");
    a.sync().await;
    c.sync().await;
    assert_eq!(b.peers().len(), 2);
}

#[tokio::test]
async fn adding_a_known_peer_keeps_what_we_heard_from_it() {
    let mesh = Mesh::default();
    let a = mesh.join("http://a");
    let b = mesh.join("http://b");
    a.add_peer("http://b").await.unwrap();
    b.node().mine(&miner()).unwrap();
    a.sync().await;

    assert_eq!(a.add_peer("http://b").await, Ok(false));
    assert_eq!(a.peers()[0].1.height, Some(2));
    assert!(a.peers()[0].1.last_seen.is_some());
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

#[tokio::test]
async fn a_node_meets_the_peers_of_its_peers() {
    let mesh = Mesh::default();
    // A line: c knows b, b knows a. a and c are linked only through b.
    let a = mesh.join("http://a");
    let b = mesh.join("http://b");
    let c = mesh.join("http://c");
    b.add_peer("http://a").await.unwrap();
    c.add_peer("http://b").await.unwrap();
    b.sync().await;
    mesh.sent();

    assert_eq!(c.discover().await, 1);
    // c introduces itself to a, and a, meeting c for the first time, introduces itself back.
    assert_eq!(mesh.sent(), ["http://b peers", "http://a introduced", "http://c introduced"]);
    let known = |n: &Network| n.peers().into_iter().map(|(url, _)| url).collect::<Vec<_>>();
    assert_eq!(known(&c), ["http://a", "http://b"]);
    assert_eq!(known(&a), ["http://b", "http://c"], "introduced both ways");

    assert_eq!(c.discover().await, 0, "nobody new");
}

#[tokio::test]
async fn only_peers_that_answered_are_passed_on() {
    let mesh = Mesh::default();
    let a = mesh.join("http://a");
    let b = mesh.join("http://b");
    b.add_peer("http://gone").await.unwrap();
    b.add_peer("http://new").await.unwrap();
    b.sync().await;
    // b added http://new after its last sync, so it hasn't heard from it yet.
    mesh.join("http://late");
    b.add_peer("http://late").await.unwrap();
    a.add_peer("http://b").await.unwrap();

    assert_eq!(a.discover().await, 0);
    assert_eq!(a.peers().len(), 1, "neither the unreachable nor the unheard-from peer");
}

#[tokio::test]
async fn discovery_stops_at_max_peers() {
    let mesh = Mesh::default();
    let hub = mesh.join("http://hub");
    for url in ["http://b", "http://c", "http://d", "http://e"] {
        mesh.join(url);
        hub.add_peer(url).await.unwrap();
    }
    hub.sync().await;
    let a = Arc::new(
        Network::new(
            Arc::new(
                NodeService::open(
                    Box::new(ChainInMemoryRepository::default()),
                    ChainRules::proof_of_work(2, 50),
                    ReorgLimit::new(3),
                )
                .unwrap(),
            ),
            Box::new(mesh.clone()),
            Some("http://a".into()),
        )
        .with_max_peers(3),
    );
    mesh.nodes.lock().unwrap().insert("http://a".into(), a.clone());
    a.add_peer("http://hub").await.unwrap();

    assert_eq!(a.discover().await, 2);
    assert_eq!(a.peers().len(), 3);
}

#[tokio::test]
async fn a_peer_unreachable_for_long_enough_is_forgotten_unless_it_was_joined() {
    let mesh = Mesh::default();
    let a = Arc::new(
        Network::new(
            Arc::new(
                NodeService::open(
                    Box::new(ChainInMemoryRepository::default()),
                    ChainRules::proof_of_work(2, 50),
                    ReorgLimit::new(3),
                )
                .unwrap(),
            ),
            Box::new(mesh.clone()),
            Some("http://a".into()),
        )
        .with_forget_after(Duration::ZERO),
    );
    mesh.join("http://up");
    a.add_peer("http://up").await.unwrap();
    a.add_peer("http://gone").await.unwrap();
    a.join("http://seed").await.unwrap();

    a.sync().await;
    let known: Vec<_> = a.peers().into_iter().map(|(url, _)| url).collect();
    assert_eq!(known, ["http://seed", "http://up"]);
    assert!(a.peers()[0].1.failing_since.is_some());
    assert_eq!(a.peers()[1].1.failing_since, None);
}

#[tokio::test]
async fn a_peer_down_for_a_moment_is_kept() {
    let mesh = Mesh::default();
    let a = mesh.join("http://a");
    a.add_peer("http://b").await.unwrap();

    a.sync().await;
    let since = a.peers()[0].1.failing_since;
    assert!(since.is_some());
    a.sync().await;
    assert_eq!(a.peers()[0].1.failing_since, since, "still the same outage");

    mesh.join("http://b");
    a.sync().await;
    let back = &a.peers()[0].1;
    assert_eq!((back.failing_since, back.error.as_deref()), (None, None), "answering ends it");
}

/// `a`, banning for `ban_for`, with `liar` as a peer: a heavier chain whose headers break a rule.
async fn facing_a_liar(mesh: &Mesh, ban_for: Duration) -> Arc<Network> {
    let a = Arc::new(
        Network::new(
            Arc::new(
                NodeService::open(
                    Box::new(ChainInMemoryRepository::default()),
                    ChainRules::proof_of_work(2, 50),
                    ReorgLimit::new(3),
                )
                .unwrap(),
            ),
            Box::new(mesh.clone()),
            Some("http://a".into()),
        )
        .with_ban_for(ban_for),
    );
    mesh.nodes.lock().unwrap().insert("http://a".into(), a.clone());
    let liar = mesh.join("http://liar");
    liar.node().mine(&miner()).unwrap();
    mesh.lying.lock().unwrap().insert("http://liar".into());
    a.add_peer("http://liar").await.unwrap();
    a
}

#[tokio::test]
async fn a_peer_that_keeps_sending_invalid_chains_is_banned_and_left_alone() {
    let mesh = Mesh::default();
    let a = facing_a_liar(&mesh, BAN_FOR).await;

    for strikes in 1..BAN_AFTER {
        a.sync().await;
        let status = &a.peers()[0].1;
        assert_eq!((status.strikes, status.banned_until), (strikes, None));
        assert!(status.error.as_deref().unwrap().contains("headers"), "{status:?}");
    }
    a.sync().await;
    let status = &a.peers()[0].1;
    assert!(status.is_banned(chrono::Utc::now()), "{status:?}");
    assert!(a.is_banned("http://liar/"));
    assert_eq!(a.queries().tip().height, 0, "none of its blocks were taken");

    mesh.sent();
    a.sync().await;
    a.discover().await;
    a.broadcast_tip(None).await;
    assert_eq!(mesh.sent(), Vec::<String>::new(), "a banned peer isn't pulled, asked or told");
}

#[tokio::test]
async fn a_ban_runs_out_and_an_honest_answer_clears_the_strikes() {
    let mesh = Mesh::default();
    let a = facing_a_liar(&mesh, Duration::ZERO).await;
    for _ in 0..BAN_AFTER {
        a.sync().await;
    }
    assert!(a.peers()[0].1.banned_until.is_some());
    assert!(!a.is_banned("http://liar"), "a ban of no time is over at once");

    a.sync().await;
    assert_eq!(a.peers()[0].1.strikes, 1, "back, and striking again");
    mesh.lying.lock().unwrap().clear();
    a.sync().await;
    let status = &a.peers()[0].1;
    assert_eq!((status.strikes, status.error.as_deref()), (0, None));
    assert_eq!(a.queries().tip().height, 1, "its honest chain is adopted");
}
