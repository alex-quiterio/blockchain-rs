//! Property-based tests: random transaction orderings and random forks, checked against the
//! invariants every node relies on.

use std::sync::Arc;

use std::cmp::Reverse;
use std::collections::HashSet;

use ed25519_dalek::SigningKey;
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

use blockchain::domain::accounting::transaction::Transaction;
use blockchain::domain::accounting::TransactionError;
use blockchain::domain::accounting::{Coins, Destination};
use blockchain::domain::chain::block::BlockError;
use blockchain::domain::chain::block::BlockHash;
use blockchain::domain::chain::block::{owned, Block};
use blockchain::domain::chain::ChainError;
use blockchain::domain::chain::{validate_blocks, Blockchain, ChainEvent};
use blockchain::domain::consensus::randao;
use blockchain::domain::consensus::rules::ChainRules;
use blockchain::domain::consensus::BlockCapacity;
use blockchain::domain::crypto::address::Address;
use blockchain::domain::mempool::Mempool;

/// A chain with a mempool beside it, coordinated as a node does, so a property can submit and
/// mine in one place. Reads go to the chain itself; what touches what's waiting is here.
#[derive(Clone)]
struct Pooled {
    chain: Blockchain,
    pool: Mempool,
}

impl std::ops::Deref for Pooled {
    type Target = Blockchain;

    fn deref(&self) -> &Blockchain {
        &self.chain
    }
}

impl From<Blockchain> for Pooled {
    fn from(chain: Blockchain) -> Self {
        let pool = Mempool::on(chain.ledger());
        Self { chain, pool }
    }
}

impl Pooled {
    fn new(rules: ChainRules) -> Self {
        Blockchain::new(rules).into()
    }

    fn restore(blocks: Vec<Block>, rules: ChainRules) -> Result<Self, ChainError> {
        Blockchain::restore(blocks, rules).map(Self::from)
    }

    fn submit(&mut self, tx: Transaction) -> Result<(), TransactionError> {
        self.pool.admit(tx)
    }

    fn pending(&self) -> &[Transaction] {
        self.pool.transactions()
    }

    fn next_nonce(&self, address: &Address) -> u64 {
        self.pool.next_nonce(address)
    }

    fn commitment_due(&self, address: &Address) -> Option<u64> {
        self.pool.commitment_due(address)
    }

    /// What the mempool offers for the next block, as a node asks for it.
    fn candidates(&self) -> Vec<Transaction> {
        self.pool.candidates(self.chain.rules().block_capacity)
    }

    fn mine(&mut self, miner: &Address) -> Result<&Block, BlockError> {
        let pending = self.candidates();
        self.chain.mine(miner, &pending)?;
        self.settle();
        Ok(self.chain.tip())
    }

    fn forge(&mut self, key: &SigningKey) -> Result<&Block, BlockError> {
        let pending = self.candidates();
        self.chain.forge(key, &pending)?;
        self.settle();
        Ok(self.chain.tip())
    }

    fn adopt_if_heavier(&mut self, blocks: Vec<Block>) -> Result<bool, ChainError> {
        let adopted = self.chain.adopt_if_heavier(blocks)?;
        self.settle();
        Ok(adopted)
    }

    /// Rebases the mempool on what the last change did.
    fn settle(&mut self) {
        let mut returned = Vec::new();
        for event in self.chain.take_events() {
            if let ChainEvent::Reorganized { returned: back, .. } = event {
                returned.extend(back);
            }
        }
        self.pool.rebase(self.chain.ledger(), returned);
    }
}

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

/// A transfer to try: from a wallet, to a wallet or (at `WALLETS`) someone who never sends, of
/// an amount, with a fee.
fn transfer() -> impl Strategy<Value = (usize, usize, u64, u64)> {
    (0..WALLETS, 0..=WALLETS, 1..40u64, 0..10u64)
}

/// Someone with a block's reward to spend and nothing waiting, who arrives after the rest.
const NEWCOMER: usize = WALLETS + 3;

/// A chain whose blocks hold at most `capacity` transfers, with a block mined for the newcomer
/// and one for each of `funded`, and `transfers` queued on top. Those that don't apply are
/// turned away, as a node would; those that spend coins still on their way wait behind the
/// payment bringing them.
fn queued(
    capacity: BlockCapacity,
    funded: &[usize],
    transfers: &[(usize, usize, u64, u64)],
) -> Pooled {
    let mut chain = Pooled::new(POW.with_block_capacity(capacity));
    chain.mine(&address(NEWCOMER)).unwrap();
    for &wallet in funded {
        chain.mine(&address(wallet)).unwrap();
    }
    for &(from, to, amount, fee) in transfers {
        let to = address(if to == WALLETS { WALLETS + 2 } else { to });
        let nonce = chain.next_nonce(&address(from));
        let tx = Transaction::signed(
            &key(from),
            chain.ledger().chain(),
            &to,
            Coins::new(amount),
            nonce,
            Coins::new(fee),
        );
        let _ = chain.submit(tx);
    }
    chain
}

