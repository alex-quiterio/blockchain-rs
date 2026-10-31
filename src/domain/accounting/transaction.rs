//! A signed movement of coins, and what kind it is. Shared by both aggregates: the chain's blocks
//! carry transactions, and the mempool holds them until one does.
//!
//! A value object: it never changes once signed, and two with the same fields are the same
//! transaction. Nothing tracks one's identity over time; the chain and the mempool each hold
//! their own copy.

use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::domain::accounting::{Coins, Destination, Memo, Source};
use crate::domain::consensus::Commitment;
use crate::domain::crypto::address::Address;
use crate::domain::crypto::signature::{self, Signature};

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
/// ```
/// # fn read(tx: &blockchain::domain::accounting::transaction::Transaction) -> blockchain::domain::accounting::Coins {
/// tx.amount()
/// # }
/// ```
///
/// ```compile_fail
/// # fn tamper(tx: &mut blockchain::domain::accounting::transaction::Transaction) {
/// tx.amount = blockchain::domain::accounting::Coins::new(1_000);
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Transaction {
    pub(in crate::domain) from: Source,
    pub(in crate::domain) to: Destination,
    pub(in crate::domain) amount: Coins,
    /// How many transactions the sender has made before this one. Stops replays.
    pub(in crate::domain) nonce: u64,
    /// Hex-encoded ed25519 signature over `signing_payload()`. Empty for coinbase.
    #[serde(with = "signature::or_empty")]
    #[cfg_attr(test, ts(as = "String"))]
    pub(in crate::domain) signature: Option<Signature>,
    /// Optional tip for whoever adds the block, paid on top of `amount`. Left out when zero, so
    /// transactions without one keep their original JSON and signature.
    #[serde(default, skip_serializing_if = "Coins::is_zero")]
    pub(in crate::domain) fee: Coins,
    /// Stakes only: the end of the staker's reveal hash chain, needed the first time they stake
    /// (see `randao`). Left out otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub(in crate::domain) commitment: Option<Commitment>,
    /// The hash of a note kept off the chain, which the signature covers, so the sender vouches
    /// for it. Left out when there's none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub(in crate::domain) memo: Option<Memo>,
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

    /// `None` for a block reward.
    pub fn signature(&self) -> Option<&Signature> {
        self.signature.as_ref()
    }

    pub fn fee(&self) -> Coins {
        self.fee
    }

    pub fn commitment(&self) -> Option<&Commitment> {
        self.commitment.as_ref()
    }

    pub fn memo(&self) -> Option<&Memo> {
        self.memo.as_ref()
    }

    pub fn signing_payload(
        from: &str,
        to: &str,
        amount: Coins,
        nonce: u64,
        fee: Coins,
        commitment: Option<&str>,
        memo: Option<&str>,
    ) -> Vec<u8> {
        let mut payload = format!("{from}|{to}|{amount}|{nonce}");
        if !fee.is_zero() {
            payload.push_str(&format!("|fee={fee}"));
        }
        if let Some(commitment) = commitment {
            payload.push_str(&format!("|commit={commitment}"));
        }
        if let Some(memo) = memo {
            payload.push_str(&format!("|memo={memo}"));
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
        Self::signed_with(key, to.into(), amount, nonce, fee, None, None)
    }

    /// A transfer like `signed`'s that also carries `memo`, under the same signature.
    pub fn with_memo(
        key: &SigningKey,
        to: impl Into<Destination>,
        amount: Coins,
        nonce: u64,
        fee: Coins,
        memo: Memo,
    ) -> Self {
        Self::signed_with(key, to.into(), amount, nonce, fee, None, Some(memo))
    }

    /// A stake from `key`'s address. `commitment` is required the first time, and refused
    /// while an earlier one is still in use.
    pub fn stake(
        key: &SigningKey,
        amount: Coins,
        nonce: u64,
        fee: Coins,
        commitment: Option<Commitment>,
    ) -> Self {
        Self::signed_with(key, Destination::Stake, amount, nonce, fee, commitment, None)
    }

    fn signed_with(
        key: &SigningKey,
        to: Destination,
        amount: Coins,
        nonce: u64,
        fee: Coins,
        commitment: Option<Commitment>,
        memo: Option<Memo>,
    ) -> Self {
        let from = Source::Account(Address::of(key));
        let (sender, recipient) = (from.to_string(), to.to_string());
        let commit = commitment.as_ref().map(Commitment::as_str);
        let note = memo.as_ref().map(Memo::as_str);
        let payload = Self::signing_payload(&sender, &recipient, amount, nonce, fee, commit, note);
        let signature = Some(Signature::sign(key, &payload));
        Self { from, to, amount, nonce, signature, fee, commitment, memo }
    }

    pub fn payload(&self) -> Vec<u8> {
        Self::signing_payload(
            &self.from.to_string(),
            &self.to.to_string(),
            self.amount,
            self.nonce,
            self.fee,
            self.commitment.as_ref().map(Commitment::as_str),
            self.memo.as_ref().map(Memo::as_str),
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
            signature: None,
            fee: Coins::ZERO,
            commitment: None,
            memo: None,
        }
    }
}
