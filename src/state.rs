use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::blob_files::BlobFiles;
use crate::config::Config;
use crate::holdings::Holdings;

/// Shared by both listeners.
pub struct AppState {
    pub config: Config,
    pub files: BlobFiles,
    pub holdings: Holdings,
}

pub type SharedState = Arc<AppState>;

impl AppState {
    /// # Errors
    /// Fails if the storage directory or the holdings database cannot be opened.
    pub fn open(config: Config) -> Result<SharedState, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(&config.storage_dir)?;
        let files = BlobFiles::open(&config.storage_dir)?;
        let holdings = Holdings::open(&config.storage_dir.join("blossom.db"))?;
        Ok(Arc::new(Self {
            config,
            files,
            holdings,
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
