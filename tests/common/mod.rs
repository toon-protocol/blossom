//! Black-box harness: starts the app and talks only to its two ports.
//!
//! By default it spawns the built binary on ephemeral ports. Set `BLOSSOM_TEST_WRITE_URL` and
//! `BLOSSOM_TEST_PUBLIC_URL` to run the same suite against a container that is already running.
//! The harness acts as the connector on the write port (it writes the payment statement itself)
//! and as a Blossom client on the public port.

#![allow(dead_code)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use k256::schnorr::SigningKey;
use k256::schnorr::signature::hazmat::PrehashSigner;
use reqwest::blocking::{Client, RequestBuilder, Response};
use sha2::{Digest, Sha256};

pub struct App {
    pub write_url: String,
    pub public_url: String,
    pub client: Client,
    child: Option<Child>,
    _storage: Option<tempfile::TempDir>,
}

impl App {
    pub fn start() -> Self {
        let client = Client::builder().build().unwrap();
        if let (Ok(write_url), Ok(public_url)) = (
            std::env::var("BLOSSOM_TEST_WRITE_URL"),
            std::env::var("BLOSSOM_TEST_PUBLIC_URL"),
        ) {
            return Self {
                write_url,
                public_url,
                client,
                child: None,
                _storage: None,
            };
        }
        let storage = tempfile::tempdir().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_blossom"))
            .args([
                "--write-addr",
                "127.0.0.1:0",
                "--public-addr",
                "127.0.0.1:0",
            ])
            .args(["--public-url", "http://blobs.test"])
            .args(["--retention-secs", "3600"])
            .arg("--storage-dir")
            .arg(storage.path())
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
            client,
            child: Some(child),
            _storage: Some(storage),
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

/// A Nostr identity that signs BUD-11 authorizations.
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
}

/// Random bytes, so a rerun against a long-lived container never collides with an old blob.
pub fn random_blob(len: usize) -> Vec<u8> {
    (0..len).map(|_| rand::random::<u8>()).collect()
}

pub fn status(response: &Response) -> u16 {
    response.status().as_u16()
}
