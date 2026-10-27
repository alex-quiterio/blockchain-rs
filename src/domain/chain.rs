use chrono::Utc;
use std::fmt;

use super::address::Address;
use super::block::{Block, Transaction};
use super::crypto;
use super::ledger::Ledger;
use super::mempool::Mempool;
use super::rules::ChainRules;

#[derive(Debug, PartialEq, Eq)]
pub enum ChainError {
    InvalidAddress(String),
    ZeroAmount,
    BadSignature,
    BadNonce { expected: u64, got: u64 },
    InsufficientFunds { balance: u64, amount: u64 },
    UnexpectedCoinbase,
    InvalidChain,
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAddress(a) => write!(f, "invalid address: {a}"),
            Self::ZeroAmount => write!(f, "amount must be greater than zero"),
            Self::BadSignature => write!(f, "signature does not match sender"),
            Self::BadNonce { expected, got } => write!(f, "nonce must be {expected}, got {got}"),
            Self::InsufficientFunds { balance, amount } => {
                write!(f, "balance {balance} is less than {amount}")
            }
            Self::UnexpectedCoinbase => write!(f, "coinbase transactions are only made by mining"),
            Self::InvalidChain => write!(f, "chain does not validate"),
        }
    }
}

impl std::error::Error for ChainError {}

/// The aggregate root: confirmed blocks, the mempool and the ledger they produce.
/// Every change goes through its methods, so the three never disagree.
#[derive(Clone)]
pub struct Blockchain {
    blocks: Vec<Block>,
    mempool: Mempool,
    /// State after every confirmed block.
    ledger: Ledger,
    rules: ChainRules,
}

impl Blockchain {
    pub fn new(rules: ChainRules) -> Self {
        Self {
            blocks: vec![Block::genesis(&rules)],
            mempool: Mempool::default(),
            ledger: Ledger::default(),
            rules,
        }
    }

    /// Rebuilds a chain from saved blocks. They are replayed in full, so tampered ones are refused.
    pub fn restore(blocks: Vec<Block>, rules: ChainRules) -> Result<Self, ChainError> {
        let ledger = replay(&blocks, &rules).ok_or(ChainError::InvalidChain)?;
        Ok(Self { blocks, mempool: Mempool::default(), ledger, rules })
    }

