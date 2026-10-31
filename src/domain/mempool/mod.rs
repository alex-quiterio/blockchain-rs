//! The mempool, an aggregate: the transfers waiting for a block, changed only through its root
//! (`Mempool`), and what it offers when more is waiting than a block holds (`selection`). Its
//! lock in a node is its own, so the chain, the other aggregate, never waits on it.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use crate::domain::accounting::ledger::Ledger;
use crate::domain::accounting::transaction::Transaction;
use crate::domain::accounting::Coins;
use crate::domain::accounting::TransactionError;
use crate::domain::consensus::BlockCapacity;
use crate::domain::crypto::address::Address;
use crate::domain::crypto::Signature;

mod selection;

/// How many transfers may wait at once, unless the node is told otherwise. Without a limit,
/// anyone holding a single coin could queue zero-fee transfers to themselves without end, since
/// each hands the coin straight back, and every node would hold and gossip them all.
pub const MAX_WAITING: usize = 5_000;

/// Signed transfers waiting to be mined, in the order they arrived. Together they always apply
/// cleanly on top of the confirmed ledger.
///
/// An aggregate of its own: what's waiting isn't part of what makes the chain consistent, so it
/// keeps its own state and, in a node, its own lock. When the chain changes, `rebase` brings it
/// up to the new confirmed ledger; whoever produces a block asks it for `candidates`, which puts
/// the highest fees first when more is waiting than a block holds.
///
/// It keeps the ledger as it will be once everything waiting is confirmed, so admitting one more
/// transaction only checks that one, instead of the whole queue again. Under load, with hundreds
/// waiting and peers gossiping each other's queues, that's the difference between a node that
/// keeps up and one that stalls.
///
/// It holds at most `limit` transfers. Once full, a new one gets in only by paying a higher fee
/// than the cheapest it can push out: the last one waiting from some other sender, since nothing
/// of that sender's comes after it to break.
#[derive(Debug, Clone, Default)]
pub struct Mempool {
    waiting: Vec<Transaction>,
    /// The most that may wait at once. Taking back what a fork dropped may briefly go past it.
    limit: usize,
    /// The confirmed ledger it sits on, which candidates for a block are picked against.
    confirmed: Ledger,
    /// The confirmed ledger with every waiting transaction applied.
    projected: Ledger,
    /// The signatures of everything waiting, to drop gossiped duplicates at once.
    signatures: HashSet<Signature>,
    /// What happened since they were last taken.
    events: Vec<MempoolEvent>,
}

/// Something that happened to the mempool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MempoolEvent {
    /// A transfer was submitted and now waits for a block.
    Accepted(Transaction),
    /// The chain moved on: `kept` still wait, `dropped` left because a block confirmed them or
    /// they no longer apply, and `returned` came back from blocks a fork dropped.
    Rebased { kept: usize, dropped: usize, returned: usize },
}

/// What a new transfer has to pay to be sure of a place in the next block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeEstimate {
    /// Places left in the next block once what's waiting is in it. While there are any, the
    /// fee doesn't matter.
    pub room: usize,
    /// The fee that's sure to win a place: nothing while there's room, or else one coin more
    /// than the lowest fee the block would take.
    pub fee: Coins,
}

impl Mempool {
    /// An empty mempool on top of `confirmed`.
    pub fn on(confirmed: &Ledger) -> Self {
        let projected = confirmed.clone();
        Self { confirmed: confirmed.clone(), projected, limit: MAX_WAITING, ..Self::default() }
    }

