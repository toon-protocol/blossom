//! The acceptance check every paid path runs, and the free preflight (`HEAD /upload`) runs too:
//! the takedown list, the blob size limit, the content-type allowlist and the total storage cap.

use axum::http::HeaderMap;

use crate::error::ApiError;
use crate::hash::BlobHash;
use crate::state::AppState;

/// What a client says it will upload. A preflight that states no hash is checked as a new blob.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub size: u64,
    pub content_type: String,
    pub hash: Option<BlobHash>,
}

/// # Errors
/// [`ApiError::Blocked`] for a hash the operator took down, [`ApiError::TooLarge`] over the size limit, [`ApiError::UnsupportedType`] outside the
/// allowlist, [`ApiError::StorageFull`] when the cap is unset or the blob's bytes would not fit.
pub fn check(state: &AppState, candidate: &Candidate, now: u64) -> Result<(), ApiError> {
    if candidate
        .hash
        .as_ref()
        .is_some_and(|h| state.blocked.contains(h))
    {
        return Err(ApiError::Blocked);
    }
    let config = &state.config;
    if candidate.size > config.max_blob_bytes {
        return Err(ApiError::TooLarge(config.max_blob_bytes));
    }
    // An empty setting (`TOON_BLOB_ALLOWED_CONTENT_TYPES=`) lists nothing, so it is no allowlist.
    let allowed: Vec<String> = config
        .allowed_content_types
        .iter()
        .flatten()
        .map(|a| self::essence(a))
        .filter(|a| !a.is_empty())
        .collect();
    if !allowed.is_empty() && !allowed.contains(&self::essence(&candidate.content_type)) {
        return Err(ApiError::UnsupportedType(candidate.content_type.clone()));
    }
    let cap = config.storage_cap_bytes.ok_or(ApiError::StorageFull)?;
    // Bytes already kept add nothing, so extending a holding is never refused for space.
    let already_kept = match &candidate.hash {
        Some(hash) => state
            .holdings
            .live_blob(hash, now)
            .map_err(ApiError::internal)?
            .is_some(),
        None => false,
    };
    if !already_kept {
        let kept = state.holdings.kept_bytes(now).map_err(ApiError::internal)?;
        if kept.saturating_add(candidate.size) > cap {
            return Err(ApiError::StorageFull);
        }
    }
    Ok(())
}

/// The media type without parameters, lowercased: `Text/Plain; charset=utf-8` is `text/plain`.
fn essence(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// The content type a client states in header `name`; `application/octet-stream` when it states
/// none. The preflight and the paid upload read it the same way, so they check the same type.
pub fn declared_type(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("application/octet-stream")
        .to_owned()
}

/// The hash a client states in `X-SHA-256`, if any. The preflight and the paid upload both refuse
/// a header that is not a sha256 hash.
///
/// # Errors
/// [`ApiError::BadRequest`] when the header is not a sha256 hash.
pub fn claimed_hash(headers: &HeaderMap) -> Result<Option<BlobHash>, ApiError> {
    headers
        .get("x-sha-256")
        .map(|v| {
            v.to_str()
                .ok()
                .and_then(|s| BlobHash::parse(s.trim()))
                .ok_or_else(|| ApiError::BadRequest("X-SHA-256 is not a sha256 hash".into()))
        })
        .transpose()
}
