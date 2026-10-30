//! Floods a network of nodes with random signed transactions and shows what happens under
//! pressure: throughput, rejections, confirmation latency, fees per block, and whether the nodes
//! keep in sync.
//!
//!   NODES=http://127.0.0.1:3000,http://127.0.0.1:3001 cargo run --release --bin loadgen
//!
//! Settings, from the environment:
//!   NODES        comma-separated node URLs              (default http://127.0.0.1:3000)
//!   WALLETS      how many wallets to send from          (12)
//!   RATE         transactions per second                (20)
//!   SECS         how long to flood for                  (60)
//!   BLOCK_EVERY  seconds between blocks it adds          (5)
//!   MAX_FEE      the largest fee it offers               (5)
//!
//! Each wallet sends through one "home" node, as a real user would, so the other nodes only hear
//! about its transactions by gossip. The wallets fund themselves by mining (or, under proof of
//! stake, forging while anyone may) and then play the miners too.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

use blockchain::domain::accounting::transaction::Transaction;
use blockchain::domain::accounting::Coins;
use blockchain::domain::chain::block::Block;
use blockchain::domain::crypto::address::Address;

struct Config {
    nodes: Vec<String>,
    wallets: usize,
    rate: f64,
    secs: u64,
    block_every: f64,
    max_fee: u64,
}

impl Config {
    fn from_env() -> Self {
        let nodes = env("NODES", "http://127.0.0.1:3000".to_string());
        Self {
            nodes: nodes.split(',').map(|u| u.trim().trim_end_matches('/').to_string()).collect(),
            wallets: env("WALLETS", 12usize).max(2),
            rate: env("RATE", 20.0f64).max(0.1),
            secs: env("SECS", 60u64),
            block_every: env("BLOCK_EVERY", 5.0f64).max(0.5),
            max_fee: env("MAX_FEE", 5u64),
        }
    }
}

fn env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

struct Wallet {
    key: SigningKey,
    address: Address,
    /// The node this wallet sends through.
    home: usize,
    /// The nonce its next transaction will carry.
    nonce: u64,
}

#[derive(Default)]
struct Stats {
    sent: u64,
    accepted: u64,
    rejected: BTreeMap<&'static str, u64>,
    /// Accepted but not yet seen in a block: signature → when it was sent.
    unconfirmed: HashMap<String, Instant>,
    latencies: Vec<f64>,
    blocks: u64,
    confirmed: u64,
    fees: u64,
}

#[derive(Deserialize, Clone, PartialEq)]
struct Status {
    height: u64,
    tip: String,
    pending: usize,
}

struct Load {
    config: Config,
    http: Client,
    wallets: Mutex<Vec<Wallet>>,
    stats: Mutex<Stats>,
    proof_of_stake: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env();
    let http = Client::builder().timeout(Duration::from_secs(30)).build()?;
    let consensus: Value =
        http.get(format!("{}/consensus", config.nodes[0])).send().await?.json().await?;
    let proof_of_stake = consensus["kind"] == "proof_of_stake";

    let wallets = (0..config.wallets)
        .map(|i| {
            let key = SigningKey::generate(&mut OsRng);
            Wallet { address: Address::of(&key), key, home: i % config.nodes.len(), nonce: 0 }
        })
        .collect();
    let load = Arc::new(Load {
        config,
        http,
        wallets: Mutex::new(wallets),
        stats: Mutex::default(),
        proof_of_stake,
    });
    let c = &load.config;
    println!(
        "Flooding {} node(s) with {} tx/s from {} wallets for {}s, a block every {}s, fees up to {}.",
        c.nodes.len(),
        c.rate,
        c.wallets,
        c.secs,
        c.block_every,
        c.max_fee
    );

    println!("Funding the wallets with one block reward each…");
    for i in 0..load.config.wallets {
        load.add_block(0, Some(i)).await?;
    }
    load.wait_for_sync(Duration::from_secs(30)).await;

    let started = Instant::now();
    let producer = tokio::spawn(load.clone().keep_adding_blocks());
    let watcher = tokio::spawn(load.clone().watch(started));

    let gap = Duration::from_secs_f64(1.0 / load.config.rate);
    let mut ticker = tokio::time::interval(gap);
    while started.elapsed() < Duration::from_secs(load.config.secs) {
        ticker.tick().await;
        let (tx, home) = load.next_transaction();
        tokio::spawn(load.clone().send(tx, home));
    }

