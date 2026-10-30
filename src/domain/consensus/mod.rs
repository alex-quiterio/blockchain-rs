//! How a network agrees on one chain: who may add the next block (`rules`), how hard proof of
//! work is (`difficulty`), what a chain weighs when forks compete (`weight`), and the randomness
//! that picks validators (`randao`, kept per validator by `validators`).
//!
//! A module grouped by concept, not by building block: its rules and weights are value objects,
//! difficulty and randao are domain services, and `validators` is a value object the ledger
//! carries. Each file says which it is.

pub mod difficulty;
pub mod randao;
pub mod rules;
pub mod validators;
pub mod weight;
