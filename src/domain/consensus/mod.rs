//! How a network agrees on one chain: who may add the next block (`rules`), how hard proof of
//! work is (`difficulty`), what a chain weighs when forks compete (`weight`), and the randomness
//! that picks validators (`randao`, kept per validator by `validators`).

pub mod difficulty;
pub mod randao;
pub mod rules;
pub mod validators;
pub mod weight;