    println!("\nDone flooding. Waiting for the last transactions to confirm…");
    let drain = Instant::now();
    while drain.elapsed() < Duration::from_secs_f64(load.config.block_every * 3.0) {
        if load.stats.lock().unwrap().unconfirmed.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    producer.abort();
    watcher.abort();
    load.summary(started).await;
    Ok(())
}

impl Load {
    /// A random transfer between two of our wallets, signed now with the sender's next nonce.
    fn next_transaction(&self) -> (Transaction, usize) {
        let mut wallets = self.wallets.lock().unwrap();
        let from = rand::random::<usize>() % wallets.len();
        let to = (from + 1 + rand::random::<usize>() % (wallets.len() - 1)) % wallets.len();
        let amount = 1 + rand::random::<u64>() % 3;
        // About half pay no fee, the rest pay up to MAX_FEE.
        let fee = if rand::random::<bool>() {
            0
        } else {
            rand::random::<u64>() % (self.config.max_fee + 1)
        };
        let to = wallets[to].address.clone();
        let wallet = &mut wallets[from];
        let (amount, fee) = (Coins::new(amount), Coins::new(fee));
        let tx = Transaction::signed(&wallet.key, to, amount, wallet.nonce, fee);
        wallet.nonce += 1;
        (tx, wallet.home)
    }

    async fn send(self: Arc<Self>, tx: Transaction, home: usize) {
        self.stats.lock().unwrap().sent += 1;
        let url = format!("{}/transactions", self.config.nodes[home]);
        let outcome = match self.http.post(&url).json(&tx).send().await {
            Ok(r) if r.status().is_success() => Ok(()),
            Ok(r) => Err(classify(&r.text().await.unwrap_or_default())),
            Err(_) => Err("unreachable"),
        };
        match outcome {
            Ok(()) => {
                let mut stats = self.stats.lock().unwrap();
                stats.accepted += 1;
                let signature = tx.signature().expect("transfers are signed").to_string();
                stats.unconfirmed.insert(signature, Instant::now());
            }
            Err(reason) => {
                *self.stats.lock().unwrap().rejected.entry(reason).or_default() += 1;
                self.resync(&tx.from().to_string(), home).await;
            }
        }
    }

    /// After a rejection, picks the nonce back up from what the home node expects next.
    async fn resync(&self, address: &str, home: usize) {
        let url = format!("{}/balances/{address}", self.config.nodes[home]);
        let Ok(account) = self.get::<Value>(&url).await else { return };
        if let Some(next) = account["next_nonce"].as_u64() {
            let mut wallets = self.wallets.lock().unwrap();
            if let Some(w) = wallets.iter_mut().find(|w| w.address.as_str() == address) {
                w.nonce = next;
            }
        }
    }

    /// Plays the miners: a block every BLOCK_EVERY seconds, on a random node.
    async fn keep_adding_blocks(self: Arc<Self>) {
        let mut ticker = tokio::time::interval(Duration::from_secs_f64(self.config.block_every));
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let node = rand::random::<usize>() % self.config.nodes.len();
            let _ = self.add_block(node, None).await;
        }
    }

    /// Mines a block on `node` (or forges one under proof of stake), paying `wallet` or a random
    /// one of ours.
    async fn add_block(&self, node: usize, wallet: Option<usize>) -> Result<(), String> {
        let base = &self.config.nodes[node];
        let (address, secret) = {
            let wallets = self.wallets.lock().unwrap();
            let pick = wallet.unwrap_or_else(|| rand::random::<usize>() % wallets.len());
            let w = &wallets[pick];
            (w.address.to_string(), hex::encode(w.key.to_bytes()))
        };
        let request = if self.proof_of_stake {
            let consensus: Value = self.get(&format!("{base}/consensus")).await?;
            let secret = match consensus["next_validator"].as_str() {
                None => secret,
                Some(turn) => {
                    let wallets = self.wallets.lock().unwrap();
                    match wallets.iter().find(|w| w.address.as_str() == turn) {
                        Some(w) => hex::encode(w.key.to_bytes()),
                        None => return Err("someone else's turn to forge".into()),
                    }
                }
            };
            self.http.post(format!("{base}/forge")).json(&json!({ "secret_key": secret }))
        } else {
            self.http.post(format!("{base}/mine")).json(&json!({ "miner": address }))
        };
        let response = request.send().await.map_err(|e| e.to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(response.text().await.unwrap_or_default())
        }
    }

    /// Once a second: what each node says, which of our transactions got confirmed, one line.
    async fn watch(self: Arc<Self>, started: Instant) {
        let mut seen = self.status(0).await.map_or(0, |s| s.height + 1);
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        loop {
            ticker.tick().await;
            if let Ok(blocks) =
                self.get::<Vec<Block>>(&format!("{}/chain?from={seen}", self.config.nodes[0])).await
            {
                seen += blocks.len() as u64;
                self.confirm(&blocks);
            }
            let statuses = self.statuses().await;
            println!("{}", self.line(started, &statuses));
        }
    }

    fn confirm(&self, blocks: &[Block]) {
        let mut stats = self.stats.lock().unwrap();
        for block in blocks {
            stats.blocks += 1;
            for tx in block.transactions().iter().skip(1) {
                stats.fees += tx.fee().value();
                let signature = tx.signature().map(|s| s.as_str()).unwrap_or_default();
                if let Some(sent) = stats.unconfirmed.remove(signature) {
                    stats.confirmed += 1;
                    stats.latencies.push(sent.elapsed().as_secs_f64());
                }
            }
        }
    }