    /// The same mempool, holding at most `limit` transfers (at least one).
    pub fn with_limit(self, limit: usize) -> Self {
        Self { limit: limit.max(1), ..self }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Keeps the candidates that still apply on top of `confirmed`, in order, dropping the rest.
    pub fn rebuilt(candidates: impl IntoIterator<Item = Transaction>, confirmed: &Ledger) -> Self {
        let mut pool = Self::on(confirmed);
        for tx in candidates {
            let _ = pool.take_in(tx);
        }
        pool
    }

    /// What happened since this was last called, oldest first.
    pub fn take_events(&mut self) -> Vec<MempoolEvent> {
        std::mem::take(&mut self.events)
    }

    /// The ledger as it will be once everything waiting is confirmed.
    pub fn projected(&self) -> &Ledger {
        &self.projected
    }

    pub fn transactions(&self) -> &[Transaction] {
        &self.waiting
    }

    /// What to offer for the next block, which holds `capacity`: everything waiting while it
    /// fits, in the order it arrived, or else the highest fees first. Either way they apply in
    /// order on top of the confirmed ledger.
    pub fn candidates(&self, capacity: BlockCapacity) -> Vec<Transaction> {
        selection::by_fee(&self.waiting, &self.confirmed, capacity)
    }

    /// The fee a new transfer needs to be sure of a place in the next block, which holds
    /// `capacity`, if nothing else arrives first. It holds for a sender with nothing else
    /// waiting, whose transfer can go in at once.
    ///
    /// When the block is full, the selection took its lowest fee only once nothing better was
    /// left to take. A transfer paying more would have been taken before it, so it's in, and the
    /// lowest one waits instead. Paying the same isn't enough: it arrived later.
    pub fn fee_estimate(&self, capacity: BlockCapacity) -> FeeEstimate {
        let chosen = self.candidates(capacity);
        let room = capacity.limit().saturating_sub(chosen.len());
        let fee = match chosen.iter().map(Transaction::fee).min() {
            Some(lowest) if room == 0 => lowest.saturating_add(Coins::new(1)),
            _ => Coins::ZERO,
        };
        // A full mempool asks more of a newcomer than the cheapest it would push out.
        let to_get_in = match self.is_full().then(|| self.cheapest_to_push_out(None)) {
            Some(Some((_, lowest))) => lowest.saturating_add(Coins::new(1)),
            _ => Coins::ZERO,
        };
        FeeEstimate { room, fee: fee.max(to_get_in) }
    }

    /// After the chain changed: keeps what still applies on top of its new `confirmed` ledger,
    /// with `returned`, the transfers of blocks a fork dropped, ahead of what was waiting. What a
    /// new block confirmed no longer applies, so it leaves.
    pub fn rebase(&mut self, confirmed: &Ledger, returned: Vec<Transaction>) {
        let (waiting, mut events) = (std::mem::take(&mut self.waiting), self.take_events());
        let (before, had) = (waiting.len(), std::mem::take(&mut self.signatures));
        *self =
            Self::rebuilt(returned.into_iter().chain(waiting), confirmed).with_limit(self.limit);
        let kept = self.signatures.intersection(&had).count();
        let returned = self.waiting.len() - kept;
        events.push(MempoolEvent::Rebased { kept, dropped: before - kept, returned });
        self.events = events;
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

    fn is_full(&self) -> bool {
        self.waiting.len() >= self.limit
    }

    /// Queues `tx` if it applies after everything already waiting. When the mempool is full, it
    /// must also pay more than the cheapest transfer it can push out, which then leaves, along
    /// with anything that spent coins only it brought.
    pub fn admit(&mut self, tx: Transaction) -> Result<(), TransactionError> {
        if !self.is_full() {
            self.take_in(tx.clone())?;
        } else {
            let needed = |lowest: Coins| lowest.saturating_add(Coins::new(1));
            let (at, lowest) = self
                .cheapest_to_push_out(tx.sender())
                .ok_or(TransactionError::MempoolFull { needed: None })?;
            if tx.fee <= lowest {
                return Err(TransactionError::MempoolFull { needed: Some(needed(lowest)) });
            }
            // Made on a copy, so a newcomer that doesn't apply pushes nothing out.
            let rest = self.waiting.iter().enumerate().filter(|&(i, _)| i != at);
            let mut made_room = Self::rebuilt(rest.map(|(_, tx)| tx.clone()), &self.confirmed)
                .with_limit(self.limit);
            made_room.take_in(tx.clone())?;
            made_room.events = std::mem::take(&mut self.events);
            *self = made_room;
        }
        self.events.push(MempoolEvent::Accepted(tx));
        Ok(())
    }

    /// Where the cheapest transfer that can be pushed out waits, and its fee: the last one of
    /// some sender other than `newcomer`, whose transfer comes after their own last. Among equal
    /// fees, the latest to arrive goes first.
    fn cheapest_to_push_out(&self, newcomer: Option<&Address>) -> Option<(usize, Coins)> {
        let mut lasts = HashMap::new();
        for (at, tx) in self.waiting.iter().enumerate() {
            lasts.insert(tx.sender(), at);
        }
        lasts
            .into_iter()
            .filter(|&(sender, _)| sender != newcomer)
            .map(|(_, at)| (at, self.waiting[at].fee))
            .min_by_key(|&(at, fee)| (fee, Reverse(at)))
    }

    /// Queues `tx` like `admit`, but quietly: rebuilding takes back what was already there.
    fn take_in(&mut self, tx: Transaction) -> Result<(), TransactionError> {
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
    use crate::domain::accounting::Coins;
    use crate::domain::crypto;
    use crate::domain::crypto::address::Address;

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
    fn it_says_what_it_accepted_and_how_a_rebase_went() {
        let (ledger, key, to) = funded();
        let mut pool = Mempool::on(&ledger);
        let (first, second) = (pay(&key, &to, 5, 0), pay(&key, &to, 5, 1));
        pool.admit(first.clone()).unwrap();
        pool.admit(second.clone()).unwrap();
        let _ = pool.admit(first.clone());
        let accepted = [MempoolEvent::Accepted(first.clone()), MempoolEvent::Accepted(second)];
        assert_eq!(pool.take_events(), accepted, "a refused one says nothing");

        // A block confirmed the first; a fork gave back a payment from someone else.
        let mut confirmed = ledger.clone();
        confirmed.apply_transfer(&first).unwrap();
        let (_, other, _) = funded();
        confirmed.credit(&Address::of(&other), Coins::new(50));
        pool.rebase(&confirmed, vec![pay(&other, &to, 3, 0)]);
        let rebased = MempoolEvent::Rebased { kept: 1, dropped: 1, returned: 1 };
        assert_eq!(pool.take_events(), [rebased], "taking back what was there is quiet");
        assert_eq!(pool.transactions().len(), 2);
    }

    #[test]
    fn a_full_block_is_offered_the_highest_fees() {
        let (mut ledger, key, to) = funded();
        let (_, other, _) = funded();
        ledger.credit(&Address::of(&other), Coins::new(50));
        let mut pool = Mempool::on(&ledger);
        let cheap = Transaction::signed(&key, &to, Coins::new(1), 0, Coins::new(1));
        let dear = Transaction::signed(&other, &to, Coins::new(1), 0, Coins::new(9));
        pool.admit(cheap.clone()).unwrap();
        pool.admit(dear.clone()).unwrap();
        assert_eq!(pool.candidates(BlockCapacity::ANY), [cheap.clone(), dear.clone()]);
        assert_eq!(pool.candidates(BlockCapacity::at_most(1)), [dear]);
        assert_eq!(pool.transactions().len(), 2, "offering takes nothing out");
    }

    #[test]
    fn a_fee_is_only_needed_once_the_next_block_is_full() {
        let (mut ledger, key, to) = funded();
        let (_, other, _) = funded();
        ledger.credit(&Address::of(&other), Coins::new(50));
        let mut pool = Mempool::on(&ledger);
        assert_eq!(
            pool.fee_estimate(BlockCapacity::at_most(2)),
            FeeEstimate { room: 2, fee: Coins::ZERO }
        );
        pool.admit(Transaction::signed(&key, &to, Coins::new(1), 0, Coins::new(3))).unwrap();
        pool.admit(Transaction::signed(&other, &to, Coins::new(1), 0, Coins::new(5))).unwrap();
        assert_eq!(
            pool.fee_estimate(BlockCapacity::at_most(3)),
            FeeEstimate { room: 1, fee: Coins::ZERO }
        );
        assert_eq!(
            pool.fee_estimate(BlockCapacity::at_most(2)),
            FeeEstimate { room: 0, fee: Coins::new(4) },
            "one more than the 3 it would take last"
        );
        assert_eq!(
            pool.fee_estimate(BlockCapacity::at_most(1)),
            FeeEstimate { room: 0, fee: Coins::new(6) }
        );
        assert_eq!(pool.fee_estimate(BlockCapacity::ANY).room, usize::MAX - 2);
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
    fn a_full_mempool_takes_a_higher_fee_in_place_of_the_cheapest_last_transfer() {
        let (mut ledger, spammer, to) = funded();
        let (_, other, _) = funded();
        ledger.credit(&Address::of(&other), Coins::new(50));
        let mut pool = Mempool::on(&ledger).with_limit(3);
        let me = Address::of(&spammer);
        // Paying yourself costs nothing, so without a limit this could go on forever.
        for nonce in 0..3 {
            pool.admit(pay(&spammer, &me, 1, nonce)).unwrap();
        }
        assert_eq!(
            pool.admit(pay(&spammer, &me, 1, 3)),
            Err(TransactionError::MempoolFull { needed: None }),
            "a sender can't push out their own, which theirs comes after"
        );
        assert_eq!(
            pool.admit(pay(&other, &to, 1, 0)),
            Err(TransactionError::MempoolFull { needed: Some(Coins::new(1)) })
        );
        assert_eq!(pool.fee_estimate(BlockCapacity::ANY).fee, Coins::new(1));

        let _ = pool.take_events();
        let paying = Transaction::signed(&other, &to, Coins::new(1), 0, Coins::new(1));
        pool.admit(paying.clone()).unwrap();
        assert_eq!(pool.take_events(), [MempoolEvent::Accepted(paying.clone())]);
        let nonces: Vec<_> = pool.transactions().iter().map(Transaction::nonce).collect();
        assert_eq!(nonces, [0, 1, 0], "the spammer's last went, and the rest still apply");
        assert_eq!(pool.next_nonce(&me), 2);
        assert_eq!(pool.limit(), 3);
    }

    #[test]
    fn a_newcomer_that_doesnt_apply_pushes_nothing_out() {
        let (mut ledger, key, to) = funded();
        let (_, other, _) = funded();
        ledger.credit(&Address::of(&other), Coins::new(50));
        let mut pool = Mempool::on(&ledger).with_limit(1);
        let waiting = pay(&key, &to, 1, 0);
        pool.admit(waiting.clone()).unwrap();
        let replayed = Transaction::signed(&other, &to, Coins::new(1), 5, Coins::new(9));
        assert_eq!(pool.admit(replayed), Err(TransactionError::BadNonce { expected: 0, got: 5 }));
        assert_eq!(pool.transactions(), [waiting]);
    }

    #[test]
    fn a_rebase_keeps_the_limit() {
        let (ledger, _, _) = funded();
        let mut pool = Mempool::on(&ledger).with_limit(7);
        pool.rebase(&ledger, Vec::new());
        assert_eq!(pool.limit(), 7);
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
