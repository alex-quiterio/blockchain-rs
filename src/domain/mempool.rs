use std::collections::HashSet;

use super::address::Address;
use super::block::Transaction;
use super::error::TransactionError;
use super::ledger::Ledger;
use super::values::Signature;

/// Signed transfers waiting to be mined, in the order they arrived. Together they always apply
/// cleanly on top of the confirmed ledger.
///
/// An aggregate of its own: what's waiting isn't part of what makes the chain consistent, so it
/// keeps its own state and, in a node, its own lock. When the chain changes, `rebase` brings it
/// up to the new confirmed ledger; whoever produces a block reads `transactions()` as candidates.
///
/// It keeps the ledger as it will be once everything waiting is confirmed, so admitting one more
/// transaction only checks that one, instead of the whole queue again. Under load, with hundreds
/// waiting and peers gossiping each other's queues, that's the difference between a node that
/// keeps up and one that stalls.
#[derive(Debug, Clone, Default)]
pub struct Mempool {
    waiting: Vec<Transaction>,
    /// The confirmed ledger with every waiting transaction applied.
    projected: Ledger,
    /// The signatures of everything waiting, to drop gossiped duplicates at once.
    signatures: HashSet<Signature>,
}

impl Mempool {
    /// An empty mempool on top of `confirmed`.
    pub fn on(confirmed: &Ledger) -> Self {
        Self { projected: confirmed.clone(), ..Self::default() }
    }

    /// Keeps the candidates that still apply on top of `confirmed`, in order, dropping the rest.
    pub fn rebuilt(candidates: impl IntoIterator<Item = Transaction>, confirmed: &Ledger) -> Self {
        let mut pool = Self::on(confirmed);
        for tx in candidates {
            let _ = pool.admit(tx);
        }
        pool
    }

    /// The ledger as it will be once everything waiting is confirmed.
    pub fn projected(&self) -> &Ledger {
        &self.projected
    }

    pub fn transactions(&self) -> &[Transaction] {
        &self.waiting
    }

    /// After the chain changed: keeps what still applies on top of its new `confirmed` ledger,
    /// with `returned`, the transfers of blocks a fork dropped, ahead of what was waiting. What a
    /// new block confirmed no longer applies, so it leaves.
    pub fn rebase(&mut self, confirmed: &Ledger, returned: Vec<Transaction>) {
        let waiting = std::mem::take(&mut self.waiting);
        *self = Self::rebuilt(returned.into_iter().chain(waiting), confirmed);
    }

    /// The waiting transfer signed `signature`.
    pub fn find(&self, signature: &str) -> Option<&Transaction> {
        let signed = |tx: &&Transaction| tx.signature().is_some_and(|s| s.as_str() == signature);
        self.waiting.iter().find(signed)
    }

    /// Waiting transfers to or from `address`.
    pub fn involving<'a>(&'a self, address: &'a Address) -> impl Iterator<Item = &'a Transaction> {
        self.waiting.iter().filter(move |tx| tx.involves(address))
    }

    /// The nonce a new transaction from `address` must carry, counting what's waiting.
    pub fn next_nonce(&self, address: &Address) -> u64 {
        self.projected.next_nonce(address)
    }

    /// If `address`'s next stake must carry a commitment, the epoch to make it for. Counts stakes
    /// already waiting, so a second stake in a row doesn't commit again.
    pub fn commitment_due(&self, address: &Address) -> Option<u64> {
        let state = self.projected.validator_state(address);
        state.reveals_left.is_none().then_some(state.next_epoch)
    }

    /// Queues `tx` if it applies after everything already waiting.
    pub fn admit(&mut self, tx: Transaction) -> Result<(), TransactionError> {
        if tx.signature.as_ref().is_some_and(|s| self.signatures.contains(s)) {
            return Err(TransactionError::AlreadyPending);
        }
        self.projected.apply_transfer(&tx)?;
        // Only signed transfers get this far: a block reward is refused above.
        self.signatures.extend(tx.signature.clone());
        self.waiting.push(tx);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::address::Address;
    use crate::domain::crypto;
    use crate::domain::values::Coins;

    fn funded() -> (Ledger, ed25519_dalek::SigningKey, Address) {
        let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
        let mut ledger = Ledger::default();
        ledger.credit(&Address::of(&key), Coins::new(50));
        let to = Address::parse(&crypto::generate_keypair().0).unwrap();
        (ledger, key, to)
    }

    fn pay(key: &ed25519_dalek::SigningKey, to: &Address, amount: u64, nonce: u64) -> Transaction {
        Transaction::signed(key, to, Coins::new(amount), nonce, Coins::new(0))
    }

    #[test]
    fn admits_transfers_that_fit_after_the_queue() {
        let (ledger, key, to) = funded();
        let mut pool = Mempool::on(&ledger);
        pool.admit(pay(&key, &to, 30, 0)).unwrap();
        pool.admit(pay(&key, &to, 20, 1)).unwrap();
        assert_eq!(
            pool.admit(pay(&key, &to, 1, 2)),
            Err(TransactionError::InsufficientFunds {
                balance: Coins::new(0),
                amount: Coins::new(1)
            })
        );
        assert_eq!(pool.projected().next_nonce(&Address::of(&key)), 2);
        assert_eq!(ledger.next_nonce(&Address::of(&key)), 0, "the confirmed ledger is untouched");
    }

    #[test]
    fn a_gossiped_duplicate_is_turned_away_at_once() {
        let (ledger, key, to) = funded();
        let mut pool = Mempool::on(&ledger);
        let tx = pay(&key, &to, 5, 0);
        pool.admit(tx.clone()).unwrap();
        assert_eq!(pool.admit(tx), Err(TransactionError::AlreadyPending));
        assert_eq!(pool.transactions().len(), 1);
    }

    #[test]
    fn a_rejected_transaction_leaves_no_trace() {
        let (ledger, key, to) = funded();
        let mut pool = Mempool::on(&ledger);
        let mut forged = pay(&key, &to, 5, 0);
        forged.amount = Coins::new(50);
        assert_eq!(pool.admit(forged), Err(TransactionError::BadSignature));
        pool.admit(pay(&key, &to, 50, 0)).unwrap();
        assert_eq!(pool.projected().balance(&Address::of(&key)), Coins::new(0));
    }

    #[test]
    fn rebuilding_drops_what_no_longer_applies() {
        let (ledger, key, to) = funded();
        let first = pay(&key, &to, 30, 0);
        let replayed = first.clone();
        let second = pay(&key, &to, 20, 1);
        let pool = Mempool::rebuilt([first.clone(), replayed, second.clone()], &ledger);
        assert_eq!(pool.transactions(), [first, second]);
    }
}
