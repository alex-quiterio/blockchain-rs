use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use ed25519_dalek::SigningKey;

use super::address::Address;
use super::crypto;
use super::rules::{meets_difficulty, ChainRules, Consensus};

// Value objects of the domain. They derive serde because their JSON form is also how
// blocks are hashed, stored and exchanged between nodes.

/// Sender used for the block reward, which has no signature.
pub const COINBASE: &str = "coinbase";
/// Recipient that locks coins up as stake, for proof of stake.
pub const STAKE: &str = "stake";
/// Recipient that unlocks staked coins back into the sender's balance.
pub const UNSTAKE: &str = "unstake";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxKind {
    Coinbase,
    Transfer,
    Stake,
    Unstake,
}

/// A signed movement of coins. Addresses are hex-encoded ed25519 public keys. Staking is a
/// transaction to `STAKE` or `UNSTAKE`, so the signature covers what kind it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub from: String,
    pub to: String,
    pub amount: u64,
    /// How many transactions the sender has made before this one. Stops replays.
    pub nonce: u64,
    /// Hex-encoded ed25519 signature over `signing_payload()`. Empty for coinbase.
    pub signature: String,
    /// Optional tip for whoever adds the block, paid on top of `amount`. Left out when zero, so
    /// transactions without one keep their original JSON and signature.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fee: u64,
    /// Stakes only: the end of the staker's reveal hash chain, needed the first time they stake
    /// (see `randao`). Left out otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commitment: Option<String>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl Transaction {
    pub fn signing_payload(
        from: &str,
        to: &str,
        amount: u64,
        nonce: u64,
        fee: u64,
        commitment: Option<&str>,
    ) -> Vec<u8> {
        let mut payload = format!("{from}|{to}|{amount}|{nonce}");
        if fee > 0 {
            payload.push_str(&format!("|fee={fee}"));
        }
        if let Some(commitment) = commitment {
            payload.push_str(&format!("|commit={commitment}"));
        }
        payload.into_bytes()
    }

    /// A transfer (or unstake, or later stake, by `to`) from `key`'s address, signed with it.
    pub fn signed(key: &SigningKey, to: &str, amount: u64, nonce: u64, fee: u64) -> Self {
        Self::signed_with(key, to, amount, nonce, fee, None)
    }

    /// A stake from `key`'s address. `commitment` is required the first time, and refused
    /// while an earlier one is still in use.
    pub fn stake(
        key: &SigningKey,
        amount: u64,
        nonce: u64,
        fee: u64,
        commitment: Option<String>,
    ) -> Self {
        Self::signed_with(key, STAKE, amount, nonce, fee, commitment)
    }

    fn signed_with(
        key: &SigningKey,
        to: &str,
        amount: u64,
        nonce: u64,
        fee: u64,
        commitment: Option<String>,
    ) -> Self {
        let from = Address::of(key).to_string();
        let payload = Self::signing_payload(&from, to, amount, nonce, fee, commitment.as_deref());
        let signature = crypto::sign(key, &payload);
        Self { from, to: to.to_string(), amount, nonce, signature, fee, commitment }
    }

    pub fn payload(&self) -> Vec<u8> {
        Self::signing_payload(
            &self.from,
            &self.to,
            self.amount,
            self.nonce,
            self.fee,
            self.commitment.as_deref(),
        )
    }

    pub fn is_coinbase(&self) -> bool {
        self.from == COINBASE
    }

    pub fn kind(&self) -> TxKind {
        match (self.from.as_str(), self.to.as_str()) {
            (COINBASE, _) => TxKind::Coinbase,
            (_, STAKE) => TxKind::Stake,
            (_, UNSTAKE) => TxKind::Unstake,
            _ => TxKind::Transfer,
        }
    }

    /// The payout for adding block `height`: the reward plus the block's fees. Its nonce is the
    /// height, so every one is unique.
    pub fn coinbase(producer: &Address, amount: u64, height: u64) -> Self {
        Self {
            from: COINBASE.to_string(),
            to: producer.to_string(),
            amount,
            nonce: height,
            signature: String::new(),
            fee: 0,
            commitment: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub index: u64,
    pub timestamp: DateTime<Utc>,
    pub prev_hash: String,
    pub transactions: Vec<Transaction>,
    /// Proof-of-work counter. Always 0 under proof of stake.
    pub nonce: u64,
    pub hash: String,
    /// Proof of stake only: who forged the block. Left out of proof-of-work blocks entirely, so
    /// their JSON and hashes are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator: Option<String>,
    /// Proof of stake only: the next link of the validator's reveal chain, mixed into the beacon
    /// that picks who forges next. Absent while nothing is staked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal: Option<String>,
    /// Proof of stake only: the validator's signature over `hash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl Block {
    /// The first block. Its timestamp is fixed, so every node with the same rules has the same one.
    pub fn genesis(rules: &ChainRules) -> Self {
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is a valid time");
        match rules.consensus {
            Consensus::ProofOfWork { difficulty } => {
                Self::mine(0, epoch, "0".repeat(64), Vec::new(), difficulty)
            }
            Consensus::ProofOfStake => {
                let mut block = Self::unsealed(0, epoch, "0".repeat(64), Vec::new());
                block.hash = block.computed_hash();
                block
            }
        }
    }

    /// Proof of work: tries nonces from zero until the hash starts with `difficulty` zeros.
    pub fn mine(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: String,
        transactions: Vec<Transaction>,
        difficulty: usize,
    ) -> Self {
        let mut block = Self::unsealed(index, timestamp, prev_hash, transactions);
        loop {
            block.hash = block.computed_hash();
            if meets_difficulty(&block.hash, difficulty) {
                return block;
            }
            block.nonce += 1;
        }
    }

    /// Proof of stake: names the validator and signs the hash with their key. No work needed.
    pub fn forge(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: String,
        transactions: Vec<Transaction>,
        key: &SigningKey,
        reveal: Option<String>,
    ) -> Self {
        let validator = Address::of(key);
        let mut block =
            Self::propose(index, timestamp, prev_hash, transactions, &validator, reveal);
        block.sign(key);
        block
    }

    /// Proof of stake, step one: a block naming `validator`, hashed but not yet signed. Lets the
    /// validator sign somewhere else, such as in a browser, without handing over their key.
    pub fn propose(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: String,
        transactions: Vec<Transaction>,
        validator: &Address,
        reveal: Option<String>,
    ) -> Self {
        let mut block = Self::unsealed(index, timestamp, prev_hash, transactions);
        block.validator = Some(validator.to_string());
        block.reveal = reveal;
        block.hash = block.computed_hash();
        block
    }

    /// Proof of stake, step two: the validator signs the hash.
    pub fn sign(&mut self, key: &SigningKey) {
        self.signature = Some(crypto::sign(key, self.hash.as_bytes()));
    }

    fn unsealed(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: String,
        transactions: Vec<Transaction>,
    ) -> Self {
        Self {
            index,
            timestamp,
            prev_hash,
            transactions,
            nonce: 0,
            hash: String::new(),
            validator: None,
            reveal: None,
            signature: None,
        }
    }

    /// The hash of every field except `hash` and `signature`.
    pub fn computed_hash(&self) -> String {
        let txs = serde_json::to_string(&self.transactions).expect("transactions serialize");
        let ts = self.timestamp.to_rfc3339_opts(SecondsFormat::Nanos, true);
        let (index, prev, nonce) = (self.index, &self.prev_hash, self.nonce);
        let mut fields = format!("{index}|{ts}|{prev}|{txs}|{nonce}");
        for extra in [&self.validator, &self.reveal].into_iter().flatten() {
            fields.push('|');
            fields.push_str(extra);
        }
        crypto::sha256_hex(fields.as_bytes())
    }

    /// Whether `signature` is the named validator's signature over `hash`.
    pub fn is_signed_by_validator(&self) -> bool {
        match (&self.validator, &self.signature) {
            (Some(validator), Some(signature)) => {
                crypto::verify(validator, self.hash.as_bytes(), signature)
            }
            _ => false,
        }
    }
}
