//! A domain service: Merkle trees over a block's transactions, after Bitcoin's. It lives with the
//! block because that's what it proves things about. It holds no
//! state; what it makes are values, a `MerkleRoot`, `MerkleNode`s and the `Step`s of a proof.
//!
//! Each transaction is hashed into a leaf, and neighbouring hashes are hashed together in pairs,
//! level by level, until one is left: the root. A block's hash covers the root instead of the
//! transactions themselves, so anyone holding just the block's header can check that a
//! transaction is in it from a proof: the sibling hash at each level, about log₂(n) of them.
//!
//! Two details close known holes. Leaves and inner nodes are hashed with different prefixes, so
//! an inner node can never pass for a transaction. And an odd hash out moves up a level as it is
//! instead of being paired with itself, so `[a, b, c]` and `[a, b, c, c]` don't share a root, as
//! they do in Bitcoin (CVE-2012-2459).

use crate::domain::transaction::Transaction;
use crate::domain::values::{MerkleNode, MerkleRoot, Side, Step};

/// A transaction's leaf: the hash of its JSON, as blocks store it.
pub fn leaf(tx: &Transaction) -> MerkleNode {
    let json = serde_json::to_string(tx).expect("transactions serialize");
    MerkleNode::of(format!("leaf|{json}").as_bytes())
}

pub fn root(transactions: &[Transaction]) -> MerkleRoot {
    let mut level: Vec<MerkleNode> = transactions.iter().map(leaf).collect();
    if level.is_empty() {
        // Only genesis has no transactions.
        return MerkleRoot::zero();
    }
    while level.len() > 1 {
        level = parent_level(&level);
    }
    MerkleRoot::parse(level[0].as_str()).expect("both are SHA-256 hex")
}

/// The sibling hashes that lead from the transaction at `index` up to the root.
pub fn proof(transactions: &[Transaction], mut index: usize) -> Option<Vec<Step>> {
    if index >= transactions.len() {
        return None;
    }
    let mut level: Vec<MerkleNode> = transactions.iter().map(leaf).collect();
    let mut steps = Vec::new();
    while level.len() > 1 {
        let sibling = index ^ 1;
        if let Some(hash) = level.get(sibling) {
            let side = if sibling < index { Side::Left } else { Side::Right };
            steps.push(Step { hash: hash.clone(), side });
        }
        level = parent_level(&level);
        index /= 2;
    }
    Some(steps)
}

/// Whether `steps` lead from `leaf` to `root`.
pub fn verify(leaf: &MerkleNode, steps: &[Step], root: &MerkleRoot) -> bool {
    let top = steps.iter().fold(leaf.clone(), |hash, step| match step.side {
        Side::Left => node(&step.hash, &hash),
        Side::Right => node(&hash, &step.hash),
    });
    top.as_str() == root.as_str()
}

fn parent_level(level: &[MerkleNode]) -> Vec<MerkleNode> {
    level
        .chunks(2)
        .map(|pair| match pair {
            [left, right] => node(left, right),
            [odd] => odd.clone(),
            _ => unreachable!("chunks of two"),
        })
        .collect()
}

fn node(left: &MerkleNode, right: &MerkleNode) -> MerkleNode {
    MerkleNode::of(format!("node|{left}|{right}").as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::address::Address;
    use crate::domain::crypto;
    use crate::domain::values::Coins;

    fn txs(n: u64) -> Vec<Transaction> {
        let to = Address::parse(&crypto::generate_keypair().0).unwrap();
        (0..n).map(|i| Transaction::coinbase(&to, Coins::new(50 + i), i)).collect()
    }

    #[test]
    fn every_transaction_proves_against_the_root() {
        for n in 1..=9 {
            let txs = txs(n);
            let root = root(&txs);
            for (i, tx) in txs.iter().enumerate() {
                let steps = proof(&txs, i).unwrap();
                assert!(verify(&leaf(tx), &steps, &root), "{i} of {n}");
                assert!(steps.len() <= (n as f64).log2().ceil() as usize);
            }
            assert_eq!(proof(&txs, n as usize), None);
        }
    }

    #[test]
    fn a_proof_fails_for_another_transaction_or_root() {
        let txs = txs(5);
        let steps = proof(&txs, 2).unwrap();
        assert!(!verify(&leaf(&txs[3]), &steps, &root(&txs)));
        assert!(!verify(&leaf(&txs[2]), &steps, &root(&txs[..4])));
        let mut flipped = steps.clone();
        flipped[0].side = Side::Left;
        assert!(!verify(&leaf(&txs[2]), &flipped, &root(&txs)));
    }

    #[test]
    fn any_change_to_the_transactions_changes_the_root() {
        let txs = txs(3);
        let original = root(&txs);
        let mut changed = txs.clone();
        changed[1].amount = changed[1].amount.checked_add(Coins::new(1)).unwrap();
        assert_ne!(root(&changed), original);
        assert_ne!(root(&[txs[1].clone(), txs[0].clone(), txs[2].clone()]), original);
        let mut padded = txs.clone();
        padded.push(txs[2].clone());
        assert_ne!(root(&padded), original, "repeating the odd one out is not free");
        assert_eq!(root(&[]), MerkleRoot::zero());
    }

    #[test]
    fn a_single_transaction_is_its_own_root() {
        let txs = txs(1);
        assert_eq!(root(&txs).as_str(), leaf(&txs[0]).as_str());
        assert_eq!(proof(&txs, 0), Some(vec![]));
    }
}