    pub fn rules(&self) -> ChainRules {
        self.rules
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn tip(&self) -> &Block {
        self.blocks.last().expect("a chain always has its genesis block")
    }

    pub fn pending(&self) -> &[Transaction] {
        self.mempool.transactions()
    }

    pub fn balance(&self, address: &Address) -> u64 {
        self.ledger.balance(address.as_str())
    }

    /// The nonce a new transaction from `address` must carry, counting the mempool.
    pub fn next_nonce(&self, address: &Address) -> u64 {
        self.ledger.next_nonce(address.as_str()) + self.mempool.sent_by(address.as_str())
    }

    /// Validates a transaction against confirmed state plus the mempool, then queues it.
    pub fn submit(&mut self, tx: Transaction) -> Result<(), ChainError> {
        self.mempool.admit(tx, &self.ledger)
    }

    /// Bundles the reward and every pending transaction into a new block.
    pub fn mine(&mut self, miner: &Address) -> Result<&Block, ChainError> {
        let index = self.blocks.len() as u64;
        let mut transactions = vec![Transaction::coinbase(miner, self.rules.reward, index)];
        transactions.extend(self.mempool.transactions().iter().cloned());

        let mut ledger = self.ledger.clone();
        ledger.credit(miner.as_str(), self.rules.reward);
        for tx in &transactions[1..] {
            ledger.apply_transfer(tx)?;
        }

        let prev_hash = self.tip().hash.clone();
        self.blocks.push(Block::mine(index, Utc::now(), prev_hash, transactions, &self.rules));
        self.mempool.clear();
        self.ledger = ledger;
        Ok(self.tip())
    }

    /// The longest-chain rule: adopts `blocks` if it is valid and longer than ours.
    /// Every block carries the same work, so the longest chain is the one with the most work.
    /// Transfers from our blocks that `blocks` drops go back to the mempool if they still apply.
    pub fn adopt_if_longer(&mut self, blocks: Vec<Block>) -> Result<bool, ChainError> {
        if blocks.len() <= self.blocks.len() {
            return Ok(false);
        }
        let ledger = replay(&blocks, &self.rules).ok_or(ChainError::InvalidChain)?;

        let shared = self
            .blocks
            .iter()
            .zip(&blocks)
            .take_while(|(ours, theirs)| ours.hash == theirs.hash)
            .count();
        let orphaned: Vec<Transaction> = self.blocks[shared..]
            .iter()
            .flat_map(|block| block.transactions.iter().skip(1))
            .chain(self.mempool.transactions())
            .cloned()
            .collect();

        self.mempool = Mempool::rebuilt(orphaned, &ledger);
        self.blocks = blocks;
        self.ledger = ledger;
        Ok(true)
    }

    /// Replays the whole chain: links, hashes, proof of work, rewards, signatures and balances.
    pub fn is_valid(&self) -> bool {
        validate_blocks(&self.blocks, &self.rules)
    }
}

pub fn validate_blocks(blocks: &[Block], rules: &ChainRules) -> bool {
    replay(blocks, rules).is_some()
}

/// Checks every block in order and returns the resulting ledger, or `None` if any check fails.
fn replay(blocks: &[Block], rules: &ChainRules) -> Option<Ledger> {
    if blocks.is_empty() {
        return None;
    }
    let mut ledger = Ledger::default();
    for (i, block) in blocks.iter().enumerate() {
        if block.index != i as u64
            || block.hash != block.computed_hash()
            || !rules.is_met_by(&block.hash)
        {
            return None;
        }
        if i == 0 {
            if !block.transactions.is_empty() {
                return None;
            }
            continue;
        }
        if block.prev_hash != blocks[i - 1].hash {
            return None;
        }
        let (coinbase, transfers) = block.transactions.split_first()?;
        if !coinbase.is_coinbase()
            || coinbase.amount != rules.reward
            || coinbase.nonce != block.index
            || !crypto::is_valid_address(&coinbase.to)
        {
            return None;
        }
        ledger.credit(&coinbase.to, coinbase.amount);
        if transfers.iter().any(|tx| ledger.apply_transfer(tx).is_err()) {
            return None;
        }
    }
    Some(ledger)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: ChainRules = ChainRules::new(2, 50);

    fn wallet() -> (Address, ed25519_dalek::SigningKey) {
        let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
        (Address::of(&key), key)
    }

    fn transfer(
        key: &ed25519_dalek::SigningKey,
        to: &Address,
        amount: u64,
        nonce: u64,
    ) -> Transaction {
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

    fn chain() -> Blockchain {
        Blockchain::new(RULES)
    }

    #[test]
    fn mining_pays_the_reward() {
        let mut chain = chain();
        let (miner, _) = wallet();
        let block = chain.mine(&miner).unwrap();
        assert!(block.hash.starts_with("00"));
        assert_eq!(chain.balance(&miner), 50);
        assert!(chain.is_valid());
    }

    #[test]
    fn transfer_moves_coins_once_mined() {
        let mut chain = chain();
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        chain.mine(&alice).unwrap();

        chain.submit(transfer(&alice_key, &bob, 20, 0)).unwrap();
        assert_eq!(chain.balance(&bob), 0);
        assert_eq!(chain.next_nonce(&alice), 1);

        chain.mine(&bob).unwrap();
        assert_eq!(chain.balance(&alice), 30);
        assert_eq!(chain.balance(&bob), 70);
        assert!(chain.pending().is_empty());
        assert!(chain.is_valid());
    }

    #[test]
    fn rejects_forged_signature() {
        let mut chain = chain();
        let (alice, _) = wallet();
        let (_, mallory_key) = wallet();
        chain.mine(&alice).unwrap();

        let mut tx = transfer(&mallory_key, &Address::of(&mallory_key), 10, 0);
        tx.from = alice.to_string();
        assert_eq!(chain.submit(tx), Err(ChainError::BadSignature));
    }

    #[test]
    fn rejects_overspend_across_mempool() {
        let mut chain = chain();
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        chain.mine(&alice).unwrap();

        chain.submit(transfer(&alice_key, &bob, 40, 0)).unwrap();
        assert_eq!(
            chain.submit(transfer(&alice_key, &bob, 20, 1)),
            Err(ChainError::InsufficientFunds { balance: 10, amount: 20 })
        );
    }

    #[test]
    fn rejects_replayed_transaction() {
        let mut chain = chain();
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        chain.mine(&alice).unwrap();

        let tx = transfer(&alice_key, &bob, 5, 0);
        chain.submit(tx.clone()).unwrap();
        chain.mine(&bob).unwrap();
        assert_eq!(chain.submit(tx), Err(ChainError::BadNonce { expected: 1, got: 0 }));
    }

    #[test]
    fn tampering_breaks_validity() {
        let mut chain = chain();
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        chain.mine(&alice).unwrap();
        chain.submit(transfer(&alice_key, &bob, 5, 0)).unwrap();
        chain.mine(&alice).unwrap();

        let mut blocks = chain.blocks().to_vec();
        blocks[2].transactions[1].amount = 50;
        assert!(!validate_blocks(&blocks, &RULES));

        let mut blocks = chain.blocks().to_vec();
        blocks[1].transactions[0].amount = 1_000;
        blocks[1].hash = blocks[1].computed_hash();
        assert!(!validate_blocks(&blocks, &RULES));
    }

    #[test]
    fn restore_rebuilds_balances() {
        let mut chain = chain();
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        chain.mine(&alice).unwrap();
        chain.submit(transfer(&alice_key, &bob, 20, 0)).unwrap();
        chain.mine(&alice).unwrap();

        let restored = Blockchain::restore(chain.blocks().to_vec(), RULES).unwrap();
        assert_eq!(restored.balance(&alice), 80);
        assert_eq!(restored.balance(&bob), 20);
        assert_eq!(restored.next_nonce(&alice), 1);
    }

    #[test]
    fn restore_refuses_tampered_or_empty_blocks() {
        let mut chain = chain();
        chain.mine(&wallet().0).unwrap();
        let mut blocks = chain.blocks().to_vec();
        blocks[1].transactions[0].amount = 1_000;
        assert_eq!(Blockchain::restore(blocks, RULES).err(), Some(ChainError::InvalidChain));
        assert_eq!(Blockchain::restore(Vec::new(), RULES).err(), Some(ChainError::InvalidChain));
    }

    #[test]
    fn restore_refuses_different_rules() {
        let mut chain = chain();
        chain.mine(&wallet().0).unwrap();
        // No SHA-256 hash is all zeros, so these blocks can't meet a difficulty of 64.
        assert!(Blockchain::restore(chain.blocks().to_vec(), ChainRules::new(64, 50)).is_err());
        assert!(Blockchain::restore(chain.blocks().to_vec(), ChainRules::new(2, 25)).is_err());
    }

    #[test]
    fn adopt_adopts_a_longer_valid_chain() {
        let mut ours = chain();
        let mut theirs = chain();
        let (alice, _) = wallet();
        let (bob, _) = wallet();
        ours.mine(&alice).unwrap();
        theirs.mine(&bob).unwrap();
        theirs.mine(&bob).unwrap();

        assert_eq!(ours.adopt_if_longer(theirs.blocks().to_vec()), Ok(true));
        assert_eq!(ours.blocks(), theirs.blocks());
        assert_eq!(ours.balance(&alice), 0);
        assert_eq!(ours.balance(&bob), 100);
        assert!(ours.is_valid());
    }

    #[test]
    fn adopt_keeps_ours_when_theirs_is_not_longer() {
        let mut ours = chain();
        let mut theirs = chain();
        ours.mine(&wallet().0).unwrap();
        theirs.mine(&wallet().0).unwrap();
        let before = ours.blocks().to_vec();

        assert_eq!(ours.adopt_if_longer(theirs.blocks().to_vec()), Ok(false));
        assert_eq!(ours.adopt_if_longer(vec![theirs.blocks()[0].clone()]), Ok(false));
        assert_eq!(ours.blocks(), before);
    }

    #[test]
    fn adopt_refuses_an_invalid_longer_chain() {
        let mut ours = chain();
        let mut theirs = chain();
        let (alice, _) = wallet();
        ours.mine(&alice).unwrap();
        theirs.mine(&wallet().0).unwrap();
        theirs.mine(&wallet().0).unwrap();
        let mut forged = theirs.blocks().to_vec();
        forged[2].transactions[0].amount = 1_000;

        assert_eq!(ours.adopt_if_longer(forged), Err(ChainError::InvalidChain));
        assert_eq!(ours.balance(&alice), 50);
    }

    #[test]
    fn adopt_returns_dropped_transfers_to_the_mempool() {
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        let mut ours = chain();
        ours.mine(&alice).unwrap();
        let mut theirs = ours.clone();

        // Our fork confirms Alice's payment; their longer fork never saw it.
        let payment = transfer(&alice_key, &bob, 20, 0);
        ours.submit(payment.clone()).unwrap();
        ours.mine(&alice).unwrap();
        theirs.mine(&bob).unwrap();
        theirs.mine(&bob).unwrap();

        assert_eq!(ours.adopt_if_longer(theirs.blocks().to_vec()), Ok(true));
        assert_eq!(ours.pending(), [payment]);
        assert_eq!(ours.balance(&alice), 50);
        assert_eq!(ours.next_nonce(&alice), 1);
    }

    #[test]
    fn adopt_drops_transfers_the_new_chain_already_confirmed() {
        let (alice, alice_key) = wallet();
        let (bob, _) = wallet();
        let mut ours = chain();
        ours.mine(&alice).unwrap();
        let mut theirs = ours.clone();

        let payment = transfer(&alice_key, &bob, 20, 0);
        ours.submit(payment.clone()).unwrap();
        theirs.submit(payment).unwrap();
        theirs.mine(&bob).unwrap();
        theirs.mine(&bob).unwrap();

        assert_eq!(ours.adopt_if_longer(theirs.blocks().to_vec()), Ok(true));
        assert!(ours.pending().is_empty());
        assert_eq!(ours.balance(&bob), 120);
    }
}
