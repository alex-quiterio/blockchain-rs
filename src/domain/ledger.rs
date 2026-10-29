use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::address::Address;
use super::block::{Transaction, TxKind};
use super::error::TransactionError;
use super::randao;
use super::values::{Beacon, BlockHash, Coins, Reveal};

/// Balances, stakes, nonces and validator randomness that result from applying blocks in order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ledger {
    balances: HashMap<Address, Coins>,
    stakes: HashMap<Address, Coins>,
    nonces: HashMap<Address, u64>,
    /// Each validator's reveal chain: the link they showed last and how many are left.
    commitments: HashMap<Address, RevealChain>,
    /// How many commitments each address has ever made, which numbers the next one.
    epochs: HashMap<Address, u64>,
    /// The randomness that picks the next validator, mixed with every reveal.
    beacon: Beacon,
}

/// A validator's reveal chain as far as it's been shown: the last link, and how many are left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RevealChain {
    head: Reveal,
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
    pub fn starting_at(genesis_hash: &BlockHash) -> Self {
        let beacon = Beacon::parse(genesis_hash.as_str()).expect("both are SHA-256 hex");
        Self { beacon, ..Self::default() }
    }

    pub fn beacon(&self) -> &Beacon {
        &self.beacon
    }

    /// Stakes that can be picked to forge: those whose reveal chain isn't used up, by address.
    pub fn validator_stakes(&self) -> Vec<(Address, Coins)> {
        self.stakes()
            .into_iter()
            .filter(|(address, _)| self.commitments.get(address).is_some_and(|c| c.remaining > 0))
            .collect()
    }

    pub fn validator_state(&self, address: &Address) -> ValidatorState {
        ValidatorState {
            next_epoch: self.epochs.get(address).copied().unwrap_or(0),
            reveals_left: self.active_commitment(address).map(|c| c.remaining),
        }
    }

    fn active_commitment(&self, address: &Address) -> Option<&RevealChain> {
        self.commitments.get(address).filter(|c| c.remaining > 0)
    }

    /// Checks that `reveal` opens `validator`'s commitment, then moves their chain along by one
    /// and mixes the reveal into the beacon.
    pub(super) fn apply_reveal(&mut self, validator: &Address, reveal: &Reveal) -> bool {
        let Some(chain) = self.commitments.get_mut(validator) else {
            return false;
        };
        if chain.remaining == 0 || !randao::opens(reveal, &chain.head) {
            return false;
        }
        chain.head = reveal.clone();
        chain.remaining -= 1;
        self.beacon = randao::mix(&self.beacon, reveal);
        true
    }

    pub fn balance(&self, address: &Address) -> Coins {
        self.balances.get(address).copied().unwrap_or_default()
    }

    pub fn staked(&self, address: &Address) -> Coins {
        self.stakes.get(address).copied().unwrap_or_default()
    }

    /// Every non-zero stake, sorted by address so validator selection is the same everywhere.
    pub fn stakes(&self) -> Vec<(Address, Coins)> {
        let mut stakes: Vec<_> = self
            .stakes
            .iter()
            .filter(|(_, n)| !n.is_zero())
            .map(|(a, &n)| (a.clone(), n))
            .collect();
        stakes.sort();
        stakes
    }

    pub fn next_nonce(&self, address: &Address) -> u64 {
        self.nonces.get(address).copied().unwrap_or(0)
    }

    pub(super) fn credit(&mut self, address: &Address, amount: Coins) {
        let balance = self.balances.entry(address.clone()).or_default();
        *balance = balance.saturating_add(amount);
    }

    /// Checks a signed transfer, stake or unstake against this state and applies it.
    pub fn apply_transfer(&mut self, tx: &Transaction) -> Result<(), TransactionError> {
        let kind = tx.kind();
        // A recipient account is a valid address by its type; only the sender needs unpacking.
        let Some(sender) = tx.sender() else {
            return Err(TransactionError::UnexpectedCoinbase);
        };
        let from = sender;
        if tx.amount.is_zero() {
            return Err(TransactionError::ZeroAmount);
        }
        // The nonce is checked before the signature: stale and replayed transactions, the
        // common case when peers gossip, are turned away without the costly verification.
        let expected = self.next_nonce(from);
        if tx.nonce != expected {
            return Err(TransactionError::BadNonce { expected, got: tx.nonce });
        }
        if !tx.signature.as_ref().is_some_and(|s| s.is_by(sender, &tx.payload())) {
            return Err(TransactionError::BadSignature);
        }
        // A first stake must commit to a reveal chain; any other transaction must not.
        let needs_commitment = kind == TxKind::Stake && self.active_commitment(from).is_none();
        match (&tx.commitment, needs_commitment) {
            (None, true) => return Err(TransactionError::MissingCommitment),
            (Some(_), false) => return Err(TransactionError::UnexpectedCommitment),
            _ => {}
        }
        // The fee always comes out of the balance. So does the amount, unless it is unstaked.
        let spent = match kind {
            TxKind::Unstake => Some(tx.fee),
            _ => tx.amount.checked_add(tx.fee),
        };
        let balance = self.balance(from);
        let Some(left) = spent.and_then(|spent| balance.checked_sub(spent)) else {
            let amount = spent.unwrap_or(Coins::new(u64::MAX));
            return Err(TransactionError::InsufficientFunds { balance, amount });
        };
        if kind == TxKind::Unstake {
            let staked = self.staked(from);
            if staked < tx.amount {
                return Err(TransactionError::InsufficientStake { staked, amount: tx.amount });
            }
            let left = staked.checked_sub(tx.amount).expect("checked just above");
            self.stakes.insert(from.clone(), left);
            if staked == tx.amount {
                // Fully unstaked: the reveal chain ends, and staking again needs a new one.
                self.commitments.remove(from);
            }
        }
        if let Some(commitment) = &tx.commitment {
            let head = randao::first_link(commitment);
            self.commitments.insert(from.clone(), RevealChain { head, remaining: randao::LENGTH });
            *self.epochs.entry(from.clone()).or_insert(0) += 1;
        }
        self.balances.insert(from.clone(), left);
        match kind {
            TxKind::Stake => {
                let stake = self.stakes.entry(from.clone()).or_default();
                *stake = stake.saturating_add(tx.amount);
            }
            TxKind::Unstake => self.credit(from, tx.amount),
            _ => {
                let to = tx.to.account().expect("a transfer goes to an account");
                self.credit(to, tx.amount);
            }
        }
        self.nonces.insert(from.clone(), expected + 1);
        Ok(())
    }
}
