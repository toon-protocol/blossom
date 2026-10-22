//! Black-box harness: starts the app and talks only to its two ports.
//!
//! By default it spawns the built binary on ephemeral ports. Set `BLOSSOM_TEST_WRITE_URL` and
//! `BLOSSOM_TEST_PUBLIC_URL` to run the same suite against a container that is already running.
//! The harness acts as the connector on the write port (it writes the payment statement itself)
//! and as a Blossom client on the public port.

#![allow(dead_code)]

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use k256::schnorr::SigningKey;
use k256::schnorr::signature::hazmat::PrehashSigner;
use reqwest::blocking::{Client, RequestBuilder, Response};
use sha2::{Digest, Sha256};

/// A storage cap large enough that no test reaches it.
pub const OPEN_CAP: [&str; 2] = ["--storage-cap-bytes", "1000000000"];

pub struct App {
    pub write_url: String,
    pub public_url: String,
    pub client: Client,
    child: Option<Child>,
    storage: Option<tempfile::TempDir>,
}

impl App {
    /// The app with a generous storage cap, so the paid path is open.
    /// Against an already-running container, that container (whose CI cap is as generous).
    pub fn start() -> Self {
        Self::running_container().unwrap_or_else(|| Self::spawn(3600, 3600, &OPEN_CAP))
    }

    /// An app with a retention period and sweep interval of a few seconds, for the expiry tests.
    /// `None` against an already-running container, whose settings the suite cannot choose.
    pub fn start_short(retention_secs: u32, sweep_secs: u32) -> Option<Self> {
        if Self::running_container().is_some() {
            eprintln!("skipped: the expiry tests need an app the suite starts itself");
            return None;
        }
        Some(Self::spawn(retention_secs, sweep_secs, &OPEN_CAP))
    }

    /// The app with exactly `settings` as its limits (and no storage cap unless they give one).
    /// `None` against an already-running container, whose settings the suite cannot choose.
    pub fn start_with(settings: &[&str]) -> Option<Self> {
        if Self::running_container().is_some() {
            eprintln!("skipped: the limit tests need an app the suite starts itself");
            return None;
        }
        Some(Self::spawn(3600, 3600, settings))
    }

    /// A `HEAD /upload` on the public port, with the headers a client would send.
    pub fn preflight(
        &self,
        length: usize,
        content_type: &str,
        hash: Option<&str>,
    ) -> RequestBuilder {
        let request = self
            .client
            .head(format!("{}/upload", self.public_url))
            .header("x-content-length", length.to_string())
            .header("x-content-type", content_type);
        match hash {
            Some(hash) => request.header("x-sha-256", hash),
            None => request,
        }
    }

    fn running_container() -> Option<Self> {
        let (Ok(write_url), Ok(public_url)) = (
            std::env::var("BLOSSOM_TEST_WRITE_URL"),
            std::env::var("BLOSSOM_TEST_PUBLIC_URL"),
        ) else {
            return None;
        };
        Some(Self {
            write_url,
            public_url,
            client: Client::new(),
            child: None,
            storage: None,
        })
    }

    /// Stops this app and starts another on the same storage with `settings`. `None` against an
    /// already-running container.
    pub fn restart_with(mut self, settings: &[&str]) -> Option<Self> {
        let storage = self.storage.take()?;
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        Some(Self::spawn_in(storage, 3600, 3600, settings))
    }

    fn spawn(retention_secs: u32, sweep_secs: u32, settings: &[&str]) -> Self {
        Self::spawn_in(
            tempfile::tempdir().unwrap(),
            retention_secs,
            sweep_secs,
            settings,
        )
    }

    /// The binary on ephemeral ports over `storage`, with `settings`, not yet started: for the
    /// tests that watch it start (or fail to).
    pub fn command(storage: &Path, settings: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_blossom"));
        command
            .args([
                "--write-addr",
                "127.0.0.1:0",
                "--public-addr",
                "127.0.0.1:0",
            ])
            .args(["--public-url", "http://blobs.test"])
            .args(settings)
            .arg("--storage-dir")
            .arg(storage);
        command
    }

