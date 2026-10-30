use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::domain::chain::block::Block;
use crate::domain::chain::repository::{ChainRepository, ChainRepositoryError};
use crate::domain::chain::{ChainEvent, Snapshot};

/// Keeps the chain in one file, one block of JSON per line. A new block is appended, so saving
/// costs the same however long the chain gets; the file is only rewritten after a fork, or when
/// what's on disk isn't known. Files from older versions, one JSON array, still load, and are
/// rewritten line by line on the next save. Snapshots go in a file beside it, `<path>.snapshot`.
pub struct ChainJsonFileRepository {
    path: PathBuf,
    /// How many blocks the file holds, one per line, when that's known for sure.
    lines: Mutex<Option<usize>>,
}

impl ChainJsonFileRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), lines: Mutex::new(None) }
    }

    /// Writes every block to a temporary file and renames it over the real one, so a crash
    /// never leaves half a chain.
    fn rewrite(&self, blocks: &[Block]) -> Result<(), ChainRepositoryError> {
        write_atomically(&self.path, &lines_of(blocks)?)
    }

    fn snapshot_path(&self) -> PathBuf {
        let mut path = self.path.as_os_str().to_owned();
        path.push(".snapshot");
        path.into()
    }
}

impl ChainRepository for ChainJsonFileRepository {
    fn load(&self) -> Result<Option<Vec<Block>>, ChainRepositoryError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if text.trim_start().starts_with('[') {
            return Ok(Some(serde_json::from_str(&text)?));
        }
        let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
        let mut blocks = Vec::with_capacity(lines.len());
        for (i, line) in lines.iter().enumerate() {
            match serde_json::from_str(line) {
                Ok(block) => blocks.push(block),
                // A crash part way through appending leaves the last line cut short. That block
                // was never confirmed as saved, so it's dropped, and peers will send it again.
                Err(_) if i + 1 == lines.len() && i > 0 => break,
                Err(e) => return Err(e.into()),
            }
        }
        let torn = blocks.len() < lines.len();
        *self.lines.lock().unwrap() = (!torn).then_some(blocks.len());
        Ok(Some(blocks))
    }

    /// Appends the new blocks when the chain only grew from what the file holds; otherwise, after
    /// a fork or when what's on disk isn't known, rewrites the file.
    fn save(&self, blocks: &[Block], events: &[ChainEvent]) -> Result<(), ChainRepositoryError> {
        let mut lines = self.lines.lock().unwrap();
        let grew_from = match events {
            [ChainEvent::Extended { from, .. }, rest @ ..]
                if rest.iter().all(|e| matches!(e, ChainEvent::Extended { .. })) =>
            {
                Some(*from)
            }
            _ => None,
        };
        match (grew_from, *lines) {
            (Some(from), Some(saved)) if from == saved && saved <= blocks.len() => {
                let mut file = OpenOptions::new().append(true).open(&self.path)?;
                file.write_all(&lines_of(&blocks[saved..])?)?;
            }
            _ => self.rewrite(blocks)?,
        }
        *lines = Some(blocks.len());
        Ok(())
    }

    fn load_snapshot(&self) -> Result<Option<Snapshot>, ChainRepositoryError> {
        match fs::read(self.snapshot_path()) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn save_snapshot(&self, snapshot: &Snapshot) -> Result<(), ChainRepositoryError> {
        write_atomically(&self.snapshot_path(), &serde_json::to_vec(snapshot)?)
    }
}

