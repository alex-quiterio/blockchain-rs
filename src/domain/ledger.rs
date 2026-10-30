use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::address::Address;
use super::block::{Transaction, TxKind};
use super::error::TransactionError;
use super::validators::{ValidatorState, Validators};
use super::values::{Beacon, BlockHash, Coins, Reveal};

/// Balances, stakes and nonces that result from applying blocks in order, and, for proof of
/// stake, the `Validators` those stakes may be drawn from.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ledger {
    balances: HashMap<Address, Coins>,
    stakes: HashMap<Address, Coins>,
    nonces: HashMap<Address, u64>,
    /// Flattened, so a snapshot reads the same as when these were the ledger's own fields.
    #[serde(flatten)]
    validators: Validators,
}

impl Ledger {
    /// An empty ledger whose beacon starts at the genesis hash.
    pub fn starting_at(genesis_hash: &BlockHash) -> Self {
        let beacon = Beacon::parse(genesis_hash.as_str()).expect("both are SHA-256 hex");
        Self { validators: Validators::starting_at(beacon), ..Self::default() }
    }

    pub fn beacon(&self) -> &Beacon {
        self.validators.beacon()
    }

    /// Stakes that can be picked to forge: those whose reveal chain isn't used up, by address.
    pub fn validator_stakes(&self) -> Vec<(Address, Coins)> {
        self.validators.eligible(self.stakes())
    }

    pub fn validator_state(&self, address: &Address) -> ValidatorState {
        self.validators.state(address)
    }

    /// Checks that `reveal` opens `validator`'s commitment, then moves their chain along by one
    /// and mixes the reveal into the beacon.
    pub(super) fn apply_reveal(&mut self, validator: &Address, reveal: &Reveal) -> bool {
        self.validators.apply_reveal(validator, reveal)
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
        let needs_commitment = kind == TxKind::Stake && !self.validators.is_committed(from);
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
                self.validators.retire(from);
            }
        }
        if let Some(commitment) = &tx.commitment {
            self.validators.commit(from, commitment);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_reads_the_same_as_before_validators_had_their_own_type() {
        let json = serde_json::to_value(Ledger::starting_at(&BlockHash::zero())).unwrap();
        let mut keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["balances", "beacon", "commitments", "epochs", "nonces", "stakes"]);
    }
}
