use std::fs;
use std::io;
use std::path::PathBuf;

use crate::domain::block::Block;
use crate::domain::repository::{ChainRepository, RepositoryError};

/// Keeps the chain as a JSON array of blocks in one file.
pub struct JsonFileRepository {
    path: PathBuf,
}

impl JsonFileRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl ChainRepository for JsonFileRepository {
    fn load(&self) -> Result<Option<Vec<Block>>, RepositoryError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    /// Writes to a temporary file and renames it over the real one, so a crash never
    /// leaves half a chain.
    fn save(&self, blocks: &[Block]) -> Result<(), RepositoryError> {
        let json = serde_json::to_vec_pretty(blocks)?;
        let mut tmp = self.path.as_os_str().to_owned();
        tmp.push(".tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::chain::Blockchain;
    use crate::domain::crypto;

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
        assert!(JsonFileRepository::new(&path.0).load().unwrap().is_none());
    }

    #[test]
    fn saved_blocks_load_back_identically() {
        let path = TempPath::new("round-trip");
        let repo = JsonFileRepository::new(&path.0);
        let mut chain = Blockchain::new(2, 50);
        chain.mine(&crypto::generate_keypair().0).unwrap();

        repo.save(chain.blocks()).unwrap();
        let loaded = repo.load().unwrap().unwrap();
        assert_eq!(loaded, chain.blocks());
        // Timestamps must survive to the nanosecond, or the block hashes stop matching.
        assert!(Blockchain::restore(loaded, 2, 50).is_ok());
    }

    #[test]
    fn garbage_file_is_an_error() {
        let path = TempPath::new("garbage");
        fs::write(&path.0, b"not json").unwrap();
        assert!(JsonFileRepository::new(&path.0).load().is_err());
    }
}