fn write_atomically(path: &PathBuf, bytes: &[u8]) -> Result<(), ChainRepositoryError> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn lines_of(blocks: &[Block]) -> Result<Vec<u8>, ChainRepositoryError> {
    let mut out = Vec::new();
    for block in blocks {
        serde_json::to_writer(&mut out, block)?;
        out.push(b'\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::chain::{Blockchain, ReorgLimit};
    use crate::domain::consensus::rules::ChainRules;
    use crate::domain::crypto;
    use crate::domain::crypto::address::Address;

    /// A path in the system temp dir that is removed when dropped.
    struct TempPath(PathBuf);

    impl TempPath {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("blockchain-{}-{name}.json", std::process::id()));
            let _ = fs::remove_file(&path);
            Self(path)
        }
    }

    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn load_is_none_before_anything_is_saved() {
        let path = TempPath::new("missing");
        assert!(ChainJsonFileRepository::new(&path.0).load().unwrap().is_none());
    }

    #[test]
    fn saved_blocks_load_back_identically() {
        let path = TempPath::new("round-trip");
        let repo = ChainJsonFileRepository::new(&path.0);
        let rules = ChainRules::proof_of_work(2, 50);
        let mut chain = Blockchain::new(rules);
        chain.mine(&Address::parse(&crypto::generate_keypair().0).unwrap(), &[]).unwrap();

        repo.save(chain.blocks(), &[ChainEvent::Started]).unwrap();
        let loaded = repo.load().unwrap().unwrap();
        assert_eq!(loaded, chain.blocks());
        // Timestamps must survive to the nanosecond, or the block hashes stop matching.
        assert!(Blockchain::restore(loaded, rules).is_ok());
    }

    #[test]
    fn garbage_file_is_an_error() {
        let path = TempPath::new("garbage");
        fs::write(&path.0, b"not json").unwrap();
        assert!(ChainJsonFileRepository::new(&path.0).load().is_err());
    }

    fn chain_of(length: usize) -> Vec<Block> {
        let mut chain = Blockchain::new(ChainRules::proof_of_work(2, 50));
        for _ in 1..length {
            chain.mine(&Address::parse(&crypto::generate_keypair().0).unwrap(), &[]).unwrap();
        }
        chain.blocks().to_vec()
    }

    #[test]
    fn new_blocks_are_appended_and_a_fork_rewrites() {
        let path = TempPath::new("append");
        let repo = ChainJsonFileRepository::new(&path.0);
        let blocks = chain_of(4);
        repo.save(&blocks[..2], &[ChainEvent::Started]).unwrap();
        repo.save(&blocks[..3], &[ChainEvent::Extended { from: 2, to: 2 }]).unwrap();
        repo.save(&blocks, &[ChainEvent::Extended { from: 3, to: 3 }]).unwrap();
        let text = fs::read_to_string(&path.0).unwrap();
        assert_eq!(text.lines().count(), 4, "one block per line");
        assert_eq!(repo.load().unwrap().unwrap(), blocks);

        // A fork replaces the last two blocks.
        let mut forked = blocks[..2].to_vec();
        forked.extend(chain_of(4)[2..].iter().cloned());
        repo.save(
            &forked,
            &[ChainEvent::Reorganized { from: 2, dropped: 2, added: 2, returned: vec![] }],
        )
        .unwrap();
        assert_eq!(ChainJsonFileRepository::new(&path.0).load().unwrap().unwrap(), forked);
    }

    #[test]
    fn a_torn_last_line_is_dropped_and_the_next_save_repairs_it() {
        let path = TempPath::new("torn");
        let repo = ChainJsonFileRepository::new(&path.0);
        let blocks = chain_of(3);
        repo.save(&blocks[..2], &[ChainEvent::Started]).unwrap();
        let line = serde_json::to_string(&blocks[2]).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&path.0)
            .unwrap()
            .write_all(&line.as_bytes()[..40])
            .unwrap();

        let repo = ChainJsonFileRepository::new(&path.0);
        assert_eq!(repo.load().unwrap().unwrap(), blocks[..2]);
        repo.save(&blocks, &[ChainEvent::Extended { from: 2, to: 2 }]).unwrap();
        assert_eq!(ChainJsonFileRepository::new(&path.0).load().unwrap().unwrap(), blocks);
    }

    #[test]
    fn snapshots_are_kept_beside_the_chain() {
        let path = TempPath::new("snapshot");
        let snapshot_path = TempPath(PathBuf::from(format!("{}.snapshot", path.0.display())));
        let repo = ChainJsonFileRepository::new(&path.0);
        assert!(repo.load_snapshot().unwrap().is_none());

        let (rules, limit) = (ChainRules::proof_of_work(2, 50), ReorgLimit::new(1));
        let mut chain = Blockchain::new(rules).with_reorg_limit(limit);
        for _ in 0..3 {
            chain.mine(&Address::parse(&crypto::generate_keypair().0).unwrap(), &[]).unwrap();
        }
        let snapshot = chain.snapshot().unwrap();
        repo.save_snapshot(&snapshot).unwrap();
        assert!(snapshot_path.0.exists());
        let loaded = ChainJsonFileRepository::new(&path.0).load_snapshot().unwrap().unwrap();
        let (restored, from) =
            Blockchain::restore_from(chain.blocks().to_vec(), rules, limit, Some(loaded)).unwrap();
        assert_eq!((from, restored.weight()), (Some(snapshot.height), chain.weight()));
    }

    #[test]
    fn an_older_file_with_a_json_array_still_loads() {
        let path = TempPath::new("legacy");
        let blocks = chain_of(4);
        fs::write(&path.0, serde_json::to_vec_pretty(&blocks[..3]).unwrap()).unwrap();
        let repo = ChainJsonFileRepository::new(&path.0);
        assert_eq!(repo.load().unwrap().unwrap(), blocks[..3]);

        // What's on disk isn't in lines yet, so the next save rewrites it whole.
        repo.save(&blocks, &[ChainEvent::Extended { from: 3, to: 3 }]).unwrap();
        assert_eq!(fs::read_to_string(&path.0).unwrap().lines().count(), 4);
        assert_eq!(repo.load().unwrap().unwrap(), blocks);
    }
}
