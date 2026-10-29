use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::block::{Transaction, TxKind};
use super::crypto;
use super::error::ChainError;
use super::randao;

/// Balances, stakes, nonces and validator randomness that result from applying blocks in order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ledger {
    balances: HashMap<String, u64>,
    stakes: HashMap<String, u64>,
    nonces: HashMap<String, u64>,
    /// Each validator's reveal chain: the link they showed last and how many are left.
    commitments: HashMap<String, Commitment>,
    /// How many commitments each address has ever made, which numbers the next one.
    epochs: HashMap<String, u64>,
    /// The randomness that picks the next validator, mixed with every reveal.
    beacon: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Commitment {
    head: String,
    remaining: u32,
}

/// Where an address stands with its reveal chain, so a wallet can compute what to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatorState {
    /// The epoch of the commitment it would make next. The active one, if any, is one less.
    pub next_epoch: u64,
    /// Reveals left on the active commitment. `None` if there isn't one.
    pub reveals_left: Option<u32>,
}

impl Ledger {
    /// An empty ledger whose beacon starts at the genesis hash.
    pub fn starting_at(genesis_hash: &str) -> Self {
        Self { beacon: genesis_hash.to_string(), ..Self::default() }
    }

    pub fn beacon(&self) -> &str {
        &self.beacon
    }

    /// Stakes that can be picked to forge: those whose reveal chain isn't used up, by address.
    pub fn validator_stakes(&self) -> Vec<(String, u64)> {
        self.stakes()
            .into_iter()
            .filter(|(address, _)| self.commitments.get(address).is_some_and(|c| c.remaining > 0))
            .collect()
    }

    pub fn validator_state(&self, address: &str) -> ValidatorState {
        ValidatorState {
            next_epoch: self.epochs.get(address).copied().unwrap_or(0),
            reveals_left: self.active_commitment(address).map(|c| c.remaining),
        }
    }

    fn active_commitment(&self, address: &str) -> Option<&Commitment> {
        self.commitments.get(address).filter(|c| c.remaining > 0)
    }

    /// Checks that `reveal` opens `validator`'s commitment, then moves their chain along by one
    /// and mixes the reveal into the beacon.
    pub(super) fn apply_reveal(&mut self, validator: &str, reveal: &str) -> bool {
        let Some(commitment) = self.commitments.get_mut(validator) else {
            return false;
        };
        if commitment.remaining == 0 || !randao::opens(reveal, &commitment.head) {
            return false;
        }
        commitment.head = reveal.to_string();
        commitment.remaining -= 1;
        self.beacon = randao::mix(&self.beacon, reveal);
        true
    }

    pub fn balance(&self, address: &str) -> u64 {
        self.balances.get(address).copied().unwrap_or(0)
    }

    pub fn staked(&self, address: &str) -> u64 {
        self.stakes.get(address).copied().unwrap_or(0)
    }

    /// Every non-zero stake, sorted by address so validator selection is the same everywhere.
    pub fn stakes(&self) -> Vec<(String, u64)> {
        let mut stakes: Vec<_> =
            self.stakes.iter().filter(|(_, &n)| n > 0).map(|(a, &n)| (a.clone(), n)).collect();
        stakes.sort();
        stakes
    }

    pub fn next_nonce(&self, address: &str) -> u64 {
        self.nonces.get(address).copied().unwrap_or(0)
    }

    pub(super) fn credit(&mut self, address: &str, amount: u64) {
        *self.balances.entry(address.to_string()).or_insert(0) += amount;
    }

    /// Checks a signed transfer, stake or unstake against this state and applies it.
    pub fn apply_transfer(&mut self, tx: &Transaction) -> Result<(), ChainError> {
        let kind = tx.kind();
        if kind == TxKind::Coinbase {
            return Err(ChainError::UnexpectedCoinbase);
        }
        if kind == TxKind::Transfer && !crypto::is_valid_address(&tx.to) {
            return Err(ChainError::InvalidAddress(tx.to.clone()));
        }
        if tx.amount == 0 {
            return Err(ChainError::ZeroAmount);
        }
        // The nonce is checked before the signature: stale and replayed transactions, the
        // common case when peers gossip, are turned away without the costly verification.
        let expected = self.next_nonce(&tx.from);
        if tx.nonce != expected {
            return Err(ChainError::BadNonce { expected, got: tx.nonce });
        }
        if !crypto::verify(&tx.from, &tx.payload(), &tx.signature) {
            return Err(ChainError::BadSignature);
        }
        // A first stake must commit to a reveal chain; any other transaction must not.
        let needs_commitment = kind == TxKind::Stake && self.active_commitment(&tx.from).is_none();
        match (&tx.commitment, needs_commitment) {
            (None, true) => return Err(ChainError::MissingCommitment),
            (Some(_), false) => return Err(ChainError::UnexpectedCommitment),
            (Some(c), true) if !is_sha256_hex(c) => return Err(ChainError::MissingCommitment),
            _ => {}
        }
        // The fee always comes out of the balance. So does the amount, unless it is unstaked.
        let spent = if kind == TxKind::Unstake { tx.fee } else { tx.amount.saturating_add(tx.fee) };
        let balance = self.balance(&tx.from);
        if balance < spent {
            return Err(ChainError::InsufficientFunds { balance, amount: spent });
        }
        if kind == TxKind::Unstake {
            let staked = self.staked(&tx.from);
            if staked < tx.amount {
                return Err(ChainError::InsufficientStake { staked, amount: tx.amount });
            }
            self.stakes.insert(tx.from.clone(), staked - tx.amount);
            if staked == tx.amount {
                // Fully unstaked: the reveal chain ends, and staking again needs a new one.
                self.commitments.remove(&tx.from);
            }
        }
        if let Some(commitment) = &tx.commitment {
            let head = commitment.clone();
            self.commitments
                .insert(tx.from.clone(), Commitment { head, remaining: randao::LENGTH });
            *self.epochs.entry(tx.from.clone()).or_insert(0) += 1;
        }
        self.balances.insert(tx.from.clone(), balance - spent);
        match kind {
            TxKind::Stake => *self.stakes.entry(tx.from.clone()).or_insert(0) += tx.amount,
            TxKind::Unstake => self.credit(&tx.from, tx.amount),
            _ => self.credit(&tx.to, tx.amount),
        }
        self.nonces.insert(tx.from.clone(), expected + 1);
        Ok(())
    }
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
