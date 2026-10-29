use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use ed25519_dalek::SigningKey;

use super::address::Address;
use super::crypto;
use super::difficulty::meets_work;
use super::merkle;
use super::rules::{ChainRules, Consensus};
use super::values::{BlockHash, Coins, Destination, MerkleRoot, Source};

// Value objects of the domain. They derive serde because their JSON form is also how
// blocks are hashed, stored and exchanged between nodes.

// The names a transaction's sender and recipient take when they aren't accounts. They live with
// `Source` and `Destination`; these keep their old paths working.
pub use super::values::{COINBASE, STAKE, UNSTAKE};

/// How many nonces a miner tries between asking whether to keep going.
const CHECK_EVERY: u64 = 1 << 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxKind {
    Coinbase,
    Transfer,
    Stake,
    Unstake,
}

/// A signed movement of coins. Addresses are hex-encoded ed25519 public keys. Staking is a
/// transaction to `Destination::Stake` or `Unstake`, so the signature covers what kind it is.
///
/// Only the domain changes one; everywhere else it can only be read:
///
/// ```compile_fail
/// # fn tamper(tx: &mut blockchain::domain::block::Transaction) {
/// tx.amount = blockchain::domain::values::Coins::new(1_000);
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub(in crate::domain) from: Source,
    pub(in crate::domain) to: Destination,
    pub(in crate::domain) amount: Coins,
    /// How many transactions the sender has made before this one. Stops replays.
    pub(in crate::domain) nonce: u64,
    /// Hex-encoded ed25519 signature over `signing_payload()`. Empty for coinbase.
    pub(in crate::domain) signature: String,
    /// Optional tip for whoever adds the block, paid on top of `amount`. Left out when zero, so
    /// transactions without one keep their original JSON and signature.
    #[serde(default, skip_serializing_if = "Coins::is_zero")]
    pub(in crate::domain) fee: Coins,
    /// Stakes only: the end of the staker's reveal hash chain, needed the first time they stake
    /// (see `randao`). Left out otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::domain) commitment: Option<String>,
}

impl Transaction {
    // Read access for code outside the domain, which may look at a transaction but not change it.

    pub fn from(&self) -> &Source {
        &self.from
    }

    pub fn to(&self) -> &Destination {
        &self.to
    }

    pub fn amount(&self) -> Coins {
        self.amount
    }

    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    /// Empty for a block reward.
    pub fn signature(&self) -> &str {
        &self.signature
    }

    pub fn fee(&self) -> Coins {
        self.fee
    }

    pub fn commitment(&self) -> Option<&str> {
        self.commitment.as_deref()
    }

    pub fn signing_payload(
        from: &str,
        to: &str,
        amount: Coins,
        nonce: u64,
        fee: Coins,
        commitment: Option<&str>,
    ) -> Vec<u8> {
        let mut payload = format!("{from}|{to}|{amount}|{nonce}");
        if !fee.is_zero() {
            payload.push_str(&format!("|fee={fee}"));
        }
        if let Some(commitment) = commitment {
            payload.push_str(&format!("|commit={commitment}"));
        }
        payload.into_bytes()
    }

    /// A transfer (or unstake, or later stake, by `to`) from `key`'s address, signed with it.
    pub fn signed(
        key: &SigningKey,
        to: impl Into<Destination>,
        amount: Coins,
        nonce: u64,
        fee: Coins,
    ) -> Self {
        Self::signed_with(key, to.into(), amount, nonce, fee, None)
    }

    /// A stake from `key`'s address. `commitment` is required the first time, and refused
    /// while an earlier one is still in use.
    pub fn stake(
        key: &SigningKey,
        amount: Coins,
        nonce: u64,
        fee: Coins,
        commitment: Option<String>,
    ) -> Self {
        Self::signed_with(key, Destination::Stake, amount, nonce, fee, commitment)
    }

    fn signed_with(
        key: &SigningKey,
        to: Destination,
        amount: Coins,
        nonce: u64,
        fee: Coins,
        commitment: Option<String>,
    ) -> Self {
        let from = Source::Account(Address::of(key));
        let (sender, recipient) = (from.to_string(), to.to_string());
        let payload =
            Self::signing_payload(&sender, &recipient, amount, nonce, fee, commitment.as_deref());
        let signature = crypto::sign(key, &payload);
        Self { from, to, amount, nonce, signature, fee, commitment }
    }

