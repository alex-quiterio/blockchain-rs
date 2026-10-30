//! Hashing and signing, the concept every other one is built on. The functions here are a domain
//! service: making, reading, using and checking ed25519 keys, and hashing. Its value objects are
//! an `Address`, which is a public key, a `Signature` made with the private one, and each kind of
//! hash, which every concept declares for itself with `sha256_type!`. `ValueError` says when a
//! string is none of these.

pub mod address;
mod error;
mod hash;
pub mod signature;

pub use address::Address;
pub use error::ValueError;
pub(crate) use hash::sha256_type;
pub use signature::Signature;

use ed25519_dalek::{Signature as Ed25519Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Returns `(address, secret_key)`, both hex.
pub fn generate_keypair() -> (String, String) {
    let key = SigningKey::generate(&mut OsRng);
    (hex::encode(key.verifying_key().to_bytes()), hex::encode(key.to_bytes()))
}

pub fn parse_secret_key(secret_hex: &str) -> Option<SigningKey> {
    let bytes: [u8; 32] = hex::decode(secret_hex).ok()?.try_into().ok()?;
    Some(SigningKey::from_bytes(&bytes))
}

pub fn address_of(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

pub fn is_valid_address(address: &str) -> bool {
    parse_address(address).is_some()
}

fn parse_address(address: &str) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = hex::decode(address).ok()?.try_into().ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}

pub fn sign(key: &SigningKey, payload: &[u8]) -> String {
    hex::encode(key.sign(payload).to_bytes())
}

pub fn verify(address: &str, payload: &[u8], signature_hex: &str) -> bool {
    let Some(public) = parse_address(address) else {
        return false;
    };
    let Some(bytes) = hex::decode(signature_hex).ok().and_then(|b| <[u8; 64]>::try_from(b).ok())
    else {
        return false;
    };
    public.verify(payload, &Ed25519Signature::from_bytes(&bytes)).is_ok()
}
