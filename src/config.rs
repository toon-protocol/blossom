use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

const THIRTY_DAYS_SECS: u64 = 30 * 24 * 60 * 60;
const ONE_HOUR_SECS: u64 = 60 * 60;

/// Settings: each is an environment variable and a matching flag.
#[derive(Clone, Debug, Parser)]
#[command(version, about = "The TOON Protocol blob app")]
pub struct Config {
    /// Where the write port listens: reached only by the connector, never published.
    #[arg(long, env = "TOON_BLOB_WRITE_ADDR", default_value = "0.0.0.0:3001")]
    pub write_addr: SocketAddr,

    /// Where the public port listens: blob reads, free.
    #[arg(long, env = "TOON_BLOB_PUBLIC_ADDR", default_value = "0.0.0.0:3000")]
    pub public_addr: SocketAddr,

    /// The public URL blob descriptors use, with no trailing slash needed.
    #[arg(long, env = "TOON_BLOB_PUBLIC_URL")]
    pub public_url: String,

    /// The directory holding blob bytes and the holdings database.
    #[arg(long, env = "TOON_BLOB_STORAGE_DIR", default_value = "/data")]
    pub storage_dir: PathBuf,

    /// The retention period one paid upload adds to its holding, in seconds (default 30 days).
    #[arg(long, env = "TOON_BLOB_RETENTION_SECS", default_value_t = THIRTY_DAYS_SECS)]
    pub retention_secs: u64,

    /// How often the sweep ends expired holdings and deletes unheld blobs, in seconds (default 1 hour).
    #[arg(long, env = "TOON_BLOB_SWEEP_INTERVAL_SECS", default_value_t = ONE_HOUR_SECS,
          value_parser = clap::value_parser!(u64).range(1..))]
    pub sweep_interval_secs: u64,
}

impl Config {
    #[must_use]
    pub fn public_url(&self) -> &str {
        self.public_url.trim_end_matches('/')
    }
}
