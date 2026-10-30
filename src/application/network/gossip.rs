//! Pushing: telling peers about a new block or an accepted transaction as soon as there is one.

use std::sync::Arc;

use futures_util::future::join_all;
use tokio::runtime::Handle;

use super::Network;
use crate::application::NodeEvent;
use crate::domain::block::Transaction;
use crate::domain::chain::Extension;
use crate::domain::mempool::MempoolEvent;

impl Network {
    /// Tells every peer but `except` about this node's newest block, after it mined, forged or
    /// adopted one, all at once. Only the tip goes out. A peer that lacks its parent gets the
    /// blocks a fork could reach back to. A push never sends more: a peer further behind than
    /// that catches up by pulling, which isn't held to a request's size limit.
    pub async fn broadcast_tip(&self, except: Option<&str>) {
        let length = self.queries.tip().height as usize + 1;
        join_all(self.peers_except(except).iter().map(|url| async move {
            let tip = length - 1;
            let recent = self.recent_start(length);
            for from in [tip, recent] {
                let blocks = self.queries.blocks_from(from);
                let offered = self.client.offer_blocks(url, &blocks, self.me.as_deref()).await;
                if from == recent || !matches!(offered, Ok(Extension::MissingParent)) {
                    break;
                }
            }
        }))
        .await;
    }

    /// Passes on every transaction the node accepts, to every peer but the one it came from, by
    /// listening to its mempool. A transaction travels on however it arrived: from a client,
    /// pushed by a peer, or pulled in a sync. A peer that already has it refuses it at once, so
    /// gossip never loops.
    pub fn gossip_accepted(self: &Arc<Self>) {
        let network = Arc::downgrade(self);
        self.node.subscribe(move |event| {
            let NodeEvent::Mempool { event: MempoolEvent::Accepted(tx), from } = event else {
                return;
            };
            let (Some(network), Ok(runtime)) = (network.upgrade(), Handle::try_current()) else {
                return;
            };
            let (tx, from) = (tx.clone(), from.clone());
            runtime.spawn(async move { network.broadcast_transaction(&tx, from.as_deref()).await });
        });
    }

    /// Passes a transaction this node accepted on to every peer but the one it came from, all
    /// at once.
    pub async fn broadcast_transaction(&self, tx: &Transaction, except: Option<&str>) {
        let me = self.me.as_deref();
        let peers = self.peers_except(except);
        join_all(peers.iter().map(|url| self.client.offer_transaction(url, tx, me))).await;
    }
}
