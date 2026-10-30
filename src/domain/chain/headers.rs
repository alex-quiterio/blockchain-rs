//! Weighing a peer's chain from its headers alone, before fetching its blocks: whether they hash,
//! link and are sealed as the rules say, where they split from our chain, and, under proof of
//! work, whether they're heavier. Only blocks can show the rest: transfers, payouts, Merkle roots,
//! and under proof of stake, whose turn it was and what the stake behind each block weighs.

use std::cmp::Reverse;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::block::{Block, BlockError, BlockHash, BlockHeader};
use super::{Blockchain, ChainError};
use crate::domain::accounting::Coins;
use crate::domain::consensus::difficulty::{self, meets_work, Timeline};
use crate::domain::consensus::rules::Consensus;

/// What a peer's headers say about its chain, once they've been checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderVerdict {
    /// Their chain is worth fetching from this height on, where it leaves ours: heavier under
    /// proof of work, and under proof of stake, where only the blocks can tell, sound so far.
    FetchFrom(usize),
    /// Nothing worth fetching: we have every block they do, or their chain is no heavier.
    Ignore,
    /// They build on a block we don't have, so they should be asked from further back.
    MissingParent,
}

/// Our blocks up to where a peer's chain leaves ours, then the peer's headers: a run the
/// difficulty rules can read without either being copied.
struct Spliced<'a> {
    ours: &'a [Arc<Block>],
    theirs: &'a [BlockHeader],
}

impl Timeline for Spliced<'_> {
    fn count(&self) -> usize {
        self.ours.len() + self.theirs.len()
    }
    fn time(&self, height: usize) -> DateTime<Utc> {
        match height.checked_sub(self.ours.len()) {
            Some(i) => self.theirs[i].timestamp,
            None => self.ours[height].timestamp,
        }
    }
}

impl Blockchain {
    /// Checks a peer's headers from some height on, as far as headers can be checked, and says
    /// whether their blocks are worth fetching and from where. A header that breaks a rule is
    /// refused, naming its height, and so is a split deeper than any fork this node may take.
    pub fn weigh_headers(&self, headers: &[BlockHeader]) -> Result<HeaderVerdict, ChainError> {
        let mut headers = headers;
        if headers.first().ok_or(ChainError::Empty)?.index == 0 {
            if headers[0].hash != self.blocks[0].hash {
                return Err(ChainError::InvalidBlock { height: 0, reason: BlockError::BadGenesis });
            }
            headers = &headers[1..];
        }
        let Some(first) = headers.first() else {
            return Ok(HeaderVerdict::Ignore);
        };
        let start = first.index as usize;
        match start.checked_sub(1).and_then(|height| self.blocks.get(height)) {
            Some(parent) if parent.hash == first.prev_hash => {}
            _ => return Ok(HeaderVerdict::MissingParent),
        }
        let known = self.blocks[start..]
            .iter()
            .zip(headers)
            .take_while(|(ours, theirs)| ours.hash == theirs.hash)
            .count();
        let (split, theirs) = (start + known, &headers[known..]);
        if theirs.is_empty() {
            return Ok(HeaderVerdict::Ignore);
        }
        let depth = self.blocks.len() - split;
        if !self.reorg_limit.allows(depth) {
            return Err(ChainError::ReorgTooDeep { depth, max: self.reorg_limit.blocks() });
        }

        let base =
            self.tallies.after(split - 1).expect("a fork we may take starts at a kept tally");
        let (mut weight, mut work) = (base.weight, base.work);
        let ours = &self.blocks[..split];
        let now = Utc::now();
        for (i, header) in theirs.iter().enumerate() {
            let height = split + i;
            let prev_hash = match i {
                0 => &ours[split - 1].hash,
                _ => &theirs[i - 1].hash,
            };
            let before = Spliced { ours, theirs: &theirs[..i] };
            work = difficulty::next_work(&before, work, self.rules.retarget());
            self.check_header(header, height, prev_hash, &before, work, now)
                .map_err(|reason| ChainError::InvalidBlock { height: height as u64, reason })?;
            weight = weight.with_block(&self.rules, Coins::ZERO, work);
        }

        if self.rules.is_proof_of_stake() {
            return Ok(HeaderVerdict::FetchFrom(split));
        }
        let tip = &theirs.last().expect("checked not empty").hash;
        let heavier = (weight, Reverse(tip)) > (self.weight(), Reverse(&self.tip().hash));
        Ok(if heavier { HeaderVerdict::FetchFrom(split) } else { HeaderVerdict::Ignore })
    }

