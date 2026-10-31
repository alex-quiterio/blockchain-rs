//! What a mempool offers for the next block when more is waiting than fits: the highest fees
//! first. A producer's choice, not a consensus rule: peers only check that a block isn't too full.

use std::cmp::Reverse;
use std::collections::{HashMap, VecDeque};

use chrono::Utc;

use crate::domain::accounting::ledger::Ledger;
use crate::domain::accounting::transaction::Transaction;
use crate::domain::consensus::BlockCapacity;
use crate::domain::crypto::address::Address;

/// As many of `fitting` as a block of `capacity` holds, highest fees first, in an order that applies on top of
/// `confirmed`. `fitting` must already apply in order there, as a rebuilt mempool's do.
///
/// Each sender's transfers keep their nonce order, so a sender is ranked by the fee of their next
/// one. Among equal fees, whichever arrived first goes first. A transfer that spends coins not
/// received yet waits until the payment it needs is chosen.
///
/// It never stops short of a full block: the earliest transfer not yet chosen always applies, since
/// everything before it in `fitting` is in, and nothing in can take coins from its sender.
pub(super) fn by_fee(
    fitting: &[Transaction],
    confirmed: &Ledger,
    capacity: BlockCapacity,
) -> Vec<Transaction> {
    if capacity.holds(fitting.len()) {
        return fitting.to_vec();
    }
    // Each sender's transfers, with where they arrived, in the order they must apply.
    let mut queues: Vec<VecDeque<(usize, &Transaction)>> = Vec::new();
    let mut senders: HashMap<&Address, usize> = HashMap::new();
    for (arrived, tx) in fitting.iter().enumerate() {
        let sender = tx.sender().expect("only signed transfers wait");
        let queue = *senders.entry(sender).or_insert_with(|| {
            queues.push(VecDeque::new());
            queues.len() - 1
        });
        queues[queue].push_back((arrived, tx));
    }
    // A sender whose next transfer didn't apply waits for a payment, instead of being checked,
    // signature and all, again every round.
    let mut waiting_for_coins = vec![false; queues.len()];
    let mut ledger = confirmed.clone();
    let mut chosen = Vec::new();
    while chosen.len() < capacity.limit() {
        let mut ranked: Vec<usize> =
            (0..queues.len()).filter(|&q| !waiting_for_coins[q] && !queues[q].is_empty()).collect();
        ranked.sort_by_key(|&q| {
            let (arrived, tx) = queues[q][0];
            (Reverse(tx.fee()), arrived)
        });
        let Some(next) = ranked.into_iter().find(|&q| {
            let applies = ledger.apply_transfer(queues[q][0].1, Utc::now()).is_ok();
            waiting_for_coins[q] = !applies;
            applies
        }) else {
            break;
        };
        let (_, tx) = queues[next].pop_front().expect("ranked queues aren't empty");
        if let Some(paid) = tx.to().account().and_then(|to| senders.get(to)) {
            waiting_for_coins[*paid] = false;
        }
        chosen.push(tx.clone());
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounting::{ChainId, Coins};
    use crate::domain::crypto;
    use ed25519_dalek::SigningKey;

    fn wallet() -> (SigningKey, Address) {
        let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
        let address = Address::of(&key);
        (key, address)
    }

    fn pay(key: &SigningKey, to: &Address, amount: u64, nonce: u64, fee: u64) -> Transaction {
        Transaction::signed(key, &ChainId::zero(), to, Coins::new(amount), nonce, Coins::new(fee))
    }

    fn fees(txs: &[Transaction]) -> Vec<u64> {
        txs.iter().map(|tx| tx.fee().value()).collect()
    }

    #[test]
    fn everything_goes_in_order_while_it_fits() {
        let ((a, from), (_, to)) = (wallet(), wallet());
        let mut ledger = Ledger::default();
        ledger.credit(&from, Coins::new(50));
        let txs = [pay(&a, &to, 1, 0, 1), pay(&a, &to, 1, 1, 9)];
        assert_eq!(by_fee(&txs, &ledger, BlockCapacity::at_most(2)), txs);
    }

    #[test]
    fn the_highest_fees_go_first() {
        let (to, mut ledger) = (wallet().1, Ledger::default());
        let txs: Vec<_> = [3, 7, 1, 7, 5]
            .into_iter()
            .map(|fee| {
                let (key, from) = wallet();
                ledger.credit(&from, Coins::new(50));
                pay(&key, &to, 1, 0, fee)
            })
            .collect();
        let chosen = by_fee(&txs, &ledger, BlockCapacity::at_most(3));
        assert_eq!(fees(&chosen), [7, 7, 5]);
        assert_eq!(chosen[0], txs[1], "equal fees go in the order they arrived");
    }

    #[test]
    fn a_sender_keeps_their_nonce_order() {
        let ((a, alice), (b, bob), (_, to)) = (wallet(), wallet(), wallet());
        let mut ledger = Ledger::default();
        ledger.credit(&alice, Coins::new(50));
        ledger.credit(&bob, Coins::new(50));
        // Alice's big fee sits behind her small one; Bob's middling one beats her small one.
        let txs = [pay(&a, &to, 1, 0, 1), pay(&a, &to, 1, 1, 20), pay(&b, &to, 1, 0, 5)];
        assert_eq!(fees(&by_fee(&txs, &ledger, BlockCapacity::at_most(2))), [5, 1]);
    }

    #[test]
    fn a_payment_goes_in_before_what_spends_it() {
        let ((a, alice), (b, bob), (_, to)) = (wallet(), wallet(), wallet());
        let mut ledger = Ledger::default();
        ledger.credit(&alice, Coins::new(50));
        // Bob can only pay once Alice's payment to him is in, however high his fee.
        let txs = [pay(&a, &bob, 10, 0, 1), pay(&b, &to, 5, 0, 9)];
        let chosen = by_fee(&txs, &ledger, BlockCapacity::at_most(1));
        assert_eq!(chosen, [txs[0].clone()]);

        let mut more = txs.to_vec();
        let (c, carol) = wallet();
        ledger.credit(&carol, Coins::new(50));
        more.push(pay(&c, &to, 1, 0, 3));
        assert_eq!(
            fees(&by_fee(&more, &ledger, BlockCapacity::at_most(2))),
            [3, 1],
            "Bob waits for his coins"
        );
    }
}