    fn line(&self, started: Instant, statuses: &[Option<Status>]) -> String {
        let stats = self.stats.lock().unwrap();
        let secs = started.elapsed().as_secs_f64().max(1.0);
        let rejected: u64 = stats.rejected.values().sum();
        let mut lat = stats.latencies.clone();
        format!(
            "{:>4.0}s  sent {:>5} ({:>5.1}/s)  ok {:>5}  rejected {:>4}  confirmed {:>5}  latency p50 {:>5} p95 {:>5}  fees/block {:>5.1}  |  {}",
            secs,
            stats.sent,
            stats.sent as f64 / secs,
            stats.accepted,
            rejected,
            stats.confirmed,
            percentile(&mut lat, 0.5),
            percentile(&mut lat, 0.95),
            stats.fees as f64 / stats.blocks.max(1) as f64,
            sync_line(statuses),
        )
    }

    async fn summary(&self, started: Instant) {
        let statuses = self.statuses().await;
        let stats = self.stats.lock().unwrap();
        let mut lat = stats.latencies.clone();
        let secs = started.elapsed().as_secs_f64();
        println!("\n──── Summary ────");
        println!(
            "  Sent            {} in {:.0}s ({:.1}/s)",
            stats.sent,
            secs,
            stats.sent as f64 / secs
        );
        println!(
            "  Accepted        {} ({:.1}%)",
            stats.accepted,
            100.0 * stats.accepted as f64 / stats.sent.max(1) as f64
        );
        for (reason, count) in &stats.rejected {
            println!("  Rejected        {count} ({reason})");
        }
        println!(
            "  Confirmed       {} ({} still waiting)",
            stats.confirmed,
            stats.unconfirmed.len()
        );
        println!(
            "  Latency         p50 {}  p95 {}  max {}",
            percentile(&mut lat, 0.5),
            percentile(&mut lat, 0.95),
            percentile(&mut lat, 1.0)
        );
        println!(
            "  Blocks          {} ({:.1} tx and {:.1} in fees each)",
            stats.blocks,
            stats.confirmed as f64 / stats.blocks.max(1) as f64,
            stats.fees as f64 / stats.blocks.max(1) as f64
        );
        println!("  Nodes           {}", sync_line(&statuses));
    }

    async fn statuses(&self) -> Vec<Option<Status>> {
        let mut out = Vec::new();
        for i in 0..self.config.nodes.len() {
            out.push(self.status(i).await.ok());
        }
        out
    }

    async fn status(&self, node: usize) -> Result<Status, String> {
        self.get(&format!("{}/status", self.config.nodes[node])).await
    }

    async fn wait_for_sync(&self, limit: Duration) {
        let start = Instant::now();
        while start.elapsed() < limit {
            let statuses = self.statuses().await;
            if statuses.iter().all(|s| s.is_some())
                && statuses
                    .windows(2)
                    .all(|w| w[0].as_ref().map(|s| &s.tip) == w[1].as_ref().map(|s| &s.tip))
            {
                println!(
                    "All {} node(s) in sync at height {}.",
                    statuses.len(),
                    statuses[0].as_ref().map_or(0, |s| s.height)
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        println!("Nodes didn't converge within {}s; starting anyway.", limit.as_secs());
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, String> {
        let response = self.http.get(url).send().await.map_err(|e| e.to_string())?;
        response.json().await.map_err(|e| e.to_string())
    }
}

/// Groups a node's error message into a short reason.
fn classify(message: &str) -> &'static str {
    if message.contains("nonce") {
        "nonce out of order"
    } else if message.contains("balance") {
        "not enough funds"
    } else if message.contains("signature") {
        "bad signature"
    } else {
        "other"
    }
}

/// "a h41 mp12 · b h41 mp9 · c h40 mp15  ✓ in sync" or "… ✗ 1 behind".
fn sync_line(statuses: &[Option<Status>]) -> String {
    let top = statuses.iter().flatten().map(|s| s.height).max().unwrap_or(0);
    let tips: Vec<_> = statuses.iter().flatten().map(|s| &s.tip).collect();
    let nodes = statuses
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let name = (b'a' + i as u8) as char;
            match s {
                Some(s) => format!("{name} h{} mp{}", s.height, s.pending),
                None => format!("{name} down"),
            }
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let behind = statuses.iter().flatten().filter(|s| s.height < top).count();
    let verdict = if statuses.iter().any(Option::is_none) {
        "✗ a node is down".to_string()
    } else if tips.windows(2).all(|w| w[0] == w[1]) {
        "✓ in sync".to_string()
    } else if behind > 0 {
        format!("… {behind} catching up")
    } else {
        "… forked, settling".to_string()
    };
    format!("{nodes}  {verdict}")
}

fn percentile(values: &mut [f64], p: f64) -> String {
    if values.is_empty() {
        return "–".to_string();
    }
    values.sort_by(f64::total_cmp);
    let index = ((values.len() - 1) as f64 * p).round() as usize;
    format!("{:.1}s", values[index])
}
