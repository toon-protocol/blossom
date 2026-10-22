//! BUD-09 abuse reports: a signed NIP-56 event, free, kept for the operator and acted on by no one.

use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;

use crate::auth::{Event, verify_signature};
use crate::error::{ApiError, read_body};
use crate::hash::BlobHash;
use crate::state::{SharedState, now};

const REPORT_KIND: u64 = 1984;
const MAX_REPORT_BYTES: usize = 64 * 1024;

/// A verified report: who made it, which blobs it names and why.
#[derive(Debug)]
pub struct Report {
    pub event_id: String,
    pub reporter: String,
    /// Each reported blob with the report type from its `x` tag (`""` when the tag has none).
    pub blobs: Vec<(BlobHash, String)>,
    pub content: String,
    /// The event exactly as received.
    pub raw: String,
}

impl Report {
    /// Parses and verifies a report event.
    ///
    /// # Errors
    /// [`ApiError::BadRequest`] when the body is not an event, is not kind 1984, names no blob
    /// by an `x` tag with a sha256 hash, or its id or signature does not verify.
    pub fn parse(body: &[u8]) -> Result<Self, ApiError> {
        let refuse = |why: &str| ApiError::BadRequest(format!("invalid report: {why}"));
        let event: Event = serde_json::from_slice(body).map_err(|_| refuse("not a Nostr event"))?;
        if event.kind != REPORT_KIND {
            return Err(refuse("kind must be 1984"));
        }
        let mut blobs: Vec<(BlobHash, String)> = Vec::new();
        for tag in &event.tags {
            if tag.first().map(String::as_str) != Some("x") {
                continue;
            }
            if let Some(hash) = tag.get(1).and_then(|h| BlobHash::parse(h))
                && !blobs.iter().any(|(seen, _)| *seen == hash)
            {
                blobs.push((hash, tag.get(2).cloned().unwrap_or_default()));
            }
        }
        if blobs.is_empty() {
            return Err(refuse("no x tag with a sha256 hash"));
        }
        verify_signature(&event).map_err(refuse)?;
        Ok(Self {
            event_id: event.id.to_ascii_lowercase(),
            reporter: event.pubkey.to_ascii_lowercase(),
            blobs,
            content: event.content,
            raw: String::from_utf8_lossy(body).into_owned(),
        })
    }
}

/// `PUT /report`: accepts a signed report with no payment and changes nothing but the report log.
///
/// # Errors
/// `400` for a report that does not verify, `413` for one over 64 KiB.
pub async fn put_report(
    State(state): State<SharedState>,
    body: Body,
) -> Result<StatusCode, ApiError> {
    let body = read_body(body, MAX_REPORT_BYTES).await?;
    let report = Report::parse(&body)?;
    tokio::task::spawn_blocking(move || state.holdings.keep_report(&report, now()))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::schnorr::SigningKey;
    use k256::schnorr::signature::hazmat::PrehashSigner;
    use sha2::{Digest, Sha256};

    const X: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    fn event(kind: u64, tags: &[&[&str]], tamper: bool) -> Vec<u8> {
        let key = SigningKey::from_bytes(&[9u8; 32]).unwrap();
        let pubkey = hex::encode(key.verifying_key().to_bytes());
        let tags: Vec<Vec<String>> = tags
            .iter()
            .map(|t| t.iter().map(|s| (*s).to_owned()).collect())
            .collect();
        let id =
            Sha256::digest(serde_json::to_string(&(0, &pubkey, 5, kind, &tags, "spam")).unwrap());
        let sig: k256::schnorr::Signature = key.sign_prehash(&id).unwrap();
        let content = if tamper { "other" } else { "spam" };
        serde_json::json!({
            "id": hex::encode(id), "pubkey": pubkey, "created_at": 5, "kind": kind,
            "tags": tags, "content": content, "sig": hex::encode(sig.to_bytes()),
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn accepts_a_signed_report_and_reads_its_blob_and_type() {
        let report = Report::parse(&event(1984, &[&["x", X, "spam"]], false)).unwrap();
        assert_eq!(report.blobs[0].0.as_str(), X);
        assert_eq!(report.blobs[0].1, "spam");
    }

    #[test]
    fn refuses_each_defect() {
        assert!(Report::parse(b"nope").is_err());
        assert!(Report::parse(&event(1984, &[&["x", X]], true)).is_err());
        assert!(Report::parse(&event(1, &[&["x", X]], false)).is_err());
        assert!(Report::parse(&event(1984, &[&["x", "abc"]], false)).is_err());
        assert!(Report::parse(&event(1984, &[], false)).is_err());
    }

    #[test]
    fn refuses_a_signature_by_another_key() {
        let mut report: serde_json::Value =
            serde_json::from_slice(&event(1984, &[&["x", X]], false)).unwrap();
        let id = hex::decode(report["id"].as_str().unwrap()).unwrap();
        let other = SigningKey::from_bytes(&[7u8; 32]).unwrap();
        let sig: k256::schnorr::Signature = other.sign_prehash(&id).unwrap();
        report["sig"] = hex::encode(sig.to_bytes()).into();
        assert!(Report::parse(report.to_string().as_bytes()).is_err());
    }
}