    pub fn payload(&self) -> Vec<u8> {
        Self::signing_payload(
            &self.from.to_string(),
            &self.to.to_string(),
            self.amount,
            self.nonce,
            self.fee,
            self.commitment.as_deref(),
        )
    }

    pub fn is_coinbase(&self) -> bool {
        self.from == Source::NewCoins
    }

    /// The account that sent it, unless it's a block reward.
    pub fn sender(&self) -> Option<&Address> {
        self.from.account()
    }

    /// Whether `address` sends or receives it.
    pub fn involves(&self, address: &Address) -> bool {
        self.from.account() == Some(address) || self.to.account() == Some(address)
    }

    pub fn kind(&self) -> TxKind {
        match (&self.from, &self.to) {
            (Source::NewCoins, _) => TxKind::Coinbase,
            (_, Destination::Stake) => TxKind::Stake,
            (_, Destination::Unstake) => TxKind::Unstake,
            (_, Destination::Account(_)) => TxKind::Transfer,
        }
    }

    /// The payout for adding block `height`: the reward plus the block's fees. Its nonce is the
    /// height, so every one is unique.
    pub fn coinbase(producer: &Address, amount: Coins, height: u64) -> Self {
        Self {
            from: Source::NewCoins,
            to: Destination::Account(producer.clone()),
            amount,
            nonce: height,
            signature: String::new(),
            fee: Coins::ZERO,
            commitment: None,
        }
    }
}

/// A block of transactions. Only the domain changes one; everywhere else it can only be read:
///
/// ```compile_fail
/// # fn tamper(block: &mut blockchain::domain::block::Block) {
/// block.nonce += 1;
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub(in crate::domain) index: u64,
    pub(in crate::domain) timestamp: DateTime<Utc>,
    pub(in crate::domain) prev_hash: BlockHash,
    pub(in crate::domain) transactions: Vec<Transaction>,
    /// The root of the Merkle tree over `transactions`. The hash covers it instead of the
    /// transactions, so a transaction can be proven to be in the block from its header alone.
    pub(in crate::domain) merkle_root: MerkleRoot,
    /// Proof-of-work counter. Always 0 under proof of stake.
    pub(in crate::domain) nonce: u64,
    pub(in crate::domain) hash: BlockHash,
    /// Proof of stake only: who forged the block. Left out of proof-of-work blocks entirely, so
    /// their JSON and hashes are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::domain) validator: Option<String>,
    /// Proof of stake only: the next link of the validator's reveal chain, mixed into the beacon
    /// that picks who forges next. Absent while nothing is staked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::domain) reveal: Option<String>,
    /// Proof of stake only: the validator's signature over `hash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::domain) signature: Option<String>,
}

impl Block {
    // Read access for code outside the domain, which may look at a block but not change it.

    pub fn index(&self) -> u64 {
        self.index
    }

    pub fn timestamp(&self) -> DateTime<Utc> {
        self.timestamp
    }

    pub fn prev_hash(&self) -> &BlockHash {
        &self.prev_hash
    }

    pub fn transactions(&self) -> &[Transaction] {
        &self.transactions
    }

    pub fn merkle_root(&self) -> &MerkleRoot {
        &self.merkle_root
    }

    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    pub fn hash(&self) -> &BlockHash {
        &self.hash
    }

    /// Proof of stake only.
    pub fn validator(&self) -> Option<&str> {
        self.validator.as_deref()
    }

    /// Proof of stake only.
    pub fn reveal(&self) -> Option<&str> {
        self.reveal.as_deref()
    }

    /// Proof of stake only.
    pub fn signature(&self) -> Option<&str> {
        self.signature.as_deref()
    }

    /// The first block. Its timestamp is fixed, so every node with the same rules has the same one.
    pub fn genesis(rules: &ChainRules) -> Self {
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is a valid time");
        match rules.consensus {
            Consensus::ProofOfWork { .. } => {
                Self::mine(0, epoch, BlockHash::zero(), Vec::new(), rules.initial_work())
            }
            Consensus::ProofOfStake => {
                let mut block = Self::unsealed(0, epoch, BlockHash::zero(), Vec::new());
                block.hash = block.computed_hash();
                block
            }
        }
    }

