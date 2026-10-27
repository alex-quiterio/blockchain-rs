use std::collections::HashMap;

use super::block::Transaction;
use super::chain::ChainError;
use super::crypto;

/// Balances and nonces that result from applying transactions in order.
#[derive(Debug, Clone, Default)]
pub struct Ledger {
    balances: HashMap<String, u64>,
    nonces: HashMap<String, u64>,
}

impl Ledger {
    pub fn balance(&self, address: &str) -> u64 {
        self.balances.get(address).copied().unwrap_or(0)
    }

    pub fn next_nonce(&self, address: &str) -> u64 {
        self.nonces.get(address).copied().unwrap_or(0)
    }

    pub(super) fn credit(&mut self, address: &str, amount: u64) {
        *self.balances.entry(address.to_string()).or_insert(0) += amount;
    }

    /// Checks a signed transfer against this state and applies it.
    pub fn apply_transfer(&mut self, tx: &Transaction) -> Result<(), ChainError> {
        if tx.is_coinbase() {
            return Err(ChainError::UnexpectedCoinbase);
        }
        if !crypto::is_valid_address(&tx.to) {
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
        let balance = self.balance(&tx.from);
        if balance < tx.amount {
            return Err(ChainError::InsufficientFunds { balance, amount: tx.amount });
        }
        self.balances.insert(tx.from.clone(), balance - tx.amount);
        self.credit(&tx.to, tx.amount);
        self.nonces.insert(tx.from.clone(), expected + 1);
        Ok(())
    }
}
