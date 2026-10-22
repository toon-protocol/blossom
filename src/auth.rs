//! BUD-11 authorization: a kind 24242 Nostr event in `Authorization: Nostr <base64>`.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use k256::schnorr::signature::hazmat::PrehashVerifier;
use k256::schnorr::{Signature, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::ApiError;
use crate::hash::BlobHash;

const AUTH_KIND: u64 = 24242;

#[derive(Debug, Deserialize)]
pub(crate) struct Event {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u64,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

/// A verified authorization: who signed it, and which blob hashes it covers.
#[derive(Debug)]
pub struct Authorization {
    /// The signer's pubkey (64 hex characters). This is the blob's owner.
    pub owner: String,
    hashes: Vec<BlobHash>,
}

impl Authorization {
    #[must_use]
    pub fn covers(&self, hash: &BlobHash) -> bool {
        self.hashes.contains(hash)
    }

    /// Verifies the `Authorization` header value for an upload at time `now` (unix seconds).
    /// `domain` is this server's lowercase domain name: an authorization that carries `server`
    /// tags must name it in one of them.
    ///
    /// # Errors
    /// As [`Authorization::verify_for`] with the verb `upload`.
    pub fn verify(header: Option<&str>, now: u64, domain: &str) -> Result<Self, ApiError> {
        Self::verify_for(header, now, domain, "upload")
    }

    /// Verifies the `Authorization` header value for the action `verb` at time `now`, on the
    /// server whose lowercase domain name is `domain`.
    ///
    /// # Errors
    /// [`ApiError::Unauthorized`] when the header is absent or malformed, the signature or id is
    /// wrong, the kind is not 24242, `created_at` is in the future, `expiration` is missing or
    /// past, `t` is not `verb`, there is no `x` tag holding a sha256 hash, or `server` tags are
    /// present and none names `domain`.
    pub fn verify_for(
        header: Option<&str>,
        now: u64,
        domain: &str,
        verb: &str,
    ) -> Result<Self, ApiError> {
        let denied = |why: &str| ApiError::Unauthorized(format!("invalid authorization: {why}"));
        let header =
            header.ok_or_else(|| ApiError::Unauthorized("authorization required".into()))?;
        let encoded = header
            .strip_prefix("Nostr ")
            .ok_or_else(|| denied("expected `Nostr <base64 event>`"))?
            .trim();
        let json = STANDARD
            .decode(encoded)
            .or_else(|_| URL_SAFE_NO_PAD.decode(encoded.trim_end_matches('=')))
            .map_err(|_| denied("not base64"))?;
        let event: Event = serde_json::from_slice(&json).map_err(|_| denied("not an event"))?;

        if event.kind != AUTH_KIND {
            return Err(denied("kind must be 24242"));
        }
        if event.created_at > now {
            return Err(denied("created_at is in the future"));
        }
        let tag = |name: &'static str| {
            event
                .tags
                .iter()
                .filter(move |t| t.first().map(String::as_str) == Some(name))
        };
        let expiration = tag("expiration")
            .find_map(|t| t.get(1)?.parse::<u64>().ok())
            .ok_or_else(|| denied("no expiration tag"))?;
        if expiration <= now {
            return Err(denied("expired"));
        }
        if !tag("t").any(|t| t.get(1).map(String::as_str) == Some(verb)) {
            return Err(denied(&format!("not authorized for {verb}")));
        }
        let mut servers = tag("server").filter_map(|t| t.get(1)).peekable();
        if servers.peek().is_some() && !servers.any(|s| s.eq_ignore_ascii_case(domain)) {
            return Err(denied("not scoped to this server"));
        }
        let hashes: Vec<BlobHash> = tag("x")
            .filter_map(|t| BlobHash::parse(t.get(1)?))
            .collect();
        if hashes.is_empty() {
            return Err(denied("no x tag with a sha256 hash"));
        }
        verify_signature(&event).map_err(denied)?;
        Ok(Self {
            owner: event.pubkey.to_ascii_lowercase(),
            hashes,
        })
    }
}

/// Checks that the event's id is the hash of its content and that its signature verifies.
pub(crate) fn verify_signature(event: &Event) -> Result<(), &'static str> {
    let serialized = serde_json::to_string(&(
        0,
        &event.pubkey,
        event.created_at,
        event.kind,
        &event.tags,
        &event.content,
    ))
    .map_err(|_| "unserializable event")?;
    let id = Sha256::digest(serialized.as_bytes());
    if hex::decode(&event.id).ok().as_deref() != Some(&id[..]) {
        return Err("event id does not match its content");
    }
    let key = hex::decode(&event.pubkey)
        .ok()
        .and_then(|b| VerifyingKey::from_bytes(&b).ok())
        .ok_or("bad pubkey")?;
    let sig = hex::decode(&event.sig)
        .ok()
        .and_then(|b| Signature::try_from(b.as_slice()).ok())
        .ok_or("bad signature encoding")?;
    key.verify_prehash(&id, &sig)
        .map_err(|_| "signature does not verify")
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::schnorr::SigningKey;
    use k256::schnorr::signature::hazmat::PrehashSigner;

    const NOW: u64 = 1_000_000;

    fn header(kind: u64, tags: &[[&str; 2]], created_at: u64, tamper: bool) -> String {
        let key = SigningKey::from_bytes(&[7u8; 32]).unwrap();
        let pubkey = hex::encode(key.verifying_key().to_bytes());
        let tags: Vec<Vec<String>> = tags
            .iter()
            .map(|t| t.iter().map(|s| (*s).to_owned()).collect())
            .collect();
        let content = "Upload";
        let id = Sha256::digest(
            serde_json::to_string(&(0, &pubkey, created_at, kind, &tags, content)).unwrap(),
        );
        let sig: Signature = key.sign_prehash(&id).unwrap();
        let content = if tamper { "Tampered" } else { content };
        let event = serde_json::json!({
            "id": hex::encode(id), "pubkey": pubkey, "created_at": created_at, "kind": kind,
            "tags": tags, "content": content, "sig": hex::encode(sig.to_bytes()),
        });
        format!("Nostr {}", STANDARD.encode(event.to_string()))
    }

    const X: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    fn good_tags() -> [[&'static str; 2]; 3] {
        [["t", "upload"], ["x", X], ["expiration", "2000000"]]
    }

    #[test]
    fn accepts_a_valid_authorization() {
        let h = header(AUTH_KIND, &good_tags(), NOW - 1, false);
        let auth = Authorization::verify(Some(&h), NOW, "blobs.test").unwrap();
        assert_eq!(auth.owner.len(), 64);
    }

    #[test]
    fn a_verb_authorization_is_only_good_for_that_verb() {
        let h = header(
            AUTH_KIND,
            &[["t", "delete"], ["x", X], ["expiration", "2000000"]],
            NOW - 1,
            false,
        );
        assert!(Authorization::verify_for(Some(&h), NOW, "blobs.test", "delete").is_ok());
        assert!(Authorization::verify_for(Some(&h), NOW, "blobs.test", "upload").is_err());
        assert!(Authorization::verify(Some(&h), NOW, "blobs.test").is_err());
    }

    #[test]
    fn server_tags_must_name_this_server() {
        let with = |servers: &[&'static str]| {
            let mut tags = good_tags().to_vec();
            tags.extend(servers.iter().map(|s| ["server", *s]));
            let h = header(AUTH_KIND, &tags, NOW - 1, false);
            Authorization::verify(Some(&h), NOW, "blobs.test").is_ok()
        };
        assert!(with(&[]), "no server tag means any server");
        assert!(with(&["other.test", "Blobs.Test"]));
        assert!(!with(&["other.test"]));
    }

    #[test]
    fn refuses_each_defect() {
        let v = |h: &str| Authorization::verify(Some(h), NOW, "blobs.test").is_err();
        assert!(Authorization::verify(None, NOW, "blobs.test").is_err());
        assert!(v("Bearer x"));
        assert!(v(&header(AUTH_KIND, &good_tags(), NOW - 1, true)));
        assert!(v(&header(1, &good_tags(), NOW - 1, false)));
        assert!(v(&header(AUTH_KIND, &good_tags(), NOW + 5, false)));
        assert!(v(&header(
            AUTH_KIND,
            &[["t", "delete"], ["x", X], ["expiration", "2000000"]],
            NOW - 1,
            false
        )));
        assert!(v(&header(
            AUTH_KIND,
            &[["t", "upload"], ["x", X], ["expiration", "999"]],
            NOW - 1,
            false
        )));
        assert!(v(&header(
            AUTH_KIND,
            &[["t", "upload"], ["x", X]],
            NOW - 1,
            false
        )));
        assert!(v(&header(
            AUTH_KIND,
            &[["t", "upload"], ["expiration", "2000000"]],
            NOW - 1,
            false
        )));
    }
}