/// Coins only enter the chain as block rewards, and fees only move them around.
fn assert_supply_conserved(chain: &Pooled) {
    let held: u64 = (0..WALLETS)
        .map(|i| chain.balance(&address(i)).value() + chain.staked(&address(i)).value())
        .sum();
    assert_eq!(held, REWARD * (chain.blocks().len() as u64 - 1), "coins appeared or vanished");
}

/// Each sender's next nonce is exactly how many of its transactions are confirmed or queued.
fn assert_nonces_match_history(chain: &Pooled) {
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
fn add_block(chain: &mut Pooled, by: usize) {
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
fn run(rules: ChainRules, ops: &[Op]) -> Pooled {
    let mut chain = Pooled::new(rules);
    for op in ops {
        let tx = match *op {
            Op::Pay { from, to, amount, fee, skew } => {
                let nonce = (chain.next_nonce(&address(from)) as i64 + skew).max(0) as u64;
                Some(Transaction::signed(
                    &key(from),
                    chain.ledger().chain(),
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
                    chain.ledger().chain(),
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
                    chain.ledger().chain(),
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
fn deliver(rules: ChainRules, funding: &[Arc<Block>], batch: &[Transaction]) -> Pooled {
    let mut chain = Pooled::restore(owned(funding), rules).unwrap();
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
        let restored = Pooled::restore(owned(chain.blocks()), POW).unwrap();
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
        let restored = Pooled::restore(owned(chain.blocks()), POS).unwrap();
        prop_assert_eq!(restored.stakes(), chain.stakes());
        prop_assert_eq!(restored.next_validator(), chain.next_validator());
    }

    #[test]
    fn the_order_transactions_arrive_in_does_not_change_the_outcome(
        payments in prop::collection::vec((0..WALLETS, 0..WALLETS, 1..70u64, 0..4u64), 1..16),
        seed in any::<u64>(),
    ) {
        // Every wallet starts with one block reward.
        let mut funded = Pooled::new(POW);
        for i in 0..WALLETS {
            funded.mine(&address(i)).unwrap();
        }
        // A batch with correct, consecutive nonces per sender; some may never be affordable.
        let mut nonces = [0u64; WALLETS];
        let batch: Vec<Transaction> = payments
            .iter()
            .map(|&(from, to, amount, fee)| {
                nonces[from] += 1;
                Transaction::signed(&key(from), funded.ledger().chain(), address(to), Coins::new(amount), nonces[from] - 1, Coins::new(fee))
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
        let mut base = Pooled::new(POW);
        for i in 0..3 {
            base.mine(&address(i)).unwrap();
        }
        // Each fork branches off the base at some height and mines its own blocks on top.
        let mut candidates = vec![base.clone()];
        for &(height, extra, miner) in &forks {
            let mut fork = Pooled::restore(owned(&base.blocks()[..=height]), POW).unwrap();
            for _ in 0..extra {
                fork.mine(&address(miner)).unwrap();
            }
            candidates.push(fork);
        }
        let winner = candidates
            .iter()
            .max_by_key(|c| (c.weight(), Reverse(c.tip().hash().clone())))
            .unwrap();

        let mut node = Pooled::new(POW);
        for &i in &order {
            node.adopt_if_heavier(owned(candidates[i].blocks())).unwrap();
        }
        prop_assert_eq!(&node.tip().hash(), &winner.tip().hash());
        for candidate in &candidates {
            prop_assert_eq!(node.adopt_if_heavier(owned(candidate.blocks())), Ok(false), "settled for good");
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
        let mut chain = Pooled::new(POS);
        for i in 0..stakes.len() {
            chain.forge(&key(i)).unwrap();
        }
        for (i, &amount) in stakes.iter().enumerate() {
            let commitment = chain.commitment_due(&address(i)).map(|e| randao::commitment(&key(i), e));
            chain.submit(Transaction::stake(&key(i), chain.ledger().chain(), Coins::new(amount), 0, Coins::new(0), commitment)).unwrap();
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
                let tx = Transaction::signed(&key(forger), attempt.ledger().chain(), address(to), Coins::new(amount), nonce, Coins::new(fee));
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
        let mut chain = Pooled::new(rules);
        let mut at = chrono::Utc::now() - chrono::TimeDelta::days(1);
        for (i, gap) in gaps.iter().enumerate() {
            at += chrono::TimeDelta::seconds(*gap) + chrono::TimeDelta::milliseconds(1);
            let work = chain.next_work();
            let (index, prev) = (chain.blocks().len() as u64, chain.tip().hash().clone());
            let payout = vec![Transaction::coinbase(&address(i % WALLETS), Coins::new(REWARD), index)];
            let mut blocks = owned(chain.blocks());
            blocks.push(Block::mine(index, at, prev, payout, work));
            chain = Pooled::restore(blocks, rules).expect("every honestly mined block is valid");
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
        let mut ours = Pooled::new(POW);
        ours.mine(&address(0)).unwrap();
        let mut theirs = Pooled::new(POW);
        for i in 0..5 + extra {
            theirs.mine(&address(i % WALLETS)).unwrap();
        }
        // Outside the domain a block can't be changed, only rewritten as JSON, as a peer would.
        let mut forged = owned(theirs.blocks());
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

        let before = owned(ours.blocks());
        // Refused at the altered block, or at the next one whose link it broke, and said so.
        let refused = validate_blocks(&forged, &POW).unwrap_err();
        let at = |height: u64| height == block as u64 || height == block as u64 + 1;
        prop_assert!(matches!(refused, ChainError::InvalidBlock { height, .. } if at(height)), "{}", refused);
        prop_assert_eq!(ours.adopt_if_heavier(forged), Err(refused));
        prop_assert_eq!(owned(ours.blocks()), before);
    }

    #[test]
    fn a_full_block_takes_the_highest_fees_it_can(
        funded in prop::collection::vec(0..WALLETS, 1..6),
        transfers in prop::collection::vec(transfer(), 0..30),
        capacity in 1..32u64,
    ) {
        let capacity = BlockCapacity::at_most(capacity);
        let mut chain = queued(capacity, &funded, &transfers);
        let waiting = chain.pending().to_vec();
        let chosen = chain.candidates();

        // As many as a block holds, never fewer; while everything fits, in the order it arrived.
        prop_assert_eq!(chosen.len(), waiting.len().min(capacity.limit()));
        if capacity.holds(waiting.len()) {
            prop_assert_eq!(&chosen, &waiting);
        }

        // Step by step, each chosen transfer applies on what was chosen before it. When more is
        // waiting than fits, of all that could have gone in at that step instead, none pays
        // more, nor as much having arrived earlier. Trying every one still waiting, not only each sender's next, keeps
        // this independent of how the selection is worked out: the ledger refuses the rest.
        let arrived = |tx: &Transaction| waiting.iter().position(|w| w == tx);
        let rank = |tx: &Transaction| (tx.fee(), Reverse(arrived(tx)));
        let full = !capacity.holds(waiting.len());
        let mut ledger = chain.ledger().clone();
        let mut taken = HashSet::new();
        for tx in &chosen {
            prop_assert!(arrived(tx).is_some(), "offered a transfer that isn't waiting");
            for (position, other) in waiting.iter().enumerate() {
                let could_go_in = full
                    && !taken.contains(&position)
                    && ledger.clone().apply_transfer(other, chrono::Utc::now()).is_ok();
                prop_assert!(!could_go_in || rank(other) <= rank(tx), "{:?} could have gone in ahead of {:?}", other, tx);
            }
            prop_assert!(ledger.apply_transfer(tx, chrono::Utc::now()).is_ok(), "a chosen transfer doesn't apply");
            prop_assert!(taken.insert(arrived(tx).unwrap()), "chosen twice");
        }

        // The block mined from them holds exactly those, and pays their fees.
        let fees = Coins::total(chosen.iter().map(Transaction::fee)).unwrap();
        let block = chain.mine(&address(0)).unwrap();
        prop_assert_eq!(&block.transactions()[1..], &chosen[..]);
        prop_assert_eq!(block.transactions()[0].amount(), Coins::new(REWARD).checked_add(fees).unwrap());
    }

    #[test]
    fn paying_the_estimated_fee_is_exactly_enough_for_the_next_block(
        funded in prop::collection::vec(0..WALLETS, 1..6),
        transfers in prop::collection::vec(transfer(), 0..30),
        capacity in 1..32u64,
    ) {
        let capacity = BlockCapacity::at_most(capacity);
        let chain = queued(capacity, &funded, &transfers);
        let estimate = chain.pool.fee_estimate(capacity);
        let newcomer_pays = |fee: Coins| {
            let mut chain = chain.clone();
            let tx = Transaction::signed(&key(NEWCOMER), chain.ledger().chain(), address(0), Coins::new(1), 0, fee);
            chain.submit(tx.clone()).unwrap();
            chain.mine(&address(0)).unwrap().transactions().contains(&tx)
        };
        prop_assert!(newcomer_pays(estimate.fee), "paying {} didn't get in", estimate.fee);
        if let Some(less) = estimate.fee.checked_sub(Coins::new(1)) {
            prop_assert_eq!(estimate.room, 0, "a fee is only asked for once the block is full");
            prop_assert!(!newcomer_pays(less), "{} was enough, but {} was asked for", less, estimate.fee);
        }
    }
}
