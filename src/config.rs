use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

const THIRTY_DAYS_SECS: u64 = 30 * 24 * 60 * 60;

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