    /// Proof of work: tries nonces from zero until the hash meets `work`, which takes `work`
    /// tries on average.
    pub fn mine(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: BlockHash,
        transactions: Vec<Transaction>,
        work: u128,
    ) -> Self {
        Self::mine_while(index, timestamp, prev_hash, transactions, work, || true)
            .expect("mining only stops when told to")
    }

    /// Like `mine`, but asks `keep_going` before the first try and every few thousand after, and
    /// gives up when it says no, for a miner whose block has gone stale.
    pub fn mine_while(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: BlockHash,
        transactions: Vec<Transaction>,
        work: u128,
        mut keep_going: impl FnMut() -> bool,
    ) -> Option<Self> {
        let mut block = Self::unsealed(index, timestamp, prev_hash, transactions);
        loop {
            if block.nonce.is_multiple_of(CHECK_EVERY) && !keep_going() {
                return None;
            }
            block.hash = block.computed_hash();
            if meets_work(block.hash.as_str(), work) {
                return Some(block);
            }
            block.nonce += 1;
        }
    }

    /// Proof of stake: names the validator and signs the hash with their key. No work needed.
    pub fn forge(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: BlockHash,
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
        prev_hash: BlockHash,
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
        self.signature = Some(crypto::sign(key, self.hash.as_str().as_bytes()));
    }

    fn unsealed(
        index: u64,
        timestamp: DateTime<Utc>,
        prev_hash: BlockHash,
        transactions: Vec<Transaction>,
    ) -> Self {
        Self {
            index,
            timestamp,
            prev_hash,
            merkle_root: merkle::root(&transactions),
            transactions,
            nonce: 0,
            hash: BlockHash::zero(),
            validator: None,
            reveal: None,
            signature: None,
        }
    }

    /// The hash of the header: every field except `transactions`, which `merkle_root` stands
    /// for, `hash` and `signature`.
    pub fn computed_hash(&self) -> BlockHash {
        header_hash(
            self.index,
            &self.timestamp,
            &self.prev_hash,
            &self.merkle_root,
            self.nonce,
            [&self.validator, &self.reveal],
        )
    }

    /// The block without its transactions.
    pub fn header(&self) -> BlockHeader {
        BlockHeader {
            index: self.index,
            timestamp: self.timestamp,
            prev_hash: self.prev_hash.clone(),
            merkle_root: self.merkle_root.clone(),
            nonce: self.nonce,
            hash: self.hash.clone(),
            validator: self.validator.clone(),
            reveal: self.reveal.clone(),
            signature: self.signature.clone(),
        }
    }

    /// Whether `merkle_root` is the root of the transactions the block carries.
    pub fn has_matching_root(&self) -> bool {
        self.merkle_root == merkle::root(&self.transactions)
    }

    /// Whether `signature` is the named validator's signature over `hash`.
    pub fn is_signed_by_validator(&self) -> bool {
        match (&self.validator, &self.signature) {
            (Some(validator), Some(signature)) => {
                crypto::verify(validator, self.hash.as_str().as_bytes(), signature)
            }
            _ => false,
        }
    }
}

/// A block without its transactions: enough to check its hash, its proof of work or signature,
/// and Merkle proofs of what it holds. What a light client keeps instead of whole blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    pub index: u64,
    pub timestamp: DateTime<Utc>,
    pub prev_hash: BlockHash,
    pub merkle_root: MerkleRoot,
    pub nonce: u64,
    pub hash: BlockHash,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl BlockHeader {
    pub fn computed_hash(&self) -> BlockHash {
        header_hash(
            self.index,
            &self.timestamp,
            &self.prev_hash,
            &self.merkle_root,
            self.nonce,
            [&self.validator, &self.reveal],
        )
    }
}

/// `index|timestamp|prev_hash|merkle_root|nonce`, then the validator and reveal if present.
fn header_hash(
    index: u64,
    timestamp: &DateTime<Utc>,
    prev_hash: &BlockHash,
    merkle_root: &MerkleRoot,
    nonce: u64,
    extras: [&Option<String>; 2],
) -> BlockHash {
    let ts = timestamp.to_rfc3339_opts(SecondsFormat::Nanos, true);
    let mut fields = format!("{index}|{ts}|{prev_hash}|{merkle_root}|{nonce}");
    for extra in extras.into_iter().flatten() {
        fields.push('|');
        fields.push_str(extra);
    }
    BlockHash::of(fields.as_bytes())
}
