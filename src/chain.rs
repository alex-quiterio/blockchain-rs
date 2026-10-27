use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, SecondsFormat, Utc};

use crate::crypto;
use crate::model::{Block, Transaction, COINBASE};

pub const DEFAULT_DIFFICULTY: usize = 4;
pub const DEFAULT_REWARD: u64 = 50;

#[derive(Debug, PartialEq, Eq)]
pub enum ChainError {
    InvalidAddress(String),
    ZeroAmount,
    BadSignature,
    BadNonce { expected: u64, got: u64 },
    InsufficientFunds { balance: u64, amount: u64 },
    UnexpectedCoinbase,
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
        }
    }
}

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

    fn credit(&mut self, address: &str, amount: u64) {
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

pub struct Blockchain {
    blocks: Vec<Block>,
    mempool: Vec<Transaction>,
    /// State after every confirmed block.
    ledger: Ledger,
    difficulty: usize,
    reward: u64,
}

impl Blockchain {
    pub fn new(difficulty: usize, reward: u64) -> Self {
        let timestamp = DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is a valid time");
        let genesis = mine_block(0, timestamp, "0".repeat(64), Vec::new(), difficulty);
        Self {
            blocks: vec![genesis],
            mempool: Vec::new(),
            ledger: Ledger::default(),
            difficulty,
            reward,
        }
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn pending(&self) -> &[Transaction] {
        &self.mempool
    }

    pub fn balance(&self, address: &str) -> u64 {
        self.ledger.balance(address)
    }

    /// The nonce a new transaction from `address` must carry, counting the mempool.
    pub fn next_nonce(&self, address: &str) -> u64 {
        self.ledger.next_nonce(address)
            + self.mempool.iter().filter(|tx| tx.from == address).count() as u64
    }

    /// Validates a transaction against confirmed state plus the mempool, then queues it.
    pub fn submit(&mut self, tx: Transaction) -> Result<(), ChainError> {
        let mut pending = self.ledger.clone();
        for queued in &self.mempool {
            pending.apply_transfer(queued)?;
        }
        pending.apply_transfer(&tx)?;
        self.mempool.push(tx);
        Ok(())
    }

    /// Bundles the reward and every pending transaction into a new block.
    pub fn mine(&mut self, miner: &str) -> Result<&Block, ChainError> {
        if !crypto::is_valid_address(miner) {
            return Err(ChainError::InvalidAddress(miner.to_string()));
        }
        let index = self.blocks.len() as u64;
        let mut transactions = vec![Transaction {
            from: COINBASE.to_string(),
            to: miner.to_string(),
            amount: self.reward,
            nonce: index,
            signature: String::new(),
        }];
        transactions.extend(self.mempool.iter().cloned());

        let mut ledger = self.ledger.clone();
        ledger.credit(miner, self.reward);
        for tx in &transactions[1..] {
            ledger.apply_transfer(tx)?;
        }

        let prev_hash = self.blocks.last().expect("genesis exists").hash.clone();
        let block = mine_block(index, Utc::now(), prev_hash, transactions, self.difficulty);
        self.mempool.clear();
        self.ledger = ledger;
        self.blocks.push(block);
        Ok(self.blocks.last().expect("just pushed"))
    }

    /// Replays the whole chain: links, hashes, proof of work, rewards, signatures and balances.
    pub fn is_valid(&self) -> bool {
        validate_blocks(&self.blocks, self.difficulty, self.reward)
    }
}

impl Default for Blockchain {
    fn default() -> Self {
        Self::new(DEFAULT_DIFFICULTY, DEFAULT_REWARD)
    }
}

pub fn validate_blocks(blocks: &[Block], difficulty: usize, reward: u64) -> bool {
    let mut ledger = Ledger::default();
    for (i, block) in blocks.iter().enumerate() {
        if block.index != i as u64
            || block.hash != hash_block(block)
            || !meets_difficulty(&block.hash, difficulty)
        {
            return false;
        }
        if i == 0 {
            if !block.transactions.is_empty() {
                return false;
            }
            continue;
        }
        if block.prev_hash != blocks[i - 1].hash {
            return false;
        }
        let Some((coinbase, transfers)) = block.transactions.split_first() else {
            return false;
        };
        if !coinbase.is_coinbase()
            || coinbase.amount != reward
            || coinbase.nonce != block.index
            || !crypto::is_valid_address(&coinbase.to)
        {
            return false;
        }
        ledger.credit(&coinbase.to, coinbase.amount);
        if transfers.iter().any(|tx| ledger.apply_transfer(tx).is_err()) {
            return false;
        }
    }
    true
}

pub fn hash_block(block: &Block) -> String {
    block_hash(
        block.index,
        &block.timestamp,
        &block.prev_hash,
        &block.transactions,
        block.nonce,
    )
}

fn block_hash(
    index: u64,
    timestamp: &DateTime<Utc>,
    prev_hash: &str,
    transactions: &[Transaction],
    nonce: u64,
) -> String {
    let txs = serde_json::to_string(transactions).expect("transactions serialize");
    let ts = timestamp.to_rfc3339_opts(SecondsFormat::Nanos, true);
    crypto::sha256_hex(format!("{index}|{ts}|{prev_hash}|{txs}|{nonce}").as_bytes())
}

fn meets_difficulty(hash: &str, difficulty: usize) -> bool {
    hash.len() >= difficulty && hash.bytes().take(difficulty).all(|b| b == b'0')
}

fn mine_block(
    index: u64,
    timestamp: DateTime<Utc>,
    prev_hash: String,
    transactions: Vec<Transaction>,
    difficulty: usize,
) -> Block {
    let mut nonce = 0;
    loop {
        let hash = block_hash(index, &timestamp, &prev_hash, &transactions, nonce);
        if meets_difficulty(&hash, difficulty) {
            return Block { index, timestamp, prev_hash, transactions, nonce, hash };
        }
        nonce += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wallet() -> (String, ed25519_dalek::SigningKey) {
        let (address, secret) = crypto::generate_keypair();
        (address, crypto::parse_secret_key(&secret).unwrap())
    }

    fn transfer(
        key: &ed25519_dalek::SigningKey,
        to: &str,
        amount: u64,
        nonce: u64,
    ) -> Transaction {
        let from = crypto::address_of(key);
        let signature = crypto::sign(key, &Transaction::signing_payload(&from, to, amount, nonce));
        Transaction { from, to: to.to_string(), amount, nonce, signature }
    }

    fn chain() -> Blockchain {
        Blockchain::new(2, 50)
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

        let mut tx = transfer(&mallory_key, &crypto::address_of(&mallory_key), 10, 0);
        tx.from = alice;
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
        assert!(!validate_blocks(&blocks, 2, 50));

        let mut blocks = chain.blocks().to_vec();
        blocks[1].transactions[0].amount = 1_000;
        blocks[1].hash = hash_block(&blocks[1]);
        assert!(!validate_blocks(&blocks, 2, 50));
    }
}
