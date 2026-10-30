/// The most of its own blocks a node will undo to switch forks. Anything deeper is treated as an
/// attack rewriting old history, however heavy the other fork is.
///
/// Each node's own choice, not a consensus rule: nodes with different limits still agree on
/// which blocks are valid and which fork is heaviest; they only differ on how far back they'll
/// follow one. So it isn't part of `ChainRules`, and changing it keeps a node's snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReorgLimit(u64);

impl ReorgLimit {
    pub const DEFAULT: Self = Self(64);

    pub const fn new(blocks: u64) -> Self {
        Self(blocks)
    }

    /// The most blocks that may be undone.
    pub const fn blocks(self) -> u64 {
        self.0
    }

    /// Whether undoing `depth` of our blocks is allowed.
    pub fn allows(self, depth: usize) -> bool {
        u64::try_from(depth).is_ok_and(|depth| depth <= self.0)
    }

    /// How many blocks' tallies to keep: enough to check any fork we may adopt from its split.
    pub(super) fn tallies_to_keep(self) -> usize {
        usize::try_from(self.0).map_or(usize::MAX, |blocks| blocks.saturating_add(1))
    }
}

impl Default for ReorgLimit {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_allows_undoing_up_to_its_blocks() {
        let two = ReorgLimit::new(2);
        assert!(two.allows(0) && two.allows(2));
        assert!(!two.allows(3));
        assert_eq!(two.tallies_to_keep(), 3, "the tally at the split, and one per block after");
        assert_eq!(ReorgLimit::new(u64::MAX).tallies_to_keep(), usize::MAX);
    }
}
