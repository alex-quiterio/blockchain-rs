//! The port to other nodes: what this node asks of a peer and offers it. The HTTP adapter that
//! implements it lives in infrastructure.

use std::future::Future;
use std::pin::Pin;

use crate::domain::block::{Block, Transaction};
use crate::domain::chain::Extension;
use crate::domain::values::BlockHash;

pub type PeerResult<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// How this node talks to other nodes. The HTTP adapter lives in infrastructure.
pub trait PeerClient: Send + Sync {
    /// Where the peer's chain stands. `me` tells the peer who's asking, so it can add us back if
    /// it forgot us.
    fn status<'a>(&'a self, peer: &'a str, me: Option<&'a str>) -> PeerResult<'a, PeerTip>;
    /// Up to `limit` of the peer's blocks, from `height` on.
    fn chain_from<'a>(
        &'a self,
        peer: &'a str,
        height: usize,
        limit: usize,
    ) -> PeerResult<'a, Vec<Block>>;
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
    pub tip: BlockHash,
    pub weight: u128,
    pub pending: usize,
}
