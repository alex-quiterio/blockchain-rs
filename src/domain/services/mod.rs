//! Domain services: what the domain does that belongs to no one object, with no state of their
//! own. Hashing and signatures (`crypto`), and Merkle trees and their proofs (`merkle`). The
//! services that decide on consensus live with its rules in `consensus/`.

pub mod crypto;
pub mod merkle;
