//! A node's settings, read from the environment. Each has a default, so a node starts with none
//! set; a value that doesn't parse falls back to the default too, except for `CONSENSUS`, which
//! must be `pow` or `pos`.

use std::time::Duration;

use crate::application::{FORGET_AFTER, MAX_PEERS, SNAPSHOT_BLOCKS};
use crate::domain::chain::ReorgLimit;
use crate::domain::consensus::rules::{ChainRules, DEFAULT_MAX_BLOCK_TRANSFERS};
use crate::domain::consensus::BlockCapacity;
use crate::domain::mempool::MAX_WAITING;

const DEFAULT_ADDR: &str = "127.0.0.1:3000";
const DEFAULT_CHAIN_FILE: &str = "chain.json";

/// Everything a node is started with.
pub struct Config {
    /// The rules every node on the network must share.
    pub rules: ChainRules,
    /// How far back this node will follow a fork: its own choice.
    pub reorg_limit: ReorgLimit,
    /// Blocks between snapshots. 0 takes none.
    pub snapshot_every: usize,
    /// The most transfers that may wait for a block at once.
    pub max_waiting: usize,
    /// Where the node listens.
    pub bind_addr: String,
    /// Where the chain is saved.
    pub chain_file: String,
    /// How peers should reach this node.
    pub public_url: String,
    /// The peers to join on startup.
    pub peers: Vec<String>,
    /// The most peers to seek out by discovery.
    pub max_peers: usize,
    /// How often to pull from peers.
    pub sync_every: Duration,
    /// How long a peer may go unanswered before it's forgotten.
    pub forget_after: Duration,
}

impl Config {
    /// Reads every setting from the environment, or its default.
    pub fn from_env() -> Result<Self, String> {
        let reward = env_or("REWARD", ChainRules::default().reward.value());
        let rules = match std::env::var("CONSENSUS").as_deref() {
            Ok("pos") => ChainRules::proof_of_stake(reward),
            Ok("pow") | Err(_) => ChainRules::proof_of_work(env_or("DIFFICULTY", 4), reward)
                .with_retarget(env_or("TARGET_BLOCK_SECS", 10), env_or("RETARGET_BLOCKS", 10)),
            Ok(other) => return Err(format!("CONSENSUS must be pow or pos, not {other}")),
        }
        .with_block_capacity(BlockCapacity::at_most(env_or(
            "MAX_BLOCK_TRANSFERS",
            DEFAULT_MAX_BLOCK_TRANSFERS,
        )));
        let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());
        Ok(Self {
            rules,
            reorg_limit: ReorgLimit::new(env_or("MAX_REORG_DEPTH", ReorgLimit::DEFAULT.blocks())),
            snapshot_every: env_or("SNAPSHOT_BLOCKS", SNAPSHOT_BLOCKS),
            max_waiting: env_or("MAX_WAITING", MAX_WAITING),
            chain_file: std::env::var("CHAIN_FILE")
                .unwrap_or_else(|_| DEFAULT_CHAIN_FILE.to_string()),
            public_url: std::env::var("PUBLIC_URL")
                .unwrap_or_else(|_| format!("http://{bind_addr}")),
            peers: std::env::var("PEERS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .map(str::to_string)
                .collect(),
            max_peers: env_or("MAX_PEERS", MAX_PEERS),
            sync_every: Duration::from_secs(env_or("SYNC_SECS", 5).max(1)),
            forget_after: Duration::from_secs(env_or("FORGET_SECS", FORGET_AFTER.as_secs())),
            bind_addr,
        })
    }
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
