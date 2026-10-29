//! Property-based tests: random transaction orderings and random forks, checked against the
//! invariants every node relies on.

use std::cmp::Reverse;

use ed25519_dalek::SigningKey;
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

use blockchain::domain::address::Address;
use blockchain::domain::block::{Block, Transaction};
use blockchain::domain::chain::{validate_blocks, Blockchain};
use blockchain::domain::error::ChainError;
use blockchain::domain::randao;
use blockchain::domain::rules::ChainRules;
use blockchain::domain::values::{BlockHash, Coins, Destination};

const WALLETS: usize = 4;
const REWARD: u64 = 50;
// Difficulty 1 keeps mining to ~16 hashes a block, so hundreds of cases stay fast.
const POW: ChainRules = ChainRules::proof_of_work(1, REWARD);
const POS: ChainRules = ChainRules::proof_of_stake(REWARD);

fn key(i: usize) -> SigningKey {
    SigningKey::from_bytes(&[i as u8 + 1; 32])
}

fn address(i: usize) -> Address {
    Address::of(&key(i))
}

/// Coins only enter the chain as block rewards, and fees only move them around.
fn assert_supply_conserved(chain: &Blockchain) {
    let held: u64 = (0..WALLETS)
        .map(|i| chain.balance(&address(i)).value() + chain.staked(&address(i)).value())
        .sum();
    assert_eq!(held, REWARD * (chain.blocks().len() as u64 - 1), "coins appeared or vanished");
}

/// Each sender's next nonce is exactly how many of its transactions are confirmed or queued.
fn assert_nonces_match_history(chain: &Blockchain) {
    for i in 0..WALLETS {
        let from = address(i);
        let sent = chain
            .blocks()
            .iter()
            .flat_map(|b| b.transactions())
            .chain(chain.pending())
            .filter(|tx| tx.sender() == Some(&from))
            .count() as u64;
        assert_eq!(chain.next_nonce(&address(i)), sent);
    }
}

/// Adds a block: mined under proof of work, forged by whoever's turn it is under proof of stake.
fn add_block(chain: &mut Blockchain, by: usize) {
    let block = if chain.rules().is_proof_of_stake() {
        let turn = chain.next_validator();
        let by = turn.map_or(by, |v| (0..WALLETS).find(|&i| address(i) == v).expect("ours"));
        chain.forge(&key(by)).map(|b| b.index())
    } else {
        chain.mine(&address(by)).map(|b| b.index())
    };
    block.expect("the mempool always applies, so adding a block never fails");
}

#[derive(Debug, Clone)]
enum Op {
    Pay { from: usize, to: usize, amount: u64, fee: u64, skew: i64 },
    Stake { from: usize, amount: u64, fee: u64 },
    Unstake { from: usize, amount: u64 },
    Block { by: usize },
}

fn op() -> impl Strategy<Value = Op> {
    let who = 0..WALLETS;
    prop_oneof![
        4 => (who.clone(), who.clone(), 0..80u64, 0..6u64, -1..=1i64)
            .prop_map(|(from, to, amount, fee, skew)| Op::Pay { from, to, amount, fee, skew }),
        1 => (who.clone(), 0..60u64, 0..4u64).prop_map(|(from, amount, fee)| Op::Stake { from, amount, fee }),
        1 => (who.clone(), 0..60u64).prop_map(|(from, amount)| Op::Unstake { from, amount }),
        2 => who.prop_map(|by| Op::Block { by }),
    ]
}

/// Runs `ops` on a fresh chain, checking the invariants after every step.
fn run(rules: ChainRules, ops: &[Op]) -> Blockchain {
    let mut chain = Blockchain::new(rules);
    for op in ops {
        let tx = match *op {
            Op::Pay { from, to, amount, fee, skew } => {
                let nonce = (chain.next_nonce(&address(from)) as i64 + skew).max(0) as u64;
                Some(Transaction::signed(
                    &key(from),
                    address(to),
                    Coins::new(amount),
                    nonce,
                    Coins::new(fee),
                ))
            }
            Op::Stake { from, amount, fee } => {
                let nonce = chain.next_nonce(&address(from));
                let commitment =
                    chain.commitment_due(&address(from)).map(|e| randao::commitment(&key(from), e));
                Some(Transaction::stake(
                    &key(from),
                    Coins::new(amount),
                    nonce,
                    Coins::new(fee),
                    commitment,
                ))
            }
            Op::Unstake { from, amount } => {
                let nonce = chain.next_nonce(&address(from));
                Some(Transaction::signed(
                    &key(from),
                    Destination::Unstake,
                    Coins::new(amount),
                    nonce,
                    Coins::new(0),
                ))
            }
            Op::Block { by } => {
                add_block(&mut chain, by);
                assert_supply_conserved(&chain);
                None
            }
        };
        if let Some(tx) = tx {
            let queued = chain.pending().len();
            let accepted = chain.submit(tx).is_ok();
            assert_eq!(chain.pending().len(), queued + usize::from(accepted));
        }
        assert_nonces_match_history(&chain);
    }
    add_block(&mut chain, 0);
    assert_supply_conserved(&chain);
    chain
}