    fn spawn_in(
        storage: tempfile::TempDir,
        retention_secs: u32,
        sweep_secs: u32,
        settings: &[&str],
    ) -> Self {
        let mut child = Self::command(storage.path(), settings)
            .args(["--retention-secs", &retention_secs.to_string()])
            .args(["--sweep-interval-secs", &sweep_secs.to_string()])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut address = |prefix: &str| {
            let line = lines.next().unwrap().unwrap();
            format!("http://{}", line.strip_prefix(prefix).unwrap())
        };
        let write_url = address("write port listening on ");
        let public_url = address("public port listening on ");
        Self {
            write_url,
            public_url,
            client: Client::new(),
            child: Some(child),
            storage: Some(storage),
        }
    }

    /// A `PUT /upload` on the write port with a correct payment statement and no authorization.
    pub fn upload(&self, body: &[u8]) -> RequestBuilder {
        self.client
            .put(format!("{}/upload", self.write_url))
            .header("x-toon-payer", "0xchannelkey")
            .header("x-toon-amount", "1010")
            .header("x-toon-chain", "evm:31337")
            .body(body.to_vec())
    }

    pub fn read(&self, path: &str) -> RequestBuilder {
        self.client.get(format!("{}/{path}", self.public_url))
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A Nostr identity that signs BUD-11 authorizations and NIP-56 reports.
pub struct Signer(SigningKey);

impl Signer {
    pub fn random() -> Self {
        Self(SigningKey::from_bytes(&rand::random::<[u8; 32]>()).unwrap())
    }

    pub fn pubkey(&self) -> String {
        hex::encode(self.0.verifying_key().to_bytes())
    }

    /// A signed kind 24242 event as an `Authorization` header value.
    pub fn authorization(
        &self,
        verb: &str,
        hash: &str,
        created_at: u64,
        expiration: u64,
    ) -> String {
        let tags = vec![
            vec!["t".to_owned(), verb.to_owned()],
            vec!["x".to_owned(), hash.to_owned()],
            vec!["expiration".to_owned(), expiration.to_string()],
        ];
        let pubkey = self.pubkey();
        let content = "Upload blob";
        let id = Sha256::digest(
            serde_json::to_string(&(0, &pubkey, created_at, 24242, &tags, content)).unwrap(),
        );
        let sig: k256::schnorr::Signature = self.0.sign_prehash(&id).unwrap();
        let event = serde_json::json!({
            "id": hex::encode(id), "pubkey": pubkey, "created_at": created_at, "kind": 24242,
            "tags": tags, "content": content, "sig": hex::encode(sig.to_bytes()),
        });
        format!("Nostr {}", STANDARD.encode(event.to_string()))
    }

    /// A valid upload authorization for `body`.
    pub fn upload_authorization(&self, body: &[u8]) -> String {
        self.authorization("upload", &sha256_hex(body), now() - 10, now() + 600)
    }

    /// A signed NIP-56 report event (kind `kind`) with the given tags, as a request body.
    pub fn report(&self, kind: u64, tags: &[Vec<&str>], tamper: bool) -> String {
        let tags: Vec<Vec<String>> = tags
            .iter()
            .map(|t| t.iter().map(|s| (*s).to_owned()).collect())
            .collect();
        let pubkey = self.pubkey();
        let created_at = now();
        let id = Sha256::digest(
            serde_json::to_string(&(0, &pubkey, created_at, kind, &tags, "illegal")).unwrap(),
        );
        let sig: k256::schnorr::Signature = self.0.sign_prehash(&id).unwrap();
        let content = if tamper { "changed" } else { "illegal" };
        serde_json::json!({
            "id": hex::encode(id), "pubkey": pubkey, "created_at": created_at, "kind": kind,
            "tags": tags, "content": content, "sig": hex::encode(sig.to_bytes()),
        })
        .to_string()
    }
}

/// Random bytes, so a rerun against a long-lived container never collides with an old blob.
pub fn random_blob(len: usize) -> Vec<u8> {
    (0..len).map(|_| rand::random::<u8>()).collect()
}

pub fn status(response: &Response) -> u16 {
    response.status().as_u16()
}
