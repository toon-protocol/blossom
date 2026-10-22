//! Content-addressed blob bytes on disk.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::hash::BlobHash;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("written bytes do not hash to {0}")]
    HashMismatch(BlobHash),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Clone, Debug)]
pub struct BlobFiles {
    blobs: PathBuf,
    tmp: PathBuf,
}

impl BlobFiles {
    /// # Errors
    /// Fails if the directories cannot be created.
    pub fn open(root: &std::path::Path) -> io::Result<Self> {
        let files = Self {
            blobs: root.join("blobs"),
            tmp: root.join("tmp"),
        };
        fs::create_dir_all(&files.blobs)?;
        fs::create_dir_all(&files.tmp)?;
        Ok(files)
    }

    #[must_use]
    pub fn path(&self, hash: &BlobHash) -> PathBuf {
        self.blobs.join(&hash.as_str()[..2]).join(hash.as_str())
    }

    /// Writes `bytes` atomically: to a temporary file, synced and re-read to verify its hash, then
    /// renamed into place. A reader never sees a partial or mismatched blob.
    ///
    /// # Errors
    /// [`StorageError::HashMismatch`] if what reached the disk does not hash to `hash`.
    pub fn put(&self, hash: &BlobHash, bytes: &[u8]) -> Result<(), StorageError> {
        let mut file = tempfile::NamedTempFile::new_in(&self.tmp)?;
        file.write_all(bytes)?;
        file.as_file().sync_all()?;
        if BlobHash::of(&fs::read(file.path())?) != *hash {
            return Err(StorageError::HashMismatch(hash.clone()));
        }
        let dest = self.path(hash);
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir)?;
        }
        file.persist(&dest).map_err(|e| e.error)?;
        Ok(())
    }

    /// Deletes the blob's bytes; a blob already gone is not an error.
    ///
    /// # Errors
    /// Fails on an I/O error other than the file being absent.
    pub fn remove(&self, hash: &BlobHash) -> io::Result<()> {
        match fs::remove_file(self.path(hash)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}
