use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

const THIRTY_DAYS_SECS: u64 = 30 * 24 * 60 * 60;
const ONE_HOUR_SECS: u64 = 60 * 60;

/// Just under the connector's 2 MiB body cap (about 1.9 MiB).
const DEFAULT_MAX_BLOB_BYTES: u64 = 1_992_294;

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

    /// The largest blob one paid upload may carry, in bytes. Over it is `413`.
    #[arg(long, env = "TOON_BLOB_MAX_BLOB_BYTES", default_value_t = DEFAULT_MAX_BLOB_BYTES)]
    pub max_blob_bytes: u64,

    /// The total bytes of kept blobs the operator allows. There is no default: until it is set,
    /// every upload is refused.
    #[arg(long, env = "TOON_BLOB_STORAGE_CAP_BYTES")]
    pub storage_cap_bytes: Option<u64>,

    /// Comma-separated content types uploads may carry (for example `image/png,image/jpeg`).
    /// When unset, any type is accepted.
    #[arg(long, env = "TOON_BLOB_ALLOWED_CONTENT_TYPES", value_delimiter = ',')]
    pub allowed_content_types: Option<Vec<String>>,
}

impl Config {
    #[must_use]
    pub fn public_url(&self) -> &str {
        self.public_url.trim_end_matches('/')
    }

    /// This server's lowercase domain name, from the public URL: no scheme, port or path.
    #[must_use]
    pub fn domain(&self) -> String {
        let url = self.public_url();
        let host = url.split_once("://").map_or(url, |(_, rest)| rest);
        let host = host.split('/').next().unwrap_or(host);
        let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
        let host = match host.rsplit_once(':') {
            Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
            _ => host,
        };
        host.to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn domain(public_url: &str) -> String {
        Config::parse_from(["blossom", "--public-url", public_url]).domain()
    }

    #[test]
    fn domain_drops_scheme_port_and_path() {
        assert_eq!(domain("https://Blobs.Example.com/"), "blobs.example.com");
        assert_eq!(domain("http://127.0.0.1:8080"), "127.0.0.1");
        assert_eq!(domain("blobs.test"), "blobs.test");
    }
}
