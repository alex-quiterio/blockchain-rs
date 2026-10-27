use super::block::Transaction;
use super::chain::ChainError;
use super::ledger::Ledger;

/// Signed transfers waiting to be mined, in the order they arrived. Together they always apply
/// cleanly on top of the confirmed ledger.
#[derive(Debug, Clone, Default)]
pub struct Mempool(Vec<Transaction>);

impl Mempool {
    /// Keeps the candidates that still apply on top of `confirmed`, in order, dropping the rest.
    pub fn rebuilt(candidates: impl IntoIterator<Item = Transaction>, confirmed: &Ledger) -> Self {
        let mut projected = confirmed.clone();
        Self(candidates.into_iter().filter(|tx| projected.apply_transfer(tx).is_ok()).collect())
    }

    pub fn transactions(&self) -> &[Transaction] {
        &self.0
    }

    /// How many waiting transfers `address` has sent.
    pub fn sent_by(&self, address: &str) -> u64 {
        self.0.iter().filter(|tx| tx.from == address).count() as u64
    }

    /// Queues `tx` if it applies after everything already waiting.
    pub fn admit(&mut self, tx: Transaction, confirmed: &Ledger) -> Result<(), ChainError> {
        let mut projected = confirmed.clone();
        for queued in &self.0 {
            projected.apply_transfer(queued)?;
        }
        projected.apply_transfer(&tx)?;
        self.0.push(tx);
        Ok(())
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::address::Address;
    use crate::domain::crypto;

    fn funded() -> (Ledger, ed25519_dalek::SigningKey, Address) {
        let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
        let mut ledger = Ledger::default();
        ledger.credit(Address::of(&key).as_str(), 50);
        let to = Address::parse(&crypto::generate_keypair().0).unwrap();
        (ledger, key, to)
    }

    fn pay(key: &ed25519_dalek::SigningKey, to: &Address, amount: u64, nonce: u64) -> Transaction {
        let from = Address::of(key).to_string();
        let payload = Transaction::signing_payload(&from, to.as_str(), amount, nonce);
        Transaction {
            from,
            to: to.to_string(),
            amount,
            nonce,
            signature: crypto::sign(key, &payload),
        }
    }

    #[test]
    fn admits_transfers_that_fit_after_the_queue() {
        let (ledger, key, to) = funded();
        let mut pool = Mempool::default();
        pool.admit(pay(&key, &to, 30, 0), &ledger).unwrap();
        pool.admit(pay(&key, &to, 20, 1), &ledger).unwrap();
        assert_eq!(
            pool.admit(pay(&key, &to, 1, 2), &ledger),
            Err(ChainError::InsufficientFunds { balance: 0, amount: 1 })
        );
        assert_eq!(pool.sent_by(Address::of(&key).as_str()), 2);
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
