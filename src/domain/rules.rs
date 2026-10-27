/// The consensus rules. Every node on a network has to share them, or their chains won't validate
/// on each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainRules {
    /// How many leading hex zeros a block hash needs.
    pub difficulty: usize,
    /// Coins paid to whoever mines a block.
    pub reward: u64,
}

impl ChainRules {
    pub const fn new(difficulty: usize, reward: u64) -> Self {
        Self { difficulty, reward }
    }

    pub fn is_met_by(&self, hash: &str) -> bool {
        hash.len() >= self.difficulty && hash.bytes().take(self.difficulty).all(|b| b == b'0')
    }
}

impl Default for ChainRules {
    fn default() -> Self {
        Self::new(4, 50)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difficulty_counts_leading_zeros() {
        let rules = ChainRules::new(3, 50);
        assert!(rules.is_met_by("000abc"));
        assert!(rules.is_met_by("0000ab"));
        assert!(!rules.is_met_by("00abcd"));
        assert!(!rules.is_met_by("00"));
    }
}
