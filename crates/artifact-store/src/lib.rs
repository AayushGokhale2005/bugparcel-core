use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigestRef(pub String);

pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn put(&self, bytes: &[u8]) -> io::Result<DigestRef> {
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        let hex = digest.trim_start_matches("sha256:");
        let path = self.root.join("objects/sha256").join(&hex[..2]).join(hex);
        if !path.exists() {
            fs::create_dir_all(path.parent().expect("object parent"))?;
            fs::write(path, bytes)?;
        }
        Ok(DigestRef(digest))
    }
    pub fn get(&self, digest: &DigestRef) -> io::Result<Vec<u8>> {
        let hex = digest
            .0
            .strip_prefix("sha256:")
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "expected sha256 digest"))?;
        fs::read(self.root.join("objects/sha256").join(&hex[..2]).join(hex))
    }
}

pub fn default_root(home: &Path) -> PathBuf {
    home.join(".bugparcel")
}
