//! Questions about an address: what it holds and stakes, and what its next transaction needs.

use super::ChainQueries;
use crate::domain::address::Address;
use crate::domain::validators::ValidatorState;
use crate::domain::values::Coins;

pub struct Account {
    pub balance: Coins,
    pub staked: Coins,
    pub next_nonce: u64,
    pub validator: ValidatorState,
    /// If the next stake must commit, the epoch to commit for.
    pub commitment_due: Option<u64>,
}

impl ChainQueries {
    pub fn account(&self, address: &Address) -> Account {
        let chain = self.chain.lock().unwrap();
        let mempool = self.mempool.lock().unwrap();
        Account {
            balance: chain.balance(address),
            staked: chain.staked(address),
            next_nonce: mempool.next_nonce(address),
            validator: chain.validator_state(address),
            commitment_due: mempool.commitment_due(address),
        }
    }

    pub fn next_nonce(&self, address: &Address) -> u64 {
        self.mempool.lock().unwrap().next_nonce(address)
    }
}
