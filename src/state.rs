use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::blob_files::BlobFiles;
use crate::config::Config;
use crate::hash::BlobHash;
use crate::holdings::Holdings;

/// Shared by both listeners.
pub struct AppState {
    pub config: Config,
    pub files: BlobFiles,
    pub holdings: Holdings,
    /// The hashes the operator has taken down: refused by the acceptance check.
    pub blocked: HashSet<BlobHash>,
    /// Held while bytes and holdings change together, so a sweep never deletes the bytes of an
    /// upload that is part-way recorded. A paid upload also runs its acceptance check under it, so
    /// two uploads at once cannot both fit under the storage cap and together exceed it.
    pub write_lock: Mutex<()>,
}

pub type SharedState = Arc<AppState>;

impl AppState {
    /// # Errors
    /// Fails if the storage directory or the holdings database cannot be opened.
    pub fn open(config: Config) -> Result<SharedState, Box<dyn std::error::Error>> {
        let blocked = config.blocked()?.into_iter().collect();
        std::fs::create_dir_all(&config.storage_dir)?;
        let files = BlobFiles::open(&config.storage_dir)?;
        let holdings = Holdings::open(&config.storage_dir.join("blossom.db"))?;
        Ok(Arc::new(Self {
            config,
            files,
            holdings,
            blocked,
            write_lock: Mutex::new(()),
        }))
    }
}

/// Unix seconds now.
#[must_use]
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