/// Whatever order transactions arrive in, a node retries until nothing more fits, as gossip would.
fn deliver(rules: ChainRules, funding: &[Block], batch: &[Transaction]) -> Blockchain {
    let mut chain = Blockchain::restore(funding.to_vec(), rules).unwrap();
    let mut waiting = batch.to_vec();
    loop {
        let before = waiting.len();
        waiting.retain(|tx| chain.submit(tx.clone()).is_err());
        if waiting.len() == before {
            break;
        }
    }
    add_block(&mut chain, 0);
    chain
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        // Failing cases are saved here and replayed first on the next run.
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/properties.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn random_operations_never_break_the_ledger_under_proof_of_work(ops in prop::collection::vec(op(), 0..40)) {
        let chain = run(POW, &ops);
        prop_assert!(chain.is_valid());
        let restored = Blockchain::restore(chain.blocks().to_vec(), POW).unwrap();
        for i in 0..WALLETS {
            prop_assert_eq!(restored.balance(&address(i)), chain.balance(&address(i)));
            prop_assert_eq!(restored.staked(&address(i)), chain.staked(&address(i)));
        }
        prop_assert_eq!(restored.weight(), chain.weight());
    }

    #[test]
    fn random_operations_never_break_the_ledger_under_proof_of_stake(ops in prop::collection::vec(op(), 0..40)) {
        let chain = run(POS, &ops);
        prop_assert!(chain.is_valid());
        let restored = Blockchain::restore(chain.blocks().to_vec(), POS).unwrap();
        prop_assert_eq!(restored.stakes(), chain.stakes());
        prop_assert_eq!(restored.next_validator(), chain.next_validator());
    }

    #[test]
    fn the_order_transactions_arrive_in_does_not_change_the_outcome(
        payments in prop::collection::vec((0..WALLETS, 0..WALLETS, 1..70u64, 0..4u64), 1..16),
        seed in any::<u64>(),
    ) {
        // Every wallet starts with one block reward.
        let mut funded = Blockchain::new(POW);
        for i in 0..WALLETS {
            funded.mine(&address(i)).unwrap();
        }
        // A batch with correct, consecutive nonces per sender; some may never be affordable.
        let mut nonces = [0u64; WALLETS];
        let batch: Vec<Transaction> = payments
            .iter()
            .map(|&(from, to, amount, fee)| {
                nonces[from] += 1;
                Transaction::signed(&key(from), address(to), Coins::new(amount), nonces[from] - 1, Coins::new(fee))
            })
            .collect();
        let mut shuffled = batch.clone();
        let mut state = seed | 1;
        for i in (1..shuffled.len()).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            shuffled.swap(i, (state % (i as u64 + 1)) as usize);
        }

        let in_order = deliver(POW, funded.blocks(), &batch);
        let any_order = deliver(POW, funded.blocks(), &shuffled);
        for i in 0..WALLETS {
            prop_assert_eq!(in_order.balance(&address(i)), any_order.balance(&address(i)));
            prop_assert_eq!(in_order.next_nonce(&address(i)), any_order.next_nonce(&address(i)));
        }
    }

    #[test]
    fn fork_choice_settles_on_the_same_chain_whatever_order_forks_arrive(
        forks in prop::collection::vec((0..=3usize, 1..5usize, 0..WALLETS), 1..5)
            .prop_flat_map(|forks| {
                let order: Vec<usize> = (0..=forks.len()).collect();
                (Just(forks), Just(order).prop_shuffle())
            })
    ) {
        let (forks, order) = forks;
        let mut base = Blockchain::new(POW);
        for i in 0..3 {
            base.mine(&address(i)).unwrap();
        }
        // Each fork branches off the base at some height and mines its own blocks on top.
        let mut candidates = vec![base.clone()];
        for &(height, extra, miner) in &forks {
            let mut fork = Blockchain::restore(base.blocks()[..=height].to_vec(), POW).unwrap();
            for _ in 0..extra {
                fork.mine(&address(miner)).unwrap();
            }
            candidates.push(fork);
        }
        let winner = candidates
            .iter()
            .max_by_key(|c| (c.weight(), Reverse(c.tip().hash().clone())))
            .unwrap();

        let mut node = Blockchain::new(POW);
        for &i in &order {
            node.adopt_if_heavier(candidates[i].blocks().to_vec()).unwrap();
        }
        prop_assert_eq!(&node.tip().hash(), &winner.tip().hash());
        for candidate in &candidates {
            prop_assert_eq!(node.adopt_if_heavier(candidate.blocks().to_vec()), Ok(false), "settled for good");
        }
        node.mine(&address(0)).unwrap();
        prop_assert!(node.is_valid());
    }

    #[test]
    fn no_version_of_a_block_changes_who_forges_next(
        stakes in prop::collection::vec(1..=50u64, 2..=WALLETS),
        versions in prop::collection::vec((0..WALLETS, 0..20u64, 0..5u64), 2..8),
    ) {
        // Everyone earns a reward while anyone may forge, then stakes some of it.
        let mut chain = Blockchain::new(POS);
        for i in 0..stakes.len() {
            chain.forge(&key(i)).unwrap();
        }
        for (i, &amount) in stakes.iter().enumerate() {
            let commitment = chain.commitment_due(&address(i)).map(|e| randao::commitment(&key(i), e));
            chain.submit(Transaction::stake(&key(i), Coins::new(amount), 0, Coins::new(0), commitment)).unwrap();
        }
        add_block(&mut chain, 0);
        let selected = chain.next_validator().expect("validators are being drawn");
        let forger = (0..WALLETS).find(|&i| address(i) == selected).unwrap();

        // The selected forger tries different blocks: other contents, fees and timestamps.
        let mut next = std::collections::HashSet::new();
        for &(to, amount, fee) in &versions {
            let mut attempt = chain.clone();
            if amount > 0 {
                let nonce = attempt.next_nonce(&selected);
                let tx = Transaction::signed(&key(forger), address(to), Coins::new(amount), nonce, Coins::new(fee));
                let _ = attempt.submit(tx);
            }
            attempt.forge(&key(forger)).unwrap();
            prop_assert!(attempt.is_valid());
            next.insert(attempt.next_validator());
        }
        prop_assert_eq!(next.len(), 1, "the forger steered the next turn");
    }

    #[test]
    fn difficulty_follows_the_pace_within_its_limits(gaps in prop::collection::vec(0..60i64, 1..40)) {
        // Starts at 16 hashes a block, aims for one every 10s, adjusts every 4 blocks.
        let rules = ChainRules::proof_of_work(1, REWARD).with_retarget(10, 4);
        let mut chain = Blockchain::new(rules);
        let mut at = chrono::Utc::now() - chrono::TimeDelta::days(1);
        for (i, gap) in gaps.iter().enumerate() {
            at += chrono::TimeDelta::seconds(*gap) + chrono::TimeDelta::milliseconds(1);
            let work = chain.next_work();
            let (index, prev) = (chain.blocks().len() as u64, chain.tip().hash().clone());
            let payout = vec![Transaction::coinbase(&address(i % WALLETS), Coins::new(REWARD), index)];
            let mut blocks = chain.blocks().to_vec();
            blocks.push(Block::mine(index, at, prev, payout, work));
            chain = Blockchain::restore(blocks, rules).expect("every honestly mined block is valid");
        }

        let history = chain.work_history();
        for (height, pair) in history.windows(2).enumerate().map(|(i, w)| (i + 1, w)) {
            let (before, after) = (pair[0], pair[1]);
            if !height.is_multiple_of(4) || height < 8 {
                prop_assert_eq!(before, after, "work only changes at adjustment heights");
            }
            prop_assert!(after <= before * 4 && after * 4 >= before, "at most 4x per adjustment");
        }
        prop_assert_eq!(chain.weight().value(), history[1..].iter().sum::<u128>());
    }

    #[test]
    fn tampered_forks_are_never_adopted(block in 1..5usize, field in 0..6u8, extra in 1..4usize) {
        let mut ours = Blockchain::new(POW);
        ours.mine(&address(0)).unwrap();
        let mut theirs = Blockchain::new(POW);
        for i in 0..5 + extra {
            theirs.mine(&address(i % WALLETS)).unwrap();
        }
        // Outside the domain a block can't be changed, only rewritten as JSON, as a peer would.
        let mut forged = theirs.blocks().to_vec();
        let mut json = serde_json::to_value(&forged[block]).unwrap();
        let bump = |value: &serde_json::Value| serde_json::Value::from(value.as_u64().unwrap() + 1);
        match field {
            0 => json["transactions"][0]["amount"] = bump(&json["transactions"][0]["amount"]),
            1 => json["transactions"][0]["to"] = address(WALLETS + 1).to_string().into(), // never mines
            2 => {
                let at: chrono::DateTime<chrono::Utc> = serde_json::from_value(json["timestamp"].clone()).unwrap();
                json["timestamp"] = serde_json::to_value(at + chrono::TimeDelta::seconds(1)).unwrap();
            }
            3 => json["nonce"] = bump(&json["nonce"]),
            4 => json["prev_hash"] = BlockHash::of(b"a block that isn't there").to_string().into(),
            _ => json["index"] = bump(&json["index"]),
        }
        // Re-hashing the altered block isn't enough either: it breaks the work or the next link.
        if field.is_multiple_of(2) {
            let altered: Block = serde_json::from_value(json.clone()).unwrap();
            json["hash"] = altered.computed_hash().to_string().into();
        }
        forged[block] = serde_json::from_value(json).unwrap();

        let before = ours.blocks().to_vec();
        prop_assert!(!validate_blocks(&forged, &POW));
        prop_assert_eq!(ours.adopt_if_heavier(forged), Err(ChainError::InvalidChain));
        prop_assert_eq!(ours.blocks(), &before[..]);
    }
}