    /// Checks one header at `height`, following the one hashed `prev_hash`: its place, its hash,
    /// its time and its seal, which under proof of work must meet `work`.
    fn check_header(
        &self,
        header: &BlockHeader,
        height: usize,
        prev_hash: &BlockHash,
        before: &Spliced,
        work: u128,
        now: DateTime<Utc>,
    ) -> Result<(), BlockError> {
        if header.index != height as u64 || header.prev_hash != *prev_hash {
            return Err(BlockError::BrokenLink);
        }
        if header.hash != header.computed_hash() {
            return Err(BlockError::HashMismatch);
        }
        difficulty::check_timestamp(before, header.timestamp, now)?;
        match self.rules.consensus {
            Consensus::ProofOfWork { .. } => {
                if header.validator.is_some()
                    || header.signature.is_some()
                    || header.reveal.is_some()
                {
                    return Err(BlockError::WrongSeal);
                }
                meets_work(header.hash.as_str(), work)
                    .then_some(())
                    .ok_or(BlockError::InsufficientWork)
            }
            Consensus::ProofOfStake => {
                if header.nonce != 0 {
                    return Err(BlockError::WrongSeal);
                }
                header.is_signed_by_validator().then_some(()).ok_or(BlockError::BadSignature)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::chain::ReorgLimit;
    use crate::domain::consensus::rules::ChainRules;
    use crate::domain::crypto;
    use crate::domain::crypto::address::Address;

    const POW: ChainRules = ChainRules::proof_of_work(2, 50);

    fn someone() -> Address {
        Address::parse(&crypto::generate_keypair().0).unwrap()
    }

    /// A chain of `blocks` mined blocks after genesis.
    fn mined(rules: ChainRules, blocks: usize) -> Blockchain {
        let mut chain = Blockchain::new(rules);
        for _ in 0..blocks {
            chain.mine(&someone(), &[]).unwrap();
        }
        chain
    }

    fn headers(chain: &Blockchain, from: usize) -> Vec<BlockHeader> {
        chain.blocks()[from..].iter().map(|block| block.header()).collect()
    }

    #[test]
    fn more_blocks_on_our_tip_are_fetched_from_where_ours_end() {
        let ours = mined(POW, 2);
        let mut theirs = ours.clone();
        theirs.mine(&someone(), &[]).unwrap();
        theirs.mine(&someone(), &[]).unwrap();
        assert_eq!(ours.weigh_headers(&headers(&theirs, 1)), Ok(HeaderVerdict::FetchFrom(3)));
        assert_eq!(ours.weigh_headers(&headers(&theirs, 0)), Ok(HeaderVerdict::FetchFrom(3)));
    }

    #[test]
    fn headers_we_already_have_are_nothing_to_fetch() {
        let ours = mined(POW, 3);
        assert_eq!(ours.weigh_headers(&headers(&ours, 1)), Ok(HeaderVerdict::Ignore));
    }

    #[test]
    fn a_fork_is_fetched_from_where_it_splits_only_if_its_headers_are_heavier() {
        let ours = mined(POW, 3);
        let (lighter, heavier) = (mined(POW, 2), mined(POW, 5));
        assert_eq!(ours.weigh_headers(&headers(&lighter, 1)), Ok(HeaderVerdict::Ignore));
        assert_eq!(ours.weigh_headers(&headers(&heavier, 1)), Ok(HeaderVerdict::FetchFrom(1)));
    }

    #[test]
    fn headers_that_build_on_a_block_we_lack_ask_for_more() {
        let ours = mined(POW, 1);
        let theirs = mined(POW, 4);
        assert_eq!(ours.weigh_headers(&headers(&theirs, 3)), Ok(HeaderVerdict::MissingParent));
    }

    #[test]
    fn a_header_that_breaks_a_rule_is_refused_by_height() {
        let ours = mined(POW, 1);
        let theirs = mined(POW, 3);

        let mut tampered = headers(&theirs, 1);
        tampered[1].timestamp += chrono::TimeDelta::seconds(1);
        assert_eq!(
            ours.weigh_headers(&tampered),
            Err(ChainError::InvalidBlock { height: 2, reason: BlockError::HashMismatch })
        );

        // Rehashed to match, but without the work behind it.
        let mut lazy = headers(&theirs, 1);
        let header = &mut lazy[1];
        while {
            header.nonce += 1;
            header.hash = header.computed_hash();
            meets_work(header.hash.as_str(), POW.initial_work())
        } {}
        assert_eq!(
            ours.weigh_headers(&lazy),
            Err(ChainError::InvalidBlock { height: 2, reason: BlockError::InsufficientWork })
        );

        let mut unlinked = headers(&theirs, 1);
        unlinked.remove(1);
        assert_eq!(
            ours.weigh_headers(&unlinked),
            Err(ChainError::InvalidBlock { height: 2, reason: BlockError::BrokenLink })
        );
    }

    #[test]
    fn a_split_deeper_than_we_may_undo_is_refused() {
        let mut ours = Blockchain::new(POW).with_reorg_limit(ReorgLimit::new(1));
        for _ in 0..3 {
            ours.mine(&someone(), &[]).unwrap();
        }
        let theirs = mined(POW, 5);
        assert_eq!(
            ours.weigh_headers(&headers(&theirs, 1)),
            Err(ChainError::ReorgTooDeep { depth: 3, max: 1 })
        );
    }

    #[test]
    fn under_proof_of_stake_signed_headers_are_fetched_and_unsigned_ones_refused() {
        let pos = ChainRules::proof_of_stake(50);
        let ours = Blockchain::new(pos);
        let mut theirs = ours.clone();
        let key = crypto::parse_secret_key(&crypto::generate_keypair().1).unwrap();
        theirs.forge(&key, &[]).unwrap();
        // Only the blocks can say what stake is behind them, so sound headers are worth fetching.
        assert_eq!(ours.weigh_headers(&headers(&theirs, 1)), Ok(HeaderVerdict::FetchFrom(1)));

        let mut unsigned = headers(&theirs, 1);
        unsigned[0].signature = None;
        assert_eq!(
            ours.weigh_headers(&unsigned),
            Err(ChainError::InvalidBlock { height: 1, reason: BlockError::BadSignature })
        );
    }
}
