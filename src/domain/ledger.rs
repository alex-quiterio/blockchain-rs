use std::collections::HashMap;

use super::block::{Transaction, TxKind};
use super::crypto;
use super::error::ChainError;

/// Balances, stakes and nonces that result from applying transactions in order.
#[derive(Debug, Clone, Default)]
pub struct Ledger {
    balances: HashMap<String, u64>,
    stakes: HashMap<String, u64>,
    nonces: HashMap<String, u64>,
}

impl Ledger {
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
        if !crypto::verify(&tx.from, &tx.payload(), &tx.signature) {
            return Err(ChainError::BadSignature);
        }
        let expected = self.next_nonce(&tx.from);
        if tx.nonce != expected {
            return Err(ChainError::BadNonce { expected, got: tx.nonce });
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
